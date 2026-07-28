//! Deterministic trace and portable-export canonicalization.
//!
//! Store-backed construction is intentionally accepted only from a complete
//! purpose-authorized closure. Partial traversal state is private and has no
//! positive verification meaning.

use std::collections::{btree_map::Entry, BTreeMap, BTreeSet, VecDeque};
use std::str::FromStr;

use mfm_canonical::{
    CanonicalBytes, CanonicalValue, RecoverabilityContractV1, ValidatedCanonicalValueV1,
};
use mfm_ids::{
    ContentDigest, ContentRef, JournalCommitDigest, RunId, SchemaId, StoreScopeId, TenantScopeId,
};
use mfm_journal::v1::{JournalHead, RecordRef, RunPhase, TransitionRef, ValueRef};
use mfm_store::v1::{
    CommittedObject, Export, RunAccessAuthority, RunJournalStore, VerifiedRunView,
};

use crate::v1::{store_error, ReplayError, Result};

mod portable_verification;

/// Exact media type of canonical portable run exports.
pub const PORTABLE_RUN_EXPORT_MEDIA_TYPE: &str = "application/vnd.mfm.run-export.v1+json";

const PORTABLE_EXPORT_CONTRACT: &str = "mfm.portable-run-export.v1";
const PORTABLE_MANIFEST_CONTRACT: &str = "mfm.portable-export-manifest.v1";
const PORTABLE_MEMBER_CONTRACT: &str = "mfm.portable-export-member.v1";
const MAX_RUN_INDEX: usize = 4_096;
const MAX_RECORD_ORDINAL: u32 = 99_999;
const MAX_OBJECT_INDEX: usize = 9_999_999;
const SOURCE_STEP_RUNS: usize = 256;

/// Closed portable-export scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportKind {
    /// Export through the current semantic head, excluding a later audit tail.
    Semantic,
    /// Export through the exact current physical journal head.
    Audit,
}

impl ExportKind {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Audit => "audit",
        }
    }
}

/// Exact coordinate that fixes a portable export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportCoordinate {
    /// Open-run semantic head.
    SemanticHead {
        /// Last record that participates in current semantic state.
        record_ref: RecordRef,
        /// Digest of the commit containing that record.
        containing_commit_digest: JournalCommitDigest,
    },
    /// Closed-run immutable semantic closure.
    SemanticClosure {
        /// Terminal transition bound by `RunClosed`.
        terminal_transition_ref: TransitionRef,
        /// Digest of the commit containing transition and closure.
        containing_commit_digest: JournalCommitDigest,
    },
    /// Audit view fixed at one complete physical journal head.
    JournalHead {
        /// Exact physical head.
        journal_head: JournalHead,
    },
}

impl ExportCoordinate {
    fn canonical_value(&self) -> Result<CanonicalValue> {
        match self {
            Self::SemanticHead {
                record_ref,
                containing_commit_digest,
            } => export_object([
                ("kind", export_string("semantic_head")),
                ("record_ref", record_ref.canonical_value()?),
                (
                    "containing_commit_digest",
                    export_string(containing_commit_digest.as_str()),
                ),
            ]),
            Self::SemanticClosure {
                terminal_transition_ref,
                containing_commit_digest,
            } => export_object([
                ("kind", export_string("semantic_closure")),
                (
                    "terminal_transition_ref",
                    terminal_transition_ref.canonical_value()?,
                ),
                (
                    "containing_commit_digest",
                    export_string(containing_commit_digest.as_str()),
                ),
            ]),
            Self::JournalHead { journal_head } => export_object([
                ("kind", export_string("journal_head")),
                ("journal_head", journal_head.canonical_value()?),
            ]),
        }
    }

    fn matches_kind(&self, kind: ExportKind) -> bool {
        matches!(
            (self, kind),
            (
                Self::SemanticHead { .. } | Self::SemanticClosure { .. },
                ExportKind::Semantic
            ) | (Self::JournalHead { .. }, ExportKind::Audit)
        )
    }
}

/// Complete canonical portable bundle plus its sole external transport digest.
///
/// The digest is raw SHA-256 over [`Self::as_bytes`]. It is deliberately not a
/// field of those bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct PortableRunExport {
    canonical: ValidatedCanonicalValueV1,
    digest: ContentDigest,
}

/// Fact completeness available to a callback-free offline verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OfflineFactCompleteness {
    /// Portable bytes prove included integrity and provenance, never omission.
    UnverifiedPortableBundle,
}

/// Callback-free verified authority derived only from one complete portable bundle.
///
/// This value contains no store handle, run-access token, append capability, or
/// same-store fact-completeness claim.
pub struct OfflineVerifiedRun {
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    export_kind: ExportKind,
    coordinate: ExportCoordinate,
    schema_id: SchemaId,
    digest: ContentDigest,
    member_count: usize,
    verified_graph: portable_verification::VerifiedPortableGraph,
}

/// Exact caller-held semantic export bound to one affine verified-history session.
///
/// [`crate::v1::VerifiedHistoryResult::verify_portable_export`] validates the complete portable
/// bundle and its externally supplied [`ContentRef`], then moves the session's authoritative view
/// into this value. It also owns the independent callback-free offline-verified closure and private
/// retained-value indexes needed by reproduction; callers cannot turn those indexes into object
/// access. This value carries no store or export authority and is intentionally non-cloneable.
pub struct VerifiedPortableExport {
    content_ref: ContentRef,
    verified_view: VerifiedRunView,
    verified_graph: portable_verification::VerifiedPortableGraph,
    content_index: BTreeMap<(SchemaId, ContentDigest), PortableValueLocation>,
}

struct PortableValueLocation {
    run_index: usize,
    value_ref: ValueRef,
}

impl OfflineVerifiedRun {
    /// Returns the verified root run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the verified export scope.
    pub const fn export_kind(&self) -> ExportKind {
        self.export_kind
    }

    /// Returns the exact verified export coordinate.
    pub const fn coordinate(&self) -> &ExportCoordinate {
        &self.coordinate
    }

    /// Returns the externally supplied digest verified over exact bundle bytes.
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Returns the number of exact canonical members in the complete closure.
    pub const fn member_count(&self) -> usize {
        self.member_count
    }

    /// Returns the only fact-completeness claim portable verification may make.
    pub const fn fact_completeness(&self) -> OfflineFactCompleteness {
        OfflineFactCompleteness::UnverifiedPortableBundle
    }
}

impl VerifiedPortableExport {
    pub(crate) fn verify(
        bytes: &[u8],
        expected_ref: &ContentRef,
        verified_view: VerifiedRunView,
    ) -> Result<Self> {
        let verified = verify_portable_run_export_inner(bytes, expected_ref.content_digest())
            .map_err(|_| ReplayError::InvalidExport)?;
        if &verified.schema_id != expected_ref.schema_id()
            || &verified.store_scope_id != verified_view.store_identity().store_scope_id()
            || &verified.tenant_scope_id != verified_view.tenant_scope_id()
            || &verified.run_id != verified_view.run_id()
            || verified.export_kind != ExportKind::Semantic
            || !coordinate_matches_verified_view(&verified.coordinate, &verified_view)?
        {
            return Err(ReplayError::InvalidExport);
        }
        let content_index = portable_content_index(&verified.verified_graph)?;
        Ok(Self {
            content_ref: expected_ref.clone(),
            verified_view,
            verified_graph: verified.verified_graph,
            content_index,
        })
    }

    /// Returns the exact interpretation-and-byte identity admitted to a reproduction plan.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    pub(crate) const fn verified_view(&self) -> &VerifiedRunView {
        &self.verified_view
    }

    pub(crate) fn retained_content(&self, content_ref: &ContentRef) -> Result<&[u8]> {
        let location = self
            .content_index
            .get(&(
                content_ref.schema_id().clone(),
                content_ref.content_digest().clone(),
            ))
            .ok_or(ReplayError::InvalidExport)?;
        let run = self
            .verified_graph
            .runs
            .get(location.run_index)
            .ok_or(ReplayError::InvalidExport)?;
        run.view
            .retained_value(&location.value_ref)
            .map(CommittedObject::bytes)
            .map_err(|error| store_error(&error))
    }
}

impl std::fmt::Debug for OfflineVerifiedRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OfflineVerifiedRun")
            .field("store_scope_id", &self.store_scope_id)
            .field("tenant_scope_id", &self.tenant_scope_id)
            .field("run_id", &self.run_id)
            .field("export_kind", &self.export_kind)
            .field("coordinate", &self.coordinate)
            .field("schema_id", &self.schema_id)
            .field("digest", &self.digest)
            .field("member_count", &self.member_count)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for VerifiedPortableExport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedPortableExport")
            .field("content_ref", &self.content_ref)
            .field("run_id", self.verified_view.run_id())
            .field("semantic_head", self.verified_view.semantic_head())
            .finish_non_exhaustive()
    }
}

fn portable_content_index(
    graph: &portable_verification::VerifiedPortableGraph,
) -> Result<BTreeMap<(SchemaId, ContentDigest), PortableValueLocation>> {
    let mut content_index = BTreeMap::new();
    for (run_index, run) in graph.runs.iter().enumerate() {
        for object in run.view.journal().objects() {
            let fields = object.value_ref().fields()?;
            content_index
                .entry((fields.schema_id, fields.content_digest))
                .or_insert_with(|| PortableValueLocation {
                    run_index,
                    value_ref: object.value_ref().clone(),
                });
        }
    }
    Ok(content_index)
}

impl PortableRunExport {
    /// Returns the exact canonical bundle bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }

    /// Returns the raw-byte SHA-256 transport digest.
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Returns the exact portable-export media type.
    pub const fn media_type(&self) -> &'static str {
        PORTABLE_RUN_EXPORT_MEDIA_TYPE
    }

    /// Returns the annex-derived portable-export schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.canonical.schema_id()
    }

    /// Returns the lightweight interpretation-and-byte identity used by a frozen reproduction
    /// plan.
    ///
    /// This is not store authority and cannot substitute for the exact bundle bytes.
    pub fn content_ref(&self) -> Result<ContentRef> {
        ContentRef::new(self.schema_id().clone(), self.digest().clone())
            .map_err(|_| ReplayError::InvalidExport)
    }
}

impl std::fmt::Debug for PortableRunExport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PortableRunExport")
            .field("schema_id", self.schema_id())
            .field("digest", self.digest())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct CompleteExportClosure {
    pub(crate) store_scope_id: StoreScopeId,
    pub(crate) tenant_scope_id: TenantScopeId,
    pub(crate) root_run_id: RunId,
    pub(crate) kind: ExportKind,
    pub(crate) coordinate: ExportCoordinate,
    pub(crate) runs: Vec<CompleteExportRun>,
}

#[derive(Debug)]
pub(crate) struct CompleteExportRun {
    pub(crate) run_id: RunId,
    pub(crate) commits: Vec<CompleteCommit>,
    pub(crate) records: Vec<CompleteRecord>,
    pub(crate) objects: Vec<CompleteObject>,
}

#[derive(Debug)]
pub(crate) struct CompleteCommit {
    pub(crate) run_sequence: u64,
    pub(crate) schema_id: SchemaId,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct CompleteRecord {
    pub(crate) run_sequence: u64,
    pub(crate) ordinal: u32,
    pub(crate) schema_id: SchemaId,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct CompleteObject {
    pub(crate) value_ref: ValueRef,
    pub(crate) bytes: Vec<u8>,
}

/// Builds one complete deterministic portable export under exact purpose authority.
///
/// The root and every discovered source run are loaded independently through
/// their own `Export` grant. The returned bytes contain no access authority and
/// no digest of themselves.
pub async fn export_portable_run<S: RunJournalStore>(
    store: &S,
    root_authority: &RunAccessAuthority<Export>,
    dependency_authorities: &[RunAccessAuthority<Export>],
    kind: ExportKind,
) -> Result<PortableRunExport> {
    let root_run_id = root_authority.run_id().clone();
    let root_view = load_export_view(store, root_authority).await?;
    let store_identity = root_view.store_identity().clone();
    let tenant_scope_id = root_view.tenant_scope_id().clone();
    let (coordinate, root_cutoff) = root_export_coordinate(&root_view, kind)?;
    let root_material = complete_export_run(&root_view, root_cutoff)?;

    let mut authorities = BTreeMap::new();
    for authority in dependency_authorities {
        if authorities
            .insert(authority.run_id().clone(), authority)
            .is_some()
        {
            return Err(ReplayError::InvalidExport);
        }
    }

    let mut graph = BTreeMap::<RunId, BTreeSet<RunId>>::new();
    let mut pending = BTreeSet::new();
    append_source_requirements(&root_run_id, &root_view, &mut graph, &mut pending)?;
    let mut views = BTreeMap::new();
    views.insert(root_run_id.clone(), root_view);
    let mut materials = BTreeMap::new();
    materials.insert(root_run_id.clone(), root_material);

    while !pending.is_empty() {
        for _ in 0..SOURCE_STEP_RUNS {
            let Some(run_id) = pending.iter().next().cloned() else {
                break;
            };
            pending.remove(&run_id);
            if materials.contains_key(&run_id) {
                continue;
            }
            let authority = authorities
                .get(&run_id)
                .copied()
                .ok_or(ReplayError::SourceRunExportDenied)?;
            let view = load_export_dependency_view(store, authority).await?;
            if view.store_identity() != &store_identity
                || view.tenant_scope_id() != &tenant_scope_id
                || view.run_id() != &run_id
                || view.run_phase() != RunPhase::Closed
            {
                return Err(ReplayError::InvalidExport);
            }
            let cutoff = closed_semantic_cutoff(&view)?;
            let material = complete_export_run(&view, cutoff)?;
            append_source_requirements(&run_id, &view, &mut graph, &mut pending)?;
            views.insert(run_id.clone(), view);
            materials.insert(run_id, material);
        }
    }

    if !authorities
        .keys()
        .eq(materials.keys().filter(|run_id| *run_id != &root_run_id))
    {
        return Err(ReplayError::InvalidExport);
    }
    reject_source_cycles(&graph, materials.keys())?;
    verify_export_source_views(&views)?;

    let root = materials
        .remove(&root_run_id)
        .ok_or(ReplayError::InvalidExport)?;
    let mut runs = Vec::with_capacity(materials.len() + 1);
    runs.push(root);
    runs.extend(materials.into_values());
    CompleteExportClosure {
        store_scope_id: store_identity.store_scope_id().clone(),
        tenant_scope_id,
        root_run_id,
        kind,
        coordinate,
        runs,
    }
    .encode()
}

async fn load_export_view<S: RunJournalStore>(
    store: &S,
    authority: &RunAccessAuthority<Export>,
) -> Result<VerifiedRunView> {
    let journal = store
        .load_committed_journal(authority)
        .await
        .map_err(|error| store_error(&error))?;
    journal
        .verify_recorded_history()
        .map_err(|error| store_error(&error))
}

async fn load_export_dependency_view<S: RunJournalStore>(
    store: &S,
    authority: &RunAccessAuthority<Export>,
) -> Result<VerifiedRunView> {
    load_export_view(store, authority)
        .await
        .map_err(classify_export_dependency_load_error)
}

fn classify_export_dependency_load_error(error: ReplayError) -> ReplayError {
    match error {
        ReplayError::AuthorityMismatch => ReplayError::SourceRunExportDenied,
        _ => ReplayError::InvalidExport,
    }
}

fn root_export_coordinate(
    view: &VerifiedRunView,
    kind: ExportKind,
) -> Result<(ExportCoordinate, u64)> {
    match kind {
        ExportKind::Audit => {
            let journal_head = view.journal_head().clone();
            let cutoff = journal_head.fields()?.run_sequence;
            Ok((ExportCoordinate::JournalHead { journal_head }, cutoff))
        }
        ExportKind::Semantic => {
            if let Some(closure) = view.semantic_closure() {
                let fields = closure.fields()?;
                let cutoff = fields.terminal_transition_ref.fields()?.run_sequence;
                return Ok((
                    ExportCoordinate::SemanticClosure {
                        terminal_transition_ref: fields.terminal_transition_ref,
                        containing_commit_digest: fields.containing_commit_digest,
                    },
                    cutoff,
                ));
            }
            let record_ref = view.semantic_head_record_ref().clone();
            let record_fields = record_ref.fields()?;
            let head_fields = view.semantic_head().fields()?;
            if record_fields.run_id != *view.run_id()
                || record_fields.run_sequence != head_fields.run_sequence
            {
                return Err(ReplayError::InvalidExport);
            }
            Ok((
                ExportCoordinate::SemanticHead {
                    record_ref,
                    containing_commit_digest: head_fields.commit_digest,
                },
                head_fields.run_sequence,
            ))
        }
    }
}

fn closed_semantic_cutoff(view: &VerifiedRunView) -> Result<u64> {
    let closure = view
        .semantic_closure()
        .ok_or(ReplayError::InvalidExport)?
        .fields()?;
    closure
        .terminal_transition_ref
        .fields()
        .map(|fields| fields.run_sequence)
        .map_err(Into::into)
}

fn complete_export_run(view: &VerifiedRunView, cutoff: u64) -> Result<CompleteExportRun> {
    let mut commits = Vec::new();
    let mut records = Vec::new();
    let mut object_refs = BTreeMap::new();
    for commit in view.journal().commits() {
        let fields = commit.envelope().fields()?;
        if fields.core.run_sequence > cutoff {
            break;
        }
        commits.push(CompleteCommit {
            run_sequence: fields.core.run_sequence,
            schema_id: commit.envelope().schema_id().clone(),
            bytes: commit.envelope().as_bytes().to_vec(),
        });
        for record in commit.records() {
            let candidate_fields = record.candidate().fields()?;
            records.push(CompleteRecord {
                run_sequence: fields.core.run_sequence,
                ordinal: candidate_fields.ordinal,
                schema_id: record.candidate().schema_id().clone(),
                bytes: record.candidate().as_bytes().to_vec(),
            });
        }
        for intent in fields.core.artifact_admission_intents {
            let intent = intent.fields()?;
            object_refs.insert(intent.value_ref.as_bytes().to_vec(), intent.value_ref);
        }
    }
    if commits.last().map(|commit| commit.run_sequence) != Some(cutoff) {
        return Err(ReplayError::InvalidExport);
    }
    let objects = object_refs
        .into_values()
        .map(|value_ref| {
            let object = view
                .retained_value(&value_ref)
                .map_err(|error| store_error(&error))?;
            Ok(CompleteObject {
                value_ref: object.value_ref().clone(),
                bytes: object.bytes().to_vec(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CompleteExportRun {
        run_id: view.run_id().clone(),
        commits,
        records,
        objects,
    })
}

fn append_source_requirements(
    consuming_run_id: &RunId,
    view: &VerifiedRunView,
    graph: &mut BTreeMap<RunId, BTreeSet<RunId>>,
    pending: &mut BTreeSet<RunId>,
) -> Result<()> {
    let edges = graph.entry(consuming_run_id.clone()).or_default();
    for requirement in view.admission_source_requirements().cross_run_sources() {
        edges.insert(requirement.source_run_id().clone());
        pending.insert(requirement.source_run_id().clone());
    }
    let dependency_count = graph
        .values()
        .flat_map(BTreeSet::iter)
        .collect::<BTreeSet<_>>()
        .len();
    if dependency_count > MAX_RUN_INDEX {
        return Err(ReplayError::InvalidExport);
    }
    Ok(())
}

fn verify_export_source_views(views: &BTreeMap<RunId, VerifiedRunView>) -> Result<()> {
    for view in views.values() {
        for requirement in view.admission_source_requirements().cross_run_sources() {
            let source = views
                .get(requirement.source_run_id())
                .ok_or(ReplayError::InvalidExport)?;
            requirement
                .verify_closed_source_view(source)
                .map_err(|_| ReplayError::InvalidExport)?;
        }
    }
    Ok(())
}

fn reject_source_cycles<'a>(
    graph: &BTreeMap<RunId, BTreeSet<RunId>>,
    runs: impl Iterator<Item = &'a RunId>,
) -> Result<()> {
    let nodes = runs.cloned().collect::<BTreeSet<_>>();
    let mut incoming = nodes
        .iter()
        .cloned()
        .map(|run_id| (run_id, 0usize))
        .collect::<BTreeMap<_, _>>();
    for dependencies in graph.values() {
        for dependency in dependencies {
            let count = incoming
                .get_mut(dependency)
                .ok_or(ReplayError::InvalidExport)?;
            *count = count.checked_add(1).ok_or(ReplayError::InvalidExport)?;
        }
    }
    let mut ready = incoming
        .iter()
        .filter_map(|(run_id, count)| (*count == 0).then_some(run_id.clone()))
        .collect::<VecDeque<_>>();
    let mut visited = 0usize;
    while let Some(run_id) = ready.pop_front() {
        visited = visited.checked_add(1).ok_or(ReplayError::InvalidExport)?;
        for dependency in graph.get(&run_id).into_iter().flatten() {
            let count = incoming
                .get_mut(dependency)
                .ok_or(ReplayError::InvalidExport)?;
            *count = count.checked_sub(1).ok_or(ReplayError::InvalidExport)?;
            if *count == 0 {
                ready.push_back(dependency.clone());
            }
        }
    }
    if visited != nodes.len() {
        return Err(ReplayError::InvalidExport);
    }
    Ok(())
}

impl CompleteExportClosure {
    pub(crate) fn encode(self) -> Result<PortableRunExport> {
        if !self.coordinate.matches_kind(self.kind)
            || self.runs.is_empty()
            || self.runs.len() > MAX_RUN_INDEX + 1
        {
            return Err(ReplayError::InvalidExport);
        }

        let mut root = None;
        let mut dependencies = Vec::new();
        for run in self.runs {
            if run.run_id == self.root_run_id {
                if root.replace(run).is_some() {
                    return Err(ReplayError::InvalidExport);
                }
            } else {
                dependencies.push(run);
            }
        }
        let root = root.ok_or(ReplayError::InvalidExport)?;
        dependencies.sort_by(|left, right| left.run_id.as_str().cmp(right.run_id.as_str()));
        if dependencies
            .windows(2)
            .any(|pair| pair[0].run_id == pair[1].run_id)
        {
            return Err(ReplayError::InvalidExport);
        }

        let dependency_run_ids = dependencies
            .iter()
            .map(|run| export_string(run.run_id.as_str()))
            .collect::<Vec<_>>();
        let mut members = BTreeMap::new();
        let mut transferred_objects = BTreeMap::new();
        append_run_members(&mut members, &mut transferred_objects, 0, root)?;
        for (index, run) in dependencies.into_iter().enumerate() {
            append_run_members(&mut members, &mut transferred_objects, index + 1, run)?;
        }
        if members.is_empty() {
            return Err(ReplayError::InvalidExport);
        }

        let contract = RecoverabilityContractV1::embedded()?;
        let closure_members = members
            .values()
            .map(|member| contract.raw_content_digest(member.bytes()))
            .collect::<BTreeSet<_>>();
        let closure_members = closure_members
            .iter()
            .map(|digest| export_string(digest.as_str()))
            .collect::<Vec<_>>();

        let manifest = export_object([
            ("version", export_string("mfm.portable-export-manifest.v1")),
            (
                "store_scope_id",
                export_string(self.store_scope_id.as_str()),
            ),
            (
                "tenant_scope_id",
                export_string(self.tenant_scope_id.as_str()),
            ),
            ("run_id", export_string(self.root_run_id.as_str())),
            ("export_kind", export_string(self.kind.as_str())),
            ("coordinate", self.coordinate.canonical_value()?),
            (
                "dependency_run_ids",
                CanonicalValue::Array(dependency_run_ids),
            ),
            ("closure_members", CanonicalValue::Array(closure_members)),
        ])?;
        let manifest = contract.encode(PORTABLE_MANIFEST_CONTRACT, &manifest)?;

        let members = members
            .into_values()
            .map(|member| {
                let value = member.canonical_value(contract)?;
                contract
                    .encode(PORTABLE_MEMBER_CONTRACT, &value)?
                    .canonical_value()
                    .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        let bundle = export_object([
            ("version", export_string("mfm.portable-run-export.v1")),
            ("media_type", export_string(PORTABLE_RUN_EXPORT_MEDIA_TYPE)),
            ("manifest", manifest.canonical_value()?),
            ("members", CanonicalValue::Array(members)),
        ])?;
        let canonical = contract.encode(PORTABLE_EXPORT_CONTRACT, &bundle)?;
        let digest = contract.raw_content_digest(canonical.as_bytes());
        Ok(PortableRunExport { canonical, digest })
    }
}

/// Verifies one portable bundle from caller-held bytes and its external digest.
///
/// Verification is callback-free and performs no store, object-service,
/// provider, executor, filesystem-domain, or signer access.
pub fn verify_portable_run_export(
    bytes: &[u8],
    expected_digest: &ContentDigest,
) -> Result<OfflineVerifiedRun> {
    verify_portable_run_export_inner(bytes, expected_digest).map_err(|_| ReplayError::InvalidExport)
}

fn verify_portable_run_export_inner(
    bytes: &[u8],
    expected_digest: &ContentDigest,
) -> Result<OfflineVerifiedRun> {
    let contract = RecoverabilityContractV1::embedded()?;
    if &contract.raw_content_digest(bytes) != expected_digest {
        return Err(ReplayError::InvalidExport);
    }
    let bundle = contract.strict_decode(PORTABLE_EXPORT_CONTRACT, bytes)?;
    let bundle_value = bundle.canonical_value()?;
    let manifest_value = required_export_field(&bundle_value, "manifest")?.clone();
    let _manifest = contract.encode(PORTABLE_MANIFEST_CONTRACT, &manifest_value)?;
    let root_run_id = RunId::from_str(export_string_field(&manifest_value, "run_id")?)
        .map_err(|_| ReplayError::InvalidExport)?;
    let store_scope_id =
        StoreScopeId::from_str(export_string_field(&manifest_value, "store_scope_id")?)
            .map_err(|_| ReplayError::InvalidExport)?;
    let tenant_scope_id =
        TenantScopeId::from_str(export_string_field(&manifest_value, "tenant_scope_id")?)
            .map_err(|_| ReplayError::InvalidExport)?;
    let export_kind = match export_string_field(&manifest_value, "export_kind")? {
        "semantic" => ExportKind::Semantic,
        "audit" => ExportKind::Audit,
        _ => return Err(ReplayError::InvalidExport),
    };
    let coordinate = parse_coordinate(
        required_export_field(&manifest_value, "coordinate")?,
        export_kind,
        &root_run_id,
    )?;
    let dependency_run_ids = export_array_field(&manifest_value, "dependency_run_ids")?
        .iter()
        .map(|value| match value {
            CanonicalValue::String(value) => {
                RunId::from_str(value).map_err(|_| ReplayError::InvalidExport)
            }
            _ => Err(ReplayError::InvalidExport),
        })
        .collect::<Result<Vec<_>>>()?;
    if dependency_run_ids
        .windows(2)
        .any(|pair| pair[0].as_str() >= pair[1].as_str())
        || dependency_run_ids
            .iter()
            .any(|run_id| run_id == &root_run_id)
    {
        return Err(ReplayError::InvalidExport);
    }

    let manifest_closure = export_array_field(&manifest_value, "closure_members")?
        .iter()
        .map(|value| match value {
            CanonicalValue::String(value) => {
                ContentDigest::from_str(value).map_err(|_| ReplayError::InvalidExport)
            }
            _ => Err(ReplayError::InvalidExport),
        })
        .collect::<Result<Vec<_>>>()?;
    if manifest_closure.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ReplayError::InvalidExport);
    }

    let member_values = export_array_field(&bundle_value, "members")?;
    let mut parsed_members = Vec::with_capacity(member_values.len());
    let mut prior_path = None;
    let mut derived_closure = BTreeSet::new();
    for value in member_values {
        contract.encode(PORTABLE_MEMBER_CONTRACT, value)?;
        let member = ParsedMember::parse(value)?;
        if prior_path
            .as_deref()
            .is_some_and(|prior| prior >= member.path.as_str())
        {
            return Err(ReplayError::InvalidExport);
        }
        prior_path = Some(member.path.clone());
        let digest = contract.raw_content_digest(&member.bytes);
        if digest != member.content_digest {
            return Err(ReplayError::InvalidExport);
        }
        derived_closure.insert(digest);
        parsed_members.push(member);
    }
    if manifest_closure != derived_closure.into_iter().collect::<Vec<ContentDigest>>() {
        return Err(ReplayError::InvalidExport);
    }

    let mut expected_runs = vec![root_run_id.clone()];
    expected_runs.extend(dependency_run_ids);
    let verified_graph = portable_verification::verify_member_graph(
        &parsed_members,
        &expected_runs,
        &store_scope_id,
        &tenant_scope_id,
        &coordinate,
    )?;

    Ok(OfflineVerifiedRun {
        store_scope_id,
        tenant_scope_id,
        run_id: root_run_id,
        export_kind,
        coordinate,
        schema_id: bundle.schema_id().clone(),
        digest: expected_digest.clone(),
        member_count: parsed_members.len(),
        verified_graph,
    })
}

fn coordinate_matches_verified_view(
    coordinate: &ExportCoordinate,
    view: &VerifiedRunView,
) -> Result<bool> {
    Ok(match coordinate {
        ExportCoordinate::SemanticHead {
            record_ref,
            containing_commit_digest,
        } => {
            view.semantic_closure().is_none()
                && record_ref.as_bytes() == view.semantic_head_record_ref().as_bytes()
                && &view.semantic_head().fields()?.commit_digest == containing_commit_digest
        }
        ExportCoordinate::SemanticClosure {
            terminal_transition_ref,
            containing_commit_digest,
        } => view.semantic_closure().is_some_and(|closure| {
            closure.fields().is_ok_and(|fields| {
                terminal_transition_ref.as_bytes() == fields.terminal_transition_ref.as_bytes()
                    && containing_commit_digest == &fields.containing_commit_digest
            })
        }),
        ExportCoordinate::JournalHead { .. } => false,
    })
}

#[derive(Debug)]
enum ExportMember {
    Journal {
        path: String,
        schema_id: SchemaId,
        bytes: Vec<u8>,
    },
    Object {
        path: String,
        schema_id: SchemaId,
        bytes: Vec<u8>,
        value_refs: Vec<ValueRef>,
    },
}

impl ExportMember {
    fn path(&self) -> &str {
        match self {
            Self::Journal { path, .. } | Self::Object { path, .. } => path,
        }
    }

    fn bytes(&self) -> &[u8] {
        match self {
            Self::Journal { bytes, .. } | Self::Object { bytes, .. } => bytes,
        }
    }

    fn canonical_value(&self, contract: &RecoverabilityContractV1) -> Result<CanonicalValue> {
        let (kind, path, schema_id, bytes, value_refs) = match self {
            Self::Journal {
                path,
                schema_id,
                bytes,
            } => ("journal", path, schema_id, bytes, None),
            Self::Object {
                path,
                schema_id,
                bytes,
                value_refs,
            } => ("object", path, schema_id, bytes, Some(value_refs)),
        };
        let digest = contract.raw_content_digest(bytes);
        let mut entries = vec![
            ("kind", export_string(kind)),
            ("path", export_string(path)),
            ("schema_id", export_string(schema_id.as_str())),
            ("content_digest", export_string(digest.as_str())),
            (
                "bytes",
                CanonicalValue::Bytes(CanonicalBytes::new(bytes.clone())),
            ),
        ];
        if let Some(value_refs) = value_refs {
            if value_refs.is_empty()
                || value_refs
                    .windows(2)
                    .any(|pair| pair[0].as_bytes() >= pair[1].as_bytes())
            {
                return Err(ReplayError::InvalidExport);
            }
            entries.push((
                "value_refs",
                CanonicalValue::Array(
                    value_refs
                        .iter()
                        .map(ValueRef::canonical_value)
                        .collect::<std::result::Result<Vec<_>, _>>()?,
                ),
            ));
        }
        export_object(entries)
    }
}

#[derive(Debug)]
struct ParsedMember {
    path: String,
    parsed_path: ParsedMemberPath,
    schema_id: SchemaId,
    content_digest: ContentDigest,
    bytes: Vec<u8>,
    value_refs: Option<Vec<ValueRef>>,
}

impl ParsedMember {
    fn parse(value: &CanonicalValue) -> Result<Self> {
        let kind = export_string_field(value, "kind")?;
        let path = export_string_field(value, "path")?.to_owned();
        let parsed_path = ParsedMemberPath::parse(&path)?;
        match (kind, parsed_path) {
            ("journal", ParsedMemberPath::Commit { .. } | ParsedMemberPath::Record { .. })
            | ("object", ParsedMemberPath::Object { .. }) => {}
            _ => return Err(ReplayError::InvalidExport),
        }
        let schema_id = SchemaId::from_str(export_string_field(value, "schema_id")?)
            .map_err(|_| ReplayError::InvalidExport)?;
        let content_digest = ContentDigest::from_str(export_string_field(value, "content_digest")?)
            .map_err(|_| ReplayError::InvalidExport)?;
        let bytes = match required_export_field(value, "bytes")? {
            CanonicalValue::String(value) => CanonicalBytes::from_base64url_no_pad(value.clone())
                .map_err(|_| ReplayError::InvalidExport)?
                .as_bytes()
                .to_vec(),
            _ => return Err(ReplayError::InvalidExport),
        };
        let value_refs = if kind == "object" {
            Some(
                export_array_field(value, "value_refs")?
                    .iter()
                    .cloned()
                    .map(ValueRef::from_canonical_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            )
        } else {
            None
        };
        Ok(Self {
            path,
            parsed_path,
            schema_id,
            content_digest,
            bytes,
            value_refs,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedMemberPath {
    Commit {
        run_index: usize,
        run_sequence: u64,
    },
    Record {
        run_index: usize,
        run_sequence: u64,
        ordinal: u32,
    },
    Object {
        run_index: usize,
        object_index: usize,
    },
}

impl ParsedMemberPath {
    fn parse(path: &str) -> Result<Self> {
        let parts = path.split('.').collect::<Vec<_>>();
        if parts.first().copied() != Some("runs") {
            return Err(ReplayError::InvalidExport);
        }
        match parts.as_slice() {
            ["runs", run, "commits", commit] => Ok(Self::Commit {
                run_index: fixed_decimal_usize(run, 'r', 4)?,
                run_sequence: fixed_decimal_u64(commit, 'c', 20)?,
            }),
            ["runs", run, "records", commit, ordinal] => Ok(Self::Record {
                run_index: fixed_decimal_usize(run, 'r', 4)?,
                run_sequence: fixed_decimal_u64(commit, 'c', 20)?,
                ordinal: fixed_decimal_u32(ordinal, 'o', 5)?,
            }),
            ["runs", run, "objects", object] => Ok(Self::Object {
                run_index: fixed_decimal_usize(run, 'r', 4)?,
                object_index: fixed_decimal_usize(object, 'o', 7)?,
            }),
            _ => Err(ReplayError::InvalidExport),
        }
    }
}

fn fixed_decimal_digits(value: &str, prefix: char, digits: usize) -> Result<&str> {
    let mut characters = value.chars();
    if characters.next() != Some(prefix) {
        return Err(ReplayError::InvalidExport);
    }
    let decimal = characters.as_str();
    if decimal.len() != digits || !decimal.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ReplayError::InvalidExport);
    }
    Ok(decimal)
}

fn fixed_decimal_usize(value: &str, prefix: char, digits: usize) -> Result<usize> {
    fixed_decimal_digits(value, prefix, digits)?
        .parse()
        .map_err(|_| ReplayError::InvalidExport)
}

fn fixed_decimal_u64(value: &str, prefix: char, digits: usize) -> Result<u64> {
    fixed_decimal_digits(value, prefix, digits)?
        .parse()
        .map_err(|_| ReplayError::InvalidExport)
}

fn fixed_decimal_u32(value: &str, prefix: char, digits: usize) -> Result<u32> {
    fixed_decimal_digits(value, prefix, digits)?
        .parse()
        .map_err(|_| ReplayError::InvalidExport)
}

fn parse_coordinate(
    value: &CanonicalValue,
    kind: ExportKind,
    run_id: &RunId,
) -> Result<ExportCoordinate> {
    let coordinate = match (kind, export_string_field(value, "kind")?) {
        (ExportKind::Semantic, "semantic_head") => {
            let record_ref = RecordRef::from_canonical_value(
                required_export_field(value, "record_ref")?.clone(),
            )?;
            if &record_ref.fields()?.run_id != run_id || export_object_len(value)? != 3 {
                return Err(ReplayError::InvalidExport);
            }
            ExportCoordinate::SemanticHead {
                record_ref,
                containing_commit_digest: JournalCommitDigest::from_str(export_string_field(
                    value,
                    "containing_commit_digest",
                )?)
                .map_err(|_| ReplayError::InvalidExport)?,
            }
        }
        (ExportKind::Semantic, "semantic_closure") => {
            let terminal_transition_ref = TransitionRef::from_canonical_value(
                required_export_field(value, "terminal_transition_ref")?.clone(),
            )?;
            if &terminal_transition_ref.fields()?.run_id != run_id || export_object_len(value)? != 3
            {
                return Err(ReplayError::InvalidExport);
            }
            ExportCoordinate::SemanticClosure {
                terminal_transition_ref,
                containing_commit_digest: JournalCommitDigest::from_str(export_string_field(
                    value,
                    "containing_commit_digest",
                )?)
                .map_err(|_| ReplayError::InvalidExport)?,
            }
        }
        (ExportKind::Audit, "journal_head") => {
            if export_object_len(value)? != 2 {
                return Err(ReplayError::InvalidExport);
            }
            ExportCoordinate::JournalHead {
                journal_head: JournalHead::from_canonical_value(
                    required_export_field(value, "journal_head")?.clone(),
                )?,
            }
        }
        _ => return Err(ReplayError::InvalidExport),
    };
    Ok(coordinate)
}

fn required_export_field<'a>(value: &'a CanonicalValue, field: &str) -> Result<&'a CanonicalValue> {
    let CanonicalValue::Object(object) = value else {
        return Err(ReplayError::InvalidExport);
    };
    object
        .entries()
        .find_map(|(key, value)| (key == field).then_some(value))
        .ok_or(ReplayError::InvalidExport)
}

fn export_string_field<'a>(value: &'a CanonicalValue, field: &str) -> Result<&'a str> {
    match required_export_field(value, field)? {
        CanonicalValue::String(value) => Ok(value),
        _ => Err(ReplayError::InvalidExport),
    }
}

fn export_array_field<'a>(value: &'a CanonicalValue, field: &str) -> Result<&'a [CanonicalValue]> {
    match required_export_field(value, field)? {
        CanonicalValue::Array(values) => Ok(values),
        _ => Err(ReplayError::InvalidExport),
    }
}

fn export_object_len(value: &CanonicalValue) -> Result<usize> {
    match value {
        CanonicalValue::Object(object) => Ok(object.entries().count()),
        _ => Err(ReplayError::InvalidExport),
    }
}

fn append_run_members(
    members: &mut BTreeMap<String, ExportMember>,
    transferred_objects: &mut BTreeMap<(SchemaId, ContentDigest), String>,
    run_index: usize,
    mut run: CompleteExportRun,
) -> Result<()> {
    if run_index > MAX_RUN_INDEX {
        return Err(ReplayError::InvalidExport);
    }
    run.commits.sort_by_key(|commit| commit.run_sequence);
    if run.commits.first().map(|commit| commit.run_sequence) != Some(1)
        || run
            .commits
            .windows(2)
            .any(|pair| pair[0].run_sequence.checked_add(1) != Some(pair[1].run_sequence))
    {
        return Err(ReplayError::InvalidExport);
    }
    let commit_sequences = run
        .commits
        .iter()
        .map(|commit| commit.run_sequence)
        .collect::<BTreeSet<_>>();
    for commit in run.commits {
        let path = format!("runs.r{run_index:04}.commits.c{:020}", commit.run_sequence);
        insert_member(
            members,
            ExportMember::Journal {
                path,
                schema_id: commit.schema_id,
                bytes: commit.bytes,
            },
        )?;
    }

    run.records
        .sort_by_key(|record| (record.run_sequence, record.ordinal));
    let mut prior = None;
    let mut next_ordinal = 0;
    let mut ordinal_sequence = None;
    for record in run.records {
        if record.ordinal > MAX_RECORD_ORDINAL
            || !commit_sequences.contains(&record.run_sequence)
            || prior.is_some_and(|prior| prior >= (record.run_sequence, record.ordinal))
        {
            return Err(ReplayError::InvalidExport);
        }
        if ordinal_sequence != Some(record.run_sequence) {
            ordinal_sequence = Some(record.run_sequence);
            next_ordinal = 0;
        }
        if record.ordinal != next_ordinal {
            return Err(ReplayError::InvalidExport);
        }
        next_ordinal = next_ordinal
            .checked_add(1)
            .ok_or(ReplayError::InvalidExport)?;
        prior = Some((record.run_sequence, record.ordinal));
        let path = format!(
            "runs.r{run_index:04}.records.c{:020}.o{:05}",
            record.run_sequence, record.ordinal
        );
        insert_member(
            members,
            ExportMember::Journal {
                path,
                schema_id: record.schema_id,
                bytes: record.bytes,
            },
        )?;
    }

    run.objects
        .sort_by(|left, right| left.value_ref.as_bytes().cmp(right.value_ref.as_bytes()));
    let mut prior_value_ref = None;
    let mut next_object_index = 0usize;
    for object in run.objects {
        if prior_value_ref
            .as_deref()
            .is_some_and(|prior| prior == object.value_ref.as_bytes())
        {
            continue;
        }
        prior_value_ref = Some(object.value_ref.as_bytes().to_vec());
        let fields = object.value_ref.fields()?;
        let expected_length =
            u64::try_from(object.bytes.len()).map_err(|_| ReplayError::InvalidExport)?;
        let digest = RecoverabilityContractV1::embedded()?.raw_content_digest(&object.bytes);
        if fields.byte_length != expected_length || fields.content_digest != digest {
            return Err(ReplayError::InvalidExport);
        }
        let identity = (fields.schema_id, fields.content_digest);
        match transferred_objects.entry(identity) {
            Entry::Vacant(entry) => {
                if next_object_index > MAX_OBJECT_INDEX {
                    return Err(ReplayError::InvalidExport);
                }
                let path = format!("runs.r{run_index:04}.objects.o{next_object_index:07}");
                next_object_index = next_object_index
                    .checked_add(1)
                    .ok_or(ReplayError::InvalidExport)?;
                insert_member(
                    members,
                    ExportMember::Object {
                        path: path.clone(),
                        schema_id: object.value_ref.fields()?.schema_id,
                        bytes: object.bytes,
                        value_refs: vec![object.value_ref],
                    },
                )?;
                entry.insert(path);
            }
            Entry::Occupied(entry) => {
                let member = members
                    .get_mut(entry.get())
                    .ok_or(ReplayError::InvalidExport)?;
                let ExportMember::Object {
                    bytes, value_refs, ..
                } = member
                else {
                    return Err(ReplayError::InvalidExport);
                };
                if bytes != &object.bytes {
                    return Err(ReplayError::InvalidExport);
                }
                match value_refs.binary_search_by(|value_ref| {
                    value_ref.as_bytes().cmp(object.value_ref.as_bytes())
                }) {
                    Ok(_) => {}
                    Err(index) => value_refs.insert(index, object.value_ref),
                }
            }
        }
    }
    Ok(())
}

fn insert_member(members: &mut BTreeMap<String, ExportMember>, member: ExportMember) -> Result<()> {
    match members.entry(member.path().to_owned()) {
        Entry::Vacant(entry) => {
            entry.insert(member);
            Ok(())
        }
        Entry::Occupied(_) => Err(ReplayError::InvalidExport),
    }
}

fn export_string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn export_object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| ReplayError::InvalidExport)
}

#[cfg(test)]
mod tests {
    use mfm_canonical::{CanonicalBytes, CanonicalValue, RecoverabilityContractV1};

    use super::{
        classify_export_dependency_load_error, export_object, export_string, ParsedMember,
        ParsedMemberPath, ReplayError, PORTABLE_MEMBER_CONTRACT,
    };

    #[test]
    fn authorized_missing_dependency_is_an_export_integrity_failure() {
        assert!(matches!(
            classify_export_dependency_load_error(ReplayError::RunNotFound),
            ReplayError::InvalidExport
        ));
        assert!(matches!(
            classify_export_dependency_load_error(ReplayError::InvalidRecordedHistory),
            ReplayError::InvalidExport
        ));
        assert!(matches!(
            classify_export_dependency_load_error(ReplayError::AuthorityMismatch),
            ReplayError::SourceRunExportDenied
        ));
    }

    #[test]
    fn portable_member_paths_have_one_exact_coordinate_spelling() {
        assert_eq!(
            ParsedMemberPath::parse("runs.r0000.commits.c00000000000000000001")
                .expect("commit path"),
            ParsedMemberPath::Commit {
                run_index: 0,
                run_sequence: 1,
            }
        );
        assert_eq!(
            ParsedMemberPath::parse("runs.r0000.records.c00000000000000000001.o00000")
                .expect("record path"),
            ParsedMemberPath::Record {
                run_index: 0,
                run_sequence: 1,
                ordinal: 0,
            }
        );
        assert_eq!(
            ParsedMemberPath::parse("runs.r0000.objects.o0000000").expect("object path"),
            ParsedMemberPath::Object {
                run_index: 0,
                object_index: 0,
            }
        );

        for alias in [
            "runs.r0.commits.c00000000000000000001",
            "runs.r0000.commits.c1",
            "runs.r0000.records.c00000000000000000001.o0",
            "runs.r0000.objects.o0",
            "runs.r00000.objects.o0000000",
            "runs.r0000.objects.o00000000",
            "runs.r+000.objects.o0000000",
            "runs.r0000.objects.o0000000.extra",
        ] {
            assert!(
                ParsedMemberPath::parse(alias).is_err(),
                "accepted alias {alias}"
            );
        }
    }

    #[test]
    fn portable_member_tag_fixes_path_class_and_object_authorities() {
        let contract = RecoverabilityContractV1::embedded().expect("recoverability contract");
        let bytes = b"portable-member".to_vec();
        let digest = contract.raw_content_digest(&bytes);
        let schema_id = contract
            .schema_id("mfm.input-manifest.v1")
            .expect("member schema");
        let wrong_path_class = export_object([
            ("kind", export_string("journal")),
            ("path", export_string("runs.r0000.objects.o0000000")),
            ("schema_id", export_string(schema_id.as_str())),
            ("content_digest", export_string(digest.as_str())),
            (
                "bytes",
                CanonicalValue::Bytes(CanonicalBytes::new(bytes.clone())),
            ),
        ])
        .expect("journal member");
        let validated = contract
            .encode(PORTABLE_MEMBER_CONTRACT, &wrong_path_class)
            .expect("shape-valid member");
        assert!(
            ParsedMember::parse(&validated.canonical_value().expect("member value")).is_err(),
            "the envelope parser must enforce the path-class invariant"
        );

        let empty_authorities = export_object([
            ("kind", export_string("object")),
            ("path", export_string("runs.r0000.objects.o0000000")),
            ("schema_id", export_string(schema_id.as_str())),
            ("content_digest", export_string(digest.as_str())),
            ("bytes", CanonicalValue::Bytes(CanonicalBytes::new(bytes))),
            ("value_refs", CanonicalValue::Array(Vec::new())),
        ])
        .expect("object member");
        assert!(
            contract
                .encode(PORTABLE_MEMBER_CONTRACT, &empty_authorities)
                .is_err(),
            "an object member must retain a nonempty full authority set"
        );
    }
}
