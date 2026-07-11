use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor};
use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
pub use mfm_facts::StoreCommitOrder;
use mfm_ids::{
    short_stable_id_fragment, AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind,
    CapabilityVersion, CellId, ContentDigest, ContextResourceKind, ContextStage, DescriptorId,
    DigestAlgorithm, EventId, IdentityError, NodeId, RunId, SchemaId, ScopeId, SeedId,
    SemanticTypeId, SideEffectPairId, SpecHash, StateKind, StateVersion, VisibleAscii512,
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

/// Result type for typed store helpers.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Boxed future returned by async typed store adapters.
pub type AsyncStoreFuture<'a, T, E> =
    Pin<Box<dyn Future<Output = std::result::Result<T, E>> + Send + 'a>>;

/// Error returned by typed store contract validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// A commit contained no payloads.
    #[error("typed commit cannot be empty")]
    EmptyCommit,
    /// Stream sequence arithmetic overflowed.
    #[error("typed stream sequence overflowed")]
    SequenceOverflow,
    /// A payload was bound to a different run.
    #[error("payload run mismatch: expected {expected}, got {actual}")]
    PayloadRunMismatch {
        /// Run supplied to the commit API.
        expected: Box<RunId>,
        /// Run found inside the payload.
        actual: Box<RunId>,
    },
    /// Payloads in one commit carried different certified spec hashes.
    #[error("payload spec hash mismatch: expected {expected}, got {actual}")]
    PayloadSpecHashMismatch {
        /// First payload spec hash.
        expected: Box<SpecHash>,
        /// Later payload spec hash.
        actual: Box<SpecHash>,
    },
    /// A purpose-specific prepared commit constructor rejected the payload/precondition shape.
    #[error("invalid prepared {purpose} commit: {message}")]
    InvalidPreparedCommitPurpose {
        /// Purpose constructor that rejected the request.
        purpose: &'static str,
        /// Stable diagnostic.
        message: String,
    },
    /// The commit key was reused for a different canonical commit.
    #[error("commit key reused for different payloads: {commit_key}")]
    CommitConflict {
        /// Reused commit key.
        commit_key: CommitKey,
    },
    /// The caller's expected next sequence is stale.
    #[error("stale expected_next_seq: expected {expected}, actual {actual}")]
    StaleExpectedNextSeq {
        /// Expected sequence supplied by the caller.
        expected: StreamSeq,
        /// Actual next sequence owned by the store.
        actual: StreamSeq,
    },
    /// A required logical key precondition failed.
    #[error("logical key precondition failed for {logical_key}: {message}")]
    LogicalKeyPreconditionFailed {
        /// Logical key that failed the precondition.
        logical_key: LogicalEventKey,
        /// Stable diagnostic.
        message: String,
    },
    /// A run-state precondition failed.
    #[error("run state precondition failed: required {required:?}, actual {actual:?}")]
    RunStatePreconditionFailed {
        /// Required run state.
        required: RequiredRunState,
        /// Actual run state.
        actual: RunState,
    },
    /// A cell-state precondition failed.
    #[error("cell state precondition failed for {cell_id}: required {required:?}")]
    CellStatePreconditionFailed {
        /// Cell id that failed the precondition.
        cell_id: CellId,
        /// Required cell state.
        required: RequiredCellState,
    },
    /// A side-effect-state precondition failed.
    #[error("side-effect state precondition failed for pair {pair_id}: required {required:?}")]
    SideEffectStatePreconditionFailed {
        /// Certified pair id that failed the precondition.
        pair_id: SideEffectPairId,
        /// Required side-effect state.
        required: RequiredSideEffectState,
    },
    /// A public output was already projected when absence was required.
    #[error("public output absence precondition failed")]
    PublicOutputPreconditionFailed,
    /// A referenced artifact is missing from the store-owned artifact evidence table.
    #[error("missing artifact evidence for {artifact_id}")]
    MissingArtifact {
        /// Missing artifact id.
        artifact_id: ArtifactId,
    },
    /// Stored artifact evidence does not match a required artifact reference.
    #[error("artifact evidence mismatch for {artifact_id} field {field}")]
    ArtifactEvidenceMismatch {
        /// Artifact id with mismatched evidence.
        artifact_id: ArtifactId,
        /// Mismatched field label.
        field: &'static str,
    },
    /// Artifact bytes or metadata could not be loaded from retained evidence storage.
    #[error("failed to read retained artifact {artifact_id}")]
    ArtifactReadFailed {
        /// Artifact id whose retained bytes could not be loaded.
        artifact_id: ArtifactId,
    },
    /// A prepared commit tried to admit artifact evidence that no event in the commit
    /// references.
    #[error("prepared commit admitted unreferenced artifact evidence {artifact_id}")]
    UnreferencedArtifactEvidence {
        /// Unreferenced artifact id.
        artifact_id: ArtifactId,
    },
    /// A logical key that must be unique already exists.
    #[error("duplicate logical key {logical_key}")]
    DuplicateLogicalKey {
        /// Duplicate logical key.
        logical_key: LogicalEventKey,
    },
    /// A logical key conflict would corrupt an existing projection.
    #[error("logical key conflict {logical_key}")]
    LogicalKeyConflict {
        /// Conflicting logical key.
        logical_key: LogicalEventKey,
    },
    /// A projection transition would corrupt an existing projection.
    #[error("projection conflict for {key}: {message}")]
    ProjectionConflict {
        /// Projection key.
        key: String,
        /// Stable diagnostic.
        message: String,
    },
    /// A run observation cursor was malformed, tampered, or belongs to an unsupported format.
    #[error("invalid run observation cursor: {message}")]
    InvalidCursor {
        /// Stable diagnostic.
        message: String,
    },
    /// A run observation cursor belongs to a previous store epoch.
    #[error("run observation cursor expired")]
    CursorExpired,
    /// A run observation limit was outside the supported v1 range.
    #[error("run observation limit {limit} is outside 1..={max}")]
    LimitOutOfRange {
        /// Requested limit.
        limit: u32,
        /// Maximum accepted limit.
        max: u32,
    },
    /// Required observation rows were unavailable or corrupt.
    #[error("run observation unavailable: {message}")]
    ObservationUnavailable {
        /// Stable diagnostic.
        message: String,
    },
    /// A prepared commit bundle did not carry bytes or exact existing evidence for an admitted artifact.
    #[error("prepared commit bundle missing artifact bytes for {artifact_id}")]
    MissingPreparedArtifactBytes {
        /// Artifact id missing from the bundle.
        artifact_id: ArtifactId,
    },
    /// A prepared commit bundle carried artifact bytes not admitted by the prepared authority.
    #[error("prepared commit bundle carried extra artifact bytes for {artifact_id}")]
    ExtraPreparedArtifactBytes {
        /// Extra artifact id carried by the bundle.
        artifact_id: ArtifactId,
    },
    /// A prepared commit bundle carried duplicate artifact bytes or evidence references.
    #[error("prepared commit bundle carried duplicate artifact evidence for {artifact_id}")]
    DuplicatePreparedArtifactBytes {
        /// Duplicated artifact id.
        artifact_id: ArtifactId,
    },
    /// A persisted event row disagrees with store-derived typed event fields.
    #[error("persisted event mismatch for {field}: {message}")]
    PersistedEventMismatch {
        /// Mismatched field label.
        field: &'static str,
        /// Stable diagnostic.
        message: String,
    },
    /// Identity construction failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// JSON serialization failed before canonicalization.
    #[error("store JSON serialization error: {0}")]
    Serialize(String),
    /// Canonical JSON construction failed.
    #[error("store canonicalization error: {0}")]
    Canonical(String),
    /// Event schema id construction failed.
    #[error("event contract error: {0}")]
    Event(String),
}

/// Exposes wrapped typed store errors without parsing display strings.
pub trait StoreErrorInspection {
    /// Returns the typed store error when this error wraps one.
    fn as_store_error(&self) -> Option<&StoreError>;
}

impl StoreErrorInspection for StoreError {
    fn as_store_error(&self) -> Option<&StoreError> {
        Some(self)
    }
}

impl From<IdentityError> for StoreError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

impl From<CodecError> for StoreError {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::Field(message) => Self::Event(message),
            CodecError::Identity(message) => Self::Identity(message),
        }
    }
}

impl From<IdentityError> for CodecError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

impl From<mfm_events::EventError> for CodecError {
    fn from(error: mfm_events::EventError) -> Self {
        Self::Field(error.to_string())
    }
}

impl From<mfm_events::EventError> for StoreError {
    fn from(error: mfm_events::EventError) -> Self {
        Self::Event(error.to_string())
    }
}

impl From<mfm_capabilities::CapabilityError> for StoreError {
    fn from(error: mfm_capabilities::CapabilityError) -> Self {
        Self::Identity(error.to_string())
    }
}

/// Backend-neutral error for the shared kernel JSON codec.
///
/// The codec is reused by the in-memory store and the Postgres adapter; each backend maps this
/// into its own error type via `From`, so the parse/encode logic lives in exactly one place.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    /// A required JSON field was missing or had the wrong shape.
    #[error("codec field error: {0}")]
    Field(String),
    /// A typed identity, digest, or enum tag failed to parse.
    #[error("codec identity error: {0}")]
    Identity(String),
}

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
mod staging;
mod stream;
mod validation;

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
    manual_resolution_outcome_str, parse_error_category, parse_error_info, parse_event_artifact,
    parse_failure_phase, parse_manual_resolution_note, parse_manual_resolution_outcome,
    parse_resource_key_evidence, parse_resource_touched_set_evidence, parse_run_completion_outcome,
    parse_side_effect_ledger_purpose, parse_skip_reason, payload_canonical_json,
    payload_from_json_value, prepared_commit_plan_fingerprint, resource_key_evidence_json,
    resource_touched_set_evidence_json, run_completion_claim_str, run_completion_outcome_json,
    run_completion_outcome_str, side_effect_ledger_purpose_json, skip_reason_json,
};
use self::event_codec::{
    kernel_event_envelope_json, parse_kernel_event_envelope, parse_vec, store_artifact_json,
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

fn checked_store_key(field: &'static str, value: impl AsRef<str>) -> Result<VisibleAscii512> {
    let value = value.as_ref();
    VisibleAscii512::new(value)
        .map_err(|_| StoreError::Identity(format!("{field} must be 1..=512 visible ASCII bytes")))
}

/// Contiguous store-owned sequence for an atomic run commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamSeq(u64);

impl StreamSeq {
    /// First run stream sequence.
    pub const FIRST: Self = Self(1);

    /// Creates a non-zero stream sequence for caller preconditions.
    pub fn new(value: u64) -> Result<Self> {
        if value == 0 {
            return Err(StoreError::Identity(
                "stream sequence must be non-zero".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the sequence as a `u64`.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    fn checked_next(self) -> Result<Self> {
        Self::new(self.0.checked_add(1).ok_or(StoreError::SequenceOverflow)?)
    }
}

impl fmt::Display for StreamSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Store-owned ordinal for one event inside an atomic commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitOrdinal(u32);

impl CommitOrdinal {
    /// Creates an ordinal loaded from a persisted typed event row.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the ordinal as a `u32`.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    fn from_index(index: usize) -> Result<Self> {
        let value = u32::try_from(index).map_err(|_| StoreError::SequenceOverflow)?;
        Ok(Self(value))
    }
}

impl fmt::Display for CommitOrdinal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Commit-key idempotency key supplied by a typed scheduler.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitKey(VisibleAscii512);

impl CommitKey {
    /// Creates a checked commit key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_store_key("commit key", value).map(Self)
    }

    /// Returns the persisted commit key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for CommitKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Store-derived logical event key used for duplicate/conflict checks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalEventKey(VisibleAscii512);

impl LogicalEventKey {
    /// Creates a checked logical key for typed preconditions.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_store_key("logical event key", value).map(Self)
    }

    /// Returns the persisted logical key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for LogicalEventKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical fingerprint of one typed commit attempt.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitFingerprint(ContentDigest);

impl CommitFingerprint {
    /// Returns the content digest backing this fingerprint.
    pub fn as_digest(&self) -> &ContentDigest {
        &self.0
    }

    /// Reconstructs a commit fingerprint loaded from durable authority rows.
    pub fn from_digest(digest: ContentDigest) -> Self {
        Self(digest)
    }
}

impl fmt::Display for CommitFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
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

/// Run state used by typed commit preconditions and projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunState {
    /// No start event has been committed.
    Absent,
    /// Run has started and is not terminal.
    Started,
    /// Run has reached a terminal outcome.
    Completed,
}

/// Stream-derived semantic run mode for saga-aware status and terminal resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunMode {
    /// Forward graph execution is still the active frontier.
    Forward,
    /// Remediation work is the active frontier.
    Remediating,
    /// The run requires typed operator evidence before terminal resolution.
    ManualBlocked,
    /// Successful forward public output completed the run.
    Completed,
    /// The run completed after closing compensating obligations.
    Compensated,
    /// The run completed after accepted manual remediation evidence.
    ManuallyResolved,
    /// The run completed without making a compensation or AC/DC-equivalence claim.
    FailedWithoutAcdcClaim,
}

impl RunMode {
    /// Returns the canonical snake-case tag for this run mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Remediating => "remediating",
            Self::ManualBlocked => "manual_blocked",
            Self::Completed => "completed",
            Self::Compensated => "compensated",
            Self::ManuallyResolved => "manually_resolved",
            Self::FailedWithoutAcdcClaim => "failed_without_acdc_claim",
        }
    }

    /// Returns the saga terminal outcome represented by this run mode, when terminal.
    pub fn saga_terminal_outcome(self) -> Option<events::RunCompletionOutcome> {
        match self {
            Self::Compensated => Some(events::RunCompletionOutcome::Compensated),
            Self::ManuallyResolved => Some(events::RunCompletionOutcome::ManuallyResolved),
            Self::FailedWithoutAcdcClaim => {
                Some(events::RunCompletionOutcome::FailedWithoutAcdcClaim)
            }
            Self::Forward | Self::Remediating | Self::ManualBlocked | Self::Completed => None,
        }
    }

    /// Returns the canonical run mode represented by a committed run-completion outcome.
    pub fn from_completion_outcome(outcome: &events::RunCompletionOutcome) -> Self {
        match outcome {
            events::RunCompletionOutcome::Completed(_) => Self::Completed,
            events::RunCompletionOutcome::Compensated => Self::Compensated,
            events::RunCompletionOutcome::ManuallyResolved => Self::ManuallyResolved,
            events::RunCompletionOutcome::FailedWithoutAcdcClaim => Self::FailedWithoutAcdcClaim,
        }
    }
}

/// Reason the derived saga mode is manually blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManualBlockReason {
    /// Certified run policy requires operator resolution after engagement.
    PolicyManualResolution,
    /// A forward ledger is ambiguous at quiescence.
    ForwardAmbiguous,
    /// A remediation ledger failed non-retryably.
    RemediationFailed,
    /// A remediation ledger is ambiguous.
    RemediationAmbiguous,
}

/// First stream event that engaged saga handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaEngagementProjection {
    /// Store-owned event id that first engaged saga handling.
    pub event_id: EventId,
    /// Reason saga handling engaged.
    pub reason: SagaEngagementReason,
}

/// Stream-derived reason for saga engagement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SagaEngagementReason {
    /// A non-retryable attempt or side-effect failure was recorded.
    NonRetryableFailure {
        /// Node id that failed.
        node_id: NodeId,
        /// Attempt id that failed.
        attempt_id: AttemptId,
    },
    /// A forward side-effect pair became ambiguous.
    ForwardAmbiguous {
        /// Forward pair id.
        pair_id: SideEffectPairId,
    },
}

/// Run-scoped manual resolution projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionProjection {
    /// Store-owned event id that recorded manual resolution.
    pub event_id: EventId,
    /// Operator-selected outcome.
    pub outcome: events::ManualResolutionOutcome,
    /// Evidence schema id.
    pub evidence_schema_id: SchemaId,
    /// Evidence hash.
    pub evidence_hash: ContentDigest,
    /// Evidence artifact id.
    pub evidence_artifact_id: ArtifactId,
    /// Authorization proof schema id.
    pub authorization_schema_id: SchemaId,
    /// Authorization proof hash.
    pub authorization_hash: ContentDigest,
    /// Authorization proof artifact id.
    pub authorization_artifact_id: ArtifactId,
    /// Optional redaction-safe operator note.
    pub note: Option<events::ManualResolutionNote>,
}

/// Run completion projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCompletionProjection {
    /// Store-owned event id that recorded completion.
    pub event_id: EventId,
    /// Recorded run completion outcome.
    pub outcome: events::RunCompletionOutcome,
}

/// Forward ledger classification at the current stream prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForwardLedgerClassification {
    /// The ledger is not owed, remediated, or unresolvable for compensation decisions yet.
    Pending,
    /// The ledger owes no compensation.
    NothingOwed,
    /// The ledger is confirmed and must be remediated under compensating policy.
    Owed,
    /// The ledger is ambiguous and cannot be platform-compensated.
    Unresolvable,
}

/// Remediation state linked to one forward ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemediationLedgerProjection {
    /// Remediation ledger key.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Remediation side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Current remediation side-effect phase.
    pub phase: SideEffectPhase,
    /// Whether remediation confirmation closed the obligation.
    pub closed: bool,
    /// Whether remediation reached an unresolved condition.
    pub unresolved: Option<ManualBlockReason>,
}

/// Derived obligation state for one forward ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaObligationProjection {
    /// Forward ledger key.
    pub forward_ledger_key: events::SideEffectLedgerKey,
    /// Forward side-effect pair id.
    pub forward_pair_id: SideEffectPairId,
    /// Current forward side-effect phase.
    pub forward_phase: SideEffectPhase,
    /// Forward ledger classification at this stream prefix.
    pub classification: ForwardLedgerClassification,
    /// Linked remediation ledger state, when one exists.
    pub remediation: Option<RemediationLedgerProjection>,
}

/// Saga projection derived from certified policy plus the store stream projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaProjection {
    /// Run id this projection describes.
    pub run_id: RunId,
    /// Derived semantic run mode.
    pub run_mode: RunMode,
    /// First engaging event, if any.
    pub engagement: Option<SagaEngagementProjection>,
    /// Whether every past-boundary forward ledger is quiescent.
    pub forward_quiescent: bool,
    /// Derived manual block reason, when manually blocked.
    pub manual_block_reason: Option<ManualBlockReason>,
    /// Derived forward-pair obligation state.
    pub obligations: BTreeMap<SideEffectPairId, SagaObligationProjection>,
    /// Manual resolution evidence recorded for the run, if any.
    pub manual_resolution: Option<ManualResolutionProjection>,
    /// Run completion recorded for the run, if any.
    pub run_completion: Option<RunCompletionProjection>,
}

/// Certified terminal evidence policy for a side-effect pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SideEffectTerminalPolicy {
    /// Receipt evidence is the terminal side-effect proof.
    Receipt,
    /// Confirmation evidence is the terminal side-effect proof.
    Confirmation,
}

impl SideEffectTerminalPolicy {
    /// Returns the terminal policy implied by a certified side-effect verification spec.
    pub const fn from_verification(verification: &SideEffectVerificationSpec) -> Self {
        match verification {
            SideEffectVerificationSpec::Receipt => Self::Receipt,
            SideEffectVerificationSpec::Finalized { .. } => Self::Confirmation,
        }
    }

    /// Returns whether the projected phase satisfies this terminal policy.
    pub const fn is_terminal_phase(self, phase: &SideEffectPhase) -> bool {
        match self {
            Self::Receipt => matches!(
                phase,
                SideEffectPhase::ReceiptObserved { .. }
                    | SideEffectPhase::ConfirmationObserved { .. }
            ),
            Self::Confirmation => matches!(phase, SideEffectPhase::ConfirmationObserved { .. }),
        }
    }
}

/// Certified terminal policies for side-effect pairs in a typed spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectTerminalPolicies {
    by_pair: BTreeMap<SideEffectPairId, SideEffectTerminalPolicy>,
}

impl SideEffectTerminalPolicies {
    /// Builds terminal policies for every side-effect node in a certified typed spec.
    pub fn from_spec(spec: &TypedExecutionSpec) -> Result<Self> {
        let mut by_pair = BTreeMap::new();
        for node in spec.nodes.iter().chain(spec.remediations.values()) {
            let Some(contract) = node.side_effect.as_ref() else {
                continue;
            };
            let pair_id = spec::side_effect_pair_id(&node.node_id, &node.output_cell, contract)
                .map_err(|error| StoreError::Identity(error.to_string()))?;
            by_pair.insert(
                pair_id,
                SideEffectTerminalPolicy::from_verification(&contract.verification),
            );
        }
        Ok(Self { by_pair })
    }

    /// Builds terminal policies from explicit pair entries.
    pub fn new(by_pair: BTreeMap<SideEffectPairId, SideEffectTerminalPolicy>) -> Self {
        Self { by_pair }
    }

    /// Returns the policy for a side-effect pair.
    pub fn get(&self, pair_id: &SideEffectPairId) -> Option<SideEffectTerminalPolicy> {
        self.by_pair.get(pair_id).copied()
    }

    /// Returns the policy for a side-effect pair or a typed projection error.
    pub fn require(&self, pair_id: &SideEffectPairId) -> Result<SideEffectTerminalPolicy> {
        self.get(pair_id)
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("sidefx:{pair_id}:terminal_policy"),
                message: "missing certified side-effect terminal policy".to_owned(),
            })
    }
}

fn require_closed_obligations_non_empty(
    policy: &SagaPolicySpec,
    saga: &SagaProjection,
) -> Result<()> {
    if !matches!(policy, SagaPolicySpec::CompensateCompleted { .. }) {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "compensated terminal requires compensating saga policy".to_owned(),
        });
    }
    if saga.run_mode != RunMode::Compensated {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: format!(
                "compensated terminal requires compensated saga mode, found {}",
                saga.run_mode.as_str()
            ),
        });
    }
    let mut closed_count = 0;
    for obligation in saga.obligations.values() {
        if obligation.classification != ForwardLedgerClassification::Owed {
            continue;
        }
        match obligation.remediation.as_ref() {
            Some(remediation) if remediation.closed && remediation.unresolved.is_none() => {
                closed_count += 1;
            }
            _ => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "run:{}:obligation:{}",
                        saga.run_id, obligation.forward_ledger_key
                    ),
                    message: "compensated terminal requires every owed obligation to be closed"
                        .to_owned(),
                });
            }
        }
    }
    if closed_count == 0 {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "compensated terminal requires a non-empty owed obligation set".to_owned(),
        });
    }
    Ok(())
}

fn require_failed_without_acdc_claim(
    policy: &SagaPolicySpec,
    saga: &SagaProjection,
    manual: Option<VerifiedManualResolutionForPrefix>,
) -> Result<Option<SpecHash>> {
    if saga.run_mode != RunMode::FailedWithoutAcdcClaim {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: format!(
                "failed-without-ACDC terminal requires failed_without_acdc_claim saga mode, found {}",
                saga.run_mode.as_str()
            ),
        });
    }
    if saga.manual_resolution.is_some() {
        let verified = manual.ok_or_else(|| StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "manual failed terminal requires verified manual resolution proof".to_owned(),
        })?;
        require_verified_manual_resolution_matches(
            saga,
            &verified,
            events::ManualResolutionOutcome::FailWithoutAcdcClaim,
        )?;
        return Ok(Some(verified.prefix().spec_hash().clone()));
    }
    if !policy_allows_failed_without_acdc_claim(policy) {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "certified saga policy does not permit failed_without_acdc_claim terminal"
                .to_owned(),
        });
    }
    Ok(None)
}

/// Opaque proof for one terminal saga outcome.
#[derive(Debug, Clone)]
pub struct SagaTerminalProof {
    run_id: RunId,
    prefix_next_seq: StreamSeq,
    saga_policy_digest: ContentDigest,
    manual_spec_hash: Option<SpecHash>,
    kind: SagaTerminalProofKind,
}

#[derive(Debug, Clone)]
enum SagaTerminalProofKind {
    Completed(Box<events::PublicOutputCompletionEvidence>),
    Compensated,
    ManuallyResolved,
    FailedWithoutAcdcClaim,
}

impl SagaTerminalProof {
    /// Mints terminal proof from certified saga policy plus current saga projection.
    pub fn new(
        policy: &SagaPolicySpec,
        saga: &SagaProjection,
        prefix_next_seq: StreamSeq,
        manual: Option<VerifiedManualResolutionForPrefix>,
    ) -> Result<Self> {
        let saga_policy_digest = policy
            .saga_policy_digest()
            .map_err(|error| StoreError::Canonical(error.to_string()))?;
        let (kind, manual_spec_hash) = match saga.run_mode {
            RunMode::Compensated => {
                require_closed_obligations_non_empty(policy, saga)?;
                (SagaTerminalProofKind::Compensated, None)
            }
            RunMode::ManuallyResolved => {
                let verified = manual.ok_or_else(|| StoreError::ProjectionConflict {
                    key: format!("run:{}:saga_terminal", saga.run_id),
                    message: "manual terminal requires verified manual resolution proof".to_owned(),
                })?;
                require_verified_manual_resolution_matches(
                    saga,
                    &verified,
                    events::ManualResolutionOutcome::ConfirmRemediated,
                )?;
                (
                    SagaTerminalProofKind::ManuallyResolved,
                    Some(verified.prefix().spec_hash().clone()),
                )
            }
            RunMode::FailedWithoutAcdcClaim => {
                let manual_spec_hash = require_failed_without_acdc_claim(policy, saga, manual)?;
                (
                    SagaTerminalProofKind::FailedWithoutAcdcClaim,
                    manual_spec_hash,
                )
            }
            RunMode::Completed => {
                let Some(RunCompletionProjection {
                    outcome: events::RunCompletionOutcome::Completed(evidence),
                    ..
                }) = saga.run_completion.as_ref()
                else {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("run:{}:saga_terminal", saga.run_id),
                        message: "completed proof requires completed run projection".to_owned(),
                    });
                };
                (SagaTerminalProofKind::Completed(evidence.clone()), None)
            }
            RunMode::Forward | RunMode::Remediating | RunMode::ManualBlocked => {
                return Err(StoreError::ProjectionConflict {
                    key: format!("run:{}:saga_terminal", saga.run_id),
                    message: format!(
                        "saga terminal proof requires terminal saga mode, found {}",
                        saga.run_mode.as_str()
                    ),
                });
            }
        };
        Ok(Self {
            run_id: saga.run_id.clone(),
            prefix_next_seq,
            saga_policy_digest,
            manual_spec_hash,
            kind,
        })
    }

    /// Returns the run id this proof was minted for.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the stream sequence immediately after the prefix this proof was minted from.
    pub const fn prefix_next_seq(&self) -> StreamSeq {
        self.prefix_next_seq
    }

    /// Returns the saga policy digest this proof was minted under.
    pub fn saga_policy_digest(&self) -> &ContentDigest {
        &self.saga_policy_digest
    }

    /// Returns the run completion outcome proven by this terminal proof.
    pub fn outcome(&self) -> events::RunCompletionOutcome {
        match &self.kind {
            SagaTerminalProofKind::Completed(evidence) => {
                events::RunCompletionOutcome::Completed(evidence.clone())
            }
            SagaTerminalProofKind::Compensated => events::RunCompletionOutcome::Compensated,
            SagaTerminalProofKind::ManuallyResolved => {
                events::RunCompletionOutcome::ManuallyResolved
            }
            SagaTerminalProofKind::FailedWithoutAcdcClaim => {
                events::RunCompletionOutcome::FailedWithoutAcdcClaim
            }
        }
    }

    fn manual_spec_hash(&self) -> Option<&SpecHash> {
        self.manual_spec_hash.as_ref()
    }
}

fn manual_policy_for_block_reason(
    policy: &SagaPolicySpec,
    reason: ManualBlockReason,
) -> Option<&ManualResolutionEvidenceSpec> {
    match (policy, reason) {
        (
            SagaPolicySpec::ManualResolution { manual },
            ManualBlockReason::PolicyManualResolution,
        ) => Some(manual),
        (
            SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved: RemediationUnresolvedSpec::ManualResolution { manual },
            },
            ManualBlockReason::ForwardAmbiguous
            | ManualBlockReason::RemediationFailed
            | ManualBlockReason::RemediationAmbiguous,
        ) => Some(manual.as_ref()),
        _ => None,
    }
}

fn policy_allows_failed_without_acdc_claim(policy: &SagaPolicySpec) -> bool {
    matches!(
        policy,
        SagaPolicySpec::NoSideEffects
            | SagaPolicySpec::FailWithoutAcdcClaim
            | SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved: RemediationUnresolvedSpec::FailWithoutAcdcClaim
            }
    )
}

fn require_verified_manual_resolution_matches(
    saga: &SagaProjection,
    verified: &VerifiedManualResolutionForPrefix,
    expected_outcome: events::ManualResolutionOutcome,
) -> Result<()> {
    if verified.prefix().run_id() != &saga.run_id {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "manual proof run id does not match saga projection".to_owned(),
        });
    }
    if verified.outcome() != expected_outcome {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "manual proof outcome does not match terminal outcome".to_owned(),
        });
    }
    let Some(manual) = saga.manual_resolution.as_ref() else {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "manual terminal requires recorded manual resolution".to_owned(),
        });
    };
    if manual.outcome != expected_outcome {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "recorded manual resolution outcome does not match terminal outcome"
                .to_owned(),
        });
    }
    if manual.evidence_schema_id != verified.evidence().schema_id
        || manual.evidence_hash != verified.evidence().content_hash
        || manual.evidence_artifact_id != verified.evidence().artifact_id
        || manual.authorization_schema_id != verified.authorization().schema_id
        || manual.authorization_hash != verified.authorization().content_hash
        || manual.authorization_artifact_id != verified.authorization().artifact_id
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "recorded manual resolution artifacts do not match verified proof".to_owned(),
        });
    }
    if saga.manual_block_reason.map(manual_block_reason_for_auth)
        != Some(verified.prefix().manual_block_reason())
        && saga.run_mode == RunMode::ManualBlocked
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "manual block reason does not match verified proof".to_owned(),
        });
    }
    Ok(())
}

const fn manual_block_reason_for_auth(reason: ManualBlockReason) -> ManualResolutionBlockReason {
    match reason {
        ManualBlockReason::PolicyManualResolution => {
            ManualResolutionBlockReason::PolicyManualResolution
        }
        ManualBlockReason::ForwardAmbiguous => ManualResolutionBlockReason::ForwardAmbiguous,
        ManualBlockReason::RemediationFailed => ManualResolutionBlockReason::RemediationFailed,
        ManualBlockReason::RemediationAmbiguous => {
            ManualResolutionBlockReason::RemediationAmbiguous
        }
    }
}

const fn manual_block_reason_from_auth(reason: ManualResolutionBlockReason) -> ManualBlockReason {
    match reason {
        ManualResolutionBlockReason::PolicyManualResolution => {
            ManualBlockReason::PolicyManualResolution
        }
        ManualResolutionBlockReason::ForwardAmbiguous => ManualBlockReason::ForwardAmbiguous,
        ManualResolutionBlockReason::RemediationFailed => ManualBlockReason::RemediationFailed,
        ManualResolutionBlockReason::RemediationAmbiguous => {
            ManualBlockReason::RemediationAmbiguous
        }
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

/// Commit preconditions checked atomically with appending the payload batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitPreconditions {
    /// Required run state.
    pub required_run_state: RequiredRunState,
    /// Logical keys that must not exist before the commit.
    pub required_absent_logical_keys: Vec<LogicalEventKey>,
    /// Logical keys that must exist before the commit.
    pub required_present_logical_keys: Vec<LogicalEventKey>,
    /// Required cell states.
    pub required_cell_states: Vec<CellStatePrecondition>,
    /// Required side-effect states.
    pub required_side_effect_states: Vec<SideEffectStatePrecondition>,
    /// Whether no public-output projection may exist.
    pub required_public_output_absent: bool,
    /// Certified run store authority used by policy-bound and side-effect commits.
    pub certified_run_authority: Option<CertifiedRunStoreAuthority>,
}

/// Payload-level typed commit request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    /// Run id to append to.
    run_id: RunId,
    /// Caller's expected next store-owned stream sequence.
    expected_next_seq: StreamSeq,
    /// Commit key for idempotency.
    commit_key: CommitKey,
    /// Ordered typed event payloads.
    payloads: Vec<KernelEventPayload>,
    /// Artifact evidence refs required for the commit.
    required_artifacts: Vec<ArtifactEvidenceRef>,
    /// Atomic commit preconditions.
    preconditions: CommitPreconditions,
}

impl CommitRequest {
    /// Creates a typed commit request after validating the raw payload collection.
    pub fn from_payloads(
        run_id: RunId,
        expected_next_seq: StreamSeq,
        commit_key: CommitKey,
        payloads: Vec<KernelEventPayload>,
        required_artifacts: Vec<ArtifactEvidenceRef>,
        preconditions: CommitPreconditions,
    ) -> Result<Self> {
        if payloads.is_empty() {
            return Err(StoreError::EmptyCommit);
        }
        validate_payload_run_and_spec(&run_id, &payloads)?;
        Ok(Self {
            run_id,
            expected_next_seq,
            commit_key,
            payloads,
            required_artifacts,
            preconditions,
        })
    }

    /// Returns the run id to append to.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the caller's expected next store-owned stream sequence.
    pub fn expected_next_seq(&self) -> StreamSeq {
        self.expected_next_seq
    }

    /// Returns the commit key for idempotency.
    pub fn commit_key(&self) -> &CommitKey {
        &self.commit_key
    }

    /// Returns the ordered typed event payloads.
    pub fn payloads(&self) -> &[KernelEventPayload] {
        &self.payloads
    }

    /// Returns artifact evidence refs required for the commit.
    pub fn required_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.required_artifacts
    }

    /// Returns atomic commit preconditions.
    pub fn preconditions(&self) -> &CommitPreconditions {
        &self.preconditions
    }

    /// Returns this request with a different expected next sequence.
    pub fn with_expected_next_seq(mut self, expected_next_seq: StreamSeq) -> Self {
        self.expected_next_seq = expected_next_seq;
        self
    }

    /// Returns this request with different required artifact evidence refs.
    pub fn with_required_artifacts(mut self, required_artifacts: Vec<ArtifactEvidenceRef>) -> Self {
        self.required_artifacts = required_artifacts;
        self
    }

    /// Returns this request with different atomic preconditions.
    pub fn with_preconditions(mut self, preconditions: CommitPreconditions) -> Self {
        self.preconditions = preconditions;
        self
    }
}

/// Artifact evidence bound to a prepared commit constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitArtifactEvidenceSet {
    required_artifacts: Vec<ArtifactEvidenceRef>,
    admitted_artifacts: Vec<ArtifactEvidenceRef>,
}

impl CommitArtifactEvidenceSet {
    /// Creates the required and admitted artifact evidence set for a commit.
    pub fn new(
        required_artifacts: Vec<ArtifactEvidenceRef>,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> Result<Self> {
        validate_unique_artifact_evidence("required", &required_artifacts)?;
        validate_unique_artifact_evidence("admitted", &admitted_artifacts)?;
        Ok(Self {
            required_artifacts,
            admitted_artifacts,
        })
    }

    /// Creates an empty artifact evidence set.
    pub fn empty() -> Self {
        Self {
            required_artifacts: Vec::new(),
            admitted_artifacts: Vec::new(),
        }
    }

    /// Returns the required artifact evidence refs.
    pub fn required_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.required_artifacts
    }

    /// Returns the artifact evidence refs to admit atomically.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.admitted_artifacts
    }
}

/// Sealed purpose marker for a run-admission commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunAdmission;

/// Sealed purpose marker for a standalone state-attempt-start commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateAttemptStarted;

/// Sealed purpose marker for an attempt-terminal commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptTerminal;

/// Sealed purpose marker for a side-effect terminal commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectTerminal;

/// Sealed purpose marker for a side-effect progress commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectProgress;

/// Sealed purpose marker for a retention projection commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention;

/// Sealed purpose marker for a manual-resolution commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualResolution;

/// Sealed purpose marker for a saga terminal-resolution commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SagaTerminal;

/// Commit purpose implemented only by store-owned marker types.
pub trait CommitPurpose: private::Sealed {
    /// Stable purpose name for diagnostics.
    const NAME: &'static str;
}

macro_rules! impl_commit_purpose {
    ($purpose:ty, $name:literal) => {
        impl private::Sealed for $purpose {}
        impl CommitPurpose for $purpose {
            const NAME: &'static str = $name;
        }
    };
}

impl_commit_purpose!(RunAdmission, "run_admission");
impl_commit_purpose!(StateAttemptStarted, "state_attempt_started");
impl_commit_purpose!(AttemptTerminal, "attempt_terminal");
impl_commit_purpose!(SideEffectTerminal, "side_effect_terminal");
impl_commit_purpose!(SideEffectProgress, "side_effect_progress");
impl_commit_purpose!(Retention, "retention");
impl_commit_purpose!(ManualResolution, "manual_resolution");
impl_commit_purpose!(SagaTerminal, "saga_terminal");

/// Purpose-specific prepared commit authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommit<Purpose: CommitPurpose> {
    inner: PreparedCommitInner,
    _purpose: PhantomData<Purpose>,
}

impl<Purpose: CommitPurpose> PreparedCommit<Purpose> {
    fn prepare_with_authority(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        validate: impl FnOnce(&CommitRequest) -> Result<()>,
        allow_saga_terminal: bool,
        allow_manual_resolution: bool,
    ) -> Result<Self> {
        if artifacts.required_artifacts != request.required_artifacts {
            return Err(invalid_prepared_commit_purpose(
                Purpose::NAME,
                "required artifact evidence set does not match request",
            ));
        }
        validate_required_artifacts_cover_payload_references(Purpose::NAME, &request)?;
        reject_store_materialized_resource_lane_payloads(Purpose::NAME, &request)?;
        validate(&request)?;
        let inner = PreparedCommitInner::new_with_authority(
            request,
            artifacts.admitted_artifacts,
            allow_saga_terminal,
            allow_manual_resolution,
        )?;
        Ok(Self {
            inner,
            _purpose: PhantomData,
        })
    }

    fn prepare_with(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        validate: impl FnOnce(&CommitRequest) -> Result<()>,
    ) -> Result<Self> {
        Self::prepare_with_authority(request, artifacts, validate, false, false)
    }

    /// Returns the typed request sealed into this prepared commit.
    pub fn request(&self) -> &CommitRequest {
        self.inner.request()
    }

    /// Returns artifact evidence to admit atomically with the event batch.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        self.inner.admitted_artifacts()
    }

    fn inner(&self) -> &PreparedCommitInner {
        &self.inner
    }
}

macro_rules! impl_prepared_commit_new {
    ($purpose:ty, $doc:literal, $validator:path) => {
        impl PreparedCommit<$purpose> {
            #[doc = $doc]
            pub fn new(
                request: CommitRequest,
                artifacts: CommitArtifactEvidenceSet,
            ) -> Result<Self> {
                Self::prepare_with(request, artifacts, $validator)
            }
        }
    };
}

impl_prepared_commit_new!(
    RunAdmission,
    "Prepares a run-admission commit.",
    validate_run_start_commit
);
impl_prepared_commit_new!(
    StateAttemptStarted,
    "Prepares a standalone state-attempt-start commit.",
    validate_state_attempt_started_commit
);
impl_prepared_commit_new!(
    AttemptTerminal,
    "Prepares an attempt-terminal commit.",
    validate_attempt_terminal_commit
);
impl_prepared_commit_new!(
    SideEffectTerminal,
    "Prepares a side-effect terminal commit.",
    validate_side_effect_terminal_commit
);
impl_prepared_commit_new!(
    SideEffectProgress,
    "Prepares a side-effect progress commit.",
    validate_side_effect_progress_commit
);
impl_prepared_commit_new!(
    Retention,
    "Prepares a retention projection commit.",
    validate_retention_commit
);
impl PreparedCommit<ManualResolution> {
    /// Prepares a proof-backed manual-resolution commit.
    pub fn new(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        proof: &VerifiedManualResolutionForPrefix,
    ) -> Result<Self> {
        Self::prepare_with_authority(
            request,
            artifacts,
            |request| validate_manual_resolution_commit_with_proof(request, proof),
            false,
            true,
        )
    }
}

impl PreparedCommit<SagaTerminal> {
    /// Prepares a saga terminal-resolution commit.
    pub fn new(
        request: CommitRequest,
        artifacts: CommitArtifactEvidenceSet,
        proof: &SagaTerminalProof,
    ) -> Result<Self> {
        Self::prepare_with_authority(
            request,
            artifacts,
            |request| validate_saga_terminal_commit_with_proof(request, proof),
            true,
            false,
        )
    }
}

/// Production prepared commit plan accepted by store mutation APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedCommitPlan {
    /// Run-admission commit plan.
    RunAdmission(PreparedCommit<RunAdmission>),
    /// State-attempt-start commit plan.
    StateAttemptStarted(PreparedCommit<StateAttemptStarted>),
    /// Attempt-terminal commit plan.
    AttemptTerminal(PreparedCommit<AttemptTerminal>),
    /// Side-effect terminal commit plan.
    SideEffectTerminal(PreparedCommit<SideEffectTerminal>),
    /// Side-effect progress commit plan.
    SideEffectProgress(PreparedCommit<SideEffectProgress>),
    /// Retention projection commit plan.
    Retention(PreparedCommit<Retention>),
    /// Manual-resolution commit plan.
    ManualResolution(PreparedCommit<ManualResolution>),
    /// Saga terminal-resolution commit plan.
    SagaTerminal(PreparedCommit<SagaTerminal>),
}

impl PreparedCommitPlan {
    /// Stable prepared commit purpose name.
    pub fn purpose_name(&self) -> &'static str {
        match self {
            Self::RunAdmission(_) => RunAdmission::NAME,
            Self::StateAttemptStarted(_) => StateAttemptStarted::NAME,
            Self::AttemptTerminal(_) => AttemptTerminal::NAME,
            Self::SideEffectTerminal(_) => SideEffectTerminal::NAME,
            Self::SideEffectProgress(_) => SideEffectProgress::NAME,
            Self::Retention(_) => Retention::NAME,
            Self::ManualResolution(_) => ManualResolution::NAME,
            Self::SagaTerminal(_) => SagaTerminal::NAME,
        }
    }

    /// Returns the sealed request for read-only planning decisions.
    pub fn request(&self) -> &CommitRequest {
        match self {
            Self::RunAdmission(commit) => commit.request(),
            Self::StateAttemptStarted(commit) => commit.request(),
            Self::AttemptTerminal(commit) => commit.request(),
            Self::SideEffectTerminal(commit) => commit.request(),
            Self::SideEffectProgress(commit) => commit.request(),
            Self::Retention(commit) => commit.request(),
            Self::ManualResolution(commit) => commit.request(),
            Self::SagaTerminal(commit) => commit.request(),
        }
    }

    /// Returns artifact evidence to admit atomically with the event batch.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        self.inner().admitted_artifacts()
    }

    fn inner(&self) -> &PreparedCommitInner {
        match self {
            Self::RunAdmission(commit) => commit.inner(),
            Self::StateAttemptStarted(commit) => commit.inner(),
            Self::AttemptTerminal(commit) => commit.inner(),
            Self::SideEffectTerminal(commit) => commit.inner(),
            Self::SideEffectProgress(commit) => commit.inner(),
            Self::Retention(commit) => commit.inner(),
            Self::ManualResolution(commit) => commit.inner(),
            Self::SagaTerminal(commit) => commit.inner(),
        }
    }
}

macro_rules! impl_prepared_commit_plan_from {
    ($purpose:ty, $variant:ident) => {
        impl From<PreparedCommit<$purpose>> for PreparedCommitPlan {
            fn from(commit: PreparedCommit<$purpose>) -> Self {
                Self::$variant(commit)
            }
        }
    };
}

impl_prepared_commit_plan_from!(RunAdmission, RunAdmission);
impl_prepared_commit_plan_from!(StateAttemptStarted, StateAttemptStarted);
impl_prepared_commit_plan_from!(AttemptTerminal, AttemptTerminal);
impl_prepared_commit_plan_from!(SideEffectTerminal, SideEffectTerminal);
impl_prepared_commit_plan_from!(SideEffectProgress, SideEffectProgress);
impl_prepared_commit_plan_from!(Retention, Retention);
impl_prepared_commit_plan_from!(ManualResolution, ManualResolution);
impl_prepared_commit_plan_from!(SagaTerminal, SagaTerminal);

/// Verified artifact bytes carried by a prepared commit bundle.
///
/// This proof object is the only production path for new artifact bytes to accompany run
/// authority. Construction verifies content addressing and typed evidence before storage is
/// called; durable stores must verify the same facts again inside their append transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedArtifactBytes {
    bytes: Vec<u8>,
    evidence: ArtifactEvidenceRef,
    evidence_hash: ContentDigest,
}

impl PreparedArtifactBytes {
    /// Verifies artifact bytes against exact typed evidence and returns a proof object.
    pub fn new(bytes: Vec<u8>, evidence: ArtifactEvidenceRef) -> Result<Self> {
        verify_retained_artifact_bytes(&bytes, &evidence)?;
        let evidence_hash = evidence.evidence_hash()?;
        Ok(Self {
            bytes,
            evidence,
            evidence_hash,
        })
    }

    /// Returns verified artifact bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns exact typed artifact evidence.
    pub fn evidence(&self) -> &ArtifactEvidenceRef {
        &self.evidence
    }

    /// Returns canonical evidence hash for exact-evidence authority.
    pub fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }

    /// Consumes this proof object into its verified parts.
    pub fn into_parts(self) -> (Vec<u8>, ArtifactEvidenceRef, ContentDigest) {
        (self.bytes, self.evidence, self.evidence_hash)
    }
}

/// Exact existing artifact evidence reused by a prepared commit bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingArtifactAdmission {
    artifact_id: ArtifactId,
    evidence_hash: ContentDigest,
}

impl ExistingArtifactAdmission {
    /// Creates an exact-evidence existing artifact admission reference.
    pub fn new(artifact_id: ArtifactId, evidence_hash: ContentDigest) -> Self {
        Self {
            artifact_id,
            evidence_hash,
        }
    }

    /// Artifact id being reused.
    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// Canonical evidence hash required for reuse.
    pub fn evidence_hash(&self) -> &ContentDigest {
        &self.evidence_hash
    }
}

/// Production append authority: a prepared commit plus exact artifact byte/evidence material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommitBundle {
    plan: PreparedCommitPlan,
    artifact_bytes: Vec<PreparedArtifactBytes>,
    existing_artifacts: Vec<ExistingArtifactAdmission>,
    execution_claim: Option<PreparedExecutionClaim>,
}

impl PreparedCommitBundle {
    /// Builds a prepared commit bundle and verifies exact artifact coverage.
    pub fn new(
        plan: PreparedCommitPlan,
        artifact_bytes: Vec<PreparedArtifactBytes>,
        existing_artifacts: Vec<ExistingArtifactAdmission>,
    ) -> Result<Self> {
        validate_prepared_bundle_artifacts(&plan, &artifact_bytes, &existing_artifacts)?;
        Ok(Self {
            plan,
            artifact_bytes,
            existing_artifacts,
            execution_claim: None,
        })
    }

    /// Attaches an execution claim that must be acquired atomically with run admission.
    pub fn with_execution_claim(mut self, claim: PreparedExecutionClaim) -> Result<Self> {
        let PreparedCommitPlan::RunAdmission(commit) = &self.plan else {
            return Err(invalid_prepared_commit_purpose(
                "prepared_commit_bundle",
                "execution claims may only be attached to run admission commits",
            ));
        };
        let run_admitted = commit
            .request()
            .payloads()
            .iter()
            .find_map(|payload| match payload {
                KernelEventPayload::RunAdmitted(payload) => Some(payload),
                _ => None,
            })
            .ok_or_else(|| {
                invalid_prepared_commit_purpose(
                    "prepared_commit_bundle",
                    "execution claim run admission commit lacks RunAdmitted payload",
                )
            })?;
        if claim.holder_run_id != run_admitted.run_id {
            return Err(invalid_prepared_commit_purpose(
                "prepared_commit_bundle",
                "execution claim holder run id does not match RunAdmitted",
            ));
        }
        let expected_scope =
            ExecutionClaimScope::from_run_identity_material(&run_admitted.identity_material);
        if claim.scope != expected_scope {
            return Err(invalid_prepared_commit_purpose(
                "prepared_commit_bundle",
                "execution claim scope does not match RunAdmitted identity material",
            ));
        }
        self.execution_claim = Some(claim);
        Ok(self)
    }

    /// Builds a zero-artifact bundle for commit plans that admit no artifact evidence.
    pub fn without_artifacts(plan: PreparedCommitPlan) -> Result<Self> {
        Self::new(plan, Vec::new(), Vec::new())
    }

    /// Returns the prepared commit plan sealed into the bundle.
    pub fn plan(&self) -> &PreparedCommitPlan {
        &self.plan
    }

    /// Returns the sealed request for read-only planning decisions.
    pub fn request(&self) -> &CommitRequest {
        self.plan.request()
    }

    /// Returns artifact evidence to admit atomically with the event batch.
    pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        self.plan.admitted_artifacts()
    }

    /// Returns verified artifact bytes carried by the bundle.
    pub fn artifact_bytes(&self) -> &[PreparedArtifactBytes] {
        &self.artifact_bytes
    }

    /// Returns exact existing artifact admissions carried by the bundle.
    pub fn existing_artifacts(&self) -> &[ExistingArtifactAdmission] {
        &self.existing_artifacts
    }

    /// Returns the execution claim that must be acquired with this bundle, if any.
    pub fn execution_claim(&self) -> Option<&PreparedExecutionClaim> {
        self.execution_claim.as_ref()
    }
}

/// Execution-claim material attached to one run admission bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExecutionClaim {
    /// Base work scope for the execution lane.
    pub scope: ExecutionClaimScope,
    /// Concrete run admitted as holder.
    pub holder_run_id: RunId,
    /// Holder token.
    pub token: AdmissionToken,
}

impl PreparedExecutionClaim {
    /// Builds execution-claim material for a run admission bundle.
    pub fn new(scope: ExecutionClaimScope, holder_run_id: RunId, token: AdmissionToken) -> Self {
        Self {
            scope,
            holder_run_id,
            token,
        }
    }
}

fn validate_prepared_bundle_artifacts(
    plan: &PreparedCommitPlan,
    artifact_bytes: &[PreparedArtifactBytes],
    existing_artifacts: &[ExistingArtifactAdmission],
) -> Result<()> {
    let mut admitted = BTreeMap::<(ArtifactId, ContentDigest), &ArtifactEvidenceRef>::new();
    for evidence in plan.admitted_artifacts() {
        admitted.insert(
            (evidence.artifact_id.clone(), evidence.evidence_hash()?),
            evidence,
        );
    }

    let mut covered = BTreeSet::<(ArtifactId, ContentDigest)>::new();
    for artifact in artifact_bytes {
        let key = (
            artifact.evidence().artifact_id.clone(),
            artifact.evidence_hash().clone(),
        );
        if !admitted.contains_key(&key) {
            return Err(StoreError::ExtraPreparedArtifactBytes {
                artifact_id: artifact.evidence().artifact_id.clone(),
            });
        }
        if !covered.insert(key) {
            return Err(StoreError::DuplicatePreparedArtifactBytes {
                artifact_id: artifact.evidence().artifact_id.clone(),
            });
        }
    }

    for existing in existing_artifacts {
        let key = (
            existing.artifact_id().clone(),
            existing.evidence_hash().clone(),
        );
        if !admitted.contains_key(&key) {
            return Err(StoreError::ExtraPreparedArtifactBytes {
                artifact_id: existing.artifact_id().clone(),
            });
        }
        if !covered.insert(key) {
            return Err(StoreError::DuplicatePreparedArtifactBytes {
                artifact_id: existing.artifact_id().clone(),
            });
        }
    }

    for (artifact_id, evidence_hash) in admitted.keys() {
        if !covered.contains(&(artifact_id.clone(), evidence_hash.clone())) {
            return Err(StoreError::MissingPreparedArtifactBytes {
                artifact_id: artifact_id.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedCommitInner {
    request: CommitRequest,
    admitted_artifacts: Vec<ArtifactEvidenceRef>,
}

impl PreparedCommitInner {
    fn new_with_authority(
        request: CommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
        allow_saga_terminal: bool,
        allow_manual_resolution: bool,
    ) -> Result<Self> {
        if !allow_saga_terminal && request_contains_saga_terminal_outcome(&request) {
            return Err(invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "saga terminal resolution requires SagaTerminalProof",
            ));
        }
        if !allow_manual_resolution && request_contains_manual_resolution(&request) {
            return Err(invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "manual resolution requires verified manual resolution proof",
            ));
        }
        let referenced_artifacts = referenced_artifact_ids(&request);
        for evidence in &request.required_artifacts {
            if !referenced_artifacts.contains(&evidence.artifact_id) {
                return Err(StoreError::UnreferencedArtifactEvidence {
                    artifact_id: evidence.artifact_id.clone(),
                });
            }
        }
        let mut deduped = BTreeMap::<ArtifactAuthorityKey, ArtifactEvidenceRef>::new();
        for evidence in admitted_artifacts {
            if !referenced_artifacts.contains(&evidence.artifact_id) {
                return Err(StoreError::UnreferencedArtifactEvidence {
                    artifact_id: evidence.artifact_id,
                });
            }
            let key = artifact_authority_key(&evidence)?;
            if let Some(existing) = deduped.get(&key) {
                if existing != &evidence {
                    return Err(StoreError::ArtifactEvidenceMismatch {
                        artifact_id: evidence.artifact_id,
                        field: "artifact",
                    });
                }
                continue;
            }
            deduped.insert(key, evidence);
        }

        Ok(Self {
            request,
            admitted_artifacts: deduped.into_values().collect(),
        })
    }

    fn request(&self) -> &CommitRequest {
        &self.request
    }

    fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
        &self.admitted_artifacts
    }
}

/// Batch of events committed atomically by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedBatch {
    run_id: RunId,
    commit_key: CommitKey,
    fingerprint: CommitFingerprint,
    seq: StreamSeq,
    store_commit_order: StoreCommitOrder,
    events: Vec<KernelEventEnvelope>,
}

impl CommittedBatch {
    /// Run id appended by this batch.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Commit key appended by this batch.
    pub fn commit_key(&self) -> &CommitKey {
        &self.commit_key
    }

    /// Canonical commit fingerprint.
    pub fn fingerprint(&self) -> &CommitFingerprint {
        &self.fingerprint
    }

    /// Store-owned sequence for this atomic commit.
    pub const fn seq(&self) -> StreamSeq {
        self.seq
    }

    /// Store-wide append coordinate for this atomic commit.
    pub const fn store_commit_order(&self) -> StoreCommitOrder {
        self.store_commit_order
    }

    /// Store-owned envelopes created for this batch.
    pub fn events(&self) -> &[KernelEventEnvelope] {
        &self.events
    }

    /// Reconstructs one committed batch from strict-loaded durable event rows.
    pub fn from_persisted_events(
        run_id: RunId,
        commit_key: CommitKey,
        fingerprint: CommitFingerprint,
        seq: StreamSeq,
        store_commit_order: StoreCommitOrder,
        events: Vec<KernelEventEnvelope>,
    ) -> Result<Self> {
        if events.is_empty() {
            return Err(StoreError::PersistedEventMismatch {
                field: "event_count",
                message: "persisted commit has no events".to_owned(),
            });
        }
        for (index, event) in events.iter().enumerate() {
            if event.run_id() != &run_id {
                return Err(StoreError::PersistedEventMismatch {
                    field: "run_id",
                    message: "persisted commit event has a different run id".to_owned(),
                });
            }
            if event.seq() != seq {
                return Err(StoreError::PersistedEventMismatch {
                    field: "seq",
                    message: "persisted commit event has a different sequence".to_owned(),
                });
            }
            if event.store_commit_order() != store_commit_order {
                return Err(StoreError::PersistedEventMismatch {
                    field: "store_commit_order",
                    message: "persisted commit event has a different store append coordinate"
                        .to_owned(),
                });
            }
            if event.commit_key() != &commit_key {
                return Err(StoreError::PersistedEventMismatch {
                    field: "commit_key",
                    message: "persisted commit event has a different commit key".to_owned(),
                });
            }
            if event.ordinal().as_u32() as usize != index {
                return Err(StoreError::PersistedEventMismatch {
                    field: "ordinal",
                    message: "persisted commit event ordinals are not contiguous".to_owned(),
                });
            }
        }
        Ok(Self {
            run_id,
            commit_key,
            fingerprint,
            seq,
            store_commit_order,
            events,
        })
    }
}

/// Result of appending a typed commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    /// A new batch was appended.
    Appended(CommittedBatch),
    /// The commit key had already appended the same canonical batch.
    Idempotent(CommittedBatch),
    /// Run admission was skipped because the execution lane already has an active holder.
    ExecutionClaimBusy(Box<NowaitSkipAdmissionBusy>),
    /// A FIFO admission was blocked before domain authority was persisted.
    AdmissionBlocked(Box<WaitFifoAdmissionBlock>),
}

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

pub use self::artifact_refs::event_artifact_requirements;
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
