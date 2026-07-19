use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor};
use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
pub use mfm_facts::StoreCommitOrder;
use mfm_ids::{
    short_stable_id_fragment, AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind,
    CapabilityVersion, CellId, ContentDigest, ContextResourceKind, ContextStage, DescriptorId,
    DigestAlgorithm, EventId, IdentityError, NodeId, RunId, RuntimeBindingId, SchemaId, ScopeId,
    SeedId, SemanticTypeId, SideEffectPairId, SpecHash, StateKind, StateVersion,
};
#[cfg(any(test, feature = "test-support"))]
use mfm_ids::{EffectKind, EffectVersion, LoweringVersion, SpecVersion};
use mfm_manual_auth::{ManualResolutionBlockReason, VerifiedManualResolutionForPrefix};
use mfm_spec::v1::{
    self as spec, CanonicalizerIdentity, CellProducer, DescriptorIdentity,
    ManualResolutionEvidenceSpec, MediaType, OperationDescriptorIdentity, PublicFieldPath,
    RemediationUnresolvedSpec, RendererDescriptorIdentity, RendererKind, RendererVersion,
    ResourceNamespace, SagaPolicySpec, SideEffectVerificationSpec, StateDescriptorIdentity,
    TypedExecutionSpec, ValueLineageRef,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use self::codec::{
    optional_obj, optional_str, parse_identity, required_bool, required_obj, required_str,
    required_u32, required_u64, CodecResult,
};

#[path = "errors.rs"]
mod errors;
pub use self::errors::{CodecError, StoreError, StoreErrorInspection};

mod keys;
pub use self::keys::{CommitFingerprint, CommitKey, CommitOrdinal, LogicalEventKey, StreamSeq};

/// Result type for typed store helpers.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Boxed future returned by async typed store adapters.
pub type AsyncStoreFuture<'a, T, E> =
    Pin<Box<dyn Future<Output = std::result::Result<T, E>> + Send + 'a>>;

/// Backend helper APIs for durable store implementations.
pub mod backend;

pub use mfm_ids::StoreScopeId;

mod admission_lanes;
pub use admission_lanes::{
    admission_advisory_lock_key, admission_waiter_id, resource_key_canonical_json,
    resource_wait_fifo_admission_token, AdmissionAdvisoryLockKey, AdmissionLane,
    AdmissionLaneClass, AdmissionLaneId, AdmissionLaneKey, AdmissionLaneMode, AdmissionLease,
    AdmissionModeSpec, AdmissionToken, AdmissionWaiter, AdmissionWaiterId,
    ExecutionClaimAdmissionLane, ExecutionClaimScope, ExecutionClaimStatus, ExpiredExecutionClaim,
    NowaitSkip, NowaitSkipAdmissionBusy, NowaitSkipAdmissionResult, ResourceAdmissionLane,
    WaitFifo, WaitFifoAdmissionBlock, EXECUTION_CLAIM_HEARTBEAT_INTERVAL_SECS,
    EXECUTION_CLAIM_LEASE_TTL_SECS,
};

/// Shared canonical-JSON codec for kernel events, projections, and saga types.
///
/// This module exists so the in-memory store and the Postgres adapter share one
/// implementation of the JSON parse/encode logic instead of maintaining parallel copies kept
/// in lockstep by parity tests. Functions return the backend-neutral [`CodecError`], which each
/// store maps into its own error type via `From`.
pub mod codec;
mod event_codec;
mod event_codec_decode;
mod staging;
mod stream;
mod validation;

#[path = "commit_models.rs"]
mod commit_models;
pub use self::commit_models::*;

pub use self::validation::validate_artifact_requirement_against_evidence;
use self::validation::{
    derive_event_id, derive_logical_key, invalid_prepared_commit_purpose, is_unique_logical_key,
    payload_spec_hash, reject_store_materialized_resource_lane_payloads,
    request_contains_manual_resolution, request_contains_saga_terminal_outcome,
    unique_logical_key_rewrite_allowed, validate_attempt_terminal_commit,
    validate_manual_resolution_commit_with_proof, validate_payload_public_diagnostics,
    validate_payload_run_and_spec, validate_required_artifacts_cover_payload_references,
    validate_retention_commit, validate_retention_manifest_pairs, validate_run_start_commit,
    validate_saga_terminal_commit_with_proof,
    validate_side_effect_attempt_failures_have_terminal_evidence,
    validate_side_effect_progress_commit, validate_side_effect_terminal_commit,
    validate_state_attempt_started_commit, validate_supported_stream_model,
    validate_terminal_attempt_cell_pairs, validate_terminal_side_effect_evidence_pairs,
    validate_unique_artifact_evidence, verify_retained_artifact_bytes,
};

#[cfg(any(test, feature = "test-support"))]
use self::staging::verify_existing_artifact_admissions;
use self::staging::{
    admit_artifact_evidence, admitted_artifact_evidence, artifact_authority_key,
    compare_artifact_field, compare_artifact_option,
};
pub use self::staging::{
    artifact_byte_authority_for_bundle, stage_prepared_commit_plan, CommitBase, StagedCommit,
    StagedCommitOutcome,
};

pub use self::stream::{
    committed_run_stream_canonical_json, committed_run_stream_from_canonical_json_slice,
    CommittedRunStream, CommittedRunStreamCommit, RetainedArtifactReadFuture,
    RetainedArtifactReadProvider, VerifiedRunArtifactBytes, VerifiedRunArtifactStore,
};
use self::stream::{committed_run_stream_commits, validate_run_stream_order};

pub use self::event_codec::{
    error_info_json, event_artifact_json, manual_resolution_note_json,
    manual_resolution_outcome_str, payload_canonical_json, prepared_commit_plan_fingerprint,
    resource_key_evidence_json, resource_touched_set_evidence_json, run_completion_claim_str,
    run_completion_outcome_json, run_completion_outcome_str, side_effect_ledger_purpose_json,
    skip_reason_json,
};
use self::event_codec::{
    kernel_event_envelope_json, parse_kernel_event_envelope, store_artifact_json,
};
use self::event_codec_decode::parse_vec;
pub use self::event_codec_decode::{
    parse_error_category, parse_error_info, parse_event_artifact, parse_failure_phase,
    parse_manual_resolution_note, parse_manual_resolution_outcome, parse_resource_key_evidence,
    parse_resource_touched_set_evidence, parse_run_completion_outcome,
    parse_side_effect_ledger_purpose, parse_skip_reason, payload_from_json_value,
};

pub use self::codec::{
    attempt_projection_json, cell_projection_json, error_category_str, failure_phase_str,
    manual_resolution_projection_json, parse_attempt_projection, parse_cell_projection,
    parse_manual_resolution_projection, parse_public_output_projection,
    parse_resource_lane_projection, parse_retention_manifest_projection,
    parse_run_completion_projection, parse_run_state, parse_saga_engagement_projection,
    parse_side_effect_projection, public_output_projection_json, resource_lane_projection_json,
    retention_manifest_projection_json, retention_ref_json, run_completion_projection_json,
    run_state_str, saga_engagement_projection_json, side_effect_projection_json,
};
use self::codec::{
    required_cell_state_str, required_run_state_str, required_side_effect_state_str,
    retention_reason_str,
};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(any(test, feature = "test-support"))]
mod memory;
#[cfg(any(test, feature = "test-support"))]
pub use memory::AsyncInMemoryRunStore;

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(&value).map_err(|error| StoreError::Serialize(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| StoreError::Canonical(error.to_string()))
}

/// Store-owned typed event envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelEventEnvelope {
    event_id: EventId,
    event_schema_id: SchemaId,
    run_id: RunId,
    seq: StreamSeq,
    store_commit_order: StoreCommitOrder,
    ordinal: CommitOrdinal,
    spec_hash: SpecHash,
    commit_key: CommitKey,
    logical_key: LogicalEventKey,
    payload_hash: ContentDigest,
    payload: KernelEventPayload,
}

/// Raw persisted event row loaded from an authoritative typed run stream.
///
/// Durable stores pass rows through [`KernelEventEnvelope::from_persisted_record`] so replay and
/// projection rebuilds re-derive the semantic envelope fields instead of trusting stored
/// projection tables or caller-supplied metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedKernelEventRecord {
    /// Persisted store-derived event id.
    pub event_id: EventId,
    /// Persisted event schema id.
    pub event_schema_id: SchemaId,
    /// Persisted run id.
    pub run_id: RunId,
    /// Persisted store-owned stream sequence.
    pub seq: StreamSeq,
    /// Persisted store-wide append coordinate.
    pub store_commit_order: StoreCommitOrder,
    /// Persisted ordinal inside the atomic commit.
    pub ordinal: CommitOrdinal,
    /// Persisted payload spec hash.
    pub spec_hash: SpecHash,
    /// Persisted commit key.
    pub commit_key: CommitKey,
    /// Persisted logical event key.
    pub logical_key: LogicalEventKey,
    /// Persisted canonical payload hash.
    pub payload_hash: ContentDigest,
    /// Persisted typed event payload.
    pub payload: KernelEventPayload,
}

impl KernelEventEnvelope {
    /// Reconstructs a store-owned envelope from a persisted typed event row.
    ///
    /// This validates every derived envelope field against the typed payload, run id, sequence,
    /// and ordinal. It is the durable-store read path counterpart to append-time envelope
    /// derivation.
    pub fn from_persisted_record(record: PersistedKernelEventRecord) -> Result<Self> {
        let canonical_payload = payload_canonical_json(&record.payload)?;
        let derived_payload_hash = canonical_payload.content_digest();
        if derived_payload_hash != record.payload_hash {
            return Err(StoreError::PersistedEventMismatch {
                field: "payload_hash",
                message: "persisted payload hash does not match canonical payload".to_owned(),
            });
        }
        let derived_schema_id = record.payload.event_schema_id()?;
        if derived_schema_id != record.event_schema_id {
            return Err(StoreError::PersistedEventMismatch {
                field: "event_schema_id",
                message: "persisted schema id does not match payload variant".to_owned(),
            });
        }
        let derived_spec_hash = payload_spec_hash(&record.payload);
        if derived_spec_hash != record.spec_hash {
            return Err(StoreError::PersistedEventMismatch {
                field: "spec_hash",
                message: "persisted spec hash does not match payload".to_owned(),
            });
        }
        let derived_logical_key = derive_logical_key(
            &record.run_id,
            record.seq,
            record.ordinal,
            &record.payload,
            &record.payload_hash,
        )?;
        if derived_logical_key != record.logical_key {
            return Err(StoreError::PersistedEventMismatch {
                field: "logical_key",
                message: "persisted logical key does not match payload".to_owned(),
            });
        }
        let derived_event_id = derive_event_id(
            &record.run_id,
            record.seq,
            record.ordinal,
            &record.event_schema_id,
            &record.payload_hash,
        )?;
        if derived_event_id != record.event_id {
            return Err(StoreError::PersistedEventMismatch {
                field: "event_id",
                message: "persisted event id does not match envelope inputs".to_owned(),
            });
        }

        Ok(Self {
            event_id: record.event_id,
            event_schema_id: record.event_schema_id,
            run_id: record.run_id,
            seq: record.seq,
            store_commit_order: record.store_commit_order,
            ordinal: record.ordinal,
            spec_hash: record.spec_hash,
            commit_key: record.commit_key,
            logical_key: record.logical_key,
            payload_hash: record.payload_hash,
            payload: record.payload,
        })
    }

    /// Store-derived event id.
    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    /// Event schema id for the payload variant.
    pub fn event_schema_id(&self) -> &SchemaId {
        &self.event_schema_id
    }

    /// Run id owning the stream.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Store-owned stream sequence.
    pub const fn seq(&self) -> StreamSeq {
        self.seq
    }

    /// Store-wide append coordinate for the atomic commit containing this event.
    pub const fn store_commit_order(&self) -> StoreCommitOrder {
        self.store_commit_order
    }

    /// Store-owned ordinal inside the atomic commit.
    pub const fn ordinal(&self) -> CommitOrdinal {
        self.ordinal
    }

    /// Certified spec hash bound to the payload.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Commit key that appended this event.
    pub fn commit_key(&self) -> &CommitKey {
        &self.commit_key
    }

    /// Store-derived logical key.
    pub fn logical_key(&self) -> &LogicalEventKey {
        &self.logical_key
    }

    /// Canonical payload hash.
    pub fn payload_hash(&self) -> &ContentDigest {
        &self.payload_hash
    }

    /// Typed event payload.
    pub fn payload(&self) -> &KernelEventPayload {
        &self.payload
    }
}

/// Artifact evidence stored before events may reference an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactEvidenceRef {
    /// Artifact id.
    pub artifact_id: ArtifactId,
    /// Artifact content digest.
    pub digest: ContentDigest,
    /// Artifact byte length.
    pub byte_len: u64,
    /// Artifact media type.
    pub media_type: MediaType,
    /// Artifact schema id, when schema-bearing.
    pub schema_id: Option<SchemaId>,
    /// Artifact semantic type id, when value-bearing.
    pub semantic_type_id: Option<SemanticTypeId>,
    /// Producer node id, when produced by a node.
    pub producer_node_id: Option<NodeId>,
    /// Producer seed id, when produced by a seed.
    pub producer_seed_id: Option<SeedId>,
    /// Artifact role.
    pub artifact_role: ArtifactRole,
}

impl ArtifactEvidenceRef {
    /// Converts run-admission artifact evidence into exact store artifact evidence.
    pub fn from_run_artifact(artifact: &events::RunArtifactEvidenceRef) -> Self {
        Self {
            artifact_id: artifact.artifact_id.clone(),
            digest: artifact.content_digest.clone(),
            byte_len: artifact.byte_len,
            media_type: artifact.media_type.clone(),
            schema_id: artifact.schema_id.clone(),
            semantic_type_id: artifact.semantic_type_id.clone(),
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: artifact.role,
        }
    }

    /// Computes the canonical evidence hash used by exact-evidence artifact authority.
    pub fn evidence_hash(&self) -> Result<ContentDigest> {
        Ok(canonical_json(store_artifact_json(self))?.content_digest())
    }

    /// Converts this exact artifact evidence into a retention reference.
    pub fn retention_ref(&self) -> Result<events::RetentionRef> {
        Ok(events::RetentionRef {
            artifact_id: self.artifact_id.clone(),
            role: self.artifact_role,
            evidence_hash: self.evidence_hash()?,
            content_digest: self.digest.clone(),
        })
    }
}

/// Required run state for a typed commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum RequiredRunState {
    /// Any current state is accepted.
    #[default]
    Any,
    /// No run stream may exist.
    Absent,
    /// The run must have started.
    Started,
    /// The run must have started and not completed.
    NotCompleted,
    /// The run must already be terminal.
    Completed,
}

/// Cell state required before a typed commit can append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RequiredCellState {
    /// The cell must not have a terminal projection.
    Absent,
    /// The cell must be produced.
    Produced,
    /// The cell must be skipped.
    Skipped,
    /// The cell may be either produced or skipped.
    Terminal,
}

/// Typed cell precondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellStatePrecondition {
    /// Cell id checked by this precondition.
    pub cell_id: CellId,
    /// Required state for the cell.
    pub required: RequiredCellState,
}

/// Side-effect phase required before a typed commit can append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RequiredSideEffectState {
    /// No ledger projection may exist.
    Absent,
    /// Intent must be persisted.
    IntentPersisted,
    /// A claim must exist.
    Claimed,
    /// Invocation must be prepared.
    InvocationPrepared,
    /// Invocation must have started.
    InvocationStarted,
    /// A submission-result phase must be projected.
    SubmissionResult,
    /// A receipt must be projected.
    ReceiptObserved,
    /// A confirmation must be projected.
    ConfirmationObserved,
    /// Ambiguity must be projected.
    Ambiguous,
    /// Failure must be projected.
    Failed,
}

/// Typed side-effect precondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectStatePrecondition {
    /// Certified side-effect pair id checked by this precondition.
    pub pair_id: SideEffectPairId,
    /// Required side-effect phase.
    pub required: RequiredSideEffectState,
}

/// Certified side-effect pair authority used by typed store admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSideEffectPairAuthority {
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Submit node that owns the mutation boundary.
    pub submit_node_id: NodeId,
    /// Verify framework node that owns terminal evidence and output.
    pub verify_node_id: NodeId,
    /// Submit node output cell used as the pair's structural anchor.
    pub submit_output_cell: CellId,
    /// Certified terminal policy for the pair.
    pub terminal_policy: SideEffectTerminalPolicy,
}

impl CertifiedSideEffectPairAuthority {
    /// Returns the certified node id for the given pair role.
    pub fn node_for_role(&self, role: events::SideEffectPairRole) -> &NodeId {
        match role {
            events::SideEffectPairRole::Submit => &self.submit_node_id,
            events::SideEffectPairRole::Verify => &self.verify_node_id,
        }
    }

    /// Returns the side-effect state that satisfies successful terminal output.
    pub const fn terminal_state_required(&self) -> RequiredSideEffectState {
        match self.terminal_policy {
            SideEffectTerminalPolicy::Receipt => RequiredSideEffectState::ReceiptObserved,
            SideEffectTerminalPolicy::Confirmation => RequiredSideEffectState::ConfirmationObserved,
        }
    }
}

/// Certified store admission authority for a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedRunStoreAuthority {
    run_id: RunId,
    spec_hash: SpecHash,
    saga_policy_digest: ContentDigest,
    saga_policy: SagaPolicySpec,
    terminal_policies: SideEffectTerminalPolicies,
    side_effect_pairs: BTreeMap<SideEffectPairId, CertifiedSideEffectPairAuthority>,
    fact_descriptor_allowlists: BTreeMap<NodeId, BTreeSet<ContentDigest>>,
}

impl CertifiedRunStoreAuthority {
    /// Mints store admission authority from a certified typed spec.
    pub fn from_spec(run_id: RunId, spec: &TypedExecutionSpec) -> Result<Self> {
        let spec_hash = spec
            .spec_hash()
            .map_err(|error| StoreError::Canonical(error.to_string()))?;
        let saga_policy = spec.saga.clone();
        let saga_policy_digest = saga_policy
            .saga_policy_digest()
            .map_err(|error| StoreError::Canonical(error.to_string()))?;
        let terminal_policies = SideEffectTerminalPolicies::from_spec(spec)?;
        let mut side_effect_pairs = BTreeMap::new();
        let mut fact_descriptor_allowlists = BTreeMap::new();
        for node in spec.nodes.iter().chain(spec.remediations.values()) {
            let allowlist = node
                .fact_descriptor_allowlist
                .iter()
                .map(|reference| reference.descriptor_hash.clone())
                .collect::<BTreeSet<_>>();
            if fact_descriptor_allowlists
                .insert(node.node_id.clone(), allowlist)
                .is_some()
            {
                return Err(StoreError::Identity(format!(
                    "duplicate node authority for {}",
                    node.node_id
                )));
            }
            if node.side_effect.is_none() {
                continue;
            }
            let pair = spec
                .side_effect_verify_pair_for_submit_node(&node.node_id)
                .map_err(|error| StoreError::Identity(error.to_string()))?;
            let terminal_policy =
                SideEffectTerminalPolicy::from_verification(&pair.submit_contract.verification);
            let authority = CertifiedSideEffectPairAuthority {
                pair_id: pair.pair_id.clone(),
                submit_node_id: pair.submit_node.node_id.clone(),
                verify_node_id: pair.verify_node.node_id.clone(),
                submit_output_cell: pair.submit_output_cell.clone(),
                terminal_policy,
            };
            if side_effect_pairs
                .insert(authority.pair_id.clone(), authority)
                .is_some()
            {
                return Err(StoreError::Identity(format!(
                    "duplicate side-effect pair authority for {}",
                    pair.pair_id
                )));
            }
        }
        Ok(Self {
            run_id,
            spec_hash,
            saga_policy_digest,
            saga_policy,
            terminal_policies,
            side_effect_pairs,
            fact_descriptor_allowlists,
        })
    }

    /// Returns the authority run id.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the certified spec hash bound by this token.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Returns the canonical saga policy digest bound by this token.
    pub fn saga_policy_digest(&self) -> &ContentDigest {
        &self.saga_policy_digest
    }

    /// Returns the certified saga policy carried by this token.
    pub fn saga_policy(&self) -> &SagaPolicySpec {
        &self.saga_policy
    }

    /// Returns the certified side-effect terminal policies carried by this token.
    pub fn terminal_policies(&self) -> &SideEffectTerminalPolicies {
        &self.terminal_policies
    }

    /// Returns certified side-effect pair authority.
    pub fn side_effect_pair(
        &self,
        pair_id: &SideEffectPairId,
    ) -> Result<&CertifiedSideEffectPairAuthority> {
        self.side_effect_pairs
            .get(pair_id)
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("sidefx_pair:{pair_id}"),
                message: "missing certified side-effect pair authority".to_owned(),
            })
    }

    /// Iterates certified side-effect pair authorities.
    pub fn side_effect_pairs(
        &self,
    ) -> impl Iterator<Item = (&SideEffectPairId, &CertifiedSideEffectPairAuthority)> {
        self.side_effect_pairs.iter()
    }

    /// Requires that a certified node is allowed to emit a fact descriptor hash.
    pub fn require_fact_descriptor_allowed(
        &self,
        node_id: &NodeId,
        descriptor_hash: &ContentDigest,
    ) -> Result<()> {
        let Some(allowlist) = self.fact_descriptor_allowlists.get(node_id) else {
            return Err(StoreError::ProjectionConflict {
                key: format!("fact_descriptor_allowlist:{node_id}"),
                message: "missing certified node fact descriptor allowlist".to_owned(),
            });
        };
        if allowlist.contains(descriptor_hash) {
            return Ok(());
        }
        Err(StoreError::ProjectionConflict {
            key: format!("fact_descriptor_allowlist:{node_id}:{descriptor_hash}"),
            message: "fact descriptor is not certified for producing node".to_owned(),
        })
    }
}

mod saga_models;
pub use self::saga_models::*;
use self::saga_models::{manual_block_reason_from_auth, manual_policy_for_block_reason};

use self::saga::{derive_saga_projection, forward_ledgers_quiescent};
mod saga;

/// Observation-only status for run list/watch pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedRunStatus {
    /// The run has been admitted and has not completed in the observed frontier.
    Started,
    /// The run has a committed terminal event in the observed frontier.
    Completed,
}

impl ObservedRunStatus {
    /// Returns the stable public tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
        }
    }

    /// Parses a stable public tag.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "started" => Ok(Self::Started),
            "completed" => Ok(Self::Completed),
            other => Err(StoreError::ObservationUnavailable {
                message: format!("unknown observed run status {other}"),
            }),
        }
    }
}

/// One run row returned by the observation list/watch API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunObservation {
    /// Run id.
    pub run_id: RunId,
    /// Observed stream head at the sealed frontier.
    pub head_seq: StreamSeq,
    /// Observation-only run status.
    pub observed_status: ObservedRunStatus,
    /// First observed commit timestamp as RFC3339 text.
    pub started_at: String,
    /// Latest observed commit timestamp as RFC3339 text.
    pub updated_at: String,
    /// Completion timestamp when the observed status is completed.
    pub completed_at: Option<String>,
    /// Optional opaque change identifier for this observation row.
    pub change_id: Option<String>,
}

/// Query for the shared run observation list/watch API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunObservationQuery {
    /// Opaque cursor returned by a previous page.
    pub cursor: Option<String>,
    /// Maximum rows to return. v1 has no separate list-page cursor.
    pub limit: u32,
    /// Long-poll wait in milliseconds. A timeout returns an empty successful page.
    pub wait_ms: u64,
}

impl RunObservationQuery {
    /// Creates a bounded observation query.
    pub fn new(cursor: Option<String>, limit: u32, wait_ms: u64) -> Self {
        Self {
            cursor,
            limit,
            wait_ms,
        }
    }
}

/// One run observation list/watch page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunObservationPage {
    /// Opaque cursor for the returned sealed frontier.
    pub next_cursor: String,
    /// Observed run rows.
    pub runs: Vec<RunObservation>,
}

/// Read-only store-owned deployment scope.
pub trait StoreScopeStore {
    /// Store-specific error type.
    type Error: StoreErrorInspection + fmt::Display + Send + Sync + 'static;

    /// Loads the store-owned deployment scope id.
    fn load_store_scope_id<'a>(&'a self) -> AsyncStoreFuture<'a, StoreScopeId, Self::Error>;
}

/// Internal storage boundary for observation list/watch pages.
pub trait RunObservationStore {
    /// Backend-specific error.
    type Error: Send + Sync + 'static;

    /// Reads one bounded list/watch observation page.
    fn read_run_observations<'a>(
        &'a self,
        query: RunObservationQuery,
    ) -> AsyncStoreFuture<'a, RunObservationPage, Self::Error>;
}

/// Async run event store commit contract for durable stores.
///
/// Runtime execution code must derive run-local read views from the authoritative stream
/// returned by [`Self::load_run_stream`]. App status rendering may additionally request
/// store-owned cross-run projection authority through [`Self::status_projection_snapshot`].
pub trait RunEventStore {
    /// Store-specific error type.
    type Error: StoreErrorInspection + fmt::Display + Send + Sync + 'static;

    /// Atomically appends one purpose-specific prepared commit bundle.
    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: PreparedCommitBundle,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error>;

    /// Loads the authoritative run stream.
    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, Vec<KernelEventEnvelope>, Self::Error>;

    /// Loads the authoritative committed run stream with retained artifact projection authority.
    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, CommittedRunStream, Self::Error>;

    /// Returns the next store-owned stream sequence for a run.
    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, StreamSeq, Self::Error>;

    /// Returns store-owned projection authority for public run status.
    ///
    /// Implementations that maintain cross-run projection families should include those
    /// families here. Callers still rebuild the queried run's local projection from its
    /// authoritative stream before rendering status.
    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error>;

    /// Rebuilds store-scoped fact authority from committed events and exact retained artifacts.
    ///
    /// This is store-scoped rather than run-scoped: public Platform fact discovery and
    /// exact-ref lookup must see all indexed public facts available in the store, not only
    /// facts associated with an arbitrary run selected by the caller. Physical descriptor,
    /// index, and term tables may accelerate discovery but are not semantic authority.
    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error>;
}

/// Mutable execution-claim coordination store.
///
/// Execution claims are operational liveness state, not append-only run authority. A claim lease
/// coordinates which process is actively driving a run, while per-run append validation and
/// side-effect resource-lane authority remain the safety boundary.
pub trait ExecutionClaimStore {
    /// Store-specific error type.
    type Error: StoreErrorInspection + fmt::Display + Send + Sync + 'static;

    /// Attempts to acquire the execution claim for `scope` with a caller-supplied holder token.
    ///
    /// Stores must not auto-reap expired holders during acquisition. If any holder token is present,
    /// including an expired one, the result is [`NowaitSkipAdmissionResult::Busy`].
    fn acquire_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: AdmissionToken,
    ) -> AsyncStoreFuture<'a, NowaitSkipAdmissionResult, Self::Error>;

    /// Returns the current holder status for the execution claim without mutating it.
    fn execution_claim_status<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
    ) -> AsyncStoreFuture<'a, ExecutionClaimStatus, Self::Error>;

    /// Renews the execution claim only when `holder_run_id` and `token` match the current holder.
    fn renew_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, Option<AdmissionLease>, Self::Error>;

    /// Releases the execution claim only when `holder_run_id` and `token` match the current holder.
    fn release_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, bool, Self::Error>;

    /// Lists currently expired execution claims for explicit recovery or reaping.
    fn expired_execution_claims<'a>(
        &'a self,
    ) -> AsyncStoreFuture<'a, Vec<ExpiredExecutionClaim>, Self::Error>;

    /// Clears an expired execution claim only when `holder_run_id` and `token` still match the current holder.
    fn reap_expired_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, bool, Self::Error>;
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitKeyRecord {
    fingerprint: CommitFingerprint,
    batch: CommittedBatch,
}

/// Set of logical keys already present for run streams.
pub type LogicalKeySet = BTreeSet<(RunId, LogicalEventKey)>;

/// Unique logical-key payload hashes already present for run streams.
pub type UniqueLogicalPayloads = BTreeMap<(RunId, LogicalEventKey), ContentDigest>;

/// Exact artifact authority key: content identity plus canonical evidence identity.
pub type ArtifactAuthorityKey = (ArtifactId, ContentDigest);

/// Artifact authority indexed by exact `(artifact_id, evidence_hash)`.
pub type ArtifactAuthorityMap = BTreeMap<ArtifactAuthorityKey, ArtifactEvidenceRef>;

/// Artifact bytes indexed by exact `(artifact_id, evidence_hash)` authority.
pub type ArtifactByteAuthorityMap = BTreeMap<ArtifactAuthorityKey, (Vec<u8>, ArtifactEvidenceRef)>;

use self::admission::require_admission_preconditions;

mod admission;

mod private {
    pub trait Sealed {}
}

pub use mfm_events::v1::{EventArtifactReferenceSource, EventArtifactRequirement};

use self::artifact_refs::referenced_artifact_ids;

mod artifact_refs;

mod projection;
use self::projection::fact_claim_projection_key;
pub use self::projection::{
    AttemptProjection, AttemptStatus, CellTerminalProjection, FactDescriptorProjection,
    FactIndexProjection, FactIndexTermProjection, FactRecordProjection, ProjectionSnapshot,
    ProjectionSnapshotParts, PublicOutputProjection, RetentionManifestProjection,
    RetentionProjection,
};

use self::resource_lanes::{
    acquire_resource_lane, release_resource_lane, require_no_resource_lane_for_holder,
    require_no_resource_lanes_for_run, resolve_active_resource_lane_release,
    ResourceLaneReleaseMatch,
};
pub use self::resource_lanes::{
    resource_lane_release_intent_resolution, ResourceLaneAuthority, ResourceLaneAuthoritySet,
    ResourceLaneKey, ResourceLaneProjection,
};
use self::side_effects::{
    note_saga_engagement, prepared_invocation_projection, require_active_attempt_for_side_effect,
    require_forward_fence_open, require_remediation_intent_admissible,
    require_side_effect_pair_consistent, require_side_effect_pair_role, require_side_effect_phase,
    require_side_effect_purpose, side_effect_projection_error, transition_side_effect_epoch_only,
    transition_side_effect_failure, EpochOnlyTransition, PairRoleRequirement,
};

mod resource_lanes;

use self::side_effects::OwnedSideEffectLedgerState;
pub use self::side_effects::{
    SideEffectArtifactProjection, SideEffectClaimProjection, SideEffectIntentProjection,
    SideEffectLedgerPhase, SideEffectLedgerState, SideEffectPairLedgerRef, SideEffectPhase,
    SideEffectProjection, SideEffectSubmissionState,
};

mod side_effects;
