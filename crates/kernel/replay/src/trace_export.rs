//! Deterministic, bounded-step portable run export streams.
//!
//! Store-backed construction begins writing only after the complete authorized
//! source closure has been loaded and verified. Offline verification consumes
//! one JSON text sequence without store, callback, capability, filesystem,
//! provider, executor, or signer access.

use std::collections::{btree_map::Entry, BTreeMap, BTreeSet, VecDeque};
use std::str::FromStr;
use std::sync::Arc;

use mfm_canonical::{
    CanonicalBytes, CanonicalValue, RawContentDigestHasher, RecoverabilityContractV2,
    ValidatedCanonicalValueV2,
};
use mfm_ids::{
    ContentDigest, ContentRef, JournalCommitDigest, RecordId, RunId, SchemaId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::v1::{
    CandidateRecordEnvelope, CommitEnvelope, JournalHead, JournalPredecessorFields,
    RecordHashPreimage, RecordIdPreimage, RecordRef, RunPhase, TransitionRef, ValueRef,
};
use mfm_store::v1::{
    verify_offline_recorded_material, CommittedJournalCommit, CommittedJournalRecord,
    CommittedObject, Export, RunAccessAuthority, RunHistoryReader, RunJournalBackend,
    StoreIdentity, UntrustedObjectPayload, VerifiedRunView,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::v1::{store_error, ReplayError, Result};

/// Exact media type of portable run export streams.
pub const PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE: &str =
    "application/vnd.mfm.run-export-stream.v1+json-seq";
/// Maximum canonical JSON byte length of one stream frame.
pub const MAX_STREAM_FRAME_JSON_BYTES: usize = 16_777_216;
/// Maximum decoded byte length carried by one chunk frame.
pub const MAX_STREAM_CHUNK_BYTES: usize = 65_536;
/// Maximum stream bytes processed before cooperatively yielding.
pub const CLOSURE_STEP_STREAM_BYTES: usize = 262_144;
/// Maximum source runs processed before cooperatively yielding.
pub const CLOSURE_STEP_SOURCES: usize = 256;
/// Maximum retained objects processed before cooperatively yielding.
pub const CLOSURE_STEP_OBJECTS: usize = 512;

const PORTABLE_STREAM_CONTRACT: &str = "mfm.portable-run-export-stream.v1";
const PORTABLE_FRAME_CONTRACT: &str = "mfm.portable-run-export-frame.v1";
const PORTABLE_FRAME_VERSION: &str = "mfm.portable-run-export-frame.v1";
const RECORD_SEPARATOR: u8 = 0x1e;
const RECORD_SUFFIX: u8 = 0x0a;

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

/// Final identity metadata for one fully written portable export stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableRunExportMetadata {
    schema_id: SchemaId,
    content_digest: ContentDigest,
    content_ref: ContentRef,
}

impl PortableRunExportMetadata {
    /// Returns the annex-derived stream schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns SHA-256 over every exact framing and canonical frame byte.
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    /// Returns the exact portable export stream media type.
    pub const fn media_type(&self) -> &'static str {
        PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE
    }

    /// Returns the interpretation-and-byte identity of the complete stream.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
}

/// Fact completeness available to a callback-free offline verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OfflineFactCompleteness {
    /// Included stream bytes prove integrity and provenance, never tenant-wide omission.
    UnverifiedPortableStream,
}

/// Callback-free verified authority derived from one complete portable stream.
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
    verified_graph: VerifiedExportGraph,
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

    /// Returns the externally supplied digest verified over exact stream bytes.
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Returns the only fact-completeness claim portable verification may make.
    pub const fn fact_completeness(&self) -> OfflineFactCompleteness {
        OfflineFactCompleteness::UnverifiedPortableStream
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
            .finish_non_exhaustive()
    }
}

/// Exact caller-held semantic stream bound to one affine verified-history session.
///
/// This value owns the callback-free closure views and a private retained
/// payload-location index used by reproduction. It grants no store, object
/// service, or export authority and is intentionally non-cloneable.
pub struct VerifiedExportStream {
    content_ref: ContentRef,
    verified_view: VerifiedRunView,
    verified_graph: VerifiedExportGraph,
    content_index: BTreeMap<(SchemaId, ContentDigest), ExportValueLocation>,
}

struct ExportValueLocation {
    run_index: usize,
    value_ref: ValueRef,
}

impl VerifiedExportStream {
    pub(crate) fn bind(
        verified: OfflineVerifiedRun,
        expected_ref: ContentRef,
        verified_view: VerifiedRunView,
    ) -> Result<Self> {
        if verified.schema_id != *expected_ref.schema_id()
            || verified.digest != *expected_ref.content_digest()
            || verified.store_scope_id != *verified_view.store_identity().store_scope_id()
            || verified.tenant_scope_id != *verified_view.tenant_scope_id()
            || verified.run_id != *verified_view.run_id()
            || verified.export_kind != ExportKind::Semantic
            || !coordinate_matches_verified_view(&verified.coordinate, &verified_view)?
        {
            return Err(ReplayError::InvalidExport);
        }
        let content_index = export_content_index(&verified.verified_graph)?;
        Ok(Self {
            content_ref: expected_ref,
            verified_view,
            verified_graph: verified.verified_graph,
            content_index,
        })
    }

    /// Returns the exact interpretation-and-byte identity admitted to reproduction.
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

impl std::fmt::Debug for VerifiedExportStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedExportStream")
            .field("content_ref", &self.content_ref)
            .field("run_id", self.verified_view.run_id())
            .field("semantic_head", self.verified_view.semantic_head())
            .finish_non_exhaustive()
    }
}

fn export_content_index(
    graph: &VerifiedExportGraph,
) -> Result<BTreeMap<(SchemaId, ContentDigest), ExportValueLocation>> {
    let mut content_index = BTreeMap::new();
    for (run_index, run) in graph.runs.iter().enumerate() {
        for object in run.view.journal().objects() {
            let fields = object.value_ref().fields()?;
            content_index
                .entry((fields.schema_id, fields.content_digest))
                .or_insert_with(|| ExportValueLocation {
                    run_index,
                    value_ref: object.value_ref().clone(),
                });
        }
    }
    Ok(content_index)
}

struct PreparedExportClosure {
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    root_run_id: RunId,
    kind: ExportKind,
    coordinate: ExportCoordinate,
    runs: Vec<PreparedExportRun>,
    payloads: BTreeMap<(SchemaId, ContentDigest), PreparedExportPayload>,
}

struct PreparedExportRun {
    view: VerifiedRunView,
    cutoff: u64,
}

struct PreparedExportPayload {
    source_run_index: usize,
    source_value_ref: ValueRef,
    value_refs: BTreeMap<Vec<u8>, ValueRef>,
}

/// Writes one complete deterministic portable run export stream.
///
/// The root and every discovered source run are loaded through their own
/// `Export` grant before the first output byte is written. The writer is
/// flushed and shut down after the terminal frame.
pub async fn write_portable_run_export_stream<B, W>(
    reader: &RunHistoryReader<B>,
    root_authority: &RunAccessAuthority<Export>,
    dependency_authorities: &[RunAccessAuthority<Export>],
    kind: ExportKind,
    writer: &mut W,
) -> Result<PortableRunExportMetadata>
where
    B: RunJournalBackend,
    W: AsyncWrite + Unpin,
{
    let closure =
        prepare_export_closure(reader, root_authority, dependency_authorities, kind).await?;
    closure.write_to(writer).await
}

async fn prepare_export_closure<B: RunJournalBackend>(
    reader: &RunHistoryReader<B>,
    root_authority: &RunAccessAuthority<Export>,
    dependency_authorities: &[RunAccessAuthority<Export>],
    kind: ExportKind,
) -> Result<PreparedExportClosure> {
    let mut state = SourceLoadStep::Pending(
        AuthorizedSourceSession::begin(reader, root_authority, dependency_authorities, kind)
            .await?,
    );
    let closure = loop {
        match state {
            SourceLoadStep::Pending(session) => {
                state = session.step().await?;
                if matches!(&state, SourceLoadStep::Pending(_)) {
                    tokio::task::yield_now().await;
                }
            }
            SourceLoadStep::Complete(closure) => break closure,
        }
    };

    let mut views = closure.views;
    let root = views
        .remove(&closure.root_run_id)
        .ok_or(ReplayError::InvalidExport)?;
    let mut runs = Vec::with_capacity(views.len() + 1);
    runs.push(PreparedExportRun {
        view: root.0,
        cutoff: root.1,
    });
    runs.extend(
        views
            .into_values()
            .map(|(view, cutoff)| PreparedExportRun { view, cutoff }),
    );
    let payloads = collect_export_payloads(&runs).await?;
    Ok(PreparedExportClosure {
        store_scope_id: closure.store_identity.store_scope_id().clone(),
        tenant_scope_id: closure.tenant_scope_id,
        root_run_id: closure.root_run_id,
        kind: closure.kind,
        coordinate: closure.coordinate,
        runs,
        payloads,
    })
}

enum SourceLoadStep<T, C> {
    Pending(T),
    Complete(C),
}

struct AuthorizedSourceSession<'authority, B> {
    reader: &'authority RunHistoryReader<B>,
    authorities: BTreeMap<RunId, &'authority RunAccessAuthority<Export>>,
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    root_run_id: RunId,
    kind: ExportKind,
    coordinate: ExportCoordinate,
    graph: BTreeMap<RunId, BTreeSet<RunId>>,
    pending: BTreeSet<RunId>,
    views: BTreeMap<RunId, (VerifiedRunView, u64)>,
}

struct VerifiedSourceClosure {
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    root_run_id: RunId,
    kind: ExportKind,
    coordinate: ExportCoordinate,
    views: BTreeMap<RunId, (VerifiedRunView, u64)>,
}

impl<'authority, B: RunJournalBackend> AuthorizedSourceSession<'authority, B> {
    async fn begin(
        reader: &'authority RunHistoryReader<B>,
        root_authority: &'authority RunAccessAuthority<Export>,
        dependency_authorities: &'authority [RunAccessAuthority<Export>],
        kind: ExportKind,
    ) -> Result<Self> {
        let root_run_id = root_authority.run_id().clone();
        let root_view = load_export_view(reader, root_authority).await?;
        let store_identity = root_view.store_identity().clone();
        let tenant_scope_id = root_view.tenant_scope_id().clone();
        let (coordinate, root_cutoff) = root_export_coordinate(&root_view, kind)?;
        let mut authorities = BTreeMap::new();
        for authority in dependency_authorities {
            if authorities
                .insert(authority.run_id().clone(), authority)
                .is_some()
            {
                return Err(ReplayError::InvalidExport);
            }
        }
        let mut graph = BTreeMap::new();
        let mut pending = BTreeSet::new();
        append_source_requirements(&root_run_id, &root_view, &mut graph, &mut pending)?;
        let views = BTreeMap::from([(root_run_id.clone(), (root_view, root_cutoff))]);
        Ok(Self {
            reader,
            authorities,
            store_identity,
            tenant_scope_id,
            root_run_id,
            kind,
            coordinate,
            graph,
            pending,
            views,
        })
    }

    async fn step(mut self) -> Result<SourceLoadStep<Self, VerifiedSourceClosure>> {
        let mut processed = 0usize;
        while processed < CLOSURE_STEP_SOURCES {
            let Some(run_id) = self.pending.iter().next().cloned() else {
                break;
            };
            self.pending.remove(&run_id);
            if self.views.contains_key(&run_id) {
                processed += 1;
                continue;
            }
            let authority = self
                .authorities
                .get(&run_id)
                .copied()
                .ok_or(ReplayError::SourceRunExportDenied)?;
            let view = load_export_dependency_view(self.reader, authority).await?;
            if view.store_identity() != &self.store_identity
                || view.tenant_scope_id() != &self.tenant_scope_id
                || view.run_id() != &run_id
                || view.run_phase() != RunPhase::Closed
            {
                return Err(ReplayError::InvalidExport);
            }
            let cutoff = closed_semantic_cutoff(&view)?;
            append_source_requirements(&run_id, &view, &mut self.graph, &mut self.pending)?;
            self.views.insert(run_id, (view, cutoff));
            processed += 1;
        }
        if !self.pending.is_empty() {
            return Ok(SourceLoadStep::Pending(self));
        }
        if !self.authorities.keys().eq(self
            .views
            .keys()
            .filter(|run_id| *run_id != &self.root_run_id))
        {
            return Err(ReplayError::InvalidExport);
        }
        reject_source_cycles(&self.graph, self.views.keys())?;
        verify_export_source_views(&self.views)?;
        Ok(SourceLoadStep::Complete(VerifiedSourceClosure {
            store_identity: self.store_identity,
            tenant_scope_id: self.tenant_scope_id,
            root_run_id: self.root_run_id,
            kind: self.kind,
            coordinate: self.coordinate,
            views: self.views,
        }))
    }
}

async fn collect_export_payloads(
    runs: &[PreparedExportRun],
) -> Result<BTreeMap<(SchemaId, ContentDigest), PreparedExportPayload>> {
    let mut payloads = BTreeMap::new();
    let mut processed = 0usize;
    for (run_index, run) in runs.iter().enumerate() {
        for commit in run.view.journal().commits() {
            let fields = commit.envelope().fields()?;
            if fields.core.run_sequence > run.cutoff {
                break;
            }
            for intent in fields.core.artifact_admission_intents {
                let value_ref = intent.fields()?.value_ref;
                let value_fields = value_ref.fields()?;
                let object = run
                    .view
                    .retained_value(&value_ref)
                    .map_err(|error| store_error(&error))?;
                let key = (
                    value_fields.schema_id.clone(),
                    value_fields.content_digest.clone(),
                );
                match payloads.entry(key) {
                    Entry::Vacant(entry) => {
                        entry.insert(PreparedExportPayload {
                            source_run_index: run_index,
                            source_value_ref: value_ref.clone(),
                            value_refs: BTreeMap::from([(
                                value_ref.as_bytes().to_vec(),
                                value_ref,
                            )]),
                        });
                    }
                    Entry::Occupied(mut entry) => {
                        let payload = entry.get_mut();
                        let source = runs
                            .get(payload.source_run_index)
                            .ok_or(ReplayError::InvalidExport)?
                            .view
                            .retained_value(&payload.source_value_ref)
                            .map_err(|error| store_error(&error))?;
                        if source.bytes() != object.bytes() {
                            return Err(ReplayError::InvalidExport);
                        }
                        payload
                            .value_refs
                            .entry(value_ref.as_bytes().to_vec())
                            .or_insert(value_ref);
                    }
                }
                processed += 1;
                if processed == CLOSURE_STEP_OBJECTS {
                    processed = 0;
                    tokio::task::yield_now().await;
                }
            }
        }
    }
    Ok(payloads)
}

impl PreparedExportClosure {
    async fn write_to<W: AsyncWrite + Unpin>(
        self,
        writer: &mut W,
    ) -> Result<PortableRunExportMetadata> {
        if !self.coordinate.matches_kind(self.kind) || self.runs.is_empty() {
            return Err(ReplayError::InvalidExport);
        }
        let contract = RecoverabilityContractV2::embedded()?;
        let schema_id = contract.schema_id(PORTABLE_STREAM_CONTRACT)?.clone();
        let mut output = FramedStreamWriter::new(writer, contract.raw_content_digest_hasher());
        output
            .write_frame(export_object([
                ("kind", export_string("header")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                (
                    "store_scope_id",
                    export_string(self.store_scope_id.as_str()),
                ),
                (
                    "tenant_scope_id",
                    export_string(self.tenant_scope_id.as_str()),
                ),
                ("root_run_id", export_string(self.root_run_id.as_str())),
                ("export_kind", export_string(self.kind.as_str())),
                ("coordinate", self.coordinate.canonical_value()?),
            ])?)
            .await?;

        for run in &self.runs {
            output
                .write_frame(export_object([
                    ("kind", export_string("run_begin")),
                    ("version", export_string(PORTABLE_FRAME_VERSION)),
                    ("run_id", export_string(run.view.run_id().as_str())),
                ])?)
                .await?;
            let mut expected_sequence = 1u64;
            for commit in run.view.journal().commits() {
                let fields = commit.envelope().fields()?;
                if fields.core.run_sequence > run.cutoff {
                    break;
                }
                if fields.core.run_sequence != expected_sequence {
                    return Err(ReplayError::InvalidExport);
                }
                write_member(
                    &mut output,
                    "commit_begin",
                    fields.core.run_sequence,
                    None,
                    commit.envelope().schema_id(),
                    commit.envelope().as_bytes(),
                )
                .await?;
                let mut expected_ordinal = 0u32;
                for record in commit.records() {
                    let record_fields = record.candidate().fields()?;
                    if record_fields.ordinal != expected_ordinal {
                        return Err(ReplayError::InvalidExport);
                    }
                    write_member(
                        &mut output,
                        "record_begin",
                        fields.core.run_sequence,
                        Some(record_fields.ordinal),
                        record.candidate().schema_id(),
                        record.candidate().as_bytes(),
                    )
                    .await?;
                    expected_ordinal = expected_ordinal
                        .checked_add(1)
                        .ok_or(ReplayError::InvalidExport)?;
                }
                if expected_ordinal == 0 {
                    return Err(ReplayError::InvalidExport);
                }
                expected_sequence = expected_sequence
                    .checked_add(1)
                    .ok_or(ReplayError::InvalidExport)?;
            }
            if expected_sequence == 1 || expected_sequence.checked_sub(1) != Some(run.cutoff) {
                return Err(ReplayError::InvalidExport);
            }
            output
                .write_frame(export_object([
                    ("kind", export_string("run_end")),
                    ("version", export_string(PORTABLE_FRAME_VERSION)),
                ])?)
                .await?;
        }

        for ((schema_id, content_digest), payload) in &self.payloads {
            let source = self
                .runs
                .get(payload.source_run_index)
                .ok_or(ReplayError::InvalidExport)?
                .view
                .retained_value(&payload.source_value_ref)
                .map_err(|error| store_error(&error))?;
            let byte_length =
                u64::try_from(source.bytes().len()).map_err(|_| ReplayError::InvalidExport)?;
            output
                .write_frame(export_object([
                    ("kind", export_string("object_begin")),
                    ("version", export_string(PORTABLE_FRAME_VERSION)),
                    ("schema_id", export_string(schema_id.as_str())),
                    ("content_digest", export_string(content_digest.as_str())),
                    ("byte_length", export_string(byte_length.to_string())),
                ])?)
                .await?;
            output.write_chunks(source.bytes()).await?;
            for value_ref in payload.value_refs.values() {
                output
                    .write_frame(export_object([
                        ("kind", export_string("object_authority")),
                        ("version", export_string(PORTABLE_FRAME_VERSION)),
                        ("value_ref", value_ref.canonical_value()?),
                    ])?)
                    .await?;
            }
            output
                .write_frame(export_object([
                    ("kind", export_string("object_end")),
                    ("version", export_string(PORTABLE_FRAME_VERSION)),
                ])?)
                .await?;
        }
        output
            .write_frame(export_object([
                ("kind", export_string("end")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
            ])?)
            .await?;
        let content_digest = output.finish().await?;
        let content_ref = ContentRef::new(schema_id.clone(), content_digest.clone())
            .map_err(|_| ReplayError::InvalidExport)?;
        Ok(PortableRunExportMetadata {
            schema_id,
            content_digest,
            content_ref,
        })
    }
}

async fn write_member<W: AsyncWrite + Unpin>(
    output: &mut FramedStreamWriter<'_, W>,
    kind: &'static str,
    run_sequence: u64,
    ordinal: Option<u32>,
    schema_id: &SchemaId,
    bytes: &[u8],
) -> Result<()> {
    let contract = RecoverabilityContractV2::embedded()?;
    let content_digest = contract.raw_content_digest(bytes);
    let byte_length = u64::try_from(bytes.len()).map_err(|_| ReplayError::InvalidExport)?;
    let mut fields = vec![
        ("kind", export_string(kind)),
        ("version", export_string(PORTABLE_FRAME_VERSION)),
        ("run_sequence", export_string(run_sequence.to_string())),
        ("schema_id", export_string(schema_id.as_str())),
        ("content_digest", export_string(content_digest.as_str())),
        ("byte_length", export_string(byte_length.to_string())),
    ];
    if let Some(ordinal) = ordinal {
        fields.push(("ordinal", CanonicalValue::Unsigned(u64::from(ordinal))));
    }
    output.write_frame(export_object(fields)?).await?;
    output.write_chunks(bytes).await
}

struct FramedStreamWriter<'writer, W> {
    writer: &'writer mut W,
    digest: RawContentDigestHasher,
    bytes_until_yield: usize,
}

impl<'writer, W: AsyncWrite + Unpin> FramedStreamWriter<'writer, W> {
    fn new(writer: &'writer mut W, digest: RawContentDigestHasher) -> Self {
        Self {
            writer,
            digest,
            bytes_until_yield: CLOSURE_STEP_STREAM_BYTES,
        }
    }

    async fn write_frame(&mut self, value: CanonicalValue) -> Result<()> {
        let contract = RecoverabilityContractV2::embedded()?;
        let frame = contract.encode(PORTABLE_FRAME_CONTRACT, &value)?;
        if frame.as_bytes().len() > MAX_STREAM_FRAME_JSON_BYTES {
            return Err(ReplayError::InvalidExport);
        }
        let mut record = Vec::with_capacity(frame.as_bytes().len() + 2);
        record.push(RECORD_SEPARATOR);
        record.extend_from_slice(frame.as_bytes());
        record.push(RECORD_SUFFIX);
        let mut remaining = record.as_slice();
        while !remaining.is_empty() {
            let count = remaining.len().min(self.bytes_until_yield);
            let (step, rest) = remaining.split_at(count);
            self.writer
                .write_all(step)
                .await
                .map_err(export_stream_io)?;
            self.digest.update(step);
            self.note_progress(step.len()).await;
            remaining = rest;
        }
        Ok(())
    }

    async fn write_chunks(&mut self, bytes: &[u8]) -> Result<()> {
        let mut offset = 0usize;
        for chunk in bytes.chunks(MAX_STREAM_CHUNK_BYTES) {
            if chunk.is_empty() {
                return Err(ReplayError::InvalidExport);
            }
            self.write_frame(export_object([
                ("kind", export_string("chunk")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("offset", export_string(offset.to_string())),
                (
                    "bytes",
                    CanonicalValue::Bytes(CanonicalBytes::new(chunk.to_vec())),
                ),
            ])?)
            .await?;
            offset = offset
                .checked_add(chunk.len())
                .ok_or(ReplayError::InvalidExport)?;
        }
        Ok(())
    }

    async fn note_progress(&mut self, count: usize) {
        if count >= self.bytes_until_yield {
            self.bytes_until_yield = CLOSURE_STEP_STREAM_BYTES;
            tokio::task::yield_now().await;
        } else {
            self.bytes_until_yield -= count;
        }
    }

    async fn finish(self) -> Result<ContentDigest> {
        self.writer.flush().await.map_err(export_stream_io)?;
        self.writer.shutdown().await.map_err(export_stream_io)?;
        Ok(self.digest.finalize())
    }
}

/// Verifies one complete portable run export stream and its external content reference.
///
/// Verification is callback-free and imposes only per-frame, per-chunk, and
/// cooperative-step bounds; it has no total frame, source, object, byte, step,
/// or elapsed-time ceiling.
pub async fn verify_portable_run_export_stream<R>(
    reader: R,
    expected_ref: &ContentRef,
) -> Result<OfflineVerifiedRun>
where
    R: AsyncRead + Unpin,
{
    let contract = RecoverabilityContractV2::embedded()?;
    let schema_id = contract.schema_id(PORTABLE_STREAM_CONTRACT)?;
    if expected_ref.schema_id() != schema_id {
        return Err(ReplayError::InvalidExport);
    }

    let mut session = Box::new(ClosureVerificationSession::new(
        reader,
        expected_ref.clone(),
        schema_id.clone(),
        contract.raw_content_digest_hasher(),
    ));
    loop {
        match session.step().await? {
            More::More(next) => {
                session = next;
                tokio::task::yield_now().await;
            }
            More::Complete(verified) => return Ok(verified),
        }
    }
}

// The continuation has one lifetime-long allocation. Boxing `Complete` would
// add a success-only allocation without changing the public inline return.
#[allow(clippy::large_enum_variant)]
enum More<R> {
    More(Box<ClosureVerificationSession<R>>),
    Complete(OfflineVerifiedRun),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamParsePhase {
    Header,
    Runs,
    Objects,
    Ended,
}

struct RunBuilder {
    run_id: RunId,
    commits: Vec<ParsedCommit>,
    next_sequence: u64,
}

struct CommitBuilder {
    run_sequence: u64,
    schema_id: SchemaId,
    bytes: Arc<[u8]>,
    records: Vec<ParsedRecord>,
    next_ordinal: u32,
}

struct ObjectBuilder {
    schema_id: SchemaId,
    content_digest: ContentDigest,
    bytes: Arc<[u8]>,
    value_refs: Vec<ValueRef>,
}

enum ActiveMemberTarget {
    Commit,
    Record { ordinal: u32 },
    Object,
}

struct ActiveMember {
    header: MemberHeader,
    target: ActiveMemberTarget,
    bytes: Vec<u8>,
    digest: RawContentDigestHasher,
}

struct StepCounts {
    remaining_stream_bytes: usize,
    dependency_sources: usize,
    unique_payloads: usize,
}

impl StepCounts {
    fn would_exceed(
        &self,
        additional_dependency_sources: usize,
        additional_unique_payloads: usize,
    ) -> bool {
        self.dependency_sources
            .checked_add(additional_dependency_sources)
            .is_none_or(|total| total > CLOSURE_STEP_SOURCES)
            || self
                .unique_payloads
                .checked_add(additional_unique_payloads)
                .is_none_or(|total| total > CLOSURE_STEP_OBJECTS)
    }
}

// Decoded frames stay inline because streams are unbounded and boxing would
// make every attacker-controlled frame incur a separate heap allocation.
#[allow(clippy::large_enum_variant)]
enum FrameRead {
    More,
    Frame(StagedFrame),
    Eof,
}

struct StagedFrame {
    frame: DecodedFrame,
    record: Vec<u8>,
    committed: usize,
    accepted: bool,
}

struct ClosureVerificationSession<R> {
    input: FramedStreamReader<R>,
    expected_ref: ContentRef,
    schema_id: SchemaId,
    header: Option<StreamHeader>,
    phase: StreamParsePhase,
    pending_frame: Option<StagedFrame>,
    runs: Vec<ParsedRun>,
    current_run: Option<RunBuilder>,
    current_commit: Option<CommitBuilder>,
    current_object: Option<ObjectBuilder>,
    active_member: Option<ActiveMember>,
    payloads: Vec<ParsedPayload>,
    prior_payload: Option<(SchemaId, ContentDigest)>,
}

impl<R: AsyncRead + Unpin> ClosureVerificationSession<R> {
    fn new(
        reader: R,
        expected_ref: ContentRef,
        schema_id: SchemaId,
        digest: RawContentDigestHasher,
    ) -> Self {
        Self {
            input: FramedStreamReader::new(reader, digest),
            expected_ref,
            schema_id,
            header: None,
            phase: StreamParsePhase::Header,
            pending_frame: None,
            runs: Vec::new(),
            current_run: None,
            current_commit: None,
            current_object: None,
            active_member: None,
            payloads: Vec::new(),
            prior_payload: None,
        }
    }

    async fn step(self: Box<Self>) -> Result<More<R>> {
        self.step_with_counts(StepCounts {
            remaining_stream_bytes: CLOSURE_STEP_STREAM_BYTES,
            dependency_sources: 0,
            unique_payloads: 0,
        })
        .await
    }

    async fn step_with_counts(mut self: Box<Self>, mut counts: StepCounts) -> Result<More<R>> {
        loop {
            let mut staged = if let Some(frame) = self.pending_frame.take() {
                frame
            } else {
                match self.input.next_frame(counts.remaining_stream_bytes).await? {
                    FrameRead::More => return Ok(More::More(self)),
                    FrameRead::Eof => return (*self).finish(),
                    FrameRead::Frame(frame) => frame,
                }
            };

            if !staged.accepted {
                if self.frame_would_exceed(&staged.frame, &counts)? {
                    self.pending_frame = Some(staged);
                    return Ok(More::More(self));
                }
                staged.accepted = true;
            }
            if !self
                .input
                .commit_frame(&mut staged, &mut counts.remaining_stream_bytes)
            {
                self.pending_frame = Some(staged);
                return Ok(More::More(self));
            }

            let StagedFrame {
                frame,
                record,
                committed: _,
                accepted: _,
            } = staged;
            self.input.recycle_record(record);
            if self.apply_frame(frame, &mut counts)?.is_some() {
                return Err(ReplayError::InvalidExport);
            }
        }
    }

    fn frame_would_exceed(&self, frame: &DecodedFrame, counts: &StepCounts) -> Result<bool> {
        if self.active_member.is_some() {
            return Ok(false);
        }
        match (self.phase, frame) {
            (StreamParsePhase::Runs, DecodedFrame::RunBegin { run_id }) => self
                .run_begin_is_dependency(run_id)
                .map(|dependency| dependency && counts.would_exceed(1, 0)),
            (StreamParsePhase::Runs, DecodedFrame::ObjectBegin(header)) => {
                if self.current_run.is_some()
                    || self.current_commit.is_some()
                    || self.runs.is_empty()
                {
                    return Err(ReplayError::InvalidExport);
                }
                self.validate_object_begin(header)?;
                Ok(counts.would_exceed(0, 1))
            }
            (StreamParsePhase::Objects, DecodedFrame::ObjectBegin(header)) => {
                self.validate_object_begin(header)?;
                Ok(counts.would_exceed(0, 1))
            }
            _ => Ok(false),
        }
    }

    fn apply_frame(
        &mut self,
        frame: DecodedFrame,
        counts: &mut StepCounts,
    ) -> Result<Option<DecodedFrame>> {
        if self.active_member.is_some() {
            let DecodedFrame::Chunk { offset, bytes } = frame else {
                return Err(ReplayError::InvalidExport);
            };
            self.append_chunk(offset, &bytes)?;
            return Ok(None);
        }

        match self.phase {
            StreamParsePhase::Header => {
                let DecodedFrame::Header(header) = frame else {
                    return Err(ReplayError::InvalidExport);
                };
                self.header = Some(header);
                self.phase = StreamParsePhase::Runs;
            }
            StreamParsePhase::Runs => match frame {
                DecodedFrame::RunBegin { run_id } => {
                    if self.run_begin_is_dependency(&run_id)? {
                        if counts.would_exceed(1, 0) {
                            return Ok(Some(DecodedFrame::RunBegin { run_id }));
                        }
                        counts.dependency_sources = counts
                            .dependency_sources
                            .checked_add(1)
                            .ok_or(ReplayError::InvalidExport)?;
                    }
                    self.current_run = Some(RunBuilder {
                        run_id,
                        commits: Vec::new(),
                        next_sequence: 1,
                    });
                }
                DecodedFrame::CommitBegin(header) => {
                    self.finish_commit()?;
                    let run = self
                        .current_run
                        .as_ref()
                        .ok_or(ReplayError::InvalidExport)?;
                    if header.run_sequence != Some(run.next_sequence) || header.ordinal.is_some() {
                        return Err(ReplayError::InvalidExport);
                    }
                    self.start_member(header, ActiveMemberTarget::Commit)?;
                }
                DecodedFrame::RecordBegin(header) => {
                    let commit = self
                        .current_commit
                        .as_ref()
                        .ok_or(ReplayError::InvalidExport)?;
                    if header.run_sequence != Some(commit.run_sequence)
                        || header.ordinal != Some(commit.next_ordinal)
                    {
                        return Err(ReplayError::InvalidExport);
                    }
                    self.start_member(
                        header,
                        ActiveMemberTarget::Record {
                            ordinal: commit.next_ordinal,
                        },
                    )?;
                }
                DecodedFrame::RunEnd => {
                    self.finish_commit()?;
                    let run = self.current_run.take().ok_or(ReplayError::InvalidExport)?;
                    if run.commits.is_empty() {
                        return Err(ReplayError::InvalidExport);
                    }
                    self.runs.push(ParsedRun {
                        run_id: run.run_id,
                        commits: run.commits,
                    });
                }
                DecodedFrame::ObjectBegin(header) => {
                    if self.current_run.is_some()
                        || self.current_commit.is_some()
                        || self.runs.is_empty()
                    {
                        return Err(ReplayError::InvalidExport);
                    }
                    self.phase = StreamParsePhase::Objects;
                    return self.begin_object(header, counts);
                }
                DecodedFrame::End => {
                    if self.current_run.is_some()
                        || self.current_commit.is_some()
                        || self.runs.is_empty()
                    {
                        return Err(ReplayError::InvalidExport);
                    }
                    self.phase = StreamParsePhase::Ended;
                }
                _ => return Err(ReplayError::InvalidExport),
            },
            StreamParsePhase::Objects => match frame {
                DecodedFrame::ObjectBegin(header) => {
                    return self.begin_object(header, counts);
                }
                DecodedFrame::ObjectAuthority { value_ref } => {
                    let object = self
                        .current_object
                        .as_mut()
                        .ok_or(ReplayError::InvalidExport)?;
                    if object
                        .value_refs
                        .last()
                        .is_some_and(|prior| prior.as_bytes() >= value_ref.as_bytes())
                    {
                        return Err(ReplayError::InvalidExport);
                    }
                    object.value_refs.push(value_ref);
                }
                DecodedFrame::ObjectEnd => {
                    let object = self
                        .current_object
                        .take()
                        .ok_or(ReplayError::InvalidExport)?;
                    let payload = UntrustedObjectPayload::new(
                        object.schema_id,
                        object.content_digest,
                        Arc::clone(&object.bytes),
                        object.value_refs,
                    )
                    .map_err(|_| ReplayError::InvalidExport)?;
                    self.payloads.push(ParsedPayload {
                        payload,
                        bytes: object.bytes,
                    });
                    counts.unique_payloads = counts
                        .unique_payloads
                        .checked_add(1)
                        .ok_or(ReplayError::InvalidExport)?;
                }
                DecodedFrame::End => {
                    if self.current_object.is_some() {
                        return Err(ReplayError::InvalidExport);
                    }
                    self.phase = StreamParsePhase::Ended;
                }
                _ => return Err(ReplayError::InvalidExport),
            },
            StreamParsePhase::Ended => return Err(ReplayError::InvalidExport),
        }
        Ok(None)
    }

    fn run_begin_is_dependency(&self, run_id: &RunId) -> Result<bool> {
        if self.current_run.is_some() {
            return Err(ReplayError::InvalidExport);
        }
        let header = self.header.as_ref().ok_or(ReplayError::InvalidExport)?;
        if self.runs.is_empty() {
            if run_id != &header.root_run_id {
                return Err(ReplayError::InvalidExport);
            }
            return Ok(false);
        }
        if run_id == &header.root_run_id
            || (self.runs.len() > 1
                && self
                    .runs
                    .last()
                    .is_some_and(|prior| prior.run_id >= *run_id))
        {
            return Err(ReplayError::InvalidExport);
        }
        Ok(true)
    }

    fn begin_object(
        &mut self,
        header: MemberHeader,
        counts: &StepCounts,
    ) -> Result<Option<DecodedFrame>> {
        let identity = self.validate_object_begin(&header)?;
        if counts.would_exceed(0, 1) {
            return Ok(Some(DecodedFrame::ObjectBegin(header)));
        }
        self.prior_payload = Some(identity);
        self.start_member(header, ActiveMemberTarget::Object)?;
        Ok(None)
    }

    fn validate_object_begin(&self, header: &MemberHeader) -> Result<(SchemaId, ContentDigest)> {
        if self.current_object.is_some()
            || header.run_sequence.is_some()
            || header.ordinal.is_some()
        {
            return Err(ReplayError::InvalidExport);
        }
        let identity = (header.schema_id.clone(), header.content_digest.clone());
        if self
            .prior_payload
            .as_ref()
            .is_some_and(|prior| prior >= &identity)
        {
            return Err(ReplayError::InvalidExport);
        }
        Ok(identity)
    }

    fn start_member(&mut self, header: MemberHeader, target: ActiveMemberTarget) -> Result<()> {
        let expected_length =
            usize::try_from(header.byte_length).map_err(|_| ReplayError::InvalidExport)?;
        let digest = RecoverabilityContractV2::embedded()?.raw_content_digest_hasher();
        self.active_member = Some(ActiveMember {
            header,
            target,
            bytes: Vec::new(),
            digest,
        });
        if expected_length == 0 {
            self.finish_member()?;
        }
        Ok(())
    }

    fn append_chunk(&mut self, offset: usize, chunk: &[u8]) -> Result<()> {
        let active = self
            .active_member
            .as_mut()
            .ok_or(ReplayError::InvalidExport)?;
        let expected_length =
            usize::try_from(active.header.byte_length).map_err(|_| ReplayError::InvalidExport)?;
        if offset != active.bytes.len()
            || chunk.is_empty()
            || chunk.len() > MAX_STREAM_CHUNK_BYTES
            || active
                .bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|length| length > expected_length)
        {
            return Err(ReplayError::InvalidExport);
        }
        active.digest.update(chunk);
        active.bytes.extend_from_slice(chunk);
        if active.bytes.len() == expected_length {
            self.finish_member()?;
        }
        Ok(())
    }

    fn finish_member(&mut self) -> Result<()> {
        let active = self
            .active_member
            .take()
            .ok_or(ReplayError::InvalidExport)?;
        if active.digest.finalize() != active.header.content_digest {
            return Err(ReplayError::InvalidExport);
        }
        let bytes = Arc::<[u8]>::from(active.bytes);
        match active.target {
            ActiveMemberTarget::Commit => {
                self.current_commit = Some(CommitBuilder {
                    run_sequence: active
                        .header
                        .run_sequence
                        .ok_or(ReplayError::InvalidExport)?,
                    schema_id: active.header.schema_id,
                    bytes,
                    records: Vec::new(),
                    next_ordinal: 0,
                });
            }
            ActiveMemberTarget::Record { ordinal } => {
                let commit = self
                    .current_commit
                    .as_mut()
                    .ok_or(ReplayError::InvalidExport)?;
                commit.records.push(ParsedRecord {
                    ordinal,
                    schema_id: active.header.schema_id,
                    bytes,
                });
                commit.next_ordinal = commit
                    .next_ordinal
                    .checked_add(1)
                    .ok_or(ReplayError::InvalidExport)?;
            }
            ActiveMemberTarget::Object => {
                self.current_object = Some(ObjectBuilder {
                    schema_id: active.header.schema_id,
                    content_digest: active.header.content_digest,
                    bytes,
                    value_refs: Vec::new(),
                });
            }
        }
        Ok(())
    }

    fn finish_commit(&mut self) -> Result<()> {
        let Some(commit) = self.current_commit.take() else {
            return Ok(());
        };
        if commit.records.is_empty() {
            return Err(ReplayError::InvalidExport);
        }
        let run = self
            .current_run
            .as_mut()
            .ok_or(ReplayError::InvalidExport)?;
        if commit.run_sequence != run.next_sequence {
            return Err(ReplayError::InvalidExport);
        }
        run.commits.push(ParsedCommit {
            run_sequence: commit.run_sequence,
            schema_id: commit.schema_id,
            bytes: commit.bytes,
            records: commit.records,
        });
        run.next_sequence = run
            .next_sequence
            .checked_add(1)
            .ok_or(ReplayError::InvalidExport)?;
        Ok(())
    }

    fn finish(mut self) -> Result<More<R>> {
        if self.phase != StreamParsePhase::Ended
            || self.active_member.is_some()
            || self.current_run.is_some()
            || self.current_commit.is_some()
            || self.current_object.is_some()
            || self.pending_frame.is_some()
        {
            return Err(ReplayError::InvalidExport);
        }
        let header = self.header.take().ok_or(ReplayError::InvalidExport)?;
        let digest = self.input.finish();
        if digest != *self.expected_ref.content_digest() {
            return Err(ReplayError::InvalidExport);
        }
        let verified_graph = verify_stream_graph(
            &self.runs,
            &self.payloads,
            &header.store_scope_id,
            &header.tenant_scope_id,
            &header.coordinate,
        )?;
        Ok(More::Complete(OfflineVerifiedRun {
            store_scope_id: header.store_scope_id,
            tenant_scope_id: header.tenant_scope_id,
            run_id: header.root_run_id,
            export_kind: header.export_kind,
            coordinate: header.coordinate,
            schema_id: self.schema_id,
            digest,
            verified_graph,
        }))
    }
}

struct StreamHeader {
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    root_run_id: RunId,
    export_kind: ExportKind,
    coordinate: ExportCoordinate,
}

struct MemberHeader {
    run_sequence: Option<u64>,
    ordinal: Option<u32>,
    schema_id: SchemaId,
    content_digest: ContentDigest,
    byte_length: u64,
}

enum DecodedFrame {
    Header(StreamHeader),
    RunBegin { run_id: RunId },
    CommitBegin(MemberHeader),
    RecordBegin(MemberHeader),
    ObjectBegin(MemberHeader),
    Chunk { offset: usize, bytes: Vec<u8> },
    ObjectAuthority { value_ref: ValueRef },
    ObjectEnd,
    RunEnd,
    End,
}

impl DecodedFrame {
    fn decode(frame: &ValidatedCanonicalValueV2) -> Result<Self> {
        let value = frame.canonical_value()?;
        match export_string_field(&value, "kind")? {
            "header" => {
                let root_run_id = RunId::from_str(export_string_field(&value, "root_run_id")?)
                    .map_err(|_| ReplayError::InvalidExport)?;
                let export_kind = parse_export_kind(export_string_field(&value, "export_kind")?)?;
                Ok(Self::Header(StreamHeader {
                    store_scope_id: StoreScopeId::from_str(export_string_field(
                        &value,
                        "store_scope_id",
                    )?)
                    .map_err(|_| ReplayError::InvalidExport)?,
                    tenant_scope_id: TenantScopeId::from_str(export_string_field(
                        &value,
                        "tenant_scope_id",
                    )?)
                    .map_err(|_| ReplayError::InvalidExport)?,
                    root_run_id: root_run_id.clone(),
                    export_kind,
                    coordinate: parse_coordinate(
                        required_export_field(&value, "coordinate")?,
                        export_kind,
                        &root_run_id,
                    )?,
                }))
            }
            "run_begin" => Ok(Self::RunBegin {
                run_id: RunId::from_str(export_string_field(&value, "run_id")?)
                    .map_err(|_| ReplayError::InvalidExport)?,
            }),
            "commit_begin" => Ok(Self::CommitBegin(parse_member_header(&value, false, true)?)),
            "record_begin" => Ok(Self::RecordBegin(parse_member_header(&value, true, true)?)),
            "object_begin" => Ok(Self::ObjectBegin(parse_member_header(
                &value, false, false,
            )?)),
            "chunk" => {
                let offset = usize::try_from(parse_decimal_field(&value, "offset")?)
                    .map_err(|_| ReplayError::InvalidExport)?;
                let bytes = match required_export_field(&value, "bytes")? {
                    CanonicalValue::String(value) => {
                        CanonicalBytes::from_base64url_no_pad(value.clone())
                            .map_err(|_| ReplayError::InvalidExport)?
                            .into_bytes()
                    }
                    _ => return Err(ReplayError::InvalidExport),
                };
                if bytes.is_empty() || bytes.len() > MAX_STREAM_CHUNK_BYTES {
                    return Err(ReplayError::InvalidExport);
                }
                Ok(Self::Chunk { offset, bytes })
            }
            "object_authority" => Ok(Self::ObjectAuthority {
                value_ref: ValueRef::from_canonical_value(
                    required_export_field(&value, "value_ref")?.clone(),
                )?,
            }),
            "object_end" => Ok(Self::ObjectEnd),
            "run_end" => Ok(Self::RunEnd),
            "end" => Ok(Self::End),
            _ => Err(ReplayError::InvalidExport),
        }
    }
}

fn parse_member_header(
    value: &CanonicalValue,
    has_ordinal: bool,
    has_run_sequence: bool,
) -> Result<MemberHeader> {
    let run_sequence = has_run_sequence
        .then(|| parse_decimal_field(value, "run_sequence"))
        .transpose()?;
    let ordinal = if has_ordinal {
        match required_export_field(value, "ordinal")? {
            CanonicalValue::Unsigned(value) => {
                Some(u32::try_from(*value).map_err(|_| ReplayError::InvalidExport)?)
            }
            _ => return Err(ReplayError::InvalidExport),
        }
    } else {
        None
    };
    Ok(MemberHeader {
        run_sequence,
        ordinal,
        schema_id: SchemaId::from_str(export_string_field(value, "schema_id")?)
            .map_err(|_| ReplayError::InvalidExport)?,
        content_digest: ContentDigest::from_str(export_string_field(value, "content_digest")?)
            .map_err(|_| ReplayError::InvalidExport)?,
        byte_length: parse_decimal_field(value, "byte_length")?,
    })
}

fn parse_decimal_field(value: &CanonicalValue, field: &str) -> Result<u64> {
    export_string_field(value, field)?
        .parse()
        .map_err(|_| ReplayError::InvalidExport)
}

struct FramedStreamReader<R> {
    reader: BufReader<R>,
    digest: RawContentDigestHasher,
    record: Vec<u8>,
    #[cfg(test)]
    committed_bytes: Vec<u8>,
}

impl<R: AsyncRead + Unpin> FramedStreamReader<R> {
    fn new(reader: R, digest: RawContentDigestHasher) -> Self {
        Self {
            reader: BufReader::new(reader),
            digest,
            record: Vec::new(),
            #[cfg(test)]
            committed_bytes: Vec::new(),
        }
    }

    async fn next_frame(&mut self, read_limit: usize) -> Result<FrameRead> {
        let mut remaining_read_bytes = read_limit;
        loop {
            if remaining_read_bytes == 0 {
                let available = self.reader.fill_buf().await.map_err(export_stream_io)?;
                if available.is_empty() {
                    return if self.record.is_empty() {
                        Ok(FrameRead::Eof)
                    } else {
                        Err(ReplayError::InvalidExport)
                    };
                }
                return Ok(FrameRead::More);
            }

            let (consumed, terminated) = {
                let available = self.reader.fill_buf().await.map_err(export_stream_io)?;
                if available.is_empty() {
                    if self.record.is_empty() {
                        return Ok(FrameRead::Eof);
                    }
                    return Err(ReplayError::InvalidExport);
                }
                let wanted = available
                    .iter()
                    .position(|byte| *byte == RECORD_SUFFIX)
                    .map_or(available.len(), |position| position + 1);
                let consumed = wanted.min(remaining_read_bytes);
                if self
                    .record
                    .len()
                    .checked_add(consumed)
                    .is_none_or(|length| length > MAX_STREAM_FRAME_JSON_BYTES + 2)
                {
                    return Err(ReplayError::InvalidExport);
                }
                self.record.extend_from_slice(&available[..consumed]);
                (
                    consumed,
                    consumed == wanted && available[consumed - 1] == RECORD_SUFFIX,
                )
            };
            self.reader.consume(consumed);
            remaining_read_bytes -= consumed;
            if !terminated {
                continue;
            }

            let record = std::mem::take(&mut self.record);
            if record.len() < 3
                || record[0] != RECORD_SEPARATOR
                || record.last() != Some(&RECORD_SUFFIX)
            {
                return Err(ReplayError::InvalidExport);
            }
            let json = &record[1..record.len() - 1];
            if json.len() > MAX_STREAM_FRAME_JSON_BYTES {
                return Err(ReplayError::InvalidExport);
            }
            let frame = RecoverabilityContractV2::embedded()?
                .strict_decode(PORTABLE_FRAME_CONTRACT, json)
                .map_err(|_| ReplayError::InvalidExport)?;
            return DecodedFrame::decode(&frame).map(|frame| {
                FrameRead::Frame(StagedFrame {
                    frame,
                    record,
                    committed: 0,
                    accepted: false,
                })
            });
        }
    }

    fn commit_frame(
        &mut self,
        frame: &mut StagedFrame,
        remaining_stream_bytes: &mut usize,
    ) -> bool {
        debug_assert!(frame.accepted);
        let remaining = frame.record.len() - frame.committed;
        let consumed = remaining.min(*remaining_stream_bytes);
        let end = frame.committed + consumed;
        let bytes = &frame.record[frame.committed..end];
        self.digest.update(bytes);
        #[cfg(test)]
        self.committed_bytes.extend_from_slice(bytes);
        frame.committed = end;
        *remaining_stream_bytes -= consumed;
        frame.committed == frame.record.len()
    }

    fn recycle_record(&mut self, mut record: Vec<u8>) {
        record.clear();
        self.record = record;
    }

    #[cfg(test)]
    fn committed_bytes(&self) -> &[u8] {
        &self.committed_bytes
    }

    fn finish(self) -> ContentDigest {
        self.digest.finalize()
    }
}

fn export_stream_io(source: std::io::Error) -> ReplayError {
    ReplayError::ExportStreamIo { source }
}

struct ParsedRun {
    run_id: RunId,
    commits: Vec<ParsedCommit>,
}

struct ParsedCommit {
    run_sequence: u64,
    schema_id: SchemaId,
    bytes: Arc<[u8]>,
    records: Vec<ParsedRecord>,
}

struct ParsedRecord {
    ordinal: u32,
    schema_id: SchemaId,
    bytes: Arc<[u8]>,
}

struct ParsedPayload {
    payload: UntrustedObjectPayload,
    bytes: Arc<[u8]>,
}

struct DecodedRun {
    store_epoch: StoreEpoch,
    commits: Vec<CommittedJournalCommit>,
    object_refs: BTreeMap<Vec<u8>, ValueRef>,
}

struct VerifiedExportRun {
    view: VerifiedRunView,
}

struct VerifiedExportGraph {
    runs: Vec<VerifiedExportRun>,
}

fn verify_stream_graph(
    runs: &[ParsedRun],
    payloads: &[ParsedPayload],
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    coordinate: &ExportCoordinate,
) -> Result<VerifiedExportGraph> {
    let mut decoded_runs = Vec::with_capacity(runs.len());
    let mut observed_store_epoch = None;
    for run in runs {
        let decoded = decode_run(run, store_scope_id)?;
        observe_common_store_epoch(&mut observed_store_epoch, decoded.store_epoch)?;
        decoded_runs.push(decoded);
    }

    let mut expected_ref_runs = BTreeMap::<Vec<u8>, BTreeSet<usize>>::new();
    for (run_index, run) in decoded_runs.iter().enumerate() {
        for value_ref in run.object_refs.values() {
            expected_ref_runs
                .entry(value_ref.as_bytes().to_vec())
                .or_default()
                .insert(run_index);
        }
    }
    let supplied_refs = payloads
        .iter()
        .flat_map(|payload| payload.payload.value_refs())
        .map(|value_ref| value_ref.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    if supplied_refs != expected_ref_runs.keys().cloned().collect() {
        return Err(ReplayError::InvalidExport);
    }

    let store_epoch = observed_store_epoch.ok_or(ReplayError::InvalidExport)?;
    let mut verified = Vec::with_capacity(decoded_runs.len());
    for (run_index, (decoded, parsed)) in decoded_runs.into_iter().zip(runs).enumerate() {
        let payloads = payloads_for_run(payloads, &expected_ref_runs, run_index)?;
        let view = verify_offline_recorded_material(
            StoreIdentity::new(store_scope_id.clone(), store_epoch),
            tenant_scope_id.clone(),
            parsed.run_id.clone(),
            decoded.commits,
            payloads,
        )
        .map_err(|_| ReplayError::InvalidExport)?;
        if run_index != 0
            && (view.run_phase() != RunPhase::Closed
                || view.journal_head() != view.semantic_head()
                || view.semantic_closure().is_none())
        {
            return Err(ReplayError::InvalidExport);
        }
        verified.push(VerifiedExportRun { view });
    }

    validate_verified_source_closure(&verified)?;
    validate_export_coordinate(
        coordinate,
        runs.first()
            .map(|run| &run.run_id)
            .ok_or(ReplayError::InvalidExport)?,
        verified.first().ok_or(ReplayError::InvalidExport)?,
    )?;
    Ok(VerifiedExportGraph { runs: verified })
}

fn observe_common_store_epoch(
    observed: &mut Option<StoreEpoch>,
    candidate: StoreEpoch,
) -> Result<()> {
    match *observed {
        None => {
            *observed = Some(candidate);
            Ok(())
        }
        Some(expected) if expected == candidate => Ok(()),
        Some(_) => Err(ReplayError::InvalidExport),
    }
}

fn decode_run(run: &ParsedRun, expected_store_scope_id: &StoreScopeId) -> Result<DecodedRun> {
    let mut store_epoch = None;
    let mut commits = Vec::with_capacity(run.commits.len());
    let mut object_refs = BTreeMap::new();
    for commit in &run.commits {
        let envelope = CommitEnvelope::strict_decode(&commit.bytes)?;
        if commit.schema_id != *envelope.schema_id() {
            return Err(ReplayError::InvalidExport);
        }
        let envelope_fields = envelope.fields()?;
        if envelope_fields.core.run_sequence != commit.run_sequence
            || envelope_fields.core.run_id != run.run_id
            || envelope_fields.core.store_scope_id != *expected_store_scope_id
        {
            return Err(ReplayError::InvalidExport);
        }
        if commit.run_sequence == 1 {
            let JournalPredecessorFields::Genesis {
                store_scope_id,
                store_epoch: genesis_store_epoch,
                run_id,
                ..
            } = envelope_fields.core.predecessor.fields()?
            else {
                return Err(ReplayError::InvalidExport);
            };
            if store_scope_id != *expected_store_scope_id || run_id != run.run_id {
                return Err(ReplayError::InvalidExport);
            }
            store_epoch = Some(genesis_store_epoch);
        }
        for intent in &envelope_fields.core.artifact_admission_intents {
            let value_ref = intent.fields()?.value_ref;
            object_refs
                .entry(value_ref.as_bytes().to_vec())
                .or_insert(value_ref);
        }

        let mut records = Vec::with_capacity(commit.records.len());
        for record in &commit.records {
            let candidate = CandidateRecordEnvelope::strict_decode(&record.bytes)?;
            if record.schema_id != *candidate.schema_id()
                || candidate.fields()?.ordinal != record.ordinal
            {
                return Err(ReplayError::InvalidExport);
            }
            let record_hash = RecordHashPreimage::from_candidate(&candidate)?.record_hash()?;
            let record_id: RecordId = RecordIdPreimage::new(
                expected_store_scope_id,
                &run.run_id,
                commit.run_sequence,
                record.ordinal,
                &record_hash,
            )?
            .record_id()?;
            records.push(CommittedJournalRecord::from_persisted(
                record_id,
                record_hash,
                candidate,
            ));
        }
        commits.push(CommittedJournalCommit::from_persisted(envelope, records));
    }
    Ok(DecodedRun {
        store_epoch: store_epoch.ok_or(ReplayError::InvalidExport)?,
        commits,
        object_refs,
    })
}

fn payloads_for_run(
    payloads: &[ParsedPayload],
    expected_ref_runs: &BTreeMap<Vec<u8>, BTreeSet<usize>>,
    run_index: usize,
) -> Result<Vec<UntrustedObjectPayload>> {
    payloads
        .iter()
        .filter_map(|payload| {
            let value_refs = payload
                .payload
                .value_refs()
                .iter()
                .filter(|value_ref| {
                    expected_ref_runs
                        .get(value_ref.as_bytes())
                        .is_some_and(|runs| runs.contains(&run_index))
                })
                .cloned()
                .collect::<Vec<_>>();
            (!value_refs.is_empty()).then(|| {
                UntrustedObjectPayload::new(
                    payload.payload.schema_id().clone(),
                    payload.payload.content_digest().clone(),
                    Arc::clone(&payload.bytes),
                    value_refs,
                )
                .map_err(|_| ReplayError::InvalidExport)
            })
        })
        .collect()
}

fn validate_verified_source_closure(runs: &[VerifiedExportRun]) -> Result<()> {
    let views = runs
        .iter()
        .map(|run| (run.view.run_id().clone(), &run.view))
        .collect::<BTreeMap<_, _>>();
    if views.len() != runs.len() {
        return Err(ReplayError::InvalidExport);
    }
    let mut graph = BTreeMap::<RunId, BTreeSet<RunId>>::new();
    for run in runs {
        let edges = graph.entry(run.view.run_id().clone()).or_default();
        for requirement in run.view.admission_source_requirements().cross_run_sources() {
            edges.insert(requirement.source_run_id().clone());
        }
    }
    let discovered = graph
        .values()
        .flat_map(BTreeSet::iter)
        .cloned()
        .collect::<BTreeSet<_>>();
    let supplied = runs
        .iter()
        .skip(1)
        .map(|run| run.view.run_id().clone())
        .collect::<BTreeSet<_>>();
    if discovered != supplied {
        return Err(ReplayError::InvalidExport);
    }
    reject_source_cycles(&graph, views.keys())?;
    for run in runs {
        for requirement in run.view.admission_source_requirements().cross_run_sources() {
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

fn validate_export_coordinate(
    coordinate: &ExportCoordinate,
    run_id: &RunId,
    root: &VerifiedExportRun,
) -> Result<()> {
    match coordinate {
        ExportCoordinate::SemanticHead {
            record_ref,
            containing_commit_digest,
        } => {
            if root.view.run_id() != run_id
                || root.view.run_phase() != RunPhase::Open
                || root.view.journal_head() != root.view.semantic_head()
                || record_ref.as_bytes() != root.view.semantic_head_record_ref().as_bytes()
                || &root.view.semantic_head().fields()?.commit_digest != containing_commit_digest
            {
                return Err(ReplayError::InvalidExport);
            }
        }
        ExportCoordinate::SemanticClosure {
            terminal_transition_ref,
            containing_commit_digest,
        } => {
            let closure = root
                .view
                .semantic_closure()
                .ok_or(ReplayError::InvalidExport)?
                .fields()?;
            if root.view.run_id() != run_id
                || root.view.run_phase() != RunPhase::Closed
                || root.view.journal_head() != root.view.semantic_head()
                || terminal_transition_ref.as_bytes() != closure.terminal_transition_ref.as_bytes()
                || containing_commit_digest != &closure.containing_commit_digest
            {
                return Err(ReplayError::InvalidExport);
            }
        }
        ExportCoordinate::JournalHead { journal_head } => {
            if journal_head != root.view.journal_head() {
                return Err(ReplayError::InvalidExport);
            }
        }
    }
    Ok(())
}

async fn load_export_view<B: RunJournalBackend>(
    reader: &RunHistoryReader<B>,
    authority: &RunAccessAuthority<Export>,
) -> Result<VerifiedRunView> {
    let journal = reader
        .load_for_export(authority)
        .await
        .map_err(|error| store_error(&error))?;
    journal
        .verify_recorded_history()
        .map_err(|error| store_error(&error))
}

async fn load_export_dependency_view<B: RunJournalBackend>(
    reader: &RunHistoryReader<B>,
    authority: &RunAccessAuthority<Export>,
) -> Result<VerifiedRunView> {
    load_export_view(reader, authority)
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
    Ok(())
}

fn verify_export_source_views(views: &BTreeMap<RunId, (VerifiedRunView, u64)>) -> Result<()> {
    for (view, _) in views.values() {
        for requirement in view.admission_source_requirements().cross_run_sources() {
            let source = views
                .get(requirement.source_run_id())
                .map(|(view, _)| view)
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

fn parse_export_kind(value: &str) -> Result<ExportKind> {
    match value {
        "semantic" => Ok(ExportKind::Semantic),
        "audit" => Ok(ExportKind::Audit),
        _ => Err(ReplayError::InvalidExport),
    }
}

fn parse_coordinate(
    value: &CanonicalValue,
    kind: ExportKind,
    run_id: &RunId,
) -> Result<ExportCoordinate> {
    match (kind, export_string_field(value, "kind")?) {
        (ExportKind::Semantic, "semantic_head") => {
            let record_ref = RecordRef::from_canonical_value(
                required_export_field(value, "record_ref")?.clone(),
            )?;
            if &record_ref.fields()?.run_id != run_id || export_object_len(value)? != 3 {
                return Err(ReplayError::InvalidExport);
            }
            Ok(ExportCoordinate::SemanticHead {
                record_ref,
                containing_commit_digest: JournalCommitDigest::from_str(export_string_field(
                    value,
                    "containing_commit_digest",
                )?)
                .map_err(|_| ReplayError::InvalidExport)?,
            })
        }
        (ExportKind::Semantic, "semantic_closure") => {
            let terminal_transition_ref = TransitionRef::from_canonical_value(
                required_export_field(value, "terminal_transition_ref")?.clone(),
            )?;
            if &terminal_transition_ref.fields()?.run_id != run_id || export_object_len(value)? != 3
            {
                return Err(ReplayError::InvalidExport);
            }
            Ok(ExportCoordinate::SemanticClosure {
                terminal_transition_ref,
                containing_commit_digest: JournalCommitDigest::from_str(export_string_field(
                    value,
                    "containing_commit_digest",
                )?)
                .map_err(|_| ReplayError::InvalidExport)?,
            })
        }
        (ExportKind::Audit, "journal_head") => {
            if export_object_len(value)? != 2 {
                return Err(ReplayError::InvalidExport);
            }
            Ok(ExportCoordinate::JournalHead {
                journal_head: JournalHead::from_canonical_value(
                    required_export_field(value, "journal_head")?.clone(),
                )?,
            })
        }
        _ => Err(ReplayError::InvalidExport),
    }
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

fn export_object_len(value: &CanonicalValue) -> Result<usize> {
    match value {
        CanonicalValue::Object(object) => Ok(object.entries().count()),
        _ => Err(ReplayError::InvalidExport),
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
mod stream_tests;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::error::Error as _;
    use std::io;
    use std::pin::Pin;
    use std::str::FromStr;
    use std::task::{Context as TaskContext, Poll};

    use mfm_canonical::RecoverabilityContractV2;
    use mfm_ids::{
        ContentDigest, ContentRef, JournalCommitDigest, RunId, StoreEpoch, StoreScopeId,
        TenantScopeId,
    };
    use mfm_journal::v1::{JournalHead, ValueRef};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::{
        classify_export_dependency_load_error, export_object, export_stream_io, export_string,
        observe_common_store_epoch, reject_source_cycles, verify_portable_run_export_stream,
        ActiveMemberTarget, ClosureVerificationSession, DecodedFrame, ExportCoordinate, ExportKind,
        FrameRead, FramedStreamReader, FramedStreamWriter, MemberHeader, More, ParsedRun,
        ReplayError, StagedFrame, StepCounts, StreamHeader, StreamParsePhase, CLOSURE_STEP_OBJECTS,
        CLOSURE_STEP_SOURCES, CLOSURE_STEP_STREAM_BYTES, MAX_STREAM_CHUNK_BYTES,
        PORTABLE_FRAME_CONTRACT, PORTABLE_FRAME_VERSION, PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE,
        RECORD_SEPARATOR, RECORD_SUFFIX,
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
    fn stream_io_error_has_one_redacted_source_preserving_classification() {
        let error = export_stream_io(io::Error::other("sentinel /private/path"));
        assert_eq!(error.code(), "MFM_REPLAY_EXPORT_STREAM_IO_FAILED");
        assert_eq!(error.to_string(), "portable run export stream I/O failed");
        assert!(error.source().is_some());
        assert!(!error.to_string().contains("sentinel"));
        assert!(!error.to_string().contains("/private/path"));
        assert_eq!(
            PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE,
            "application/vnd.mfm.run-export-stream.v1+json-seq"
        );
    }

    #[tokio::test]
    async fn reader_io_error_preserves_source_without_reclassifying_the_artifact() {
        let expected_ref = stream_ref(b"unread");
        let error = verify_portable_run_export_stream(FailingReader, &expected_ref)
            .await
            .expect_err("reader failure");
        assert_stream_io(error);
    }

    #[tokio::test]
    async fn writer_flush_and_shutdown_errors_have_one_stream_io_classification() {
        for failure in [
            WriterFailure::Write,
            WriterFailure::Flush,
            WriterFailure::Shutdown,
        ] {
            let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
            let mut writer = FailingWriter { failure };
            let mut stream =
                FramedStreamWriter::new(&mut writer, contract.raw_content_digest_hasher());
            let write = stream
                .write_frame(
                    export_object([
                        ("kind", export_string("end")),
                        ("version", export_string(PORTABLE_FRAME_VERSION)),
                    ])
                    .expect("end frame"),
                )
                .await;
            let error = match failure {
                WriterFailure::Write => write.expect_err("write failure"),
                WriterFailure::Flush | WriterFailure::Shutdown => {
                    write.expect("frame write");
                    stream.finish().await.expect_err("finalization failure")
                }
            };
            assert_stream_io(error);
        }
    }

    #[tokio::test]
    async fn exact_byte_budget_observes_eof_without_a_ceremonial_more() {
        let full = frame_record(chunk_frame(&vec![0; MAX_STREAM_CHUNK_BYTES]));
        let final_chunk = frame_record(chunk_frame(&vec![0; 65_339]));
        assert_eq!(full.len(), 87_469);
        assert_eq!(final_chunk.len(), 87_206);
        let exact = [full.as_slice(), full.as_slice(), final_chunk.as_slice()].concat();
        assert_eq!(exact.len(), CLOSURE_STEP_STREAM_BYTES);

        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let mut reader = FramedStreamReader::new(
            ShortReader::new(exact.clone(), 7),
            contract.raw_content_digest_hasher(),
        );
        let mut remaining = CLOSURE_STEP_STREAM_BYTES;
        for _ in 0..3 {
            let FrameRead::Frame(frame) = reader
                .next_frame(remaining)
                .await
                .expect("frame at exact budget")
            else {
                panic!("expected frame at exact budget");
            };
            assert!(matches!(
                accept_staged_frame(&mut reader, frame, &mut remaining),
                DecodedFrame::Chunk { .. }
            ));
        }
        assert_eq!(remaining, 0);
        assert_eq!(reader.committed_bytes(), exact);
        assert!(matches!(
            reader
                .next_frame(remaining)
                .await
                .expect("zero-cost EOF observation"),
            FrameRead::Eof
        ));
        assert_eq!(reader.finish(), contract.raw_content_digest(&exact));
    }

    #[tokio::test]
    async fn byte_after_exact_budget_is_left_for_the_next_consumed_step() {
        let full = frame_record(chunk_frame(&vec![0; MAX_STREAM_CHUNK_BYTES]));
        let final_chunk = frame_record(chunk_frame(&vec![0; 65_339]));
        let mut plus_one = [full.as_slice(), full.as_slice(), final_chunk.as_slice()].concat();
        plus_one.push(RECORD_SEPARATOR);
        assert_eq!(plus_one.len(), CLOSURE_STEP_STREAM_BYTES + 1);

        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let mut reader = FramedStreamReader::new(
            ShortReader::new(plus_one, 8_192),
            contract.raw_content_digest_hasher(),
        );
        let mut remaining = CLOSURE_STEP_STREAM_BYTES;
        for _ in 0..3 {
            let FrameRead::Frame(frame) = reader
                .next_frame(remaining)
                .await
                .expect("frame before exact budget")
            else {
                panic!("expected frame before exact budget");
            };
            assert!(matches!(
                accept_staged_frame(&mut reader, frame, &mut remaining),
                DecodedFrame::Chunk { .. }
            ));
        }
        assert!(matches!(
            reader
                .next_frame(remaining)
                .await
                .expect("next byte requires another step"),
            FrameRead::More
        ));
        assert_eq!(remaining, 0);

        remaining = CLOSURE_STEP_STREAM_BYTES;
        assert!(matches!(
            reader.next_frame(remaining).await,
            Err(ReplayError::InvalidExport)
        ));
        assert_eq!(remaining, CLOSURE_STEP_STREAM_BYTES);
    }

    #[tokio::test]
    async fn framed_reader_has_no_total_stream_size_ceiling() {
        let record = frame_record(chunk_frame(&vec![0; MAX_STREAM_CHUNK_BYTES]));
        let bytes = record.repeat(192);
        assert!(bytes.len() > 16_777_216);

        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let mut reader = FramedStreamReader::new(
            ShortReader::new(bytes.clone(), 4_093),
            contract.raw_content_digest_hasher(),
        );
        let mut remaining = CLOSURE_STEP_STREAM_BYTES;
        let mut decoded = 0usize;
        loop {
            match reader
                .next_frame(remaining)
                .await
                .expect("individually bounded frame")
            {
                FrameRead::Frame(frame) => {
                    assert!(matches!(
                        accept_staged_frame(&mut reader, frame, &mut remaining),
                        DecodedFrame::Chunk { .. }
                    ));
                    decoded += 1;
                }
                FrameRead::More => remaining = CLOSURE_STEP_STREAM_BYTES,
                FrameRead::Eof => break,
            }
        }
        assert_eq!(decoded, 192);
        assert_eq!(reader.committed_bytes(), bytes);
        assert_eq!(reader.finish(), contract.raw_content_digest(&bytes));
    }

    #[tokio::test]
    async fn source_257_is_staged_until_the_next_consuming_step() {
        let mut bytes = Vec::new();
        let mut starts = Vec::new();
        for discriminator in 1..=513 {
            starts.push(bytes.len());
            append_empty_run(&mut bytes, &run_id(discriminator));
        }
        assert!(starts[256] < CLOSURE_STEP_STREAM_BYTES);
        assert!(starts[512] - starts[256] < CLOSURE_STEP_STREAM_BYTES);

        let first = configured_source_session(bytes.as_slice());
        let More::More(first) = first.step().await.expect("source boundary step") else {
            panic!("source 257 must begin in the next step");
        };
        assert_staged_run_begin(&first, &bytes, starts[256], 257);

        let restarted = configured_source_session(bytes.as_slice());
        let More::More(restarted) = restarted.step().await.expect("restarted source step") else {
            panic!("restart must reach the same source boundary");
        };
        assert_staged_run_begin(&restarted, &bytes, starts[256], 257);
        assert_eq!(
            restarted.input.committed_bytes(),
            first.input.committed_bytes()
        );

        let More::More(second) = first.step().await.expect("second source boundary step") else {
            panic!("source 513 must begin in the third step");
        };
        assert_staged_run_begin(&second, &bytes, starts[512], 513);
        let accepted_end = starts[512]
            + second
                .pending_frame
                .as_ref()
                .expect("staged source 513")
                .record
                .len();
        let More::More(accepted) = second
            .step_with_counts(StepCounts {
                remaining_stream_bytes: accepted_end - starts[512],
                dependency_sources: 0,
                unique_payloads: 0,
            })
            .await
            .expect("source 513 consuming step")
        else {
            panic!("the remaining run frames require another step");
        };
        assert!(accepted.pending_frame.is_none());
        assert_eq!(accepted.input.committed_bytes(), &bytes[..accepted_end]);
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        assert_eq!(
            accepted.input.finish(),
            contract.raw_content_digest(&bytes[..accepted_end])
        );
    }

    #[tokio::test]
    async fn object_513_is_staged_until_the_next_consuming_step() {
        let mut bytes = Vec::new();
        append_empty_object(&mut bytes, 513);
        let object_begin_length = object_begin_record(513).len();

        let first = configured_object_session(bytes.as_slice());
        let More::More(first) = first
            .step_with_counts(StepCounts {
                remaining_stream_bytes: CLOSURE_STEP_STREAM_BYTES,
                dependency_sources: 0,
                unique_payloads: CLOSURE_STEP_OBJECTS,
            })
            .await
            .expect("object boundary step")
        else {
            panic!("object 513 must begin in the next step");
        };
        assert_staged_object_begin(&first, &bytes, 0, 513);

        let restarted = configured_object_session(bytes.as_slice());
        let More::More(restarted) = restarted
            .step_with_counts(StepCounts {
                remaining_stream_bytes: CLOSURE_STEP_STREAM_BYTES,
                dependency_sources: 0,
                unique_payloads: CLOSURE_STEP_OBJECTS,
            })
            .await
            .expect("restarted object step")
        else {
            panic!("restart must reach the same object boundary");
        };
        assert_staged_object_begin(&restarted, &bytes, 0, 513);
        assert_eq!(
            restarted.input.committed_bytes(),
            first.input.committed_bytes()
        );

        let More::More(accepted) = first
            .step_with_counts(StepCounts {
                remaining_stream_bytes: object_begin_length,
                dependency_sources: 0,
                unique_payloads: 0,
            })
            .await
            .expect("object 513 consuming step")
        else {
            panic!("the remaining object frame requires another step");
        };
        assert!(accepted.pending_frame.is_none());
        assert_eq!(
            accepted.input.committed_bytes(),
            &bytes[..object_begin_length]
        );
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        assert_eq!(
            accepted.input.finish(),
            contract.raw_content_digest(&bytes[..object_begin_length])
        );
    }

    #[test]
    fn source_257_and_payload_513_are_deferred_without_total_caps() {
        let root_run_id = run_id(0);
        let mut session = test_session();
        session.header = Some(stream_header(root_run_id.clone()));
        session.phase = StreamParsePhase::Runs;
        session.runs.push(ParsedRun {
            run_id: root_run_id,
            commits: Vec::new(),
        });
        let mut counts = step_counts();
        let mut source_more = 0usize;
        for discriminator in 1..=4_097 {
            let frame = DecodedFrame::RunBegin {
                run_id: run_id(discriminator),
            };
            let frame = match session
                .apply_frame(frame, &mut counts)
                .expect("ordered dependency source")
            {
                None => None,
                Some(frame) => {
                    assert_eq!(counts.dependency_sources, CLOSURE_STEP_SOURCES);
                    assert!(session.current_run.is_none());
                    source_more += 1;
                    counts = step_counts();
                    Some(frame)
                }
            };
            if let Some(frame) = frame {
                assert!(session
                    .apply_frame(frame, &mut counts)
                    .expect("same source begins in next step")
                    .is_none());
            }
            let run = session
                .current_run
                .take()
                .expect("accepted dependency source");
            session.runs.push(ParsedRun {
                run_id: run.run_id,
                commits: Vec::new(),
            });
        }
        assert_eq!(source_more, 16);

        session.phase = StreamParsePhase::Objects;
        session.runs.truncate(1);
        session.prior_payload = None;
        let schema_id = RecoverabilityContractV2::embedded()
            .expect("recoverability contract")
            .schema_id("mfm.portable-run-export-stream.v1")
            .expect("stream schema")
            .clone();
        counts.unique_payloads = CLOSURE_STEP_OBJECTS;
        let empty_digest = RecoverabilityContractV2::embedded()
            .expect("recoverability contract")
            .raw_content_digest(&[]);
        let first = member_header(schema_id.clone(), empty_digest.clone(), 0);
        let deferred = session
            .apply_frame(DecodedFrame::ObjectBegin(first), &mut counts)
            .expect("ordered payload at next limit");
        let Some(frame) = deferred else {
            panic!("payload 513 must be deferred");
        };
        assert_eq!(counts.unique_payloads, CLOSURE_STEP_OBJECTS);
        assert!(session.current_object.is_none());
        assert!(session.prior_payload.is_none());

        counts = step_counts();
        assert!(session
            .apply_frame(frame, &mut counts)
            .expect("payload begins in next step")
            .is_none());
        assert!(session.current_object.is_some());

        session.current_object = None;
        counts.unique_payloads = CLOSURE_STEP_OBJECTS;
        let duplicate = member_header(schema_id, empty_digest, 0);
        assert!(matches!(
            session.apply_frame(DecodedFrame::ObjectBegin(duplicate), &mut counts),
            Err(ReplayError::InvalidExport)
        ));
    }

    #[test]
    fn exact_joint_counter_equality_keeps_root_and_terminal_checks_in_the_same_step() {
        let root_run_id = run_id(0);
        let mut session = test_session();
        session.header = Some(stream_header(root_run_id.clone()));
        session.phase = StreamParsePhase::Runs;
        let mut counts = StepCounts {
            remaining_stream_bytes: 0,
            dependency_sources: CLOSURE_STEP_SOURCES,
            unique_payloads: CLOSURE_STEP_OBJECTS,
        };
        assert!(session
            .apply_frame(
                DecodedFrame::RunBegin {
                    run_id: root_run_id.clone(),
                },
                &mut counts,
            )
            .expect("root is excluded from dependency accounting")
            .is_none());
        assert_eq!(counts.dependency_sources, CLOSURE_STEP_SOURCES);
        assert_eq!(counts.unique_payloads, CLOSURE_STEP_OBJECTS);

        session.current_run = None;
        session.runs.push(ParsedRun {
            run_id: root_run_id,
            commits: Vec::new(),
        });
        session.phase = StreamParsePhase::Objects;
        assert!(session
            .apply_frame(DecodedFrame::End, &mut counts)
            .expect("zero-cost terminal checks at exact equality")
            .is_none());
        assert!(matches!(session.phase, StreamParsePhase::Ended));
        assert_eq!(counts.remaining_stream_bytes, 0);
        assert_eq!(counts.dependency_sources, CLOSURE_STEP_SOURCES);
        assert_eq!(counts.unique_payloads, CLOSURE_STEP_OBJECTS);
    }

    #[test]
    fn every_authority_is_retained_without_counting_as_another_payload() {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let schema_id = contract
            .schema_id("mfm.access-audit-entry.v1")
            .expect("authority payload schema")
            .clone();
        let mut session = test_session();
        session.phase = StreamParsePhase::Objects;
        session
            .start_member(
                member_header(schema_id, contract.raw_content_digest(&[]), 0),
                ActiveMemberTarget::Object,
            )
            .expect("begin empty authority fixture");
        let mut counts = StepCounts {
            remaining_stream_bytes: 0,
            dependency_sources: CLOSURE_STEP_SOURCES,
            unique_payloads: CLOSURE_STEP_OBJECTS,
        };
        let first = sample_value_ref("sample");
        let second = sample_value_ref("sample2");
        for value_ref in [first, second] {
            assert!(session
                .apply_frame(DecodedFrame::ObjectAuthority { value_ref }, &mut counts)
                .expect("authority does not consume a payload unit")
                .is_none());
            assert_eq!(counts.unique_payloads, CLOSURE_STEP_OBJECTS);
        }
        assert_eq!(
            session
                .current_object
                .as_ref()
                .expect("active object")
                .value_refs
                .len(),
            2
        );
    }

    #[test]
    fn source_cycle_is_rejected_without_a_total_node_cap() {
        let first = run_id(1);
        let second = run_id(2);
        let cyclic = BTreeMap::from([
            (first.clone(), BTreeSet::from([second.clone()])),
            (second.clone(), BTreeSet::from([first.clone()])),
        ]);
        assert!(matches!(
            reject_source_cycles(&cyclic, cyclic.keys()),
            Err(ReplayError::InvalidExport)
        ));

        let acyclic = BTreeMap::from([
            (first.clone(), BTreeSet::from([second.clone()])),
            (second, BTreeSet::new()),
        ]);
        reject_source_cycles(&acyclic, acyclic.keys()).expect("acyclic source graph");
    }

    #[test]
    fn every_run_in_one_export_must_share_the_root_store_epoch() {
        let mut observed = None;
        observe_common_store_epoch(&mut observed, StoreEpoch::new(1))
            .expect("first epoch establishes lineage");
        observe_common_store_epoch(&mut observed, StoreEpoch::new(1))
            .expect("same epoch remains valid");
        assert!(matches!(
            observe_common_store_epoch(&mut observed, StoreEpoch::new(2)),
            Err(ReplayError::InvalidExport)
        ));
    }

    #[test]
    fn active_member_accepts_max_chunks_and_rejects_chunk_boundary_failures() {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let schema_id = contract
            .schema_id("mfm.primitive-canonical_value.v1")
            .expect("canonical value schema")
            .clone();
        let bytes = vec![0x5a; MAX_STREAM_CHUNK_BYTES * 2 + 1];
        let mut session = test_session();
        session
            .start_member(
                member_header(
                    schema_id.clone(),
                    contract.raw_content_digest(&bytes),
                    bytes.len() as u64,
                ),
                ActiveMemberTarget::Object,
            )
            .expect("begin legal multichunk object");
        session
            .append_chunk(0, &bytes[..MAX_STREAM_CHUNK_BYTES])
            .expect("first maximum chunk");
        session
            .append_chunk(
                MAX_STREAM_CHUNK_BYTES,
                &bytes[MAX_STREAM_CHUNK_BYTES..MAX_STREAM_CHUNK_BYTES * 2],
            )
            .expect("second maximum chunk");
        session
            .append_chunk(
                MAX_STREAM_CHUNK_BYTES * 2,
                &bytes[MAX_STREAM_CHUNK_BYTES * 2..],
            )
            .expect("terminal partial chunk");
        assert_eq!(
            session
                .current_object
                .as_ref()
                .expect("completed object")
                .bytes
                .as_ref(),
            bytes
        );

        for (offset, chunk) in [
            (0, Vec::new()),
            (0, vec![0; MAX_STREAM_CHUNK_BYTES + 1]),
            (1, vec![0]),
        ] {
            let mut session = test_session();
            session
                .start_member(
                    member_header(schema_id.clone(), content_digest(2), 1),
                    ActiveMemberTarget::Object,
                )
                .expect("begin invalid-chunk case");
            assert!(matches!(
                session.append_chunk(offset, &chunk),
                Err(ReplayError::InvalidExport)
            ));
        }

        let mut overlap = test_session();
        overlap
            .start_member(
                member_header(schema_id.clone(), content_digest(3), 2),
                ActiveMemberTarget::Object,
            )
            .expect("begin overlap case");
        overlap.append_chunk(0, &[0]).expect("first byte");
        assert!(matches!(
            overlap.append_chunk(0, &[0]),
            Err(ReplayError::InvalidExport)
        ));

        let mut over_length = test_session();
        over_length
            .start_member(
                member_header(schema_id.clone(), content_digest(4), 1),
                ActiveMemberTarget::Object,
            )
            .expect("begin over-length case");
        assert!(matches!(
            over_length.append_chunk(0, &[0, 1]),
            Err(ReplayError::InvalidExport)
        ));

        let mut wrong_digest = test_session();
        wrong_digest
            .start_member(
                member_header(schema_id, content_digest(5), 1),
                ActiveMemberTarget::Object,
            )
            .expect("begin digest case");
        assert!(matches!(
            wrong_digest.append_chunk(0, &[0]),
            Err(ReplayError::InvalidExport)
        ));
    }

    #[tokio::test]
    async fn discarded_partial_session_restarts_from_immutable_stream_bytes() {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let schema_id = contract
            .schema_id("mfm.primitive-canonical_value.v1")
            .expect("canonical value schema")
            .clone();
        let member = vec![0x5a; 200_000];
        let mut bytes = frame_record(
            export_object([
                ("kind", export_string("run_begin")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("run_id", export_string(run_id(0).as_str())),
            ])
            .expect("run frame"),
        );
        bytes.extend(frame_record(
            export_object([
                ("kind", export_string("commit_begin")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("run_sequence", export_string("1")),
                ("schema_id", export_string(schema_id.as_str())),
                (
                    "content_digest",
                    export_string(contract.raw_content_digest(&member).as_str()),
                ),
                ("byte_length", export_string(member.len().to_string())),
            ])
            .expect("member frame"),
        ));
        for (index, chunk) in member.chunks(MAX_STREAM_CHUNK_BYTES).enumerate() {
            bytes.extend(frame_record(
                export_object([
                    ("kind", export_string("chunk")),
                    ("version", export_string(PORTABLE_FRAME_VERSION)),
                    (
                        "offset",
                        export_string((index * MAX_STREAM_CHUNK_BYTES).to_string()),
                    ),
                    (
                        "bytes",
                        mfm_canonical::CanonicalValue::Bytes(mfm_canonical::CanonicalBytes::new(
                            chunk.to_vec(),
                        )),
                    ),
                ])
                .expect("chunk frame"),
            ));
        }
        assert!(bytes.len() > CLOSURE_STEP_STREAM_BYTES);

        let first = configured_partial_session(bytes.as_slice());
        let More::More(partial) = first.step().await.expect("first bounded step") else {
            panic!("stream must remain incomplete at the byte boundary");
        };
        let first_progress = partial
            .active_member
            .as_ref()
            .expect("partially retained member")
            .bytes
            .len();
        assert!(first_progress > 0);
        assert!(first_progress < member.len());
        drop(partial);

        let restarted = configured_partial_session(bytes.as_slice());
        let More::More(partial) = restarted.step().await.expect("restarted bounded step") else {
            panic!("restart must reach the same incomplete boundary");
        };
        assert_eq!(
            partial
                .active_member
                .as_ref()
                .expect("restarted partial member")
                .bytes
                .len(),
            first_progress
        );
    }

    fn stream_ref(bytes: &[u8]) -> ContentRef {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        ContentRef::new(
            contract
                .schema_id("mfm.portable-run-export-stream.v1")
                .expect("stream schema")
                .clone(),
            contract.raw_content_digest(bytes),
        )
        .expect("stream reference")
    }

    fn frame_record(value: mfm_canonical::CanonicalValue) -> Vec<u8> {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let frame = contract
            .encode(PORTABLE_FRAME_CONTRACT, &value)
            .expect("valid portable frame");
        let mut record = Vec::with_capacity(frame.as_bytes().len() + 2);
        record.push(RECORD_SEPARATOR);
        record.extend_from_slice(frame.as_bytes());
        record.push(RECORD_SUFFIX);
        record
    }

    fn accept_staged_frame<R: AsyncRead + Unpin>(
        reader: &mut FramedStreamReader<R>,
        mut staged: StagedFrame,
        remaining: &mut usize,
    ) -> DecodedFrame {
        staged.accepted = true;
        assert!(
            reader.commit_frame(&mut staged, remaining),
            "test frame must fit in one bounded step"
        );
        let StagedFrame {
            frame,
            record,
            committed: _,
            accepted: _,
        } = staged;
        reader.recycle_record(record);
        frame
    }

    fn append_empty_run(bytes: &mut Vec<u8>, run_id: &RunId) {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let schema_id = contract
            .schema_id("mfm.primitive-canonical_value.v1")
            .expect("canonical value schema");
        let content_digest = contract.raw_content_digest(&[]);
        bytes.extend(frame_record(
            export_object([
                ("kind", export_string("run_begin")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("run_id", export_string(run_id.as_str())),
            ])
            .expect("run-begin frame"),
        ));
        bytes.extend(frame_record(
            export_object([
                ("kind", export_string("commit_begin")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("run_sequence", export_string("1")),
                ("schema_id", export_string(schema_id.as_str())),
                ("content_digest", export_string(content_digest.as_str())),
                ("byte_length", export_string("0")),
            ])
            .expect("commit-begin frame"),
        ));
        bytes.extend(frame_record(
            export_object([
                ("kind", export_string("record_begin")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("run_sequence", export_string("1")),
                ("ordinal", mfm_canonical::CanonicalValue::Unsigned(0)),
                ("schema_id", export_string(schema_id.as_str())),
                ("content_digest", export_string(content_digest.as_str())),
                ("byte_length", export_string("0")),
            ])
            .expect("record-begin frame"),
        ));
        bytes.extend(frame_record(
            export_object([
                ("kind", export_string("run_end")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
            ])
            .expect("run-end frame"),
        ));
    }

    fn append_empty_object(bytes: &mut Vec<u8>, discriminator: u64) {
        bytes.extend(object_begin_record(discriminator));
        bytes.extend(frame_record(
            export_object([
                ("kind", export_string("object_end")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
            ])
            .expect("object-end frame"),
        ));
    }

    fn object_begin_record(discriminator: u64) -> Vec<u8> {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let schema_id = boundary_object_schema_id(discriminator);
        let content_digest = contract.raw_content_digest(&[]);
        frame_record(
            export_object([
                ("kind", export_string("object_begin")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("schema_id", export_string(schema_id.as_str())),
                ("content_digest", export_string(content_digest.as_str())),
                ("byte_length", export_string("0")),
            ])
            .expect("object-begin frame"),
        )
    }

    fn boundary_object_schema_id(discriminator: u64) -> mfm_ids::SchemaId {
        mfm_ids::SchemaId::from_str(&format!(
            "schema:mfm.boundary-object-{discriminator:04}:1:sha256-jcs-v1:{discriminator:064x}"
        ))
        .expect("ordered object schema id")
    }

    fn chunk_frame(bytes: &[u8]) -> mfm_canonical::CanonicalValue {
        export_object([
            ("kind", export_string("chunk")),
            ("version", export_string(PORTABLE_FRAME_VERSION)),
            ("offset", export_string("0")),
            (
                "bytes",
                mfm_canonical::CanonicalValue::Bytes(mfm_canonical::CanonicalBytes::new(
                    bytes.to_vec(),
                )),
            ),
        ])
        .expect("chunk frame")
    }

    fn run_id(discriminator: u64) -> RunId {
        RunId::from_str(&format!("run:sha256-jcs-v1:{discriminator:064x}")).expect("run id")
    }

    fn content_digest(discriminator: u64) -> ContentDigest {
        ContentDigest::from_str(&format!("content:sha256-v1:{discriminator:064x}"))
            .expect("content digest")
    }

    fn stream_header(root_run_id: RunId) -> StreamHeader {
        let commit_digest =
            JournalCommitDigest::from_str(&format!("sha256-jcs-v1:{}", "1".repeat(64)))
                .expect("commit digest");
        StreamHeader {
            store_scope_id: StoreScopeId::from_str(&format!(
                "{}{}",
                StoreScopeId::PREFIX,
                "1".repeat(32)
            ))
            .expect("store scope"),
            tenant_scope_id: TenantScopeId::from_str(&format!(
                "{}{}",
                TenantScopeId::PREFIX,
                "2".repeat(32)
            ))
            .expect("tenant scope"),
            root_run_id,
            export_kind: ExportKind::Audit,
            coordinate: ExportCoordinate::JournalHead {
                journal_head: JournalHead::new(1, &commit_digest).expect("journal head"),
            },
        }
    }

    fn member_header(
        schema_id: mfm_ids::SchemaId,
        content_digest: ContentDigest,
        byte_length: u64,
    ) -> MemberHeader {
        MemberHeader {
            run_sequence: None,
            ordinal: None,
            schema_id,
            content_digest,
            byte_length,
        }
    }

    fn sample_value_ref(field_path: &str) -> ValueRef {
        let bytes = format!(
            concat!(
                r#"{{"artifact_id":"artifact:sha256-jcs-v1:{zero}","#,
                r#""byte_length":"0","#,
                r#""content_digest":"content:sha256-v1:{zero}","#,
                r#""evidence_contract_ref":{{"#,
                r#""content_digest":"content:sha256-v1:{zero}","#,
                r#""schema_id":"schema:mfm.access-audit-entry:1:sha256-jcs-v1:"#,
                r#"894081a808afc17827a4a5b191696979fb2690ac8d66efed8fd8de16829510dc"}},"#,
                r#""evidence_hash":"sha256-jcs-v1:{zero}","#,
                r#""media_type":"application/json","#,
                r#""producer_binding":{{"field_path":"{field_path}","kind":"run_admission","#,
                r#""record_ref":{{"ordinal":0,"record_hash":"sha256-jcs-v1:{zero}","#,
                r#""run_id":"run:sha256-jcs-v1:{zero}","run_sequence":"0"}}}},"#,
                r#""role":"sample","#,
                r#""schema_id":"schema:mfm.access-audit-entry:1:sha256-jcs-v1:"#,
                r#"894081a808afc17827a4a5b191696979fb2690ac8d66efed8fd8de16829510dc","#,
                r#""semantic_type_id":"semantic:mfm:sample:1:sha256-jcs-v1:{zero}"}}"#,
            ),
            zero = "0".repeat(64),
            field_path = field_path,
        );
        ValueRef::strict_decode(bytes.as_bytes()).expect("annex-valid sample authority")
    }

    fn step_counts() -> StepCounts {
        StepCounts {
            remaining_stream_bytes: CLOSURE_STEP_STREAM_BYTES,
            dependency_sources: 0,
            unique_payloads: 0,
        }
    }

    fn test_session() -> ClosureVerificationSession<&'static [u8]> {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let schema_id = contract
            .schema_id("mfm.portable-run-export-stream.v1")
            .expect("stream schema")
            .clone();
        ClosureVerificationSession::new(
            &[],
            stream_ref(&[]),
            schema_id,
            contract.raw_content_digest_hasher(),
        )
    }

    fn configured_partial_session(bytes: &[u8]) -> Box<ClosureVerificationSession<&[u8]>> {
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        let schema_id = contract
            .schema_id("mfm.portable-run-export-stream.v1")
            .expect("stream schema")
            .clone();
        let mut session = ClosureVerificationSession::new(
            bytes,
            stream_ref(bytes),
            schema_id,
            contract.raw_content_digest_hasher(),
        );
        session.header = Some(stream_header(run_id(0)));
        session.phase = StreamParsePhase::Runs;
        Box::new(session)
    }

    fn configured_source_session(bytes: &[u8]) -> Box<ClosureVerificationSession<&[u8]>> {
        let mut session = configured_partial_session(bytes);
        session.runs.push(ParsedRun {
            run_id: run_id(0),
            commits: Vec::new(),
        });
        session
    }

    fn configured_object_session(bytes: &[u8]) -> Box<ClosureVerificationSession<&[u8]>> {
        let mut session = configured_source_session(bytes);
        session.phase = StreamParsePhase::Objects;
        session
    }

    fn assert_staged_run_begin(
        session: &ClosureVerificationSession<&[u8]>,
        bytes: &[u8],
        start: usize,
        discriminator: u64,
    ) {
        let expected_run_id = run_id(discriminator);
        let expected_record = frame_record(
            export_object([
                ("kind", export_string("run_begin")),
                ("version", export_string(PORTABLE_FRAME_VERSION)),
                ("run_id", export_string(expected_run_id.as_str())),
            ])
            .expect("run-begin frame"),
        );
        let staged = session.pending_frame.as_ref().expect("staged run frame");
        assert!(!staged.accepted);
        assert_eq!(staged.committed, 0);
        assert!(matches!(
            &staged.frame,
            DecodedFrame::RunBegin { run_id } if run_id == &expected_run_id
        ));
        assert_eq!(staged.record, expected_record);
        assert_committed_prefix(session, bytes, start);
    }

    fn assert_staged_object_begin(
        session: &ClosureVerificationSession<&[u8]>,
        bytes: &[u8],
        start: usize,
        discriminator: u64,
    ) {
        let expected_schema_id = boundary_object_schema_id(discriminator);
        let staged = session.pending_frame.as_ref().expect("staged object frame");
        assert!(!staged.accepted);
        assert_eq!(staged.committed, 0);
        assert!(matches!(
            &staged.frame,
            DecodedFrame::ObjectBegin(MemberHeader { schema_id, .. })
                if schema_id == &expected_schema_id
        ));
        assert_committed_prefix(session, bytes, start);
    }

    fn assert_committed_prefix(
        session: &ClosureVerificationSession<&[u8]>,
        bytes: &[u8],
        end: usize,
    ) {
        let committed = session.input.committed_bytes();
        assert_eq!(committed, &bytes[..end]);
        let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
        assert_eq!(
            contract.raw_content_digest(committed),
            contract.raw_content_digest(&bytes[..end])
        );
        assert_ne!(
            contract.raw_content_digest(committed),
            contract.raw_content_digest(bytes)
        );
    }

    fn assert_stream_io(error: ReplayError) {
        assert!(matches!(error, ReplayError::ExportStreamIo { .. }));
        assert_eq!(error.code(), "MFM_REPLAY_EXPORT_STREAM_IO_FAILED");
        assert_eq!(error.to_string(), "portable run export stream I/O failed");
        assert!(error.source().is_some());
        assert!(!error.to_string().contains("sentinel"));
        assert!(!error.to_string().contains("/private/path"));
    }

    struct ShortReader {
        bytes: Vec<u8>,
        position: usize,
        max_read: usize,
    }

    impl ShortReader {
        fn new(bytes: Vec<u8>, max_read: usize) -> Self {
            Self {
                bytes,
                position: 0,
                max_read,
            }
        }
    }

    impl AsyncRead for ShortReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _context: &mut TaskContext<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let count = self
                .max_read
                .min(buffer.remaining())
                .min(self.bytes.len().saturating_sub(self.position));
            if count != 0 {
                let end = self.position + count;
                buffer.put_slice(&self.bytes[self.position..end]);
                self.position = end;
            }
            Poll::Ready(Ok(()))
        }
    }

    struct FailingReader;

    impl AsyncRead for FailingReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut TaskContext<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::other("sentinel /private/path")))
        }
    }

    #[derive(Clone, Copy)]
    enum WriterFailure {
        Write,
        Flush,
        Shutdown,
    }

    struct FailingWriter {
        failure: WriterFailure,
    }

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut TaskContext<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            if matches!(self.failure, WriterFailure::Write) {
                Poll::Ready(Err(io::Error::other("sentinel /private/path")))
            } else {
                Poll::Ready(Ok(buffer.len()))
            }
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut TaskContext<'_>,
        ) -> Poll<io::Result<()>> {
            if matches!(self.failure, WriterFailure::Flush) {
                Poll::Ready(Err(io::Error::other("sentinel /private/path")))
            } else {
                Poll::Ready(Ok(()))
            }
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _context: &mut TaskContext<'_>,
        ) -> Poll<io::Result<()>> {
            if matches!(self.failure, WriterFailure::Shutdown) {
                Poll::Ready(Err(io::Error::other("sentinel /private/path")))
            } else {
                Poll::Ready(Ok(()))
            }
        }
    }
}
