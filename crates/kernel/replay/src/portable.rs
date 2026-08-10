//! Portable structured-run frame streams and offline verification.
//!
//! `mfm-replay` owns the only portable format, frame validation, digest rules,
//! and offline validator. The application supplies sealed export evidence and
//! never constructs or interprets the wire frames.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, RunId, SchemaId, StableId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, CommittedBatch, JournalHead, RecordRef, SemanticHead, TenantFactCoordinate,
    TenantFactFrontier,
};
use mfm_store::structured::{
    verify_offline_recorded_history, ExportEncoderView, ExportRunEvidence, PhysicalTargetIdentity,
    ProgramVerifier, PublicPhysicalBindingVerifier, RawRunHistory, RecordedRunEvidence,
    StructuredStoreError,
};
use mfm_values::{
    CanonicalJsonLinesBounds, CanonicalJsonProfile, EnumTagging, EnumVariantDescriptor,
    FieldDefaultPolicy, FieldDescriptor, PersistedSchema, SchemaIdentity, SchemaKind, SchemaShape,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::structured::{project_replay_result, StructuredReplayError, StructuredReplayResult};

struct ReplayEncoderConsumer;

impl mfm_authority_seal::ExportEncoderConsumerSeal for ReplayEncoderConsumer {}

/// Opaque, bounded authorization closure consumed by portable export construction.
///
/// The application supplies one content-addressed policy decision for the root and for every
/// recursively retained source prefix. The closure validates that the decision set exactly
/// matches the sealed evidence before any frame bytes are built.
pub struct AuthorizedExportClosure {
    evidence: ExportRunEvidence,
    principal_id: StableId,
    root_decision_ref: ContentDigest,
    source_decision_refs: BTreeMap<RunId, ContentDigest>,
}

impl fmt::Debug for AuthorizedExportClosure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedExportClosure")
            .field("principal_id", &self.principal_id)
            .field("source_run_count", &self.source_decision_refs.len())
            .finish()
    }
}

impl AuthorizedExportClosure {
    /// Seals policy-decision references to one already-authorized recursive evidence closure.
    pub fn new(
        evidence: ExportRunEvidence,
        principal_id: StableId,
        root_decision_ref: ContentDigest,
        source_decision_refs: BTreeMap<RunId, ContentDigest>,
    ) -> Result<Self, PortableExportError> {
        if source_decision_refs.len() > MAX_PORTABLE_SOURCE_RUNS {
            return Err(PortableExportError::TooLarge);
        }
        let source_ids = evidence.with_encoder_view(ReplayEncoderConsumer, |view| {
            view.authorized_source_prefixes()
                .map(|source| source.run_id().clone())
                .collect::<BTreeSet<_>>()
        });
        if source_ids
            != source_decision_refs
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>()
        {
            return Err(PortableExportError::Invalid);
        }
        Ok(Self {
            evidence,
            principal_id,
            root_decision_ref,
            source_decision_refs,
        })
    }

    fn with_encoder_view<T>(&self, f: impl FnOnce(ExportEncoderView<'_>) -> T) -> T {
        self.evidence.with_encoder_view(ReplayEncoderConsumer, f)
    }

    fn portable_export_decisions(
        &self,
        root_run_id: &RunId,
        source_run_ids: &[RunId],
    ) -> Vec<PortableAuthorizationDecision> {
        let mut decisions = Vec::with_capacity(source_run_ids.len() + 1);
        decisions.push(PortableAuthorizationDecision {
            decision_ref: self.root_decision_ref.clone(),
            grant: PORTABLE_EXPORT_GRANT.to_owned(),
            principal_id: self.principal_id.clone(),
            run_id: root_run_id.clone(),
        });
        decisions.extend(source_run_ids.iter().filter_map(|run_id| {
            self.source_decision_refs.get(run_id).map(|decision_ref| {
                PortableAuthorizationDecision {
                    decision_ref: decision_ref.clone(),
                    grant: PORTABLE_EXPORT_GRANT.to_owned(),
                    principal_id: self.principal_id.clone(),
                    run_id: run_id.clone(),
                }
            })
        }));
        decisions
    }
}

/// Exact media type of the current bounded structured portable frame stream.
pub const PORTABLE_RUN_EXPORT_MEDIA_TYPE: &str =
    "application/vnd.mfm.structured-run-export-stream.v3";

/// Current portable-export contract version retained in the terminal seal.
pub const PORTABLE_EXPORT_VERSION: &str = "mfm.structured-portable-run-export-stream.v3";

/// The one complete portable frame stream, owned by this crate.
///
/// A frame is an internal typed record of this stream codec; it carries no
/// independent identity. Only the complete stream has a `ContentRef`, so its
/// declared encoding is the bounded JSON-lines form and its record shape is the
/// closed `batch`/`seal` frame union.
struct PortableRunExportStream;

impl PersistedSchema for PortableRunExportStream {
    fn schema_identity() -> mfm_values::Result<SchemaIdentity> {
        SchemaIdentity::new_canonical_json_lines(
            SchemaKind::PersistedContract,
            "mfm.portable-run-export-stream",
            mfm_ids::SchemaVersion::new("3")
                .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))?,
            SchemaShape::Struct {
                fields: vec![
                    FieldDescriptor {
                        name: "kind".to_owned(),
                        shape: SchemaShape::Enum {
                            tagging: EnumTagging::External,
                            variants: vec![
                                EnumVariantDescriptor {
                                    name: "batch".to_owned(),
                                    shape: SchemaShape::Unit,
                                },
                                EnumVariantDescriptor {
                                    name: "seal".to_owned(),
                                    shape: SchemaShape::Unit,
                                },
                            ],
                        },
                        default: FieldDefaultPolicy::Required,
                    },
                    FieldDescriptor {
                        name: "payload".to_owned(),
                        shape: SchemaShape::CanonicalJsonTerminal {
                            profile: CanonicalJsonProfile::GeneralFloatFree,
                        },
                        default: FieldDefaultPolicy::Required,
                    },
                ],
            },
            CanonicalJsonLinesBounds {
                minimum_records: 2,
                maximum_records: u32::try_from(MAX_PORTABLE_FRAMES).unwrap_or(u32::MAX),
                maximum_framed_record_bytes: u32::try_from(MAX_PORTABLE_FRAME_BYTES)
                    .unwrap_or(u32::MAX),
                maximum_stream_bytes: MAX_PORTABLE_EXPORT_BYTES,
            },
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        Ok(())
    }
}

fn portable_stream_schema_id() -> Result<SchemaId, PortableExportError> {
    <PortableRunExportStream as PersistedSchema>::schema_id()
        .map_err(|_| PortableExportError::Invalid)
}

/// The exact encoded portable stream together with its complete-stream identity.
///
/// Encoding happens once: the bytes and their `ContentRef` are produced by the
/// same pass and exposed read-only, so no caller can pair one stream's bytes
/// with another stream's reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPortableRunExport {
    bytes: Vec<u8>,
    content_ref: ContentRef,
}

impl EncodedPortableRunExport {
    /// Returns the exact encoded newline-delimited canonical frame stream.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the complete-stream content reference of these exact bytes.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    /// Consumes this encoding and returns its exact bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

const PORTABLE_EXPORT_GRANT: &str = "export";

/// Maximum bytes in one complete portable frame stream.
pub const MAX_PORTABLE_EXPORT_BYTES: u64 = 16777216;

/// Maximum bytes in one framed portable record, including its delimiter.
pub const MAX_PORTABLE_FRAME_BYTES: usize = 1048576;

/// Maximum frames in one complete portable stream.
pub const MAX_PORTABLE_FRAMES: usize = 1048577;

/// Maximum batches carried by one portable stream.
pub const MAX_PORTABLE_BATCHES: usize = 1048576;

/// Maximum retained objects carried by one portable stream.
pub const MAX_PORTABLE_OBJECTS: usize = 1048576;

pub use mfm_store::structured::{MAX_PORTABLE_FACT_ROUTES, MAX_PORTABLE_SOURCE_RUNS};

/// Closed portable-export scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    /// Export through the current semantic head, including every full atomic batch selected.
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

/// Explicit semantic and physical fixation retained by one portable export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableFixation {
    /// Semantic head fixed by the export.
    pub semantic_head: SemanticHead,
    /// Physical journal head fixed by the export.
    pub journal_head: JournalHead,
    /// Store lineage of every included batch.
    pub store_scope_id: StoreScopeId,
    /// Writer epoch of every included batch.
    pub store_epoch: StoreEpoch,
    /// Tenant scope authorized for this exact portable closure.
    pub tenant_scope_id: TenantScopeId,
    /// Exact physical target, fence generation, release, and current incarnation.
    pub physical_target: PhysicalTargetIdentity,
}

/// One strict portable export assembled from sealed evidence.
///
/// The fields are deliberately private. Callers can request bytes or an
/// offline verification result, but cannot inspect or rewrite export content.
#[derive(Clone, PartialEq, Eq)]
pub struct PortableRunExport {
    version: String,
    kind: ExportKind,
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    fixation: PortableFixation,
    source_run_ids: Vec<RunId>,
    source_prefixes: Vec<PortableRunPrefix>,
    batches: Vec<CommittedBatch>,
    fact_routes: Vec<PortableFactRoute>,
    export_decisions: Vec<PortableAuthorizationDecision>,
    closure_reference: ContentDigest,
}

impl fmt::Debug for PortableRunExport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PortableRunExport")
            .field("version", &self.version)
            .field("kind", &self.kind)
            .field("store_scope_id", &self.store_scope_id)
            .field("tenant_scope_id", &self.tenant_scope_id)
            .field("run_id", &self.run_id)
            .field("source_run_count", &self.source_run_ids.len())
            .field("batch_count", &self.batches.len())
            .field("closure_reference", &self.closure_reference)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortableRunPrefix {
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
    fixation: PortableFixation,
    batches: Vec<CommittedBatch>,
    fact_frontiers: Vec<TenantFactFrontier>,
    fact_routes: Vec<PortableFactRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableFactRoute {
    consumer_record: RecordRef,
    producer_transition: RecordRef,
    publication_frontier: TenantFactFrontier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableAuthorizationDecision {
    decision_ref: ContentDigest,
    grant: String,
    principal_id: StableId,
    run_id: RunId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PortableFrameKind {
    Batch,
    Seal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableFrame {
    kind: PortableFrameKind,
    payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableBatchPayload {
    batch: CommittedBatch,
    run_id: RunId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableRunFixation {
    fact_frontiers: Vec<TenantFactFrontier>,
    fact_routes: Vec<PortableFactRoute>,
    fixation: PortableFixation,
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableSeal {
    /// Export authorization decisions, named without the persisted-surface
    /// secret marker `authorization` so untrusted stream bytes stay checkable
    /// under the strict canonical-terminal key policy.
    export_decisions: Vec<PortableAuthorizationDecision>,
    closure_reference: ContentDigest,
    fixation: PortableFixation,
    kind: ExportKind,
    root_run_id: RunId,
    run_fixations: Vec<PortableRunFixation>,
    source_run_ids: Vec<RunId>,
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    version: String,
}

#[derive(Serialize)]
struct ClosureDigestBody<'a> {
    export_decisions: &'a [PortableAuthorizationDecision],
    fixation: &'a PortableFixation,
    kind: ExportKind,
    root_run_id: &'a RunId,
    run_fixations: &'a [PortableRunFixation],
    source_run_ids: &'a [RunId],
    store_scope_id: &'a StoreScopeId,
    tenant_scope_id: &'a TenantScopeId,
    version: &'a str,
}

impl PortableRunExport {
    /// Builds one export from an opaque, policy-authorized recursive closure.
    pub fn from_authorized_export_closure(
        closure: &AuthorizedExportClosure,
        kind: ExportKind,
    ) -> Result<Self, PortableExportError> {
        closure.with_encoder_view(|view| {
            let root_run_id = view.run_id().clone();
            Self::from_encoder_view(view, kind, |source_run_ids| {
                closure.portable_export_decisions(&root_run_id, source_run_ids)
            })
        })
    }

    fn from_encoder_view(
        view: ExportEncoderView<'_>,
        kind: ExportKind,
        export_decisions_for: impl FnOnce(&[RunId]) -> Vec<PortableAuthorizationDecision>,
    ) -> Result<Self, PortableExportError> {
        let batches = select_batches(&view, kind)?;
        if batches.is_empty() {
            return Err(PortableExportError::Invalid);
        }
        let first = &batches[0];
        let last = batches.last().expect("non-empty batches");
        let store_scope_id = first.store_scope_id.clone();
        let store_epoch = first.store_epoch;
        for batch in &batches {
            if batch.store_scope_id != store_scope_id || batch.store_epoch != store_epoch {
                return Err(PortableExportError::Invalid);
            }
        }
        let root_cutoff = batches
            .last()
            .map(|batch| batch.head.run_sequence)
            .ok_or(PortableExportError::Invalid)?;
        let root_fact_routes = view
            .fact_routes()
            .iter()
            .filter(|route| route.consumer_record().run_sequence <= root_cutoff)
            .map(portable_fact_route)
            .collect::<Vec<_>>();
        let sources = view.authorized_source_prefixes().collect::<Vec<_>>();
        let mut required_source_heads = BTreeMap::<RunId, u64>::new();
        for route in view.fact_routes() {
            if route.consumer_record().run_sequence <= root_cutoff
                && route.producer_transition().run_id != *view.run_id()
            {
                required_source_heads
                    .entry(route.producer_transition().run_id.clone())
                    .and_modify(|head| {
                        *head = (*head).max(route.producer_transition().run_sequence)
                    })
                    .or_insert(route.producer_transition().run_sequence);
            }
        }
        // Expand the route graph using the same maximum-required-head rule for
        // every producer. A producer is folded once at its largest routed
        // transition, never once per selected fact.
        let mut changed = true;
        while changed {
            changed = false;
            for source in &sources {
                let Some(required_head) = required_source_heads.get(source.run_id()).copied()
                else {
                    continue;
                };
                for route in source.fact_routes() {
                    if route.consumer_record().run_sequence <= required_head
                        && route.producer_transition().run_id != *source.run_id()
                    {
                        let entry = required_source_heads
                            .entry(route.producer_transition().run_id.clone())
                            .or_insert(0);
                        if *entry < route.producer_transition().run_sequence {
                            *entry = route.producer_transition().run_sequence;
                            changed = true;
                        }
                    }
                }
            }
        }
        let mut source_run_ids = required_source_heads.keys().cloned().collect::<Vec<_>>();
        source_run_ids.sort();
        let export_decisions = export_decisions_for(&source_run_ids);
        let source_prefixes = sources
            .into_iter()
            .filter(|source| required_source_heads.contains_key(source.run_id()))
            .map(|source| {
                let required_head = required_source_heads.get(source.run_id()).copied();
                portable_prefix_from_source(
                    source,
                    required_head,
                    matches!(kind, ExportKind::Semantic),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if source_run_ids.len() > MAX_PORTABLE_SOURCE_RUNS {
            return Err(PortableExportError::TooLarge);
        }
        let fixation = PortableFixation {
            semantic_head: view.semantic_head().clone(),
            journal_head: last.head.clone(),
            store_scope_id: store_scope_id.clone(),
            store_epoch,
            tenant_scope_id: view.header().tenant_scope_id().clone(),
            physical_target: view.physical_target().clone(),
        };
        let mut export = Self {
            version: PORTABLE_EXPORT_VERSION.to_owned(),
            kind,
            store_scope_id,
            tenant_scope_id: view.header().tenant_scope_id().clone(),
            run_id: view.run_id().clone(),
            fixation,
            source_run_ids,
            source_prefixes,
            batches,
            fact_routes: root_fact_routes,
            export_decisions,
            closure_reference: placeholder_digest(),
        };
        export.closure_reference = export.compute_closure_reference()?;
        export.validate_structure()?;
        Ok(export)
    }

    /// Encodes the exact bounded newline-delimited canonical frame stream and
    /// its complete-stream content reference together.
    pub fn encode(&self) -> Result<EncodedPortableRunExport, PortableExportError> {
        self.validate_structure()?;
        let bytes = encode_frames(self)?;
        let content_ref = ContentRef::new(portable_stream_schema_id()?, raw_digest(&bytes))
            .map_err(|_| PortableExportError::Invalid)?;
        Ok(EncodedPortableRunExport { bytes, content_ref })
    }

    /// Returns the exact recursive source-closure identity authorized by an
    /// external trust snapshot.
    pub const fn closure_reference(&self) -> &ContentDigest {
        &self.closure_reference
    }

    /// Returns the number of recursively authorized source prefixes in this export.
    pub const fn source_run_count(&self) -> usize {
        self.source_run_ids.len()
    }

    /// Strictly decodes and validates one frame stream without ambient IO.
    ///
    /// Structural decode is not the production door: a caller that trusts a
    /// stream must go through [`Self::verify_offline`], which proves the
    /// complete-stream identity first.
    #[cfg(feature = "test-support")]
    pub fn strict_decode(bytes: &[u8]) -> Result<Self, PortableExportError> {
        decode_frames(bytes)
    }

    #[cfg(not(feature = "test-support"))]
    fn strict_decode(bytes: &[u8]) -> Result<Self, PortableExportError> {
        decode_frames(bytes)
    }

    /// Offline verification using only bundle bytes, the caller's expected
    /// complete-stream identity, and an explicit trust snapshot.
    ///
    /// The expected reference is compared against the bytes before anything is
    /// decoded, so substituted bytes are rejected by identity rather than by a
    /// structural coincidence.
    pub fn verify_offline(
        bytes: &[u8],
        expected_content_ref: &ContentRef,
        trust: &ReplayTrustSnapshot<'_>,
    ) -> Result<StructuredReplayResult, PortableExportError> {
        let observed = ContentRef::new(portable_stream_schema_id()?, raw_digest(bytes))
            .map_err(|_| PortableExportError::Invalid)?;
        if &observed != expected_content_ref {
            return Err(PortableExportError::Invalid);
        }
        let export = Self::strict_decode(bytes)?;
        let evidence = export.fold_with_trust(trust)?;
        project_replay_result(&evidence).map_err(|_| PortableExportError::Invalid)
    }

    /// Folds this export through the store's sole offline entry point.
    pub fn fold_with_trust(
        &self,
        trust: &ReplayTrustSnapshot<'_>,
    ) -> Result<RecordedRunEvidence, PortableExportError> {
        self.validate_structure()?;
        if trust.authorized_closure_reference != Some(&self.closure_reference) {
            return Err(PortableExportError::Invalid);
        }
        let Some(retained_release_trust) = trust.retained_release_trust else {
            return Err(PortableExportError::Invalid);
        };
        let Some(store_lineage_trust) = trust.store_lineage_trust else {
            return Err(PortableExportError::Invalid);
        };
        for fixation in self.run_fixations() {
            if !retained_release_trust.verify(&fixation.fixation, self.kind)
                || !store_lineage_trust.verify(
                    &fixation.fixation,
                    self.kind,
                    &self.closure_reference,
                )
            {
                return Err(PortableExportError::Invalid);
            }
        }
        let verified = verify_offline_recorded_history(
            RawRunHistory {
                run_id: self.run_id.clone(),
                batches: self.batches.clone(),
            },
            trust.program_verifier,
            trust.physical_binding_verifier,
        )
        .map_err(classify_fold_error)?;
        if portable_routes(verified.fact_routes()) != self.fact_routes {
            return Err(PortableExportError::Invalid);
        }
        let mut verified_sources = BTreeMap::new();
        for prefix in &self.source_prefixes {
            let source = verify_offline_recorded_history(
                RawRunHistory {
                    run_id: prefix.run_id.clone(),
                    batches: prefix.batches.clone(),
                },
                trust.program_verifier,
                trust.physical_binding_verifier,
            )
            .map_err(classify_fold_error)?;
            if portable_routes(source.fact_routes()) != prefix.fact_routes {
                return Err(PortableExportError::Invalid);
            }
            if source.run_id() != &prefix.run_id
                || source.tenant_scope_id() != &self.tenant_scope_id
                || source.journal_head() != &prefix.fixation.journal_head
                || source.semantic_head() != &prefix.fixation.semantic_head
                || verified_sources
                    .insert(prefix.run_id.clone(), BTreeSet::new())
                    .is_some()
            {
                return Err(PortableExportError::Invalid);
            }
            let nested = source.direct_source_run_ids().clone();
            if nested
                .iter()
                .any(|run_id| run_id == &self.run_id || run_id == &prefix.run_id)
            {
                return Err(PortableExportError::Invalid);
            }
            verified_sources.insert(prefix.run_id.clone(), nested);
        }
        let root_sources = verified.direct_source_run_ids().clone();
        if root_sources
            .iter()
            .any(|run_id| !self.source_run_ids.contains(run_id))
        {
            return Err(PortableExportError::Invalid);
        }
        let mut source_graph = verified_sources.clone();
        source_graph.insert(self.run_id.clone(), root_sources.clone());
        let mut colors = BTreeMap::<RunId, u8>::new();
        for start in source_graph.keys().cloned().collect::<Vec<_>>() {
            if colors.get(&start).copied().unwrap_or_default() != 0 {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((run_id, exiting)) = stack.pop() {
                if exiting {
                    colors.insert(run_id, 2);
                    continue;
                }
                match colors.get(&run_id).copied().unwrap_or_default() {
                    1 => return Err(PortableExportError::Invalid),
                    2 => continue,
                    _ => {}
                }
                colors.insert(run_id.clone(), 1);
                stack.push((run_id.clone(), true));
                let children = source_graph
                    .get(&run_id)
                    .ok_or(PortableExportError::Invalid)?;
                for child in children.iter().rev() {
                    if !source_graph.contains_key(child) {
                        return Err(PortableExportError::Invalid);
                    }
                    stack.push((child.clone(), false));
                }
            }
        }
        let mut reachable = BTreeSet::new();
        let mut pending = root_sources;
        while let Some(run_id) = pending.pop_first() {
            if !reachable.insert(run_id.clone()) {
                continue;
            }
            let nested = verified_sources
                .get(&run_id)
                .ok_or(PortableExportError::Invalid)?;
            for child in nested {
                if child == &self.run_id || child == &run_id {
                    return Err(PortableExportError::Invalid);
                }
                pending.insert(child.clone());
            }
        }
        if verified.run_id() != &self.run_id
            || verified.tenant_scope_id() != &self.tenant_scope_id
            || verified.journal_head() != &self.fixation.journal_head
            || verified.semantic_head() != &self.fixation.semantic_head
            || reachable != self.source_run_ids.iter().cloned().collect()
        {
            return Err(PortableExportError::Invalid);
        }
        let required_frontiers = self
            .batches
            .iter()
            .chain(
                self.source_prefixes
                    .iter()
                    .flat_map(|prefix| prefix.batches.iter()),
            )
            .filter_map(|batch| match &batch.tenant_fact_coordinate {
                TenantFactCoordinate::FactSelectionBarrier { frontier } => Some(frontier),
                TenantFactCoordinate::None | TenantFactCoordinate::FactPublication { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        for frontier in required_frontiers {
            let covered = self.source_prefixes.iter().any(|prefix| {
                prefix
                    .fact_frontiers
                    .iter()
                    .any(|published| published == frontier)
            });
            if !covered {
                return Err(PortableExportError::Invalid);
            }
        }
        Ok(verified.into_recorded())
    }

    fn compute_closure_reference(&self) -> Result<ContentDigest, PortableExportError> {
        let run_fixations = self.run_fixations();
        let body = ClosureDigestBody {
            export_decisions: &self.export_decisions,
            fixation: &self.fixation,
            kind: self.kind,
            root_run_id: &self.run_id,
            run_fixations: &run_fixations,
            source_run_ids: &self.source_run_ids,
            store_scope_id: &self.store_scope_id,
            tenant_scope_id: &self.tenant_scope_id,
            version: &self.version,
        };
        let canonical = canonical_json(&body).map_err(|_| PortableExportError::Invalid)?;
        Ok(raw_digest(canonical.as_bytes()))
    }

    fn validate_structure(&self) -> Result<(), PortableExportError> {
        let total_batches = self
            .batches
            .len()
            .checked_add(
                self.source_prefixes
                    .iter()
                    .try_fold(0usize, |count, prefix| {
                        count.checked_add(prefix.batches.len())
                    })
                    .ok_or(PortableExportError::TooLarge)?,
            )
            .ok_or(PortableExportError::TooLarge)?;
        if self.version != PORTABLE_EXPORT_VERSION
            || self.batches.is_empty()
            || total_batches > MAX_PORTABLE_BATCHES
            || total_batches.saturating_add(1) > MAX_PORTABLE_FRAMES
            || self.source_run_ids.len() > MAX_PORTABLE_SOURCE_RUNS
            || self.store_scope_id != self.fixation.store_scope_id
            || self.tenant_scope_id != self.fixation.tenant_scope_id
            || self.source_run_ids.binary_search(&self.run_id).is_ok()
            || self.source_prefixes.len() != self.source_run_ids.len()
            || self
                .source_prefixes
                .iter()
                .map(|prefix| &prefix.run_id)
                .collect::<Vec<_>>()
                != self.source_run_ids.iter().collect::<Vec<_>>()
            || self.compute_closure_reference()? != self.closure_reference
        {
            return Err(PortableExportError::Invalid);
        }
        validate_export_decisions(self)?;
        let mut object_count = 0usize;
        let mut previous: Option<JournalHead> = None;
        for batch in &self.batches {
            if batch.store_scope_id != self.fixation.store_scope_id
                || batch.store_epoch != self.fixation.store_epoch
                || batch.predecessor != previous
                || batch.records.is_empty()
            {
                return Err(PortableExportError::Invalid);
            }
            object_count = object_count
                .checked_add(batch.objects.len())
                .ok_or(PortableExportError::TooLarge)?;
            if object_count > MAX_PORTABLE_OBJECTS {
                return Err(PortableExportError::TooLarge);
            }
            previous = Some(batch.head.clone());
        }
        let last = self.batches.last().expect("non-empty");
        if last.head != self.fixation.journal_head {
            return Err(PortableExportError::Invalid);
        }
        if matches!(self.kind, ExportKind::Semantic) {
            let semantic_sequence = match &self.fixation.semantic_head {
                SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
                SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
            };
            if last.head.run_sequence != semantic_sequence {
                return Err(PortableExportError::Invalid);
            }
        }
        if self
            .source_run_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(PortableExportError::Invalid);
        }
        for prefix in &self.source_prefixes {
            validate_prefix(
                prefix,
                &self.store_scope_id,
                &self.tenant_scope_id,
                self.fixation.store_epoch,
                &self.fixation.physical_target,
                matches!(self.kind, ExportKind::Semantic),
            )?;
        }
        validate_fact_routes(self)?;
        if !dense_bundled_publication_frontiers(self) {
            return Err(PortableExportError::Invalid);
        }
        validate_required_source_heads(self)?;
        Ok(())
    }

    fn run_fixations(&self) -> Vec<PortableRunFixation> {
        let mut fixations = Vec::with_capacity(self.source_prefixes.len() + 1);
        fixations.push(PortableRunFixation {
            fact_frontiers: fact_frontiers(&self.batches),
            fact_routes: self.fact_routes.clone(),
            fixation: self.fixation.clone(),
            run_id: self.run_id.clone(),
            tenant_scope_id: self.tenant_scope_id.clone(),
        });
        fixations.extend(
            self.source_prefixes
                .iter()
                .map(|prefix| PortableRunFixation {
                    fact_frontiers: prefix.fact_frontiers.clone(),
                    fact_routes: prefix.fact_routes.clone(),
                    fixation: prefix.fixation.clone(),
                    run_id: prefix.run_id.clone(),
                    tenant_scope_id: prefix.tenant_scope_id.clone(),
                }),
        );
        fixations
    }
}

fn validate_export_decisions(export: &PortableRunExport) -> Result<(), PortableExportError> {
    if export.export_decisions.len() != export.source_run_ids.len() + 1 {
        return Err(PortableExportError::Invalid);
    }
    let principal = export
        .export_decisions
        .first()
        .map(|decision| &decision.principal_id)
        .ok_or(PortableExportError::Invalid)?;
    let mut expected = std::iter::once(&export.run_id).chain(export.source_run_ids.iter());
    for decision in &export.export_decisions {
        if decision.grant != PORTABLE_EXPORT_GRANT
            || &decision.principal_id != principal
            || Some(&decision.run_id) != expected.next()
        {
            return Err(PortableExportError::Invalid);
        }
    }
    if expected.next().is_some() {
        return Err(PortableExportError::Invalid);
    }
    Ok(())
}

fn validate_fact_routes(export: &PortableRunExport) -> Result<(), PortableExportError> {
    let mut batches_by_run = BTreeMap::<RunId, &[CommittedBatch]>::new();
    batches_by_run.insert(export.run_id.clone(), &export.batches);
    for prefix in &export.source_prefixes {
        if batches_by_run
            .insert(prefix.run_id.clone(), &prefix.batches)
            .is_some()
        {
            return Err(PortableExportError::Invalid);
        }
    }
    let route_count = export
        .fact_routes
        .len()
        .checked_add(
            export
                .source_prefixes
                .iter()
                .try_fold(0usize, |count, prefix| {
                    count.checked_add(prefix.fact_routes.len())
                })
                .ok_or(PortableExportError::TooLarge)?,
        )
        .ok_or(PortableExportError::TooLarge)?;
    if route_count > MAX_PORTABLE_FACT_ROUTES {
        return Err(PortableExportError::TooLarge);
    }
    let check = |consumer_run_id: &RunId,
                 routes: &[PortableFactRoute]|
     -> Result<(), PortableExportError> {
        let consumer_batches = batches_by_run
            .get(consumer_run_id)
            .ok_or(PortableExportError::Invalid)?;
        for route in routes {
            if route.consumer_record.run_id != *consumer_run_id
                || route.publication_frontier.store_scope_id != export.store_scope_id
                || route.publication_frontier.store_epoch != export.fixation.store_epoch
                || route.publication_frontier.tenant_scope_id != export.tenant_scope_id
            {
                return Err(PortableExportError::Invalid);
            }
            let consumer_present = consumer_batches.iter().any(|batch| {
                batch
                    .records
                    .iter()
                    .any(|assigned| assigned.record_ref == route.consumer_record)
            });
            let producer_batches = batches_by_run
                .get(&route.producer_transition.run_id)
                .ok_or(PortableExportError::Invalid)?;
            let producer_present = producer_batches.iter().any(|batch| {
                batch.tenant_fact_coordinate
                    == TenantFactCoordinate::FactPublication {
                        frontier: route.publication_frontier.clone(),
                    }
                    && batch
                        .records
                        .iter()
                        .any(|assigned| assigned.record_ref == route.producer_transition)
            });
            if !consumer_present || !producer_present {
                return Err(PortableExportError::Invalid);
            }
        }
        Ok(())
    };
    check(&export.run_id, &export.fact_routes)?;
    for prefix in &export.source_prefixes {
        check(&prefix.run_id, &prefix.fact_routes)?;
    }
    Ok(())
}

fn validate_required_source_heads(export: &PortableRunExport) -> Result<(), PortableExportError> {
    let mut required = BTreeMap::<RunId, u64>::new();
    let mut record_routes = |routes: &[PortableFactRoute]| {
        for route in routes {
            if route.consumer_record.run_id == route.producer_transition.run_id {
                continue;
            }
            required
                .entry(route.producer_transition.run_id.clone())
                .and_modify(|sequence| {
                    *sequence = (*sequence).max(route.producer_transition.run_sequence)
                })
                .or_insert(route.producer_transition.run_sequence);
        }
    };
    record_routes(&export.fact_routes);
    for prefix in &export.source_prefixes {
        record_routes(&prefix.fact_routes);
    }
    let actual = export
        .source_prefixes
        .iter()
        .map(|prefix| {
            (
                prefix.run_id.clone(),
                prefix.fixation.journal_head.run_sequence,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if required != actual {
        return Err(PortableExportError::Invalid);
    }
    Ok(())
}

/// Explicit trust material for offline portable verification.
pub trait RetainedPhysicalReleaseTrust:
    mfm_authority_seal::RetainedPhysicalReleaseTrustSeal + Send + Sync
{
    /// Verifies the retained public release history for one fixation.
    fn verify(&self, fixation: &PortableFixation, kind: ExportKind) -> bool;
}

/// Explicit external store-lineage trust for one portable closure.
pub trait StoreLineageTrust: mfm_authority_seal::StoreLineageTrustSeal + Send + Sync {
    /// Verifies target lineage, writer epoch, and exact authorized closure.
    fn verify(
        &self,
        fixation: &PortableFixation,
        kind: ExportKind,
        closure_reference: &ContentDigest,
    ) -> bool;
}

/// Callback-free trust inputs captured when an export closure is authorized.
pub struct ReplayTrustSnapshot<'a> {
    /// Deterministic program verifier.
    ///
    /// The verifier is retained by reference only for the duration of one fold.
    program_verifier: &'a dyn ProgramVerifier,
    /// Public physical-binding verifier.
    physical_binding_verifier: &'a dyn PublicPhysicalBindingVerifier,
    /// Optional retained-release lineage verifier.
    retained_release_trust: Option<&'a dyn RetainedPhysicalReleaseTrust>,
    /// Optional external store-lineage verifier.
    store_lineage_trust: Option<&'a dyn StoreLineageTrust>,
    /// Exact authorized closure reference, when one was captured.
    authorized_closure_reference: Option<&'a ContentDigest>,
}

impl<'a> ReplayTrustSnapshot<'a> {
    /// Binds concrete callback-free verifiers without live store or network access.
    pub fn new(
        program_verifier: &'a dyn ProgramVerifier,
        physical_binding_verifier: &'a dyn PublicPhysicalBindingVerifier,
    ) -> Self {
        Self {
            program_verifier,
            physical_binding_verifier,
            retained_release_trust: None,
            store_lineage_trust: None,
            authorized_closure_reference: None,
        }
    }

    /// Adds the exact closure reference and deployment trust required for
    /// isolated verification.
    pub fn with_authorized_closure(
        mut self,
        closure_reference: &'a ContentDigest,
        retained_release_trust: &'a dyn RetainedPhysicalReleaseTrust,
        store_lineage_trust: &'a dyn StoreLineageTrust,
    ) -> Self {
        self.authorized_closure_reference = Some(closure_reference);
        self.retained_release_trust = Some(retained_release_trust);
        self.store_lineage_trust = Some(store_lineage_trust);
        self
    }
}

/// Stable portable-export failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortableExportError {
    /// The stream is malformed, non-canonical, or fails offline fold verification.
    #[error("portable export is invalid")]
    Invalid,
    /// The stream exceeds a frozen byte, frame, count, or depth bound.
    #[error("portable export exceeds frozen bounds")]
    TooLarge,
}

impl From<PortableExportError> for StructuredReplayError {
    fn from(error: PortableExportError) -> Self {
        match error {
            PortableExportError::Invalid | PortableExportError::TooLarge => {
                StructuredReplayError::InvalidRecordedHistory
            }
        }
    }
}

fn select_batches(
    evidence: &ExportEncoderView<'_>,
    kind: ExportKind,
) -> Result<Vec<CommittedBatch>, PortableExportError> {
    let batches = evidence
        .batch_frames()
        .map(|frame| {
            serde_json::from_slice::<CommittedBatch>(frame)
                .map_err(|_| PortableExportError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if batches.is_empty() {
        return Err(PortableExportError::Invalid);
    }
    match kind {
        ExportKind::Audit => Ok(batches.to_vec()),
        ExportKind::Semantic => {
            let cutoff = match evidence.semantic_head() {
                SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
                SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
            };
            let selected = batches
                .iter()
                .take_while(|batch| batch.head.run_sequence <= cutoff)
                .cloned()
                .collect::<Vec<_>>();
            if selected
                .last()
                .is_none_or(|batch| batch.head.run_sequence != cutoff)
            {
                return Err(PortableExportError::Invalid);
            }
            Ok(selected)
        }
    }
}

fn portable_prefix_from_source(
    source: mfm_store::structured::ExportEncoderSource<'_>,
    required_head: Option<u64>,
    semantic_cutoff_required: bool,
) -> Result<PortableRunPrefix, PortableExportError> {
    // A source is retained only through the largest transition actually routed
    // by the selected closure. This applies to audit exports as well: carrying
    // an unrelated later source append would violate the exact required-head
    // rule used to prove the recursive dependency closure.
    let semantic_cutoff = match source.semantic_head() {
        SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
        SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
    };
    let cutoff = required_head.unwrap_or(semantic_cutoff);
    let batches = source
        .batch_frames()
        .map(|frame| {
            serde_json::from_slice::<CommittedBatch>(frame)
                .map_err(|_| PortableExportError::Invalid)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .take_while(|batch| batch.head.run_sequence <= cutoff)
        .collect::<Vec<_>>();
    if batches.is_empty() {
        return Err(PortableExportError::Invalid);
    }
    let first = batches.first().ok_or(PortableExportError::Invalid)?;
    let semantic_head = semantic_head_at_cutoff(source.semantic_head(), &batches)?;
    let fixation = PortableFixation {
        semantic_head,
        journal_head: batches
            .last()
            .ok_or(PortableExportError::Invalid)?
            .head
            .clone(),
        store_scope_id: first.store_scope_id.clone(),
        store_epoch: first.store_epoch,
        tenant_scope_id: source.header().tenant_scope_id().clone(),
        physical_target: source.physical_target().clone(),
    };
    let cutoff = batches
        .last()
        .map(|batch| batch.head.run_sequence)
        .ok_or(PortableExportError::Invalid)?;
    let fact_routes = source
        .fact_routes()
        .iter()
        .filter(|route| route.consumer_record().run_sequence <= cutoff)
        .map(portable_fact_route)
        .collect::<Vec<_>>();
    let prefix_frontiers = fact_frontiers(&batches);
    let prefix = PortableRunPrefix {
        run_id: source.run_id().clone(),
        tenant_scope_id: source.header().tenant_scope_id().clone(),
        fixation,
        batches,
        fact_frontiers: prefix_frontiers,
        fact_routes,
    };
    validate_prefix(
        &prefix,
        &prefix.fixation.store_scope_id,
        &prefix.tenant_scope_id,
        prefix.fixation.store_epoch,
        &prefix.fixation.physical_target,
        semantic_cutoff_required,
    )?;
    Ok(prefix)
}

fn portable_fact_route(route: &mfm_store::structured::ExportFactRoute) -> PortableFactRoute {
    PortableFactRoute {
        consumer_record: route.consumer_record().clone(),
        producer_transition: route.producer_transition().clone(),
        publication_frontier: route.publication_frontier().clone(),
    }
}

fn portable_routes(routes: &[mfm_store::structured::ExportFactRoute]) -> Vec<PortableFactRoute> {
    routes.iter().map(portable_fact_route).collect()
}

fn semantic_head_at_cutoff(
    original: &SemanticHead,
    batches: &[CommittedBatch],
) -> Result<SemanticHead, PortableExportError> {
    let original_cutoff = match original {
        SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
        SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
    };
    let last_sequence = batches
        .last()
        .map(|batch| batch.head.run_sequence)
        .ok_or(PortableExportError::Invalid)?;
    if last_sequence >= original_cutoff {
        return Ok(original.clone());
    }
    let mut transition = None;
    for batch in batches {
        for assigned in &batch.records {
            if let mfm_journal::structured::RunRecord::StateTransitionCommitted(value) =
                &assigned.record
            {
                transition = Some(SemanticHead::Transition {
                    transition_ref: assigned.record_ref.clone(),
                    semantic_state_digest: value.after_semantic_state_digest.clone(),
                });
            }
        }
    }
    transition.ok_or(PortableExportError::Invalid)
}

fn fact_frontiers(batches: &[CommittedBatch]) -> Vec<TenantFactFrontier> {
    let mut frontiers = batches
        .iter()
        .filter_map(|batch| match &batch.tenant_fact_coordinate {
            TenantFactCoordinate::FactPublication { frontier }
            | TenantFactCoordinate::FactSelectionBarrier { frontier } => Some(frontier.clone()),
            TenantFactCoordinate::None => None,
        })
        .collect::<Vec<_>>();
    frontiers.sort();
    frontiers.dedup();
    frontiers
}

fn dense_bundled_publication_frontiers(export: &PortableRunExport) -> bool {
    let mut orders = BTreeMap::<(&StoreScopeId, StoreEpoch, &TenantScopeId), Vec<u64>>::new();
    let batch_sets = std::iter::once(export.batches.as_slice()).chain(
        export
            .source_prefixes
            .iter()
            .map(|prefix| prefix.batches.as_slice()),
    );
    for batch_set in batch_sets {
        for batch in batch_set {
            let TenantFactCoordinate::FactPublication { frontier } = &batch.tenant_fact_coordinate
            else {
                continue;
            };
            orders
                .entry((
                    &frontier.store_scope_id,
                    frontier.store_epoch,
                    &frontier.tenant_scope_id,
                ))
                .or_default()
                .push(frontier.fact_order);
        }
    }
    orders.values_mut().all(|orders| {
        orders.sort_unstable();
        orders.dedup();
        orders
            .iter()
            .enumerate()
            .all(|(index, order)| *order == u64::try_from(index + 1).unwrap_or_default())
    })
}

fn validate_prefix(
    prefix: &PortableRunPrefix,
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    store_epoch: StoreEpoch,
    physical_target: &PhysicalTargetIdentity,
    semantic_cutoff: bool,
) -> Result<(), PortableExportError> {
    if prefix.batches.is_empty()
        || &prefix.fixation.store_scope_id != store_scope_id
        || prefix.fixation.store_epoch != store_epoch
        || &prefix.fixation.physical_target != physical_target
        || &prefix.fixation.tenant_scope_id != tenant_scope_id
        || &prefix.tenant_scope_id != tenant_scope_id
        || prefix.batches.last().map(|batch| &batch.head) != Some(&prefix.fixation.journal_head)
        || prefix.fact_frontiers != fact_frontiers(&prefix.batches)
    {
        return Err(PortableExportError::Invalid);
    }
    if prefix
        .fact_frontiers
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    // A consumer may carry only a selection barrier at frontier N; the
    // producer publication route, not the consumer run's own batches, is
    // responsible for proving density through N.
    {
        return Err(PortableExportError::Invalid);
    }
    let mut previous = None;
    let mut objects = 0usize;
    for batch in &prefix.batches {
        if batch.store_scope_id != prefix.fixation.store_scope_id
            || batch.store_epoch != prefix.fixation.store_epoch
            || batch.predecessor != previous
            || batch.records.is_empty()
        {
            return Err(PortableExportError::Invalid);
        }
        objects = objects
            .checked_add(batch.objects.len())
            .ok_or(PortableExportError::TooLarge)?;
        if objects > MAX_PORTABLE_OBJECTS {
            return Err(PortableExportError::TooLarge);
        }
        previous = Some(batch.head.clone());
    }
    if semantic_cutoff {
        let cutoff = match &prefix.fixation.semantic_head {
            SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
            SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
        };
        if prefix
            .batches
            .last()
            .is_none_or(|batch| batch.head.run_sequence != cutoff)
        {
            return Err(PortableExportError::Invalid);
        }
    }
    Ok(())
}

fn encode_frames(export: &PortableRunExport) -> Result<Vec<u8>, PortableExportError> {
    let mut stream = Vec::new();
    // The root is emitted first, followed by source prefixes in run-id order.
    // Physical line order carries frame sequence, and the canonical batch
    // predecessor chains already prove batch order, so frames retain no ordinal
    // and no chain digest of their own.
    let prefixes = std::iter::once((&export.run_id, &export.batches)).chain(
        export
            .source_prefixes
            .iter()
            .map(|prefix| (&prefix.run_id, &prefix.batches)),
    );
    for (run_id, batches) in prefixes {
        for batch in batches {
            let payload = serde_json::to_value(PortableBatchPayload {
                batch: batch.clone(),
                run_id: run_id.clone(),
            })
            .map_err(|_| PortableExportError::Invalid)?;
            let encoded = encode_frame(&PortableFrame {
                kind: PortableFrameKind::Batch,
                payload,
            })?;
            let next_len = stream
                .len()
                .checked_add(encoded.len())
                .ok_or(PortableExportError::TooLarge)?;
            if u64::try_from(next_len).map_err(|_| PortableExportError::TooLarge)?
                > MAX_PORTABLE_EXPORT_BYTES
            {
                return Err(PortableExportError::TooLarge);
            }
            stream.extend_from_slice(&encoded);
        }
    }
    if stream.is_empty() {
        return Err(PortableExportError::Invalid);
    }

    // The seal no longer restates the stream's own size or frame count, so it
    // encodes once instead of converging through a fixed point.
    let seal = PortableSeal {
        export_decisions: export.export_decisions.clone(),
        closure_reference: export.closure_reference.clone(),
        fixation: export.fixation.clone(),
        kind: export.kind,
        root_run_id: export.run_id.clone(),
        run_fixations: export.run_fixations(),
        source_run_ids: export.source_run_ids.clone(),
        store_scope_id: export.store_scope_id.clone(),
        tenant_scope_id: export.tenant_scope_id.clone(),
        version: export.version.clone(),
    };
    let payload = serde_json::to_value(seal).map_err(|_| PortableExportError::Invalid)?;
    let seal_bytes = encode_frame(&PortableFrame {
        kind: PortableFrameKind::Seal,
        payload,
    })?;
    let final_len = stream
        .len()
        .checked_add(seal_bytes.len())
        .ok_or(PortableExportError::TooLarge)?;
    if u64::try_from(final_len).map_err(|_| PortableExportError::TooLarge)?
        > MAX_PORTABLE_EXPORT_BYTES
    {
        return Err(PortableExportError::TooLarge);
    }
    stream.extend_from_slice(&seal_bytes);
    Ok(stream)
}

fn decode_frames(bytes: &[u8]) -> Result<PortableRunExport, PortableExportError> {
    // Size is separated from structure only so an oversized stream reports the
    // distinct bound failure instead of a generic rejection.
    if bytes.len() as u64 > MAX_PORTABLE_EXPORT_BYTES
        || bytes
            .split(|byte| *byte == b'\n')
            .any(|line| line.len().saturating_add(1) > MAX_PORTABLE_FRAME_BYTES)
    {
        return Err(PortableExportError::TooLarge);
    }
    // Framing, delimiters, record bounds, and the closed frame-union record
    // shape are all owned by the declared JSON-lines encoding.
    <PortableRunExportStream as PersistedSchema>::schema_identity()
        .and_then(|identity| identity.validate_canonical_json_lines(bytes))
        .map_err(|_| PortableExportError::Invalid)?;
    let mut batches_by_run: BTreeMap<RunId, Vec<CommittedBatch>> = BTreeMap::new();
    let mut seal = None;
    let mut frame_count = 0usize;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty())
    {
        // A frame is an internal typed record of the one stream codec, so it
        // carries no identity of its own beyond the record shape already
        // enforced above.
        let frame: PortableFrame =
            serde_json::from_slice(line).map_err(|_| PortableExportError::Invalid)?;
        frame_count = frame_count.saturating_add(1);
        match frame.kind {
            PortableFrameKind::Batch => {
                if seal.is_some() {
                    return Err(PortableExportError::Invalid);
                }
                let payload: PortableBatchPayload = serde_json::from_value(frame.payload)
                    .map_err(|_| PortableExportError::Invalid)?;
                let batches = batches_by_run.entry(payload.run_id).or_default();
                batches.push(payload.batch);
            }
            PortableFrameKind::Seal => {
                if seal.is_some() || frame_count < 2 {
                    return Err(PortableExportError::Invalid);
                }
                seal = Some(
                    serde_json::from_value::<PortableSeal>(frame.payload)
                        .map_err(|_| PortableExportError::Invalid)?,
                );
            }
        }
    }
    // Exact complete-stream identity is proved by the caller's expected
    // reference before decoding, terminal placement by the batch-after-seal
    // rejection above, and batch order by the canonical predecessor chains, so
    // the seal restates neither its own size nor a frame chain.
    let seal = seal.ok_or(PortableExportError::Invalid)?;
    let root_batches = batches_by_run
        .remove(&seal.root_run_id)
        .ok_or(PortableExportError::Invalid)?;
    let seal_run_fixations = seal.run_fixations.clone();
    let mut source_prefixes = Vec::new();
    for fixation in seal.run_fixations.iter().skip(1) {
        let batches = batches_by_run
            .remove(&fixation.run_id)
            .ok_or(PortableExportError::Invalid)?;
        source_prefixes.push(PortableRunPrefix {
            run_id: fixation.run_id.clone(),
            tenant_scope_id: fixation.tenant_scope_id.clone(),
            fixation: fixation.fixation.clone(),
            batches,
            fact_frontiers: fixation.fact_frontiers.clone(),
            fact_routes: fixation.fact_routes.clone(),
        });
    }
    if !batches_by_run.is_empty() || seal.run_fixations.is_empty() {
        return Err(PortableExportError::Invalid);
    }
    let export = PortableRunExport {
        export_decisions: seal.export_decisions.clone(),
        version: seal.version,
        kind: seal.kind,
        store_scope_id: seal.store_scope_id,
        tenant_scope_id: seal.tenant_scope_id,
        run_id: seal.root_run_id,
        fixation: seal.fixation,
        source_run_ids: seal.source_run_ids,
        source_prefixes,
        batches: root_batches,
        fact_routes: seal_run_fixations
            .first()
            .ok_or(PortableExportError::Invalid)?
            .fact_routes
            .clone(),
        closure_reference: seal.closure_reference,
    };
    if export.compute_closure_reference()? != export.closure_reference {
        return Err(PortableExportError::Invalid);
    }
    if seal_run_fixations != export.run_fixations() {
        return Err(PortableExportError::Invalid);
    }
    export.validate_structure()?;
    if encode_frames(&export)?.as_slice() != bytes {
        return Err(PortableExportError::Invalid);
    }
    Ok(export)
}

fn encode_frame(frame: &PortableFrame) -> Result<Vec<u8>, PortableExportError> {
    let canonical = canonical_json(frame).map_err(|_| PortableExportError::Invalid)?;
    if canonical.as_bytes().len().saturating_add(1) > MAX_PORTABLE_FRAME_BYTES {
        return Err(PortableExportError::TooLarge);
    }
    let mut bytes = canonical.as_bytes().to_vec();
    bytes.push(b'\n');
    Ok(bytes)
}

fn raw_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes))
}

fn placeholder_digest() -> ContentDigest {
    raw_digest(b"mfm.portable-export.digest-placeholder")
}

fn classify_fold_error(error: StructuredStoreError) -> PortableExportError {
    match error {
        StructuredStoreError::RunNotFound
        | StructuredStoreError::InvalidHistory
        | StructuredStoreError::CandidateRejected
        | StructuredStoreError::Certification
        | StructuredStoreError::StaleHead
        | StructuredStoreError::AppendConflict
        | StructuredStoreError::BackendUnavailable
        | StructuredStoreError::AcknowledgementUnknown => PortableExportError::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use mfm_canonical::sha256_digest_bytes;
    use mfm_ids::{
        AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, JournalCommitDigest,
        JournalRecordHash, RunId, RunSemanticStateDigest, SchemaId, StableId, StoreEpoch,
        StoreScopeId, TenantScopeId,
    };
    use mfm_journal::structured::{
        AssignedRecord, CommittedBatch, HistoryObject, JournalHead, RecordRef, RunClosed,
        RunRecord, SemanticHead, TenantFactCoordinate, TenantFactFrontier,
    };
    use mfm_spec::structured::CertifiedProgramRoot;
    use mfm_spec::CanonicalJsonValue;
    use mfm_store::structured::{
        expand_export_source_closure, ExportSourceClosureError, PhysicalBindingAuthorization,
        PhysicalBindingSupersession, PhysicalTargetIdentity, ProgramVerifier,
        PublicPhysicalBindingVerifier, StructuredStoreError, VerifiedProgramData,
    };

    use super::{
        AuthorizedExportClosure, ExportKind, PortableAuthorizationDecision, PortableExportError,
        PortableFactRoute, PortableFixation, PortableRunExport, PortableRunPrefix,
        ReplayTrustSnapshot, RetainedPhysicalReleaseTrust, StoreLineageTrust,
        MAX_PORTABLE_EXPORT_BYTES, MAX_PORTABLE_FRAME_BYTES, PORTABLE_EXPORT_GRANT,
    };

    struct RejectProgram;

    impl mfm_authority_seal::ProgramVerifierSeal for RejectProgram {}

    impl ProgramVerifier for RejectProgram {
        fn verify(
            &self,
            _entry_point_id: &StableId,
            _root: &CertifiedProgramRoot,
            _authored: &CanonicalJsonValue,
        ) -> Result<Arc<VerifiedProgramData>, StructuredStoreError> {
            Err(StructuredStoreError::Certification)
        }
    }

    struct RejectPhysical;

    impl mfm_authority_seal::PhysicalBindingVerifierSeal for RejectPhysical {}

    impl PublicPhysicalBindingVerifier for RejectPhysical {
        fn verify_authorization(
            &self,
            _context: &PhysicalBindingAuthorization<'_>,
            _certificate: &mfm_journal::structured::HistoryObject,
        ) -> Result<(), StructuredStoreError> {
            Err(StructuredStoreError::Certification)
        }

        fn verify_supersession(
            &self,
            _context: &PhysicalBindingSupersession<'_>,
            _public_lineage_head: &mfm_journal::structured::HistoryObject,
            _evidence: &mfm_journal::structured::HistoryObject,
        ) -> Result<(), StructuredStoreError> {
            Err(StructuredStoreError::Certification)
        }
    }

    struct AcceptRelease;

    impl mfm_authority_seal::RetainedPhysicalReleaseTrustSeal for AcceptRelease {}

    impl RetainedPhysicalReleaseTrust for AcceptRelease {
        fn verify(&self, _fixation: &PortableFixation, _kind: ExportKind) -> bool {
            true
        }
    }

    struct AcceptStoreLineage;

    impl mfm_authority_seal::StoreLineageTrustSeal for AcceptStoreLineage {}

    impl StoreLineageTrust for AcceptStoreLineage {
        fn verify(
            &self,
            _fixation: &PortableFixation,
            _kind: ExportKind,
            _closure_reference: &ContentDigest,
        ) -> bool {
            true
        }
    }

    struct ExpectedTarget {
        target_key: String,
    }

    impl mfm_authority_seal::RetainedPhysicalReleaseTrustSeal for ExpectedTarget {}

    impl RetainedPhysicalReleaseTrust for ExpectedTarget {
        fn verify(&self, fixation: &PortableFixation, _kind: ExportKind) -> bool {
            fixation.physical_target.target_key == self.target_key
        }
    }

    struct ExpectedTenant {
        tenant_scope_id: TenantScopeId,
    }

    impl mfm_authority_seal::StoreLineageTrustSeal for ExpectedTenant {}

    impl StoreLineageTrust for ExpectedTenant {
        fn verify(
            &self,
            fixation: &PortableFixation,
            _kind: ExportKind,
            _closure_reference: &ContentDigest,
        ) -> bool {
            fixation.tenant_scope_id == self.tenant_scope_id
        }
    }

    #[test]
    fn oversized_and_monolithic_documents_fail_before_full_decode() {
        let oversize = vec![b'{'; (MAX_PORTABLE_EXPORT_BYTES as usize) + 1];
        assert_eq!(
            PortableRunExport::strict_decode(&oversize),
            Err(PortableExportError::TooLarge)
        );
        assert_eq!(json_depth(br#"{\"kind\":\"batch\"}"#), Some(1));
        assert!(PortableRunExport::strict_decode(b"").is_err());
        assert!(PortableRunExport::strict_decode(br#"{\"kind\":\"semantic\"}"#).is_err());
        assert!(PortableRunExport::strict_decode(
            br#"{\"version\":\"mfm.structured-portable-run-export.v1\"}"#
        )
        .is_err());
        assert_eq!(
            PortableRunExport::strict_decode(
                &[vec![b'{'; MAX_PORTABLE_FRAME_BYTES], vec![b'\n']].concat()
            ),
            Err(PortableExportError::TooLarge)
        );
    }

    #[test]
    fn exact_frame_limit_succeeds_and_one_byte_over_fails() {
        let frame = |payload_len| super::PortableFrame {
            kind: super::PortableFrameKind::Batch,
            payload: serde_json::Value::String("x".repeat(payload_len)),
        };
        let target = MAX_PORTABLE_FRAME_BYTES - 1;
        let mut low = 0usize;
        let mut high = target;
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            let length = mfm_journal::structured::canonical_json(&frame(middle))
                .expect("canonical frame")
                .as_bytes()
                .len();
            if length <= target {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let exact = frame(low);
        assert_eq!(
            mfm_journal::structured::canonical_json(&exact)
                .expect("exact canonical frame")
                .as_bytes()
                .len()
                .saturating_add(1),
            MAX_PORTABLE_FRAME_BYTES
        );
        assert_eq!(
            super::encode_frame(&exact)
                .expect("exact frame limit")
                .len(),
            MAX_PORTABLE_FRAME_BYTES
        );
        assert_eq!(
            super::encode_frame(&frame(low + 1)),
            Err(PortableExportError::TooLarge)
        );
    }

    #[test]
    fn exact_total_limit_succeeds_and_one_byte_over_fails() {
        let target = MAX_PORTABLE_EXPORT_BYTES as usize;
        let mut low_payload = 0usize;
        let mut high_payload = MAX_PORTABLE_FRAME_BYTES;
        while low_payload < high_payload {
            let middle = low_payload + (high_payload - low_payload).div_ceil(2);
            if padding_frame(middle).is_ok() {
                low_payload = middle;
            } else {
                high_payload = middle - 1;
            }
        }
        let mut fixed_payload = low_payload;
        loop {
            let base = super::encode_frames(&sized_audit_export(fixed_payload, 0));
            if let Ok(bytes) = base {
                assert!(bytes.len() < target);
                let full_len =
                    super::encode_frames(&sized_audit_export(fixed_payload, fixed_payload))
                        .ok()
                        .map(|bytes| bytes.len());
                assert!(full_len.is_none_or(|length| length >= target));
                break;
            }
            fixed_payload = fixed_payload
                .checked_sub(1_000)
                .expect("portable frame can hold one batch");
        }

        let mut low = 0usize;
        let mut high = fixed_payload;
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            let length = super::encode_frames(&sized_audit_export(fixed_payload, middle))
                .ok()
                .map(|bytes| bytes.len());
            if length.is_some_and(|length| length <= target) {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let exact = super::encode_frames(&sized_audit_export(fixed_payload, low))
            .expect("exact total limit");
        assert_eq!(exact.len(), target);
        assert_eq!(
            super::encode_frames(&sized_audit_export(fixed_payload, low + 1)),
            Err(PortableExportError::TooLarge)
        );
        assert_eq!(
            PortableRunExport::strict_decode(&exact)
                .expect("decode exact total limit")
                .encode()
                .expect("re-encode exact total limit")
                .into_bytes(),
            exact
        );
        let many_small =
            super::encode_frames(&sized_audit_export(128, 128)).expect("many-small-frame export");
        assert_eq!(
            PortableRunExport::strict_decode(&many_small)
                .expect("decode many-small-frame export")
                .encode()
                .expect("re-encode many-small-frame export")
                .into_bytes(),
            many_small
        );
    }

    #[test]
    fn golden_frame_stream_rejects_omission_extra_substitution_reordering_and_stale_head() {
        let export = golden_export();
        let bytes = export
            .encode()
            .expect("encode synthetic portable golden")
            .into_bytes();
        let decoded = PortableRunExport::strict_decode(&bytes).expect("decode synthetic golden");
        assert_eq!(decoded, export);

        let reject_program = RejectProgram;
        let reject_physical = RejectPhysical;
        let release = AcceptRelease;
        let lineage = AcceptStoreLineage;
        let trust = ReplayTrustSnapshot::new(&reject_program, &reject_physical)
            .with_authorized_closure(export.closure_reference(), &release, &lineage);
        assert_eq!(
            PortableRunExport::verify_offline(&bytes, &stream_ref(&bytes), &trust),
            Err(PortableExportError::Invalid)
        );

        let frames = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<Vec<u8>>>();
        assert_eq!(frames.len(), 2);
        let join = |frames: &[Vec<u8>]| {
            let mut stream = Vec::new();
            for frame in frames {
                stream.extend_from_slice(frame);
                stream.push(b'\n');
            }
            stream
        };

        let mut omitted = frames.clone();
        omitted.remove(0);
        assert!(PortableRunExport::strict_decode(&join(&omitted)).is_err());

        let mut extra = frames.clone();
        extra.insert(0, frames[0].clone());
        assert!(PortableRunExport::strict_decode(&join(&extra)).is_err());

        let mut substituted = frames.clone();
        substituted[0][0] = b'[';
        assert!(PortableRunExport::strict_decode(&join(&substituted)).is_err());

        let mut reordered = frames.clone();
        reordered.swap(0, 1);
        assert!(PortableRunExport::strict_decode(&join(&reordered)).is_err());

        let mut stale_head = frames.clone();
        let position = stale_head[0]
            .iter()
            .position(|byte| *byte == b'1')
            .expect("synthetic batch sequence digit");
        stale_head[0][position] = b'2';
        assert!(PortableRunExport::strict_decode(&join(&stale_head)).is_err());

        let mut semantic_suffix = golden_export();
        let first_batch = semantic_suffix.batches[0].clone();
        let mut later_batch = first_batch.clone();
        later_batch.predecessor = Some(first_batch.head.clone());
        later_batch.append_request_id =
            AppendRequestId::new("portable-golden-later-append").expect("later append request");
        later_batch.candidate_digest = ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(b"portable-later-candidate"),
        );
        later_batch.head = JournalHead {
            run_sequence: 2,
            commit_digest: JournalCommitDigest::from_digest(sha256_digest_bytes(
                b"portable-later-commit",
            )),
        };
        semantic_suffix.fixation.journal_head = later_batch.head.clone();
        semantic_suffix.batches.push(later_batch);
        semantic_suffix.closure_reference = semantic_suffix
            .compute_closure_reference()
            .expect("semantic suffix closure reference");
        let semantic_suffix_bytes =
            super::encode_frames(&semantic_suffix).expect("encode hostile semantic suffix");
        assert_eq!(
            PortableRunExport::strict_decode(&semantic_suffix_bytes),
            Err(PortableExportError::Invalid)
        );

        semantic_suffix.kind = ExportKind::Audit;
        semantic_suffix.closure_reference = semantic_suffix
            .compute_closure_reference()
            .expect("audit suffix closure reference");
        let audit_suffix_bytes =
            super::encode_frames(&semantic_suffix).expect("encode audit suffix");
        assert!(PortableRunExport::strict_decode(&audit_suffix_bytes).is_ok());
    }

    /// Offline verification is gated on the caller's expected stream identity.
    #[test]
    fn offline_verification_rejects_bytes_that_are_not_the_expected_stream() {
        let export = golden_export();
        let encoded = export.encode().expect("encode expectation golden");
        assert_eq!(encoded.content_ref(), &stream_ref(encoded.as_bytes()));

        let release = AcceptRelease;
        let lineage = AcceptStoreLineage;
        let trust = ReplayTrustSnapshot::new(&RejectProgram, &RejectPhysical)
            .with_authorized_closure(export.closure_reference(), &release, &lineage);

        let mut other = golden_export();
        other.export_decisions[0].decision_ref = decision_ref(9);
        other.closure_reference = other
            .compute_closure_reference()
            .expect("other closure reference");
        let other_bytes = super::encode_frames(&other).expect("other bytes");
        assert_ne!(other_bytes, encoded.as_bytes());
        assert_eq!(
            PortableRunExport::verify_offline(&other_bytes, encoded.content_ref(), &trust),
            Err(PortableExportError::Invalid)
        );
    }

    #[test]
    fn serialized_authorization_decision_tampering_is_rejected() {
        let mut wrong_grant = golden_export();
        wrong_grant.export_decisions[0].grant = "read_public".to_owned();
        wrong_grant.closure_reference = wrong_grant
            .compute_closure_reference()
            .expect("wrong grant closure reference");
        let wrong_grant_bytes = super::encode_frames(&wrong_grant).expect("wrong grant bytes");
        assert_eq!(
            PortableRunExport::strict_decode(&wrong_grant_bytes),
            Err(PortableExportError::Invalid)
        );

        let mut omitted = golden_export();
        omitted.export_decisions.clear();
        omitted.closure_reference = omitted
            .compute_closure_reference()
            .expect("omitted decision closure reference");
        let omitted_bytes = super::encode_frames(&omitted).expect("omitted decision bytes");
        assert_eq!(
            PortableRunExport::strict_decode(&omitted_bytes),
            Err(PortableExportError::Invalid)
        );

        let mut substituted = golden_export();
        substituted.export_decisions[0].decision_ref = decision_ref(9);
        substituted.closure_reference = substituted
            .compute_closure_reference()
            .expect("substituted decision closure reference");
        let substituted_bytes = super::encode_frames(&substituted).expect("substituted bytes");
        let decoded = PortableRunExport::strict_decode(&substituted_bytes)
            .expect("self-consistent decision substitution decodes");
        assert_ne!(
            decoded.closure_reference(),
            golden_export().closure_reference()
        );
        let reject_program = RejectProgram;
        let reject_physical = RejectPhysical;
        let release = AcceptRelease;
        let lineage = AcceptStoreLineage;
        let trusted_golden = golden_export();
        let trusted_original = ReplayTrustSnapshot::new(&reject_program, &reject_physical)
            .with_authorized_closure(trusted_golden.closure_reference(), &release, &lineage);
        assert_eq!(
            PortableRunExport::verify_offline(
                &substituted_bytes,
                &stream_ref(&substituted_bytes),
                &trusted_original,
            ),
            Err(PortableExportError::Invalid)
        );
    }

    #[test]
    fn serialized_recursive_prefix_tampering_is_rejected() {
        let export = recursive_golden_export();
        let bytes = export
            .encode()
            .expect("encode recursive portable golden")
            .into_bytes();
        assert_eq!(
            PortableRunExport::strict_decode(&bytes).expect("decode recursive portable golden"),
            export
        );

        let frames = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<Vec<u8>>>();
        assert_eq!(frames.len(), 3);
        let join = |frames: &[Vec<u8>]| {
            let mut stream = Vec::new();
            for frame in frames {
                stream.extend_from_slice(frame);
                stream.push(b'\n');
            }
            stream
        };

        let mut omitted_prefix = frames.clone();
        omitted_prefix.remove(1);
        assert!(PortableRunExport::strict_decode(&join(&omitted_prefix)).is_err());

        let mut extra_prefix = frames.clone();
        extra_prefix.insert(1, frames[1].clone());
        assert!(PortableRunExport::strict_decode(&join(&extra_prefix)).is_err());

        let mut reordered_prefix = frames.clone();
        reordered_prefix.swap(0, 1);
        assert!(PortableRunExport::strict_decode(&join(&reordered_prefix)).is_err());

        let mut substituted_publication = export.clone();
        substituted_publication.source_prefixes[0]
            .fact_frontiers
            .clear();
        substituted_publication.closure_reference = substituted_publication
            .compute_closure_reference()
            .expect("publication substitution closure");
        assert!(substituted_publication.encode().is_err());

        let mut stale_head = export.clone();
        stale_head.source_prefixes[0]
            .fixation
            .journal_head
            .run_sequence = 2;
        stale_head.closure_reference = stale_head
            .compute_closure_reference()
            .expect("stale source head closure");
        assert!(stale_head.encode().is_err());

        let mut omitted_route = export;
        omitted_route.fact_routes.clear();
        omitted_route.closure_reference = omitted_route
            .compute_closure_reference()
            .expect("omitted route closure");
        assert!(omitted_route.encode().is_err());
    }

    /// Online and offline replay agree on one export built from current owners.
    ///
    /// The evidence is agreement between the two paths over bytes this tree
    /// produces, not equality with a frozen artifact: the schema reset changes
    /// those bytes by design.
    #[tokio::test]
    async fn online_and_offline_replay_agree_on_a_semantic_export() {
        let fixture = mfm_store::structured::test_support::zero_state_export(201)
            .await
            .expect("offline acceptance fixture");
        let (fixture_export, fixture_recorded, fixture_program, fixture_physical) =
            fixture.into_replay_parts();
        let closure = AuthorizedExportClosure::new(
            fixture_export,
            StableId::new("mfm.portable-test/principal").expect("principal"),
            decision_ref(1),
            BTreeMap::new(),
        )
        .expect("seal offline acceptance fixture");
        let generated =
            PortableRunExport::from_authorized_export_closure(&closure, ExportKind::Semantic)
                .expect("encode offline acceptance fixture");
        let bytes = generated
            .encode()
            .expect("canonical offline acceptance fixture")
            .into_bytes();

        let release = AcceptRelease;
        let lineage = AcceptStoreLineage;
        let trust = ReplayTrustSnapshot::new(&fixture_program, fixture_physical.as_ref())
            .with_authorized_closure(generated.closure_reference(), &release, &lineage);
        let offline = PortableRunExport::verify_offline(&bytes, &stream_ref(&bytes), &trust)
            .expect("generated offline acceptance");
        let online =
            super::project_replay_result(&fixture_recorded).expect("online acceptance projection");
        assert_eq!(offline.as_bytes(), online.as_bytes());

        let untrusted = ReplayTrustSnapshot::new(&RejectProgram, &RejectPhysical)
            .with_authorized_closure(generated.closure_reference(), &release, &lineage);
        assert!(
            PortableRunExport::verify_offline(&bytes, &stream_ref(&bytes), &untrusted).is_err(),
            "an untrusted program snapshot must fail the offline fold",
        );
    }

    /// An audit export carrying a real observed-Read suffix agrees in both paths.
    #[tokio::test]
    async fn online_and_offline_replay_agree_on_an_observed_read_audit_export() {
        let fixture = mfm_store::structured::test_support::observed_read_export(202)
            .await
            .expect("observed-read audit fixture");
        let (fixture_export, fixture_recorded, fixture_program, fixture_physical) =
            fixture.into_replay_parts();
        let closure = AuthorizedExportClosure::new(
            fixture_export,
            StableId::new("mfm.portable-test/principal").expect("principal"),
            decision_ref(3),
            BTreeMap::new(),
        )
        .expect("seal observed-read audit fixture");
        let generated =
            PortableRunExport::from_authorized_export_closure(&closure, ExportKind::Audit)
                .expect("encode observed-read audit fixture");
        let bytes = generated
            .encode()
            .expect("canonical observed-read audit fixture")
            .into_bytes();

        let release = AcceptRelease;
        let lineage = AcceptStoreLineage;
        let trust = ReplayTrustSnapshot::new(&fixture_program, fixture_physical.as_ref())
            .with_authorized_closure(generated.closure_reference(), &release, &lineage);
        let offline = PortableRunExport::verify_offline(&bytes, &stream_ref(&bytes), &trust)
            .expect("offline observed-read audit fixture");
        let online = super::project_replay_result(&fixture_recorded)
            .expect("online observed-read audit projection");
        assert_eq!(offline.as_bytes(), online.as_bytes());
    }

    /// A source run that collides with the root run is not a second source.
    #[test]
    fn a_source_that_collides_with_the_root_run_is_rejected() {
        let mut collided = recursive_golden_export();
        let root_run_id = collided.run_id.clone();
        collided.source_run_ids = vec![root_run_id.clone()];
        collided.source_prefixes[0].run_id = root_run_id;
        collided.closure_reference = collided
            .compute_closure_reference()
            .expect("collided closure reference");
        assert_eq!(collided.encode(), Err(PortableExportError::Invalid));
    }

    /// The same source run may be fixed only once in one export.
    #[test]
    fn a_duplicated_source_fixation_is_rejected() {
        let mut duplicated = recursive_golden_export();
        let source = duplicated.source_prefixes[0].clone();
        duplicated.source_run_ids.push(source.run_id.clone());
        duplicated.source_prefixes.push(source);
        duplicated.closure_reference = duplicated
            .compute_closure_reference()
            .expect("duplicated closure reference");
        assert_eq!(duplicated.encode(), Err(PortableExportError::Invalid));
    }

    /// The recursive source closure is bounded before any per-source work.
    #[test]
    fn a_source_count_over_the_budget_is_rejected() {
        let mut over_budget = recursive_golden_export();
        let template = over_budget.source_prefixes[0].clone();
        let mut run_ids = std::collections::BTreeSet::new();
        while run_ids.len() <= super::MAX_PORTABLE_SOURCE_RUNS {
            let index = run_ids.len();
            run_ids.insert(source_run_id(0, index));
        }
        over_budget.source_run_ids = run_ids.iter().cloned().collect();
        over_budget.source_prefixes = over_budget
            .source_run_ids
            .iter()
            .map(|run_id| {
                let mut prefix = template.clone();
                prefix.run_id = run_id.clone();
                prefix
            })
            .collect();
        assert_eq!(over_budget.encode(), Err(PortableExportError::Invalid));
    }

    /// A nested source graph expands to its shared transitive closure.
    #[test]
    fn a_nested_shared_source_graph_expands_to_its_transitive_closure() {
        let root = source_run_id(1, 0);
        let first = source_run_id(1, 1);
        let second = source_run_id(1, 2);
        let shared = source_run_id(1, 3);
        let edges = BTreeMap::from([
            (
                root.clone(),
                BTreeSet::from([first.clone(), second.clone()]),
            ),
            (first.clone(), BTreeSet::from([shared.clone()])),
            (second.clone(), BTreeSet::from([shared.clone()])),
            (shared.clone(), BTreeSet::new()),
        ]);
        let expanded = expand_export_source_closure(
            &root,
            edges.get(&root).cloned().expect("root edges"),
            |run_id| {
                Ok::<_, ExportSourceClosureError>(edges.get(run_id).cloned().unwrap_or_default())
            },
        )
        .expect("nested shared source graph");
        assert_eq!(expanded, BTreeSet::from([first, second, shared]));
    }

    /// A cyclic source graph is rejected instead of expanded forever.
    #[test]
    fn a_cyclic_source_graph_is_rejected() {
        let root = source_run_id(2, 0);
        let first = source_run_id(2, 1);
        let second = source_run_id(2, 2);
        let edges = BTreeMap::from([
            (root.clone(), BTreeSet::from([first.clone()])),
            (first.clone(), BTreeSet::from([second.clone()])),
            (second, BTreeSet::from([first])),
        ]);
        assert_eq!(
            expand_export_source_closure(
                &root,
                edges.get(&root).cloned().expect("root edges"),
                |run_id| {
                    Ok::<_, ExportSourceClosureError>(
                        edges.get(run_id).cloned().unwrap_or_default(),
                    )
                },
            )
            .expect_err("cyclic source graph"),
            ExportSourceClosureError::Cycle
        );
    }

    #[tokio::test]
    async fn production_store_export_folds_offline_and_preserves_projection_bytes() {
        let fixture = mfm_store::structured::test_support::zero_state_export(201)
            .await
            .expect("build verified store export fixture");
        let (fixture_export, fixture_recorded, fixture_program, fixture_physical) =
            fixture.into_replay_parts();
        let closure = AuthorizedExportClosure::new(
            fixture_export,
            StableId::new("mfm.portable-test/principal").expect("principal"),
            decision_ref(1),
            BTreeMap::new(),
        )
        .expect("seal verified export evidence");
        let export =
            PortableRunExport::from_authorized_export_closure(&closure, ExportKind::Semantic)
                .expect("encode verified export evidence");
        let bytes = export
            .encode()
            .expect("encode canonical portable frames")
            .into_bytes();
        let release = AcceptRelease;
        let lineage = AcceptStoreLineage;
        let trust = ReplayTrustSnapshot::new(&fixture_program, fixture_physical.as_ref())
            .with_authorized_closure(export.closure_reference(), &release, &lineage);

        let online = super::project_replay_result(&fixture_recorded).expect("online projection");
        let offline = PortableRunExport::verify_offline(&bytes, &stream_ref(&bytes), &trust)
            .expect("offline projection");
        assert_eq!(offline.as_bytes(), online.as_bytes());

        let expected_target = export.fixation.physical_target.target_key.clone();
        let mut wrong_target = export;
        wrong_target.fixation.physical_target.target_key = "forged-target".to_owned();
        wrong_target.closure_reference = wrong_target
            .compute_closure_reference()
            .expect("wrong-target closure reference");
        let wrong_target_bytes = super::encode_frames(&wrong_target).expect("wrong-target bytes");
        let expected_target = ExpectedTarget {
            target_key: expected_target,
        };
        let accepted_lineage = AcceptStoreLineage;
        let target_trust = ReplayTrustSnapshot::new(&fixture_program, fixture_physical.as_ref())
            .with_authorized_closure(
                wrong_target.closure_reference(),
                &expected_target,
                &accepted_lineage,
            );
        assert_eq!(
            PortableRunExport::verify_offline(
                &wrong_target_bytes,
                &stream_ref(&wrong_target_bytes),
                &target_trust,
            ),
            Err(PortableExportError::Invalid)
        );

        let tenant_fixture = mfm_store::structured::test_support::zero_state_export(202)
            .await
            .expect("build tenant fixture");
        let (tenant_export, _tenant_recorded, tenant_program, tenant_physical) =
            tenant_fixture.into_replay_parts();
        let mut wrong_tenant = PortableRunExport::from_authorized_export_closure(
            &AuthorizedExportClosure::new(
                tenant_export,
                StableId::new("mfm.portable-test/principal").expect("principal"),
                decision_ref(2),
                BTreeMap::new(),
            )
            .expect("seal tenant fixture"),
            ExportKind::Semantic,
        )
        .expect("encode tenant fixture");
        let original_tenant = wrong_tenant.tenant_scope_id.clone();
        wrong_tenant.tenant_scope_id =
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "f".repeat(32)))
                .expect("forged tenant");
        wrong_tenant.fixation.tenant_scope_id = wrong_tenant.tenant_scope_id.clone();
        wrong_tenant.closure_reference = wrong_tenant
            .compute_closure_reference()
            .expect("wrong-tenant closure reference");
        let wrong_tenant_bytes = super::encode_frames(&wrong_tenant).expect("wrong-tenant bytes");
        let expected_tenant = ExpectedTenant {
            tenant_scope_id: original_tenant,
        };
        let accepted_release = AcceptRelease;
        let tenant_trust = ReplayTrustSnapshot::new(&tenant_program, tenant_physical.as_ref())
            .with_authorized_closure(
                wrong_tenant.closure_reference(),
                &accepted_release,
                &expected_tenant,
            );
        assert_eq!(
            PortableRunExport::verify_offline(
                &wrong_tenant_bytes,
                &stream_ref(&wrong_tenant_bytes),
                &tenant_trust,
            ),
            Err(PortableExportError::Invalid)
        );
    }

    #[tokio::test]
    async fn production_audit_export_accepts_later_observation_suffix() {
        let fixture = mfm_store::structured::test_support::observed_read_export(203)
            .await
            .expect("build observed-read export fixture");
        let (fixture_export, fixture_recorded, fixture_program, fixture_physical) =
            fixture.into_replay_parts();
        let closure = AuthorizedExportClosure::new(
            fixture_export,
            StableId::new("mfm.portable-test/principal").expect("principal"),
            decision_ref(3),
            BTreeMap::new(),
        )
        .expect("seal observed-read export evidence");
        let audit = PortableRunExport::from_authorized_export_closure(&closure, ExportKind::Audit)
            .expect("encode later-audit export");
        assert!(
            audit.batches.len() >= 3,
            "audit export must carry the suffix"
        );
        let audit_bytes = audit
            .encode()
            .expect("encode later-audit frames")
            .into_bytes();
        let release = AcceptRelease;
        let lineage = AcceptStoreLineage;
        let trust = ReplayTrustSnapshot::new(&fixture_program, fixture_physical.as_ref())
            .with_authorized_closure(audit.closure_reference(), &release, &lineage);
        let online = super::project_replay_result(&fixture_recorded).expect("online audit result");
        let offline =
            PortableRunExport::verify_offline(&audit_bytes, &stream_ref(&audit_bytes), &trust)
                .expect("offline later-audit result");
        assert_eq!(offline.as_bytes(), online.as_bytes());

        let mut semantic_suffix = audit;
        semantic_suffix.kind = ExportKind::Semantic;
        semantic_suffix.closure_reference = semantic_suffix
            .compute_closure_reference()
            .expect("semantic suffix closure reference");
        let semantic_suffix_bytes =
            super::encode_frames(&semantic_suffix).expect("encode semantic suffix candidate");
        assert_eq!(
            PortableRunExport::strict_decode(&semantic_suffix_bytes),
            Err(PortableExportError::Invalid)
        );
    }

    /// The complete-stream reference an exporter publishes for exact bytes.
    fn stream_ref(bytes: &[u8]) -> ContentRef {
        ContentRef::new(
            super::portable_stream_schema_id().expect("portable stream schema id"),
            super::raw_digest(bytes),
        )
        .expect("portable stream content ref")
    }

    fn golden_export() -> PortableRunExport {
        let store_scope_id =
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "1".repeat(32)))
                .expect("store scope");
        let tenant_scope_id =
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
                .expect("tenant scope");
        let run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"portable-golden-run"),
        );
        let record_ref = RecordRef {
            run_id: run_id.clone(),
            run_sequence: 1,
            ordinal: 0,
            record_hash: JournalRecordHash::from_digest(sha256_digest_bytes(b"portable-record")),
        };
        let journal_head = JournalHead {
            run_sequence: 1,
            commit_digest: JournalCommitDigest::from_digest(sha256_digest_bytes(
                b"portable-commit",
            )),
        };
        let outcome_ref = ContentRef::new(
            SchemaId::new(
                "mfm.portable-test.outcome",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"portable-outcome-schema"),
            )
            .expect("outcome schema"),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(b"portable-outcome"),
            ),
        )
        .expect("outcome ref");
        let batch = CommittedBatch {
            store_scope_id: store_scope_id.clone(),
            store_epoch: StoreEpoch::new(1),
            predecessor: None,
            append_request_id: AppendRequestId::new("portable-golden-append")
                .expect("append request id"),
            tenant_fact_coordinate: TenantFactCoordinate::None,
            candidate_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(b"portable-candidate"),
            ),
            records: vec![AssignedRecord {
                record_ref: record_ref.clone(),
                record: RunRecord::RunClosed(RunClosed { outcome_ref }),
            }],
            objects: Vec::new(),
            head: journal_head.clone(),
        };
        let fixation = PortableFixation {
            semantic_head: SemanticHead::Genesis {
                admission_ref: record_ref,
                semantic_state_digest: RunSemanticStateDigest::from_digest(sha256_digest_bytes(
                    b"portable-semantic",
                )),
            },
            journal_head,
            store_scope_id: store_scope_id.clone(),
            store_epoch: StoreEpoch::new(1),
            tenant_scope_id: tenant_scope_id.clone(),
            physical_target: PhysicalTargetIdentity {
                target_key: "portable-test-target".to_owned(),
                database_oid: 7,
                fence_generation: 1,
                release_epoch: 1,
                current_incarnation_ref: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(b"portable-incarnation"),
                ),
            },
        };
        let mut export = PortableRunExport {
            version: super::PORTABLE_EXPORT_VERSION.to_owned(),
            kind: super::ExportKind::Semantic,
            store_scope_id,
            tenant_scope_id,
            run_id: run_id.clone(),
            fixation,
            source_run_ids: Vec::new(),
            source_prefixes: Vec::new(),
            batches: vec![batch],
            fact_routes: Vec::new(),
            export_decisions: vec![PortableAuthorizationDecision {
                decision_ref: decision_ref(0),
                grant: PORTABLE_EXPORT_GRANT.to_owned(),
                principal_id: StableId::new("mfm.portable-test/principal").expect("principal"),
                run_id: run_id.clone(),
            }],
            closure_reference: super::placeholder_digest(),
        };
        export.closure_reference = export
            .compute_closure_reference()
            .expect("closure reference");
        export
    }

    fn recursive_golden_export() -> PortableRunExport {
        let mut export = golden_export();
        let source_run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"portable-recursive-source"),
        );
        let source_record_ref = RecordRef {
            run_id: source_run_id.clone(),
            run_sequence: 1,
            ordinal: 0,
            record_hash: JournalRecordHash::from_digest(sha256_digest_bytes(
                b"portable-source-record",
            )),
        };
        let source_head = JournalHead {
            run_sequence: 1,
            commit_digest: JournalCommitDigest::from_digest(sha256_digest_bytes(
                b"portable-source-commit",
            )),
        };
        let frontier = TenantFactFrontier::new(
            export.store_scope_id.clone(),
            StoreEpoch::new(1),
            export.tenant_scope_id.clone(),
            1,
        );
        let outcome_ref = match &export.batches[0].records[0].record {
            RunRecord::RunClosed(closed) => closed.outcome_ref.clone(),
            _ => panic!("golden run must be closed"),
        };
        let source_batch = CommittedBatch {
            store_scope_id: export.store_scope_id.clone(),
            store_epoch: StoreEpoch::new(1),
            predecessor: None,
            append_request_id: AppendRequestId::new("portable-source-append")
                .expect("source append request"),
            tenant_fact_coordinate: TenantFactCoordinate::FactPublication {
                frontier: frontier.clone(),
            },
            candidate_digest: decision_ref(7),
            records: vec![AssignedRecord {
                record_ref: source_record_ref.clone(),
                record: RunRecord::RunClosed(RunClosed { outcome_ref }),
            }],
            objects: Vec::new(),
            head: source_head.clone(),
        };
        let source_fixation = PortableFixation {
            semantic_head: SemanticHead::Genesis {
                admission_ref: source_record_ref.clone(),
                semantic_state_digest: RunSemanticStateDigest::from_digest(sha256_digest_bytes(
                    b"portable-source-semantic",
                )),
            },
            journal_head: source_head,
            store_scope_id: export.store_scope_id.clone(),
            store_epoch: StoreEpoch::new(1),
            tenant_scope_id: export.tenant_scope_id.clone(),
            physical_target: export.fixation.physical_target.clone(),
        };
        let frontier_route = PortableFactRoute {
            consumer_record: export.batches[0].records[0].record_ref.clone(),
            producer_transition: source_record_ref,
            publication_frontier: frontier,
        };
        export.source_run_ids = vec![source_run_id.clone()];
        export.source_prefixes = vec![PortableRunPrefix {
            run_id: source_run_id.clone(),
            tenant_scope_id: export.tenant_scope_id.clone(),
            fixation: source_fixation,
            batches: vec![source_batch],
            fact_frontiers: vec![frontier_route.publication_frontier.clone()],
            fact_routes: Vec::new(),
        }];
        export.fact_routes = vec![frontier_route];
        export.export_decisions.push(PortableAuthorizationDecision {
            decision_ref: decision_ref(8),
            grant: PORTABLE_EXPORT_GRANT.to_owned(),
            principal_id: StableId::new("mfm.portable-test/principal").expect("principal"),
            run_id: source_run_id,
        });
        export.closure_reference = export
            .compute_closure_reference()
            .expect("recursive closure reference");
        export
    }

    fn decision_ref(digit: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(&[b'd', digit]),
        )
    }

    fn sized_audit_export(fixed_payload: usize, variable_payload: usize) -> PortableRunExport {
        let template = golden_export();
        let base_batch = template.batches[0].clone();
        let mut export = template;
        export.kind = ExportKind::Audit;
        let mut predecessor = None;
        let mut batches = Vec::with_capacity(17);
        for sequence in 1..=17u64 {
            let mut batch = base_batch.clone();
            batch.predecessor = predecessor.clone();
            batch.head = JournalHead {
                run_sequence: sequence,
                commit_digest: JournalCommitDigest::from_digest(sha256_digest_bytes(
                    &sequence.to_be_bytes(),
                )),
            };
            let payload_len = if sequence == 17 {
                variable_payload
            } else {
                fixed_payload
            };
            batch.objects = vec![HistoryObject {
                object_type: StableId::new("mfm.portable-test.padding").expect("padding type"),
                content_ref: ContentRef::new(
                    SchemaId::new(
                        "mfm.portable-test.padding",
                        "1",
                        DigestAlgorithm::Sha256JcsV1,
                        sha256_digest_bytes(b"padding-schema"),
                    )
                    .expect("padding schema"),
                    ContentDigest::from_digest(
                        DigestAlgorithm::Sha256V1,
                        sha256_digest_bytes(b"padding-content"),
                    ),
                )
                .expect("padding content ref"),
                canonical_json: format!("\"{}\"", "x".repeat(payload_len)),
            }];
            predecessor = Some(batch.head.clone());
            batches.push(batch);
        }
        export.fixation.journal_head = batches.last().expect("sized batches").head.clone();
        export.batches = batches;
        export.closure_reference = export
            .compute_closure_reference()
            .expect("sized closure reference");
        export
    }

    fn padding_frame(payload_len: usize) -> Result<Vec<u8>, PortableExportError> {
        let export = golden_export();
        let payload = serde_json::to_value(super::PortableBatchPayload {
            batch: padding_batch(payload_len),
            run_id: export.run_id,
        })
        .map_err(|_| PortableExportError::Invalid)?;
        super::encode_frame(&super::PortableFrame {
            kind: super::PortableFrameKind::Batch,
            payload,
        })
    }

    fn padding_batch(payload_len: usize) -> CommittedBatch {
        let mut batch = golden_export().batches[0].clone();
        batch.objects = vec![HistoryObject {
            object_type: StableId::new("mfm.portable-test.padding").expect("padding type"),
            content_ref: ContentRef::new(
                SchemaId::new(
                    "mfm.portable-test.padding",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    sha256_digest_bytes(b"padding-schema"),
                )
                .expect("padding schema"),
                ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(b"padding-content"),
                ),
            )
            .expect("padding content ref"),
            canonical_json: format!("\"{}\"", "x".repeat(payload_len)),
        }];
        batch
    }

    fn json_depth(bytes: &[u8]) -> Option<usize> {
        let mut depth = 0usize;
        let mut max = 0usize;
        let mut in_string = false;
        let mut escape = false;
        for &byte in bytes {
            if in_string {
                if escape {
                    escape = false;
                } else if byte == b'\\' {
                    escape = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth = depth.checked_add(1)?;
                    max = max.max(depth);
                }
                b'}' | b']' => depth = depth.checked_sub(1)?,
                _ => {}
            }
        }
        Some(max)
    }

    /// A distinct synthetic source run identity.
    fn source_run_id(seed: u8, index: usize) -> RunId {
        RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(
                &[
                    b"portable-source-graph".as_slice(),
                    &[seed],
                    &index.to_be_bytes(),
                ]
                .concat(),
            ),
        )
    }
}
