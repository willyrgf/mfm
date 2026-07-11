use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
#[cfg(any(test, feature = "test-support"))]
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Cell terminal projection derived from committed run events.
///
/// Both variants already carry many identity/digest fields; boxing a single digest would not
/// meaningfully shrink the type and would complicate exact-evidence authority fields.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellTerminalProjection {
    /// Produced cell projection.
    Produced {
        /// Store-owned event id that produced the terminal projection.
        event_id: EventId,
        /// Producer node id.
        node_id: NodeId,
        /// Attempt id.
        attempt_id: AttemptId,
        /// Schema id.
        schema_id: SchemaId,
        /// Semantic type id.
        semantic_type_id: SemanticTypeId,
        /// Value artifact id.
        artifact_id: ArtifactId,
        /// Canonical content digest.
        content_digest: ContentDigest,
        /// Exact retained-artifact evidence identity.
        evidence_hash: ContentDigest,
    },
    /// Skipped cell projection.
    Skipped {
        /// Store-owned event id that produced the terminal projection.
        event_id: EventId,
        /// Producer node id.
        node_id: NodeId,
        /// Attempt id.
        attempt_id: AttemptId,
        /// Schema id.
        schema_id: SchemaId,
        /// Semantic type id.
        semantic_type_id: SemanticTypeId,
        /// Skip reason.
        skip_reason: events::SkipReason,
    },
}

impl CellTerminalProjection {
    fn attempt_key(&self) -> (&NodeId, &AttemptId) {
        match self {
            Self::Produced {
                node_id,
                attempt_id,
                ..
            }
            | Self::Skipped {
                node_id,
                attempt_id,
                ..
            } => (node_id, attempt_id),
        }
    }
}

/// Side-effect projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectProjection {
    /// Run id that owns this ledger.
    pub run_id: RunId,
    /// Ledger key.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Ledger purpose.
    pub ledger_purpose: events::SideEffectLedgerPurpose,
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Last event id that updated this projection.
    pub event_id: EventId,
    /// Intent evidence that opened this ledger key.
    pub intent: SideEffectIntentProjection,
    /// Prepared invocation artifact evidence, when one has been recorded.
    pub prepared_invocation: Option<SideEffectArtifactProjection>,
    /// Exclusive resource key evidence recorded by `ResourceLaneClaimed`.
    pub resource_key: Option<events::ResourceKeyEvidence>,
    /// Submission artifact evidence, when one has been recorded.
    pub submission: Option<SideEffectArtifactProjection>,
    /// Receipt artifact evidence, when one has been recorded.
    pub receipt: Option<SideEffectArtifactProjection>,
    /// Confirmation artifact evidence, when one has been recorded.
    pub confirmation: Option<SideEffectArtifactProjection>,
    /// Exact touched-set evidence recorded on receipt or confirmation.
    pub resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    /// Active claim, when one exists.
    pub claim: Option<SideEffectClaimProjection>,
    /// Current projected phase.
    pub phase: SideEffectPhase,
}

impl SideEffectProjection {
    /// Returns a read-only semantic view over this projected ledger state.
    pub fn ledger_state(&self) -> Result<SideEffectLedgerState<'_>> {
        SideEffectLedgerState::new(self)
    }
}

/// Validated read-only side-effect ledger state.
///
/// This wraps the persisted projection DTO and exposes only phase-appropriate evidence for the
/// current semantic state.
#[derive(Debug, Clone, Copy)]
pub struct SideEffectLedgerState<'a> {
    projection: &'a SideEffectProjection,
    phase: SideEffectLedgerPhase<'a>,
}

impl<'a> SideEffectLedgerState<'a> {
    fn new(projection: &'a SideEffectProjection) -> Result<Self> {
        let phase = match &projection.phase {
            SideEffectPhase::IntentPersisted { invocation_epoch } => {
                require_intent_epoch(projection, *invocation_epoch)?;
                SideEffectLedgerPhase::IntentPersisted {
                    invocation_epoch: *invocation_epoch,
                }
            }
            SideEffectPhase::Claimed {
                claim_owner,
                invocation_epoch,
                claim_generation,
                claim_fencing_token,
            } => {
                let claim = require_projected_claim(
                    projection,
                    *invocation_epoch,
                    Some(*claim_generation),
                    Some(claim_owner),
                    Some(claim_fencing_token),
                )?;
                SideEffectLedgerPhase::Claimed { claim }
            }
            SideEffectPhase::InvocationPrepared {
                invocation_epoch,
                claim_generation,
                claim_fencing_token,
            } => {
                let claim = require_projected_claim(
                    projection,
                    *invocation_epoch,
                    Some(*claim_generation),
                    None,
                    Some(claim_fencing_token),
                )?;
                SideEffectLedgerPhase::Prepared {
                    claim,
                    prepared_invocation: projection.prepared_invocation.as_ref(),
                    resource_key: projection.resource_key.as_ref(),
                }
            }
            SideEffectPhase::InvocationStarted {
                claim_owner,
                invocation_epoch,
                claim_generation,
                claim_fencing_token,
            } => {
                let claim = require_projected_claim(
                    projection,
                    *invocation_epoch,
                    Some(*claim_generation),
                    Some(claim_owner),
                    Some(claim_fencing_token),
                )?;
                SideEffectLedgerPhase::Started {
                    claim,
                    prepared_invocation: projection.prepared_invocation.as_ref(),
                    resource_key: projection.resource_key.as_ref(),
                }
            }
            SideEffectPhase::SubmissionObserved { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                let submission = require_side_effect_artifact(projection, "submission")?;
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::Observed { submission },
                }
            }
            SideEffectPhase::NotSubmittedProven { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::NotSubmitted,
                }
            }
            SideEffectPhase::SubmissionUnknown { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::Unknown,
                }
            }
            SideEffectPhase::ReceiptObserved { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                let submission = require_side_effect_artifact(projection, "submission")?;
                let receipt = projection.receipt.as_ref().ok_or_else(|| {
                    side_effect_projection_conflict(projection, "receipt evidence is missing")
                })?;
                SideEffectLedgerPhase::ReceiptObserved {
                    claim,
                    submission,
                    receipt,
                    resource_touched_set: projection.resource_touched_set.as_ref(),
                }
            }
            SideEffectPhase::ConfirmationObserved { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                let submission = require_side_effect_artifact(projection, "submission")?;
                let receipt = projection.receipt.as_ref().ok_or_else(|| {
                    side_effect_projection_conflict(projection, "receipt evidence is missing")
                })?;
                let confirmation = projection.confirmation.as_ref().ok_or_else(|| {
                    side_effect_projection_conflict(projection, "confirmation evidence is missing")
                })?;
                SideEffectLedgerPhase::Confirmed {
                    claim,
                    submission,
                    receipt,
                    confirmation,
                    resource_touched_set: projection.resource_touched_set.as_ref(),
                }
            }
            SideEffectPhase::Ambiguous { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                SideEffectLedgerPhase::Ambiguous {
                    claim,
                    invocation_epoch: *invocation_epoch,
                    submission: projection.submission.as_ref(),
                    receipt: projection.receipt.as_ref(),
                }
            }
            SideEffectPhase::Failed {
                invocation_epoch,
                failure_phase,
            } => {
                if matches!(
                    failure_phase,
                    side_effect::FailurePhase::AfterNotSubmittedProven
                ) || projection.claim.is_some()
                {
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                } else {
                    require_intent_epoch(projection, *invocation_epoch)?;
                }
                SideEffectLedgerPhase::Failed {
                    invocation_epoch: *invocation_epoch,
                    failure_phase: *failure_phase,
                    claim: projection.claim.as_ref(),
                }
            }
        };
        Ok(Self { projection, phase })
    }

    /// Returns the backing persisted projection DTO.
    pub fn projection(&self) -> &'a SideEffectProjection {
        self.projection
    }

    /// Returns the validated semantic phase.
    pub fn phase(&self) -> SideEffectLedgerPhase<'a> {
        self.phase
    }

    /// Returns the ledger purpose.
    pub fn ledger_purpose(&self) -> &'a events::SideEffectLedgerPurpose {
        &self.projection.ledger_purpose
    }

    /// Returns true when this is a forward ledger that can still complete after saga engagement.
    pub fn is_forward_completion_candidate(&self) -> bool {
        matches!(
            self.ledger_purpose(),
            events::SideEffectLedgerPurpose::Forward
        ) && matches!(
            self.phase,
            SideEffectLedgerPhase::IntentPersisted { .. }
                | SideEffectLedgerPhase::Claimed { .. }
                | SideEffectLedgerPhase::Prepared { .. }
                | SideEffectLedgerPhase::Started { .. }
                | SideEffectLedgerPhase::SubmissionKnown { .. }
                | SideEffectLedgerPhase::ReceiptObserved { .. }
                | SideEffectLedgerPhase::Confirmed { .. }
        )
    }

    /// Returns true when this phase is ambiguous.
    pub fn is_ambiguous(&self) -> bool {
        matches!(self.phase, SideEffectLedgerPhase::Ambiguous { .. })
    }

    /// Returns true when this phase is failed.
    pub fn is_failed(&self) -> bool {
        matches!(self.phase, SideEffectLedgerPhase::Failed { .. })
    }

    /// Returns true when this phase has observed confirmation.
    pub fn is_confirmed(&self) -> bool {
        matches!(self.phase, SideEffectLedgerPhase::Confirmed { .. })
    }
}

/// Validated side-effect ledger phase.
#[derive(Debug, Clone, Copy)]
pub enum SideEffectLedgerPhase<'a> {
    /// Intent has been persisted and no active claim exists.
    IntentPersisted {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// A claim is active.
    Claimed {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
    },
    /// Invocation preparation was recorded.
    Prepared {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Prepared invocation artifact evidence, when the ledger carries one.
        prepared_invocation: Option<&'a SideEffectArtifactProjection>,
        /// Exclusive resource key evidence, when the ledger carries one.
        resource_key: Option<&'a events::ResourceKeyEvidence>,
    },
    /// Invocation start was recorded.
    Started {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Prepared invocation artifact evidence, when the ledger carries one.
        prepared_invocation: Option<&'a SideEffectArtifactProjection>,
        /// Exclusive resource key evidence, when the ledger carries one.
        resource_key: Option<&'a events::ResourceKeyEvidence>,
    },
    /// Submission result is known for the current epoch.
    SubmissionKnown {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Submission result status.
        status: SideEffectSubmissionState<'a>,
    },
    /// Receipt was observed after a submission artifact.
    ReceiptObserved {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Submission artifact evidence.
        submission: &'a SideEffectArtifactProjection,
        /// Receipt artifact evidence.
        receipt: &'a SideEffectArtifactProjection,
        /// Exact touched-set evidence, when present.
        resource_touched_set: Option<&'a events::ResourceTouchedSetEvidence>,
    },
    /// Confirmation was observed after receipt.
    Confirmed {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Submission artifact evidence.
        submission: &'a SideEffectArtifactProjection,
        /// Receipt artifact evidence.
        receipt: &'a SideEffectArtifactProjection,
        /// Confirmation artifact evidence.
        confirmation: &'a SideEffectArtifactProjection,
        /// Exact touched-set evidence, when present.
        resource_touched_set: Option<&'a events::ResourceTouchedSetEvidence>,
    },
    /// Terminal ambiguity was recorded.
    Ambiguous {
        /// Claim retained from the ambiguity source state.
        claim: &'a SideEffectClaimProjection,
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Submission artifact evidence retained from the source state, when any.
        submission: Option<&'a SideEffectArtifactProjection>,
        /// Receipt artifact evidence retained from the source state, when any.
        receipt: Option<&'a SideEffectArtifactProjection>,
    },
    /// Terminal failure was recorded.
    Failed {
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Failure phase.
        failure_phase: side_effect::FailurePhase,
        /// Claim retained from the source state, when one existed.
        claim: Option<&'a SideEffectClaimProjection>,
    },
}

impl SideEffectLedgerPhase<'_> {
    /// Returns the invocation epoch for this phase.
    pub fn invocation_epoch(&self) -> u32 {
        match self {
            Self::IntentPersisted { invocation_epoch }
            | Self::Failed {
                invocation_epoch, ..
            } => *invocation_epoch,
            Self::Claimed { claim }
            | Self::Prepared { claim, .. }
            | Self::Started { claim, .. }
            | Self::SubmissionKnown { claim, .. }
            | Self::ReceiptObserved { claim, .. }
            | Self::Confirmed { claim, .. }
            | Self::Ambiguous { claim, .. } => claim.invocation_epoch,
        }
    }
}

/// Validated submission-result state for a side-effect ledger epoch.
#[derive(Debug, Clone, Copy)]
pub enum SideEffectSubmissionState<'a> {
    /// Submission artifact was observed.
    Observed {
        /// Submission artifact evidence.
        submission: &'a SideEffectArtifactProjection,
    },
    /// A proof established that submission did not happen.
    NotSubmitted,
    /// Submission outcome remains unknown.
    Unknown,
}

fn require_intent_epoch(projection: &SideEffectProjection, invocation_epoch: u32) -> Result<()> {
    if projection.intent.invocation_epoch == invocation_epoch {
        Ok(())
    } else {
        Err(side_effect_projection_conflict(
            projection,
            "phase invocation epoch does not match intent",
        ))
    }
}

fn require_projected_claim<'a>(
    projection: &'a SideEffectProjection,
    invocation_epoch: u32,
    claim_generation: Option<u32>,
    claim_owner: Option<&events::RunnerInvocationId>,
    claim_fencing_token: Option<&side_effect::ClaimFencingToken>,
) -> Result<&'a SideEffectClaimProjection> {
    let claim = projection.claim.as_ref().ok_or_else(|| {
        side_effect_projection_conflict(projection, "phase requires active claim")
    })?;
    if claim.node_id != projection.intent.node_id
        || claim.attempt_id != projection.intent.attempt_id
        || claim.invocation_epoch != invocation_epoch
        || claim_generation.is_some_and(|expected| claim.claim_generation != expected)
        || claim_owner.is_some_and(|expected| &claim.claim_owner != expected)
        || claim_fencing_token.is_some_and(|expected| &claim.claim_fencing_token != expected)
    {
        return Err(side_effect_projection_conflict(
            projection,
            "active claim does not match projected phase",
        ));
    }
    Ok(claim)
}

fn require_side_effect_artifact<'a>(
    projection: &'a SideEffectProjection,
    label: &'static str,
) -> Result<&'a SideEffectArtifactProjection> {
    match label {
        "submission" => projection.submission.as_ref(),
        _ => None,
    }
    .ok_or_else(|| {
        side_effect_projection_conflict(projection, format!("{label} evidence is missing"))
    })
}

fn side_effect_projection_conflict(
    projection: &SideEffectProjection,
    message: impl Into<String>,
) -> StoreError {
    StoreError::ProjectionConflict {
        key: format!("sidefx_pair:{}", projection.pair_id),
        message: message.into(),
    }
}

#[derive(Debug, Clone)]
struct OwnedSideEffectLedgerState {
    core: SideEffectLedgerCore,
    retained: SideEffectLedgerRetained,
    phase: OwnedSideEffectLedgerPhase,
}

struct SubmissionResultContext<'a> {
    event_id: EventId,
    ledger_key: &'a events::SideEffectLedgerKey,
    ledger_purpose: &'a events::SideEffectLedgerPurpose,
    pair_role: events::SideEffectPairRole,
    node_id: &'a NodeId,
    attempt_id: &'a AttemptId,
    invocation_epoch: u32,
}

#[derive(Debug, Clone)]
struct SideEffectLedgerCore {
    run_id: RunId,
    ledger_key: events::SideEffectLedgerKey,
    ledger_purpose: events::SideEffectLedgerPurpose,
    pair_id: SideEffectPairId,
    event_id: EventId,
    intent: SideEffectIntentProjection,
}

#[derive(Debug, Clone, Default)]
struct SideEffectLedgerRetained {
    prepared_invocation: Option<SideEffectArtifactProjection>,
    resource_key: Option<events::ResourceKeyEvidence>,
    submission: Option<SideEffectArtifactProjection>,
    receipt: Option<SideEffectArtifactProjection>,
    confirmation: Option<SideEffectArtifactProjection>,
    resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
}

#[derive(Debug, Clone)]
enum OwnedSideEffectLedgerPhase {
    IntentPersisted {
        invocation_epoch: u32,
    },
    Claimed {
        claim: SideEffectClaimProjection,
    },
    Prepared {
        claim: SideEffectClaimProjection,
    },
    Started {
        claim: SideEffectClaimProjection,
    },
    SubmissionObserved {
        claim: SideEffectClaimProjection,
    },
    NotSubmitted {
        claim: SideEffectClaimProjection,
    },
    SubmissionUnknown {
        claim: SideEffectClaimProjection,
    },
    ReceiptObserved {
        claim: SideEffectClaimProjection,
    },
    Confirmed {
        claim: SideEffectClaimProjection,
    },
    Ambiguous {
        claim: SideEffectClaimProjection,
        invocation_epoch: u32,
    },
    Failed {
        claim: Option<SideEffectClaimProjection>,
        invocation_epoch: u32,
        failure_phase: side_effect::FailurePhase,
    },
}

impl OwnedSideEffectLedgerState {
    fn intent_persisted(
        run_id: RunId,
        ledger_key: events::SideEffectLedgerKey,
        ledger_purpose: events::SideEffectLedgerPurpose,
        pair_id: SideEffectPairId,
        event_id: EventId,
        intent: SideEffectIntentProjection,
        invocation_epoch: u32,
    ) -> Self {
        Self {
            core: SideEffectLedgerCore {
                run_id,
                ledger_key,
                ledger_purpose,
                pair_id,
                event_id,
                intent,
            },
            retained: SideEffectLedgerRetained::default(),
            phase: OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch },
        }
    }

    fn from_projection(projection: SideEffectProjection) -> Result<Self> {
        projection.ledger_state()?;
        let core = SideEffectLedgerCore {
            run_id: projection.run_id,
            ledger_key: projection.ledger_key,
            ledger_purpose: projection.ledger_purpose,
            pair_id: projection.pair_id,
            event_id: projection.event_id,
            intent: projection.intent,
        };
        let retained = SideEffectLedgerRetained {
            prepared_invocation: projection.prepared_invocation,
            resource_key: projection.resource_key,
            submission: projection.submission,
            receipt: projection.receipt,
            confirmation: projection.confirmation,
            resource_touched_set: projection.resource_touched_set,
        };
        let phase = match projection.phase {
            SideEffectPhase::IntentPersisted { invocation_epoch } => {
                OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch }
            }
            SideEffectPhase::Claimed { .. } => OwnedSideEffectLedgerPhase::Claimed {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::InvocationPrepared { .. } => OwnedSideEffectLedgerPhase::Prepared {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::InvocationStarted { .. } => OwnedSideEffectLedgerPhase::Started {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::SubmissionObserved { .. } => {
                OwnedSideEffectLedgerPhase::SubmissionObserved {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::NotSubmittedProven { .. } => {
                OwnedSideEffectLedgerPhase::NotSubmitted {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::SubmissionUnknown { .. } => {
                OwnedSideEffectLedgerPhase::SubmissionUnknown {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::ReceiptObserved { .. } => {
                OwnedSideEffectLedgerPhase::ReceiptObserved {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::ConfirmationObserved { .. } => OwnedSideEffectLedgerPhase::Confirmed {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::Ambiguous { invocation_epoch } => {
                OwnedSideEffectLedgerPhase::Ambiguous {
                    claim: required_owned_claim(projection.claim)?,
                    invocation_epoch,
                }
            }
            SideEffectPhase::Failed {
                invocation_epoch,
                failure_phase,
            } => OwnedSideEffectLedgerPhase::Failed {
                claim: projection.claim,
                invocation_epoch,
                failure_phase,
            },
        };
        Ok(Self {
            core,
            retained,
            phase,
        })
    }

    fn into_projection(self) -> SideEffectProjection {
        let (claim, phase) = self.phase.into_projection_parts();
        SideEffectProjection {
            run_id: self.core.run_id,
            ledger_key: self.core.ledger_key,
            ledger_purpose: self.core.ledger_purpose,
            pair_id: self.core.pair_id,
            event_id: self.core.event_id,
            intent: self.core.intent,
            prepared_invocation: self.retained.prepared_invocation,
            resource_key: self.retained.resource_key,
            submission: self.retained.submission,
            receipt: self.retained.receipt,
            confirmation: self.retained.confirmation,
            resource_touched_set: self.retained.resource_touched_set,
            claim,
            phase,
        }
    }

    fn claim(mut self, event_id: EventId, payload: &side_effect::Claimed) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let next_claim = match &self.phase {
            OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch } => {
                if payload.invocation_epoch != *invocation_epoch {
                    return Err(self.error("initial claim invocation epoch does not match intent"));
                }
                self.require_intent_context(
                    &payload.node_id,
                    &payload.attempt_id,
                    payload.invocation_epoch,
                )?;
                side_effect_claim_projection(
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.claim_owner,
                    payload.invocation_epoch,
                    payload.claim_generation,
                    &payload.claim_fencing_token,
                )
            }
            OwnedSideEffectLedgerPhase::NotSubmitted { claim } => {
                self.require_intent_attempt_context(&payload.node_id, &payload.attempt_id)?;
                if payload.claim_generation <= claim.claim_generation {
                    return Err(self.error("retry claim generation must increase"));
                }
                if payload.claim_fencing_token == claim.claim_fencing_token {
                    return Err(self.error("retry claim fencing token must change"));
                }
                let next_epoch = claim
                    .invocation_epoch
                    .checked_add(1)
                    .ok_or_else(|| self.error("invocation epoch overflow"))?;
                if payload.invocation_epoch != next_epoch {
                    return Err(self.error("retry claim must advance to the next invocation epoch"));
                }
                side_effect_claim_projection(
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.claim_owner,
                    payload.invocation_epoch,
                    payload.claim_generation,
                    &payload.claim_fencing_token,
                )
            }
            _ => return Err(self.error("claim requires intent or not-submitted phase")),
        };
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Claimed { claim: next_claim };
        Ok(self)
    }

    fn take_over(
        mut self,
        event_id: EventId,
        payload: &side_effect::ClaimTakenOver,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_claimed_or_prepared_claim()?;
        if claim.node_id != payload.node_id
            || claim.attempt_id != payload.attempt_id
            || claim.claim_owner != payload.previous_claim_owner
            || claim.claim_generation != payload.previous_claim_generation
            || claim.invocation_epoch != payload.invocation_epoch
        {
            return Err(self.error("takeover claim context does not match active claim"));
        }
        if payload.claim_generation <= payload.previous_claim_generation {
            return Err(self.error("takeover claim generation must increase"));
        }
        if payload.claim_fencing_token == claim.claim_fencing_token {
            return Err(self.error("takeover claim fencing token must change"));
        }
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Claimed {
            claim: side_effect_claim_projection(
                &payload.node_id,
                &payload.attempt_id,
                &payload.new_claim_owner,
                payload.invocation_epoch,
                payload.claim_generation,
                &payload.claim_fencing_token,
            ),
        };
        Ok(self)
    }

    fn prepare(
        mut self,
        event_id: EventId,
        payload: &side_effect::InvocationPrepared,
        prepared_invocation: Option<SideEffectArtifactProjection>,
        resource_key: Option<events::ResourceKeyEvidence>,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("claim")?.clone();
        require_claim_context_for_payload(
            &self.core.ledger_key,
            &claim,
            &payload.node_id,
            &payload.attempt_id,
            payload.invocation_epoch,
            payload.claim_generation,
            &payload.claim_fencing_token,
            None,
        )?;
        self.core.event_id = event_id;
        self.retained.prepared_invocation =
            prepared_invocation.or(self.retained.prepared_invocation);
        self.retained.resource_key = resource_key;
        self.phase = OwnedSideEffectLedgerPhase::Prepared { claim };
        Ok(self)
    }

    fn start(
        mut self,
        event_id: EventId,
        payload: &side_effect::InvocationStarted,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("prepared")?.clone();
        require_claim_context_for_payload(
            &self.core.ledger_key,
            &claim,
            &payload.node_id,
            &payload.attempt_id,
            payload.invocation_epoch,
            payload.claim_generation,
            &payload.claim_fencing_token,
            Some(&payload.claim_owner),
        )?;
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Started { claim };
        Ok(self)
    }

    fn mark_not_submitted(
        self,
        event_id: EventId,
        payload: &side_effect::NotSubmittedProven,
    ) -> Result<Self> {
        self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::NotSubmitted {
                claim: claim.clone(),
            },
        )
    }

    fn record_submission(
        mut self,
        event_id: EventId,
        payload: &side_effect::SubmissionObserved,
    ) -> Result<Self> {
        let next = self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::SubmissionObserved {
                claim: claim.clone(),
            },
        )?;
        self = next;
        self.retained.submission = Some(SideEffectArtifactProjection {
            artifact_id: payload.submission_artifact_id.clone(),
            content_digest: payload.submission_hash.clone(),
            evidence_hash: payload.submission_artifact_evidence_hash.clone(),
            schema_id: Some(payload.submission_schema_id.clone()),
        });
        Ok(self)
    }

    fn mark_submission_unknown(
        self,
        event_id: EventId,
        payload: &side_effect::SubmissionUnknown,
    ) -> Result<Self> {
        self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::SubmissionUnknown {
                claim: claim.clone(),
            },
        )
    }

    fn record_receipt(
        mut self,
        event_id: EventId,
        payload: &side_effect::ReceiptObserved,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_observed_submission_claim()?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.retained.receipt = Some(SideEffectArtifactProjection {
            artifact_id: payload.receipt_artifact_id.clone(),
            content_digest: payload.receipt_hash.clone(),
            evidence_hash: payload.receipt_artifact_evidence_hash.clone(),
            schema_id: Some(payload.receipt_schema_id.clone()),
        });
        if let Some(touched_set) = payload.resource_touched_set.clone() {
            self.retained.resource_touched_set = Some(touched_set);
        }
        self.phase = OwnedSideEffectLedgerPhase::ReceiptObserved { claim };
        Ok(self)
    }

    fn confirm(
        mut self,
        event_id: EventId,
        payload: &side_effect::ConfirmationObserved,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("receipt")?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.retained.confirmation = Some(SideEffectArtifactProjection {
            artifact_id: payload.confirmation_artifact_id.clone(),
            content_digest: payload.confirmation_hash.clone(),
            evidence_hash: payload.confirmation_artifact_evidence_hash.clone(),
            schema_id: Some(payload.confirmation_schema_id.clone()),
        });
        if let Some(touched_set) = payload.resource_touched_set.clone() {
            self.retained.resource_touched_set = Some(touched_set);
        }
        self.phase = OwnedSideEffectLedgerPhase::Confirmed { claim };
        Ok(self)
    }

    fn mark_ambiguous(
        mut self,
        event_id: EventId,
        payload: &side_effect::Ambiguous,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_ambiguity_source_claim()?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Ambiguous {
            claim,
            invocation_epoch: payload.invocation_epoch,
        };
        Ok(self)
    }

    fn fail(mut self, event_id: EventId, payload: &side_effect::Failed) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = match payload.failure_phase {
            side_effect::FailurePhase::BeforeInvocationStarted => match &self.phase {
                OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch } => {
                    if payload.invocation_epoch != *invocation_epoch {
                        return Err(self.error("failure invocation epoch does not match intent"));
                    }
                    self.require_intent_context(
                        &payload.node_id,
                        &payload.attempt_id,
                        payload.invocation_epoch,
                    )?;
                    None
                }
                OwnedSideEffectLedgerPhase::Claimed { claim }
                | OwnedSideEffectLedgerPhase::Prepared { claim } => {
                    require_claim_or_verify_context_for_observation(
                        &self.core.ledger_key,
                        claim,
                        ObservationClaimContext {
                            ledger_pair_id: &self.core.pair_id,
                            payload_pair_id: &payload.pair_id,
                            payload_pair_role: payload.pair_role,
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                        },
                    )?;
                    Some(claim.clone())
                }
                _ => {
                    return Err(self
                        .error("before-start failure requires intent, claim, or prepared phase"));
                }
            },
            side_effect::FailurePhase::AfterNotSubmittedProven => match &self.phase {
                OwnedSideEffectLedgerPhase::NotSubmitted { claim } => {
                    require_claim_or_verify_context_for_observation(
                        &self.core.ledger_key,
                        claim,
                        ObservationClaimContext {
                            ledger_pair_id: &self.core.pair_id,
                            payload_pair_id: &payload.pair_id,
                            payload_pair_role: payload.pair_role,
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                        },
                    )?;
                    Some(claim.clone())
                }
                _ => {
                    return Err(
                        self.error("after-not-submitted failure requires not-submitted phase")
                    );
                }
            },
        };
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Failed {
            claim,
            invocation_epoch: payload.invocation_epoch,
            failure_phase: payload.failure_phase,
        };
        Ok(self)
    }

    fn submission_result(
        mut self,
        context: SubmissionResultContext<'_>,
        next_phase: impl FnOnce(&SideEffectClaimProjection) -> OwnedSideEffectLedgerPhase,
    ) -> Result<Self> {
        self.require_purpose(context.ledger_key, context.ledger_purpose)?;
        let claim = self
            .require_submission_recovery_claim_for_role(context.pair_role)?
            .clone();
        match context.pair_role {
            events::SideEffectPairRole::Submit => {
                require_claim_identity(
                    &self.core.ledger_key,
                    &claim,
                    context.node_id,
                    context.attempt_id,
                    context.invocation_epoch,
                )?;
            }
            events::SideEffectPairRole::Verify => {
                if claim.invocation_epoch != context.invocation_epoch {
                    return Err(
                        self.error("verify recovery invocation epoch does not match active claim")
                    );
                }
            }
        }
        self.core.event_id = context.event_id;
        self.phase = next_phase(&claim);
        Ok(self)
    }

    fn require_purpose(
        &self,
        ledger_key: &events::SideEffectLedgerKey,
        purpose: &events::SideEffectLedgerPurpose,
    ) -> Result<()> {
        if self.core.ledger_key != *ledger_key {
            return Err(self.error("side-effect ledger key changed"));
        }
        if self.core.ledger_purpose != *purpose {
            return Err(self.error("side-effect ledger purpose changed"));
        }
        Ok(())
    }

    fn require_intent_context(
        &self,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        invocation_epoch: u32,
    ) -> Result<()> {
        self.require_intent_attempt_context(node_id, attempt_id)?;
        if self.core.intent.invocation_epoch != invocation_epoch {
            return Err(self.error("invocation epoch does not match intent projection"));
        }
        Ok(())
    }

    fn require_intent_attempt_context(
        &self,
        node_id: &NodeId,
        attempt_id: &AttemptId,
    ) -> Result<()> {
        if self.core.intent.node_id != *node_id {
            return Err(self.error("node id does not match intent projection"));
        }
        if self.core.intent.attempt_id != *attempt_id {
            return Err(self.error("attempt id does not match intent projection"));
        }
        Ok(())
    }

    fn require_phase_claim(&self, expected: &'static str) -> Result<&SideEffectClaimProjection> {
        match (&self.phase, expected) {
            (OwnedSideEffectLedgerPhase::Claimed { claim }, "claim")
            | (OwnedSideEffectLedgerPhase::Prepared { claim }, "prepared")
            | (OwnedSideEffectLedgerPhase::ReceiptObserved { claim }, "receipt") => Ok(claim),
            _ => Err(self.error(format!(
                "illegal side-effect transition; expected {expected}"
            ))),
        }
    }

    fn require_claimed_or_prepared_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Claimed { claim }
            | OwnedSideEffectLedgerPhase::Prepared { claim } => Ok(claim),
            _ => Err(self.error("takeover requires claim or prepared phase")),
        }
    }

    fn require_submission_recovery_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Started { claim }
            | OwnedSideEffectLedgerPhase::SubmissionUnknown { claim } => Ok(claim),
            _ => Err(self.error("submission recovery requires started or unknown phase")),
        }
    }

    fn require_submission_recovery_claim_for_role(
        &self,
        pair_role: events::SideEffectPairRole,
    ) -> Result<&SideEffectClaimProjection> {
        match pair_role {
            events::SideEffectPairRole::Submit => self.require_submission_recovery_claim(),
            events::SideEffectPairRole::Verify => match &self.phase {
                OwnedSideEffectLedgerPhase::SubmissionUnknown { claim } => Ok(claim),
                _ => Err(self.error("verify submission recovery requires unknown phase")),
            },
        }
    }

    fn require_observed_submission_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::SubmissionObserved { claim } => {
                if self.retained.submission.is_none() {
                    return Err(self.error("submission evidence is missing"));
                }
                Ok(claim)
            }
            _ => Err(self.error("receipt requires submission-observed phase")),
        }
    }

    fn require_ambiguity_source_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Started { claim }
            | OwnedSideEffectLedgerPhase::SubmissionUnknown { claim }
            | OwnedSideEffectLedgerPhase::SubmissionObserved { claim }
            | OwnedSideEffectLedgerPhase::ReceiptObserved { claim } => Ok(claim),
            _ => {
                Err(self.error("ambiguity requires started, unknown, submitted, or receipt phase"))
            }
        }
    }

    fn error(&self, message: impl Into<String>) -> StoreError {
        StoreError::ProjectionConflict {
            key: format!("sidefx_pair:{}", self.core.pair_id),
            message: message.into(),
        }
    }
}

impl OwnedSideEffectLedgerPhase {
    fn into_projection_parts(self) -> (Option<SideEffectClaimProjection>, SideEffectPhase) {
        match self {
            Self::IntentPersisted { invocation_epoch } => {
                (None, SideEffectPhase::IntentPersisted { invocation_epoch })
            }
            Self::Claimed { claim } => (
                Some(claim.clone()),
                SideEffectPhase::Claimed {
                    claim_owner: claim.claim_owner,
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::Prepared { claim } => (
                Some(claim.clone()),
                SideEffectPhase::InvocationPrepared {
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::Started { claim } => (
                Some(claim.clone()),
                SideEffectPhase::InvocationStarted {
                    claim_owner: claim.claim_owner,
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::SubmissionObserved { claim } => (
                Some(claim.clone()),
                SideEffectPhase::SubmissionObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::NotSubmitted { claim } => (
                Some(claim.clone()),
                SideEffectPhase::NotSubmittedProven {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::SubmissionUnknown { claim } => (
                Some(claim.clone()),
                SideEffectPhase::SubmissionUnknown {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::ReceiptObserved { claim } => (
                Some(claim.clone()),
                SideEffectPhase::ReceiptObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::Confirmed { claim } => (
                Some(claim.clone()),
                SideEffectPhase::ConfirmationObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::Ambiguous {
                claim,
                invocation_epoch,
            } => (Some(claim), SideEffectPhase::Ambiguous { invocation_epoch }),
            Self::Failed {
                claim,
                invocation_epoch,
                failure_phase,
            } => (
                claim,
                SideEffectPhase::Failed {
                    invocation_epoch,
                    failure_phase,
                },
            ),
        }
    }
}

fn required_owned_claim(
    claim: Option<SideEffectClaimProjection>,
) -> Result<SideEffectClaimProjection> {
    claim.ok_or_else(|| StoreError::ProjectionConflict {
        key: "sidefx:owned".to_owned(),
        message: "phase requires active claim".to_owned(),
    })
}

fn side_effect_claim_projection(
    node_id: &NodeId,
    attempt_id: &AttemptId,
    claim_owner: &events::RunnerInvocationId,
    invocation_epoch: u32,
    claim_generation: u32,
    claim_fencing_token: &side_effect::ClaimFencingToken,
) -> SideEffectClaimProjection {
    SideEffectClaimProjection {
        node_id: node_id.clone(),
        attempt_id: attempt_id.clone(),
        claim_owner: claim_owner.clone(),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: claim_fencing_token.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn require_claim_context_for_payload(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
    claim_generation: u32,
    claim_fencing_token: &side_effect::ClaimFencingToken,
    claim_owner: Option<&events::RunnerInvocationId>,
) -> Result<()> {
    require_claim_identity(ledger_key, claim, node_id, attempt_id, invocation_epoch)?;
    if claim.claim_generation != claim_generation {
        return Err(side_effect_key_conflict(
            ledger_key,
            "claim generation does not match active claim",
        ));
    }
    if claim.claim_fencing_token != *claim_fencing_token {
        return Err(side_effect_key_conflict(
            ledger_key,
            "claim fencing token does not match active claim",
        ));
    }
    if claim_owner.is_some_and(|owner| &claim.claim_owner != owner) {
        return Err(side_effect_key_conflict(
            ledger_key,
            "claim owner does not match active claim",
        ));
    }
    Ok(())
}

struct ObservationClaimContext<'a> {
    ledger_pair_id: &'a SideEffectPairId,
    payload_pair_id: &'a SideEffectPairId,
    payload_pair_role: events::SideEffectPairRole,
    node_id: &'a NodeId,
    attempt_id: &'a AttemptId,
    invocation_epoch: u32,
}

fn require_claim_or_verify_context_for_observation(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    context: ObservationClaimContext<'_>,
) -> Result<()> {
    if context.payload_pair_role == events::SideEffectPairRole::Verify
        && context.ledger_pair_id == context.payload_pair_id
    {
        if claim.invocation_epoch != context.invocation_epoch {
            return Err(side_effect_key_conflict(
                ledger_key,
                "invocation epoch does not match active claim",
            ));
        }
        return Ok(());
    }
    require_claim_context_for_payload(
        ledger_key,
        claim,
        context.node_id,
        context.attempt_id,
        context.invocation_epoch,
        claim.claim_generation,
        &claim.claim_fencing_token,
        Some(&claim.claim_owner),
    )
}

fn require_claim_identity(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) -> Result<()> {
    if claim.node_id != *node_id {
        return Err(side_effect_key_conflict(
            ledger_key,
            "node id does not match active claim",
        ));
    }
    if claim.attempt_id != *attempt_id {
        return Err(side_effect_key_conflict(
            ledger_key,
            "attempt id does not match active claim",
        ));
    }
    if claim.invocation_epoch != invocation_epoch {
        return Err(side_effect_key_conflict(
            ledger_key,
            "invocation epoch does not match active claim",
        ));
    }
    Ok(())
}

fn side_effect_key_conflict(
    ledger_key: &events::SideEffectLedgerKey,
    message: impl Into<String>,
) -> StoreError {
    StoreError::ProjectionConflict {
        key: format!("sidefx:{ledger_key}"),
        message: message.into(),
    }
}

/// Durable identity for one certified side-effect pair within a run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SideEffectPairLedgerRef {
    /// Run id that owns the pair.
    pub run_id: RunId,
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
}

impl SideEffectPairLedgerRef {
    /// Creates a side-effect pair ledger reference.
    pub fn new(run_id: RunId, pair_id: SideEffectPairId) -> Self {
        Self { run_id, pair_id }
    }
}

/// Cross-run resource lane key derived from resource key evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceLaneKey {
    /// Resource namespace.
    pub namespace: ResourceNamespace,
    /// Schema id for the typed resource-key evidence.
    pub key_schema_id: SchemaId,
    /// Store-comparable resource key.
    pub key: events::ResourceKey,
}

impl ResourceLaneKey {
    /// Creates a lane key from typed resource key evidence.
    pub fn from_evidence(evidence: &events::ResourceKeyEvidence) -> Self {
        Self {
            namespace: evidence.namespace.clone(),
            key_schema_id: evidence.key_schema_id.clone(),
            key: evidence.key.clone(),
        }
    }
}

/// Active holder for an exclusive resource lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLaneProjection {
    /// Store event id that acquired or refreshed the lane.
    pub event_id: EventId,
    /// Run-scoped side-effect pair holding the lane.
    pub holder: SideEffectPairLedgerRef,
    /// Diagnostic side-effect ledger key that claimed the lane.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Ledger purpose.
    pub ledger_purpose: events::SideEffectLedgerPurpose,
    /// Node id that prepared the invocation.
    pub node_id: NodeId,
    /// Attempt id that prepared the invocation.
    pub attempt_id: AttemptId,
    /// Invocation epoch that prepared the invocation.
    pub invocation_epoch: u32,
    /// Store-visible claim id for the active lane claim.
    pub claim_id: events::ResourceLaneClaimId,
    /// Lane-local fencing token for the active claim.
    pub claim_fencing_token: u64,
    /// Lane-local transition sequence that acquired the active claim.
    pub lane_transition_seq: u64,
}

/// Durable lane-local authority folded from committed resource-lane transitions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceLaneAuthority {
    /// Highest committed lane-local transition sequence for this lane.
    pub last_transition_seq: u64,
    /// Highest committed lane-local fencing token assigned to a claim for this lane.
    pub last_claim_fencing_token: u64,
}

/// Resource-lane authority rows keyed by lane identity.
pub type ResourceLaneAuthoritySet = BTreeMap<ResourceLaneKey, ResourceLaneAuthority>;

/// Side-effect artifact evidence retained by the projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectArtifactProjection {
    /// Artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical content digest.
    pub content_digest: ContentDigest,
    /// Exact retained-artifact evidence identity.
    pub evidence_hash: ContentDigest,
    /// Schema id, when the artifact is a typed value.
    pub schema_id: Option<SchemaId>,
}

/// Side-effect intent evidence projected from the authoritative run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectIntentProjection {
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Scope id.
    pub scope_id: ScopeId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Intent schema id.
    pub intent_schema_id: SchemaId,
    /// Intent hash.
    pub intent_hash: ContentDigest,
    /// Intent artifact id.
    pub intent_artifact_id: ArtifactId,
    /// Idempotency input schema id.
    pub idempotency_input_schema_id: SchemaId,
    /// Idempotency input hash.
    pub idempotency_input_hash: ContentDigest,
    /// Idempotency key.
    pub idempotency_key: events::IdempotencyKeyRef,
    /// Capability kind.
    pub capability_kind: CapabilityKind,
    /// Capability version.
    pub capability_version: CapabilityVersion,
    /// Adapter kind.
    pub adapter_kind: AdapterKind,
    /// Adapter version.
    pub adapter_version: AdapterVersion,
}

/// Active side-effect claim evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectClaimProjection {
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Claim owner.
    pub claim_owner: events::RunnerInvocationId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
}

/// Side-effect projected phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideEffectPhase {
    /// Intent persisted.
    IntentPersisted {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Claim acquired.
    Claimed {
        /// Claim owner.
        claim_owner: events::RunnerInvocationId,
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Claim generation.
        claim_generation: u32,
        /// Fencing token.
        claim_fencing_token: side_effect::ClaimFencingToken,
    },
    /// Invocation prepared.
    InvocationPrepared {
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Claim generation.
        claim_generation: u32,
        /// Fencing token.
        claim_fencing_token: side_effect::ClaimFencingToken,
    },
    /// Invocation started.
    InvocationStarted {
        /// Claim owner.
        claim_owner: events::RunnerInvocationId,
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Claim generation.
        claim_generation: u32,
        /// Fencing token.
        claim_fencing_token: side_effect::ClaimFencingToken,
    },
    /// Submission was observed.
    SubmissionObserved {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Not-submitted proof was persisted.
    NotSubmittedProven {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Submission status is unknown.
    SubmissionUnknown {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Receipt was observed.
    ReceiptObserved {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Confirmation was observed.
    ConfirmationObserved {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Side effect is ambiguous.
    Ambiguous {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Side effect failed.
    Failed {
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Failure phase.
        failure_phase: side_effect::FailurePhase,
    },
}

impl SideEffectPhase {
    /// Returns the canonical snake-case tag for this side-effect phase.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::IntentPersisted { .. } => "intent_persisted",
            Self::Claimed { .. } => "claimed",
            Self::InvocationPrepared { .. } => "invocation_prepared",
            Self::InvocationStarted { .. } => "invocation_started",
            Self::SubmissionObserved { .. } => "submission_observed",
            Self::NotSubmittedProven { .. } => "not_submitted_proven",
            Self::SubmissionUnknown { .. } => "submission_unknown",
            Self::ReceiptObserved { .. } => "receipt_observed",
            Self::ConfirmationObserved { .. } => "confirmation_observed",
            Self::Ambiguous { .. } => "ambiguous",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Attempt lifecycle projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptProjection {
    /// Run id.
    pub run_id: RunId,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Last event id that updated the attempt.
    pub event_id: EventId,
    /// Current attempt status.
    pub status: AttemptStatus,
}

/// Attempt lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptStatus {
    /// Attempt has started.
    Started {
        /// Attempt number.
        attempt_no: u32,
        /// State kind.
        state_kind: StateKind,
        /// State version.
        state_version: StateVersion,
    },
    /// Attempt completed.
    Completed {
        /// Output cell id produced by the attempt.
        output_cell_id: CellId,
    },
    /// Attempt failed.
    Failed {
        /// Whether retry is allowed.
        retryable: bool,
        /// Redaction-safe error information.
        error: Box<events::MfmErrorInfo>,
    },
    /// Attempt was interrupted and may be retried.
    Interrupted,
}

/// Descriptor catalog projection derived from certified descriptor artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactDescriptorProjection {
    /// Canonical descriptor content hash.
    pub descriptor_hash: ContentDigest,
    /// Descriptor artifact id.
    pub descriptor_artifact_id: ArtifactId,
    /// Exact descriptor artifact evidence.
    pub descriptor_artifact_evidence: ArtifactEvidenceRef,
    /// Fact kind declared by the descriptor.
    pub fact_kind: mfm_facts::FactKind,
    /// Descriptor schema id.
    pub descriptor_schema_id: SchemaId,
    /// Subject schema id.
    pub subject_schema_id: SchemaId,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Descriptor-derived subject namespace hash.
    pub fact_subject_namespace_hash: ContentDigest,
}

/// Store-owned projection for every recorded fact claim, indexed or private.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRecordProjection {
    /// Store-derived claim id from run-stream coordinates.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Store-owned event id that recorded the fact.
    pub source_event_id: EventId,
    /// Producing run id.
    pub source_run_id: RunId,
    /// Producing stream sequence.
    pub source_seq: u64,
    /// Producing event ordinal.
    pub source_ordinal: u32,
    /// Producing node id.
    pub node_id: NodeId,
    /// Producing attempt id.
    pub attempt_id: AttemptId,
    /// Exact response artifact evidence when this projection was hydrated with artifact authority.
    pub response_artifact_evidence: Option<ArtifactEvidenceRef>,
    /// Normalized claim payload.
    pub claim: mfm_facts::FactClaim,
}

impl FactRecordProjection {
    /// Builds the store-owned fact record projection for a `FactRecorded` event.
    ///
    /// The fact claim id is derived from the event envelope's run-stream coordinates. Attempt
    /// state validation, duplicate response-artifact checks, and indexed visibility checks remain
    /// the caller's responsibility because they depend on the surrounding projection state.
    pub fn from_recorded_event(
        envelope: &KernelEventEnvelope,
        payload: &mfm_events::v1::FactRecorded,
        response_artifact_evidence: Option<ArtifactEvidenceRef>,
    ) -> Result<Self> {
        let fact_claim_id = mfm_facts::derive_fact_claim_id(
            envelope.run_id().clone(),
            envelope.seq().as_u64(),
            envelope.ordinal().as_u32(),
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?;
        Ok(Self {
            fact_claim_id,
            source_event_id: envelope.event_id().clone(),
            source_run_id: envelope.run_id().clone(),
            source_seq: envelope.seq().as_u64(),
            source_ordinal: envelope.ordinal().as_u32(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            response_artifact_evidence,
            claim: payload.claim.clone(),
        })
    }

    /// Returns true when this stream-derived record projection agrees with an indexed row.
    pub fn matches_index_projection(&self, index: &FactIndexProjection) -> bool {
        let record_ref =
            match internal_fact_ref_from_record_projection(self, index.recorded_at.clone()) {
                Ok(Some(fact_ref)) => fact_ref,
                Ok(None) | Err(_) => return false,
            };
        let index_ref = match index.internal_ref() {
            Ok(fact_ref) => fact_ref,
            Err(_) => return false,
        };
        self.fact_claim_id == index.fact_claim_id
            && self.source_run_id == index.source_run_id
            && self.source_seq == index.source_seq
            && self.source_ordinal == index.source_ordinal
            && self.source_event_id == index.source_event_id
            && self.node_id == index.producer_node_id
            && record_ref == index_ref
    }
}

fn internal_fact_ref_from_record_projection(
    record: &FactRecordProjection,
    recorded_at: String,
) -> Result<Option<mfm_facts::InternalFactRef>> {
    mfm_facts::InternalFactRef::from_claim(
        record.fact_claim_id.clone(),
        record.source_event_id.clone(),
        recorded_at,
        record.node_id.clone(),
        &record.claim,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))
}

/// Queryable indexed fact projection for one recorded claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexProjection {
    /// Store-derived claim id from run-stream coordinates.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Producing run id.
    pub source_run_id: RunId,
    /// Producing stream sequence.
    pub source_seq: u64,
    /// Producing event ordinal.
    pub source_ordinal: u32,
    /// Store-owned event id that recorded the fact.
    pub source_event_id: EventId,
    /// Producing node id.
    pub producer_node_id: NodeId,
    /// Commit idempotency key for the append.
    pub commit_id: CommitKey,
    /// Deterministic store commit ordering coordinate.
    pub store_commit_order: u64,
    /// Store-observed record timestamp.
    pub recorded_at: String,
    /// Optional source observation timestamp.
    pub observed_at: Option<String>,
    /// Indexed audience.
    pub audience: mfm_facts::FactAudience,
    /// Indexed visibility scope.
    pub visibility_scope: mfm_facts::FactVisibilityScope,
    /// Fact kind.
    pub fact_kind: mfm_facts::FactKind,
    /// Fact descriptor hash.
    pub fact_descriptor_hash: ContentDigest,
    /// Subject namespace hash.
    pub fact_subject_namespace_hash: ContentDigest,
    /// Descriptor-derived fact key.
    pub fact_key: mfm_facts::FactKey,
    /// Canonical subject material hash.
    pub subject_material_hash: ContentDigest,
    /// Request schema id, when request evidence is present.
    pub request_schema_id: Option<SchemaId>,
    /// Canonical request hash, when request evidence is present.
    pub request_hash: Option<ContentDigest>,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Canonical response hash.
    pub response_hash: ContentDigest,
    /// Response artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical response artifact evidence hash.
    pub artifact_evidence_hash: ContentDigest,
    /// Adapter capability kind.
    pub capability_kind: CapabilityKind,
    /// Adapter capability version.
    pub capability_version: CapabilityVersion,
    /// Adapter kind.
    pub adapter_kind: AdapterKind,
    /// Adapter version.
    pub adapter_version: AdapterVersion,
}

impl FactIndexProjection {
    /// Builds an indexed fact projection from a recorded fact projection.
    ///
    /// Run-private records return `Ok(None)`. Indexed records are validated through the same
    /// internal reference shape used by fact query and replay surfaces.
    pub fn from_record_projection(
        record: &FactRecordProjection,
        commit_id: CommitKey,
        store_commit_order: u64,
        recorded_at: impl Into<String>,
    ) -> Result<Option<Self>> {
        let recorded_at = recorded_at.into();
        let Some(fact_ref) = internal_fact_ref_from_record_projection(record, recorded_at.clone())?
        else {
            return Ok(None);
        };
        let _metadata = mfm_facts::FactExtractionMetadata::new(
            recorded_at.clone(),
            fact_ref.observed_at().map(str::to_owned),
            store_commit_order,
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?;
        let projection = Self::from_internal_ref(&fact_ref, commit_id, store_commit_order)?;
        projection.internal_ref()?;
        if !record.matches_index_projection(&projection) {
            return Err(StoreError::ProjectionConflict {
                key: format!("fact-index:{:?}", projection.fact_claim_id),
                message: "fact index projection does not match recorded fact".to_owned(),
            });
        }
        Ok(Some(projection))
    }

    fn from_internal_ref(
        fact_ref: &mfm_facts::InternalFactRef,
        commit_id: CommitKey,
        store_commit_order: u64,
    ) -> Result<Self> {
        let (audience, visibility_scope) = match fact_ref.visibility() {
            mfm_facts::FactVisibility::Indexed { audience, scope } => (*audience, *scope),
            mfm_facts::FactVisibility::RunPrivate => {
                return Err(StoreError::ProjectionConflict {
                    key: format!("fact-index:{:?}", fact_ref.fact_claim_id()),
                    message: "internal fact ref was not indexed".to_owned(),
                });
            }
        };
        Ok(Self {
            fact_claim_id: fact_ref.fact_claim_id().clone(),
            source_run_id: fact_ref.fact_claim_id().source_run_id().clone(),
            source_seq: fact_ref.fact_claim_id().source_seq(),
            source_ordinal: fact_ref.fact_claim_id().source_ordinal(),
            source_event_id: fact_ref.source_event_id().clone(),
            producer_node_id: fact_ref.producer_node_id().clone(),
            commit_id,
            store_commit_order,
            recorded_at: fact_ref.recorded_at().to_owned(),
            observed_at: fact_ref.observed_at().map(str::to_owned),
            audience,
            visibility_scope,
            fact_kind: fact_ref.fact_kind().clone(),
            fact_descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
            fact_subject_namespace_hash: fact_ref.fact_subject_namespace_hash().clone(),
            fact_key: fact_ref.fact_key().clone(),
            subject_material_hash: fact_ref.subject_material_hash().clone(),
            request_schema_id: fact_ref.request_schema_id().cloned(),
            request_hash: fact_ref.request_hash().cloned(),
            response_schema_id: fact_ref.response_schema_id().clone(),
            response_hash: fact_ref.response_hash().clone(),
            artifact_id: fact_ref.artifact_id().clone(),
            artifact_evidence_hash: fact_ref.artifact_evidence_hash().clone(),
            capability_kind: fact_ref.capability_kind().clone(),
            capability_version: fact_ref.capability_version().clone(),
            adapter_kind: fact_ref.adapter_kind().clone(),
            adapter_version: fact_ref.adapter_version().clone(),
        })
    }

    /// Builds the durable internal fact reference represented by this index row.
    pub fn internal_ref(&self) -> Result<mfm_facts::InternalFactRef> {
        let request = match (&self.request_schema_id, &self.request_hash) {
            (Some(schema_id), Some(hash)) => Some(mfm_facts::FactRequestEvidence::new(
                schema_id.clone(),
                hash.clone(),
            )),
            (None, None) => None,
            _ => {
                return Err(StoreError::Identity(
                    "internal fact index row has partial request evidence".to_owned(),
                ));
            }
        };
        let parts = mfm_facts::InternalFactRefParts {
            fact_claim_id: self.fact_claim_id.clone(),
            source_event_id: self.source_event_id.clone(),
            recorded_at: self.recorded_at.clone(),
            producer_node_id: self.producer_node_id.clone(),
            observed_at: self.observed_at.clone(),
            visibility: mfm_facts::FactVisibility::Indexed {
                audience: self.audience,
                scope: self.visibility_scope,
            },
            fact_kind: self.fact_kind.clone(),
            fact_descriptor_hash: self.fact_descriptor_hash.clone(),
            subject: mfm_facts::FactSubjectRef::new(
                self.fact_subject_namespace_hash.clone(),
                self.fact_key.clone(),
                self.subject_material_hash.clone(),
            ),
            request,
            response: mfm_facts::FactResponseEvidence::new(
                self.response_schema_id.clone(),
                self.response_hash.clone(),
                self.artifact_id.clone(),
                self.artifact_evidence_hash.clone(),
            ),
            producer: mfm_facts::FactProducerProvenance::new(
                self.capability_kind.clone(),
                self.capability_version.clone(),
                self.adapter_kind.clone(),
                self.adapter_version.clone(),
            ),
        };
        mfm_facts::InternalFactRef::new(parts)
            .map_err(|error| StoreError::Identity(error.to_string()))
    }
}

/// Extracted index term projection for one indexed fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexTermProjection {
    /// Store-derived claim id from run-stream coordinates.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Fact descriptor hash.
    pub fact_descriptor_hash: ContentDigest,
    /// Descriptor-owned field id.
    pub field_id: mfm_facts::FactFieldId,
    /// Descriptor field source category.
    pub source: mfm_facts::FactFieldSource,
    /// Descriptor value type.
    pub value_type: mfm_facts::FactFieldValueType,
    /// Extracted canonical scalar.
    pub value: mfm_facts::FactCanonicalScalar,
    /// Optional descriptor unit.
    pub unit: Option<mfm_facts::FactUnit>,
    /// Optional descriptor scale.
    pub scale: Option<mfm_facts::FactScale>,
}

impl FactIndexTermProjection {
    /// Builds an index term projection from descriptor-extracted fact term material.
    pub fn from_extracted_term(
        fact_claim_id: &mfm_facts::FactClaimId,
        fact_descriptor_hash: &ContentDigest,
        term: &mfm_facts::FactIndexTerm,
    ) -> Self {
        Self {
            fact_claim_id: fact_claim_id.clone(),
            fact_descriptor_hash: fact_descriptor_hash.clone(),
            field_id: term.field_id().clone(),
            source: term.source(),
            value_type: term.value_type(),
            value: term.value().clone(),
            unit: term.unit().cloned(),
            scale: term.scale(),
        }
    }
}

/// Public-output projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicOutputProjection {
    /// Public output rendered successfully.
    Produced {
        /// Producing event id.
        event_id: EventId,
        /// Rendered output digest.
        rendered_digest: ContentDigest,
        /// Optional rendered artifact id.
        rendered_artifact_id: Option<ArtifactId>,
    },
    /// Public output render failed.
    RenderFailed {
        /// Failure event id.
        event_id: EventId,
        /// Redaction-safe error information.
        error: Box<events::MfmErrorInfo>,
    },
}

/// Retention projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionProjection {
    /// Retained artifact refs by exact `(artifact_id, evidence_hash)` authority.
    pub refs: BTreeMap<ArtifactAuthorityKey, events::RetentionRef>,
    /// Projected retention manifests by manifest sequence.
    pub manifests: BTreeMap<u64, RetentionManifestProjection>,
    /// Last retention manifest projected for this run.
    pub manifest: Option<RetentionManifestProjection>,
}

/// Retention manifest projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionManifestProjection {
    /// Manifest sequence.
    pub manifest_seq: u64,
    /// Manifest digest.
    pub manifest_digest: ContentDigest,
    /// Previous manifest digest, when any.
    pub previous_manifest_digest: Option<ContentDigest>,
    /// Manifest artifact id.
    pub manifest_artifact_id: ArtifactId,
}

/// Store-owned projection snapshot derived from authoritative run streams.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionSnapshot {
    run_states: BTreeMap<RunId, RunState>,
    run_spec_hashes: BTreeMap<RunId, SpecHash>,
    saga_policy_digests: BTreeMap<RunId, ContentDigest>,
    run_completions: BTreeMap<RunId, RunCompletionProjection>,
    saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
    manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
    attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
    cells: BTreeMap<(RunId, CellId), CellTerminalProjection>,
    fact_descriptors: BTreeMap<ContentDigest, FactDescriptorProjection>,
    fact_records: BTreeMap<mfm_facts::FactClaimId, FactRecordProjection>,
    fact_index_entries: BTreeMap<mfm_facts::FactClaimId, FactIndexProjection>,
    fact_term_entries:
        BTreeMap<(mfm_facts::FactClaimId, mfm_facts::FactFieldId), FactIndexTermProjection>,
    side_effects: BTreeMap<SideEffectPairLedgerRef, SideEffectProjection>,
    resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
    public_outputs: BTreeMap<(RunId, SchemaId), PublicOutputProjection>,
    retentions: BTreeMap<RunId, RetentionProjection>,
}

/// Storage-owned projection maps used to construct a [`ProjectionSnapshot`].
///
/// Hydrating callers fill the projection families they own and default the rest, replacing the
/// previous telescoping `from_parts*` constructors.
#[derive(Debug, Default)]
pub struct ProjectionSnapshotParts {
    /// Run lifecycle states.
    pub run_states: BTreeMap<RunId, RunState>,
    /// Certified spec hash recorded at run start.
    pub run_spec_hashes: BTreeMap<RunId, SpecHash>,
    /// Saga policy digest recorded at run start.
    pub saga_policy_digests: BTreeMap<RunId, ContentDigest>,
    /// Terminal run completion projections.
    pub run_completions: BTreeMap<RunId, RunCompletionProjection>,
    /// First saga engagement per run.
    pub saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
    /// Manual resolution evidence per run.
    pub manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
    /// Attempt projections.
    pub attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
    /// Terminal cell projections.
    pub cells: BTreeMap<(RunId, CellId), CellTerminalProjection>,
    /// Descriptor catalog projections.
    pub fact_descriptors: BTreeMap<ContentDigest, FactDescriptorProjection>,
    /// Recorded fact projections.
    pub fact_records: BTreeMap<mfm_facts::FactClaimId, FactRecordProjection>,
    /// Indexed fact projections.
    pub fact_index_entries: BTreeMap<mfm_facts::FactClaimId, FactIndexProjection>,
    /// Extracted fact term projections.
    pub fact_term_entries:
        BTreeMap<(mfm_facts::FactClaimId, mfm_facts::FactFieldId), FactIndexTermProjection>,
    /// Side-effect pair projections.
    pub side_effects: BTreeMap<SideEffectPairLedgerRef, SideEffectProjection>,
    /// Cross-run resource lane projections.
    pub resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
    /// Public output projections.
    pub public_outputs: BTreeMap<(RunId, SchemaId), PublicOutputProjection>,
    /// Retention projections.
    pub retentions: BTreeMap<RunId, RetentionProjection>,
}

impl ProjectionSnapshotParts {
    /// Clones every projection family from an existing snapshot.
    pub fn from_snapshot(snapshot: &ProjectionSnapshot) -> Self {
        Self {
            run_states: snapshot.run_states.clone(),
            run_spec_hashes: snapshot.run_spec_hashes.clone(),
            saga_policy_digests: snapshot.saga_policy_digests.clone(),
            run_completions: snapshot.run_completions.clone(),
            saga_engagements: snapshot.saga_engagements.clone(),
            manual_resolutions: snapshot.manual_resolutions.clone(),
            attempts: snapshot.attempts.clone(),
            cells: snapshot.cells.clone(),
            fact_descriptors: snapshot.fact_descriptors.clone(),
            fact_records: snapshot.fact_records.clone(),
            fact_index_entries: snapshot.fact_index_entries.clone(),
            fact_term_entries: snapshot.fact_term_entries.clone(),
            side_effects: snapshot.side_effects.clone(),
            resource_lanes: snapshot.resource_lanes.clone(),
            public_outputs: snapshot.public_outputs.clone(),
            retentions: snapshot.retentions.clone(),
        }
    }

    fn replace_fact_authority_from(&mut self, authority: &ProjectionSnapshot) {
        self.fact_descriptors = authority.fact_descriptors.clone();
        self.fact_records = authority.fact_records.clone();
        self.fact_index_entries = authority.fact_index_entries.clone();
        self.fact_term_entries = authority.fact_term_entries.clone();
    }

    fn replace_resource_lanes_from(&mut self, authority: &ProjectionSnapshot) {
        self.resource_lanes = authority.resource_lanes.clone();
    }
}

impl ProjectionSnapshot {
    /// Returns a snapshot with store-owned fact authority and resource lanes copied together.
    pub fn with_store_authority_from(&self, authority: &ProjectionSnapshot) -> Result<Self> {
        let mut parts = ProjectionSnapshotParts::from_snapshot(self);
        parts.replace_fact_authority_from(authority);
        parts.replace_resource_lanes_from(authority);
        Self::from_parts(parts)
    }

    /// Creates a projection snapshot from storage-owned projection maps.
    ///
    /// Callers populate only the projection families they hydrate and leave the rest empty via
    /// [`ProjectionSnapshotParts`]'s [`Default`].
    pub fn from_parts(parts: ProjectionSnapshotParts) -> Result<Self> {
        let ProjectionSnapshotParts {
            run_states,
            run_spec_hashes,
            saga_policy_digests,
            run_completions,
            saga_engagements,
            manual_resolutions,
            attempts,
            cells,
            fact_descriptors,
            fact_records,
            fact_index_entries,
            fact_term_entries,
            side_effects,
            resource_lanes,
            public_outputs,
            retentions,
        } = parts;
        for ((node_id, attempt_id), projection) in &attempts {
            if node_id != &projection.node_id || attempt_id != &projection.attempt_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("attempt:{}:{}", projection.node_id, projection.attempt_id),
                    message: "attempt projection key does not match projection identity".to_owned(),
                });
            }
        }
        for ((run_id, cell_id), projection) in &cells {
            let (projection_node_id, projection_attempt_id) = projection.attempt_key();
            let Some(attempt) =
                attempts.get(&(projection_node_id.clone(), projection_attempt_id.clone()))
            else {
                return Err(StoreError::ProjectionConflict {
                    key: format!("cell:{run_id}:{cell_id}:terminal"),
                    message: "cell projection references missing attempt projection".to_owned(),
                });
            };
            if &attempt.run_id != run_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("cell:{run_id}:{cell_id}:terminal"),
                    message: "cell projection key does not match attempt run id".to_owned(),
                });
            }
        }
        for (descriptor_hash, projection) in &fact_descriptors {
            if descriptor_hash != &projection.descriptor_hash {
                return Err(StoreError::ProjectionConflict {
                    key: format!("fact_descriptor:{}", projection.descriptor_hash),
                    message: "fact descriptor projection key does not match descriptor hash"
                        .to_owned(),
                });
            }
        }
        for (claim_id, projection) in &fact_records {
            if claim_id != &projection.fact_claim_id {
                return Err(StoreError::ProjectionConflict {
                    key: fact_claim_projection_key("fact_record", claim_id),
                    message: "fact record projection key does not match claim id".to_owned(),
                });
            }
        }
        for (claim_id, projection) in &fact_index_entries {
            if claim_id != &projection.fact_claim_id {
                return Err(StoreError::ProjectionConflict {
                    key: fact_claim_projection_key("fact_index", claim_id),
                    message: "fact index projection key does not match claim id".to_owned(),
                });
            }
        }
        for ((claim_id, field_id), projection) in &fact_term_entries {
            if claim_id != &projection.fact_claim_id || field_id != &projection.field_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "{}:{}",
                        fact_claim_projection_key("fact_term", claim_id),
                        field_id
                    ),
                    message: "fact term projection key does not match projection identity"
                        .to_owned(),
                });
            }
        }
        for (ledger_ref, projection) in &side_effects {
            if ledger_ref.run_id != projection.run_id || ledger_ref.pair_id != projection.pair_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("sidefx_pair:{}", ledger_ref.pair_id),
                    message: "side-effect projection key does not match pair reference".to_owned(),
                });
            }
            projection.ledger_state()?;
        }
        Ok(Self {
            run_states,
            run_spec_hashes,
            saga_policy_digests,
            run_completions,
            saga_engagements,
            manual_resolutions,
            attempts,
            cells,
            fact_descriptors,
            fact_records,
            fact_index_entries,
            fact_term_entries,
            side_effects,
            resource_lanes,
            public_outputs,
            retentions,
        })
    }

    /// Validates that a loaded run stream is ordered and contiguous.
    pub fn validate_run_stream(events: &[KernelEventEnvelope]) -> Result<()> {
        validate_run_stream_order(events)?;
        validate_supported_stream_model(events)
    }

    /// Rebuilds projections from store-owned event envelopes.
    pub fn rebuild_from_run_stream(events: &[KernelEventEnvelope]) -> Result<Self> {
        Self::validate_run_stream(events)?;
        for event in events {
            match event.payload() {
                KernelEventPayload::RunAdmitted(payload)
                    if !payload.fact_descriptor_artifacts.is_empty() =>
                {
                    return Err(StoreError::ProjectionConflict {
                        key: "fact_descriptor:artifact_bytes".to_owned(),
                        message:
                            "fact descriptor projection requires retained descriptor artifact bytes"
                                .to_owned(),
                    });
                }
                KernelEventPayload::FactRecorded(_) => {
                    return Err(StoreError::ProjectionConflict {
                        key: "fact:artifact_bytes".to_owned(),
                        message: "fact projection requires retained artifact bytes".to_owned(),
                    });
                }
                _ => {}
            }
        }
        let mut snapshot = Self::default();
        let artifact_bytes = ArtifactByteAuthorityMap::new();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection(&mut snapshot, &event, &artifact_bytes)?;
            }
        }
        Ok(snapshot)
    }

    /// Rebuilds projections from a run stream using exact retained artifact bytes.
    ///
    /// Durable stores use this for validation and physical projection rebuilds when fact descriptor
    /// and response artifacts are already loaded from authoritative storage. The supplied artifact
    /// byte authority must be keyed by exact `(artifact_id, evidence_hash)`.
    pub fn rebuild_from_run_stream_with_artifact_bytes(
        events: &[KernelEventEnvelope],
        artifact_bytes: &ArtifactByteAuthorityMap,
    ) -> Result<Self> {
        Self::validate_run_stream(events)?;
        let mut snapshot = Self::default();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection(&mut snapshot, &event, artifact_bytes)?;
            }
        }
        Ok(snapshot)
    }

    /// Rebuilds non-fact projections plus fact record identities for stores with physical fact indexes.
    ///
    /// This helper is for stores that maintain descriptor, index, and term projections in separate
    /// validated tables. It does not rebuild queryable fact indexes from the stream, and it is not a
    /// replay validation substitute for retained descriptor and response artifact authority.
    pub fn rebuild_for_external_fact_indexes(events: &[KernelEventEnvelope]) -> Result<Self> {
        Self::validate_run_stream(events)?;
        let mut snapshot = Self::default();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection_for_external_fact_indexes(&mut snapshot, &event)?;
            }
        }
        Ok(snapshot)
    }

    /// Returns the run state for a run id.
    pub fn run_state(&self, run_id: &RunId) -> RunState {
        self.run_states
            .get(run_id)
            .copied()
            .unwrap_or(RunState::Absent)
    }

    /// Returns the certified spec hash recorded at run start.
    pub fn run_spec_hash(&self, run_id: &RunId) -> Option<&SpecHash> {
        self.run_spec_hashes.get(run_id)
    }

    /// Returns the saga policy digest recorded at run start.
    pub fn saga_policy_digest(&self, run_id: &RunId) -> Option<&ContentDigest> {
        self.saga_policy_digests.get(run_id)
    }

    /// Returns the run completion projection for a run id.
    pub fn run_completion(&self, run_id: &RunId) -> Option<&RunCompletionProjection> {
        self.run_completions.get(run_id)
    }

    /// Returns the first saga engagement projection for a run id.
    pub fn saga_engagement(&self, run_id: &RunId) -> Option<&SagaEngagementProjection> {
        self.saga_engagements.get(run_id)
    }

    /// Returns the manual resolution projection for a run id.
    pub fn manual_resolution(&self, run_id: &RunId) -> Option<&ManualResolutionProjection> {
        self.manual_resolutions.get(run_id)
    }

    /// Returns a cell terminal projection.
    pub fn cell_terminal(&self, cell_id: &CellId) -> Option<&CellTerminalProjection> {
        self.cells
            .iter()
            .find_map(|((_run_id, key_cell_id), projection)| {
                (key_cell_id == cell_id).then_some(projection)
            })
    }

    /// Returns a cell terminal projection for a specific run.
    pub fn cell_terminal_for_run(
        &self,
        run_id: &RunId,
        cell_id: &CellId,
    ) -> Option<&CellTerminalProjection> {
        self.cells.get(&(run_id.clone(), cell_id.clone()))
    }

    /// Returns a descriptor catalog projection.
    pub fn fact_descriptor(
        &self,
        descriptor_hash: &ContentDigest,
    ) -> Option<&FactDescriptorProjection> {
        self.fact_descriptors.get(descriptor_hash)
    }

    /// Returns a recorded fact projection.
    pub fn fact_record(&self, claim_id: &mfm_facts::FactClaimId) -> Option<&FactRecordProjection> {
        self.fact_records.get(claim_id)
    }

    /// Returns an indexed fact projection.
    pub fn fact_index_entry(
        &self,
        claim_id: &mfm_facts::FactClaimId,
    ) -> Option<&FactIndexProjection> {
        self.fact_index_entries.get(claim_id)
    }

    /// Returns an extracted fact term for a claim and field id.
    pub fn fact_term(
        &self,
        claim_id: &mfm_facts::FactClaimId,
        field_id: &mfm_facts::FactFieldId,
    ) -> Option<&FactIndexTermProjection> {
        self.fact_term_entries
            .get(&(claim_id.clone(), field_id.clone()))
    }

    /// Iterates extracted fact terms for one claim id.
    pub fn fact_terms_for_claim<'a>(
        &'a self,
        claim_id: &'a mfm_facts::FactClaimId,
    ) -> impl Iterator<Item = &'a FactIndexTermProjection> + 'a {
        self.fact_term_entries
            .iter()
            .filter(move |((term_claim_id, _field_id), _term)| term_claim_id == claim_id)
            .map(|(_key, term)| term)
    }

    /// Returns an attempt lifecycle projection.
    pub fn attempt(&self, node_id: &NodeId, attempt_id: &AttemptId) -> Option<&AttemptProjection> {
        self.attempts.get(&(node_id.clone(), attempt_id.clone()))
    }

    /// Returns the first open semantic attempt projected for a run.
    pub fn open_attempt_for_run(&self, run_id: &RunId) -> Option<&AttemptProjection> {
        self.attempts.values().find(|attempt| {
            &attempt.run_id == run_id && matches!(attempt.status, AttemptStatus::Started { .. })
        })
    }

    /// Requires that a run prefix has no open semantic attempt.
    pub fn require_no_open_semantic_attempts_for_run(&self, run_id: &RunId) -> Result<()> {
        if let Some(attempt) = self.open_attempt_for_run(run_id) {
            return Err(StoreError::ProjectionConflict {
                key: format!("run:{run_id}:attempts"),
                message: format!(
                    "manual resolution requires no open semantic attempts; attempt {}:{} is still started",
                    attempt.node_id, attempt.attempt_id
                ),
            });
        }
        Ok(())
    }

    /// Returns a side-effect projection for a run-scoped certified pair id.
    pub fn side_effect_for_pair(
        &self,
        run_id: &RunId,
        pair_id: &SideEffectPairId,
    ) -> Option<&SideEffectProjection> {
        self.side_effects.get(&SideEffectPairLedgerRef::new(
            run_id.clone(),
            pair_id.clone(),
        ))
    }

    /// Returns a validated side-effect ledger state for a run-scoped certified pair id.
    pub fn side_effect_state_for_pair(
        &self,
        run_id: &RunId,
        pair_id: &SideEffectPairId,
    ) -> Result<Option<SideEffectLedgerState<'_>>> {
        self.side_effect_for_pair(run_id, pair_id)
            .map(SideEffectProjection::ledger_state)
            .transpose()
    }

    /// Returns an active resource lane holder.
    pub fn resource_lane(&self, key: &ResourceLaneKey) -> Option<&ResourceLaneProjection> {
        self.resource_lanes.get(key)
    }

    /// Returns a public-output projection.
    pub fn public_output(
        &self,
        run_id: &RunId,
        schema_id: &SchemaId,
    ) -> Option<&PublicOutputProjection> {
        self.public_outputs
            .get(&(run_id.clone(), schema_id.clone()))
    }

    /// Returns a retention projection.
    pub fn retention(&self, run_id: &RunId) -> Option<&RetentionProjection> {
        self.retentions.get(run_id)
    }

    /// Returns whether terminal public-output authority is projected.
    pub fn has_public_output(&self, run_id: &RunId) -> bool {
        self.public_outputs
            .iter()
            .any(|((projection_run_id, _), projection)| {
                projection_run_id == run_id
                    && matches!(projection, PublicOutputProjection::Produced { .. })
            })
    }

    /// Returns whether all past-boundary forward ledgers for the current projection are quiescent.
    pub fn forward_ledgers_quiescent(
        &self,
        run_id: &RunId,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<bool> {
        forward_ledgers_quiescent(self, run_id, terminal_policies)
    }

    /// Derives saga status from certified saga policy plus the current stream projection.
    pub fn derive_saga_projection(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<SagaProjection> {
        derive_saga_projection(self, run_id, policy, terminal_policies)
    }

    /// Requires that the current prefix derives a manual-blocked saga mode.
    pub fn require_manual_resolution_admissible(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<()> {
        self.require_no_open_semantic_attempts_for_run(run_id)?;
        let saga = self.derive_saga_projection(run_id, policy, terminal_policies)?;
        if saga.run_mode == RunMode::ManualBlocked {
            Ok(())
        } else {
            Err(StoreError::ProjectionConflict {
                key: format!("run:{run_id}:manual_resolution"),
                message: format!(
                    "manual resolution requires prefix-derived manual_blocked saga mode, found {}",
                    saga.run_mode.as_str()
                ),
            })
        }
    }

    /// Returns the saga terminal outcome supported by the current prefix.
    pub fn saga_terminal_completion_outcome(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<events::RunCompletionOutcome> {
        let saga = self.derive_saga_projection(run_id, policy, terminal_policies)?;
        saga.run_mode
            .saga_terminal_outcome()
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("run:{run_id}:saga_terminal"),
                message: format!(
                    "saga terminal resolution requires terminal saga mode, found {}",
                    saga.run_mode.as_str()
                ),
            })
    }

    /// Iterates projected run states.
    pub fn run_states(&self) -> impl Iterator<Item = (&RunId, &RunState)> {
        self.run_states.iter()
    }

    /// Iterates run-start certified spec hashes.
    pub fn run_spec_hashes(&self) -> impl Iterator<Item = (&RunId, &SpecHash)> {
        self.run_spec_hashes.iter()
    }

    /// Iterates run-start saga policy digests.
    pub fn saga_policy_digests(&self) -> impl Iterator<Item = (&RunId, &ContentDigest)> {
        self.saga_policy_digests.iter()
    }

    /// Iterates run completion projections.
    pub fn run_completions(&self) -> impl Iterator<Item = (&RunId, &RunCompletionProjection)> {
        self.run_completions.iter()
    }

    /// Iterates saga engagement projections.
    pub fn saga_engagements(&self) -> impl Iterator<Item = (&RunId, &SagaEngagementProjection)> {
        self.saga_engagements.iter()
    }

    /// Iterates manual resolution projections.
    pub fn manual_resolutions(
        &self,
    ) -> impl Iterator<Item = (&RunId, &ManualResolutionProjection)> {
        self.manual_resolutions.iter()
    }

    /// Iterates attempt lifecycle projections.
    pub fn attempts(&self) -> impl Iterator<Item = (&(NodeId, AttemptId), &AttemptProjection)> {
        self.attempts.iter()
    }

    /// Iterates cell terminal projections.
    pub fn cells(&self) -> impl Iterator<Item = (&RunId, &CellId, &CellTerminalProjection)> {
        self.cells
            .iter()
            .map(|((run_id, cell_id), projection)| (run_id, cell_id, projection))
    }

    /// Iterates descriptor catalog projections.
    pub fn fact_descriptors(
        &self,
    ) -> impl Iterator<Item = (&ContentDigest, &FactDescriptorProjection)> {
        self.fact_descriptors.iter()
    }

    /// Iterates recorded fact projections.
    pub fn fact_records(
        &self,
    ) -> impl Iterator<Item = (&mfm_facts::FactClaimId, &FactRecordProjection)> {
        self.fact_records.iter()
    }

    /// Iterates indexed fact projections.
    pub fn fact_index_entries(
        &self,
    ) -> impl Iterator<Item = (&mfm_facts::FactClaimId, &FactIndexProjection)> {
        self.fact_index_entries.iter()
    }

    /// Iterates extracted fact term projections.
    pub fn fact_term_entries(
        &self,
    ) -> impl Iterator<
        Item = (
            &(mfm_facts::FactClaimId, mfm_facts::FactFieldId),
            &FactIndexTermProjection,
        ),
    > {
        self.fact_term_entries.iter()
    }

    /// Iterates side-effect projections.
    pub fn side_effects(
        &self,
    ) -> impl Iterator<Item = (&SideEffectPairLedgerRef, &SideEffectProjection)> {
        self.side_effects.iter()
    }

    /// Iterates active resource lane projections.
    pub fn resource_lanes(
        &self,
    ) -> impl Iterator<Item = (&ResourceLaneKey, &ResourceLaneProjection)> {
        self.resource_lanes.iter()
    }

    /// Iterates public-output projections.
    pub fn public_outputs(
        &self,
    ) -> impl Iterator<Item = (&RunId, &SchemaId, &PublicOutputProjection)> {
        self.public_outputs
            .iter()
            .map(|((run_id, schema_id), projection)| (run_id, schema_id, projection))
    }

    /// Iterates retention projections.
    pub fn retentions(&self) -> impl Iterator<Item = (&RunId, &RetentionProjection)> {
        self.retentions.iter()
    }
}

fn fact_claim_projection_key(prefix: &str, claim_id: &mfm_facts::FactClaimId) -> String {
    format!(
        "{}:{}:{}:{}",
        prefix,
        claim_id.source_run_id(),
        claim_id.source_seq(),
        claim_id.source_ordinal()
    )
}

/// One atomically committed run-stream batch reconstructed from persisted envelopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRunStreamCommit {
    seq: StreamSeq,
    commit_key: CommitKey,
    events: Vec<KernelEventEnvelope>,
}

impl CommittedRunStreamCommit {
    /// Returns the stream sequence shared by this committed batch.
    pub fn seq(&self) -> StreamSeq {
        self.seq
    }

    /// Returns the commit key shared by this committed batch.
    pub fn commit_key(&self) -> &CommitKey {
        &self.commit_key
    }

    /// Returns the events committed atomically in ordinal order.
    pub fn events(&self) -> &[KernelEventEnvelope] {
        &self.events
    }
}

/// Store-owned verified run-stream authority.
///
/// This type proves store-level ordering, commit grouping, projection rebuild, next sequence,
/// and artifact role requirements for one run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRunStream {
    run_id: RunId,
    events: Vec<KernelEventEnvelope>,
    commits: Vec<CommittedRunStreamCommit>,
    projection: ProjectionSnapshot,
    next_seq: StreamSeq,
    artifact_requirements: Vec<EventArtifactRequirement>,
    artifact_bytes: ArtifactByteAuthorityMap,
}

impl CommittedRunStream {
    /// Rebuilds store-owned stream authority from persisted event envelopes.
    pub fn from_events(run_id: RunId, events: Vec<KernelEventEnvelope>) -> Result<Self> {
        Self::from_events_with_artifact_bytes(run_id, events, &ArtifactByteAuthorityMap::new())
    }

    /// Rebuilds store-owned stream authority from persisted event envelopes and retained bytes.
    pub fn from_events_with_artifact_bytes(
        run_id: RunId,
        events: Vec<KernelEventEnvelope>,
        artifact_bytes: &ArtifactByteAuthorityMap,
    ) -> Result<Self> {
        if let Some(event) = events.iter().find(|event| event.run_id() != &run_id) {
            return Err(StoreError::PersistedEventMismatch {
                field: "run_id",
                message: format!(
                    "run stream requested run {} but stream contains {}",
                    run_id,
                    event.run_id()
                ),
            });
        }
        ProjectionSnapshot::validate_run_stream(&events)?;
        let commits = committed_run_stream_commits(&events);
        let projection = ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
            &events,
            artifact_bytes,
        )?;
        let next_seq = next_seq_after_committed_stream(&events)?;
        let artifact_requirements = events
            .iter()
            .flat_map(|event| event_artifact_requirements(event.payload()))
            .collect();
        Ok(Self {
            run_id,
            events,
            commits,
            projection,
            next_seq,
            artifact_requirements,
            artifact_bytes: artifact_bytes.clone(),
        })
    }

    /// Returns the run id covered by this stream.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the committed envelopes in stream order.
    pub fn events(&self) -> &[KernelEventEnvelope] {
        &self.events
    }

    /// Returns atomically committed batches reconstructed from sequence and commit key.
    pub fn commits(&self) -> &[CommittedRunStreamCommit] {
        &self.commits
    }

    /// Returns the projection rebuilt from this verified stream.
    pub fn projection(&self) -> &ProjectionSnapshot {
        &self.projection
    }

    /// Returns the next store-owned stream sequence for this run.
    pub fn next_seq(&self) -> StreamSeq {
        self.next_seq
    }

    /// Returns artifact role requirements referenced by this stream.
    pub fn artifact_requirements(&self) -> &[EventArtifactRequirement] {
        &self.artifact_requirements
    }

    /// Returns exact retained artifact-byte authority used to rebuild this stream projection.
    pub fn artifact_byte_authority(&self) -> &ArtifactByteAuthorityMap {
        &self.artifact_bytes
    }
}

/// Returns canonical JSON bytes for a committed run stream.
///
/// The encoded shape contains store envelope fields plus canonical typed payload JSON. Decoding it
/// with [`committed_run_stream_from_canonical_json_slice`] re-derives event ids, payload hashes,
/// logical keys, projection state, and commit grouping through the normal store authority path.
pub fn committed_run_stream_canonical_json(
    committed: &CommittedRunStream,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "stream_version": 1,
        "run_id": committed.run_id().as_str(),
        "events": committed.events().iter().map(kernel_event_envelope_json).collect::<Vec<_>>(),
    }))
}

/// Rebuilds store-owned stream authority from canonical committed-stream JSON.
///
/// `artifact_bytes` must contain exact byte authority for artifact-bearing events such as terminal
/// state-output cells. The decoder fails closed when the embedded run id differs from
/// `expected_run_id` or when any envelope field no longer derives from its typed payload.
pub fn committed_run_stream_from_canonical_json_slice(
    expected_run_id: &RunId,
    bytes: &[u8],
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<CommittedRunStream> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let json: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    if required_u64(&json, "stream_version")? != 1 {
        return Err(StoreError::Identity(
            "unsupported committed stream version".to_owned(),
        ));
    }
    let run_id: RunId = parse_identity(required_str(&json, "run_id")?)?;
    if &run_id != expected_run_id {
        return Err(StoreError::PersistedEventMismatch {
            field: "run_id",
            message: format!(
                "committed stream artifact covers run {} but import expected {}",
                run_id, expected_run_id
            ),
        });
    }
    let events = parse_vec(&json, "events", parse_kernel_event_envelope)?;
    CommittedRunStream::from_events_with_artifact_bytes(run_id, events, artifact_bytes)
}

/// Boxed future returned by retained artifact read providers.
pub type RetainedArtifactReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<VerifiedRunArtifactBytes>> + Send + 'a>>;

/// Store-owned retained artifact reader for verified run-history construction.
pub trait RetainedArtifactReadProvider: Send + Sync {
    /// Reads artifact bytes and full evidence for one event-derived artifact requirement.
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a EventArtifactRequirement,
    ) -> RetainedArtifactReadFuture<'a>;
}

/// Verified retained artifact bytes and full typed evidence for one run-history artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRunArtifactBytes {
    bytes: Vec<u8>,
    evidence: ArtifactEvidenceRef,
}

impl VerifiedRunArtifactBytes {
    /// Verifies bytes and evidence against one event-derived artifact requirement.
    pub fn new(
        bytes: Vec<u8>,
        evidence: ArtifactEvidenceRef,
        requirement: &EventArtifactRequirement,
    ) -> Result<Self> {
        validate_artifact_requirement_against_evidence(requirement, &evidence)?;
        verify_retained_artifact_bytes(&bytes, &evidence)?;
        Ok(Self { bytes, evidence })
    }

    /// Returns verified retained bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes this proof object into verified retained bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns full typed artifact evidence.
    pub fn evidence(&self) -> &ArtifactEvidenceRef {
        &self.evidence
    }
}

/// Verified retained evidence for every artifact required by one committed run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRunArtifactStore {
    run_id: RunId,
    requirements: Vec<EventArtifactRequirement>,
    artifacts: BTreeMap<ArtifactAuthorityKey, VerifiedRunArtifactBytes>,
}

impl VerifiedRunArtifactStore {
    /// Loads and verifies all retained artifacts required by a committed run stream.
    pub async fn from_committed_stream<P>(
        committed: &CommittedRunStream,
        provider: &P,
    ) -> Result<Self>
    where
        P: RetainedArtifactReadProvider + ?Sized,
    {
        let mut artifacts = BTreeMap::new();
        for requirement in committed.artifact_requirements() {
            let artifact = provider.read_retained_artifact(requirement).await?;
            validate_artifact_requirement_against_evidence(requirement, artifact.evidence())?;
            insert_verified_run_artifact(&mut artifacts, artifact)?;
        }
        Ok(Self {
            run_id: committed.run_id().clone(),
            requirements: committed.artifact_requirements().to_vec(),
            artifacts,
        })
    }

    /// Run id covered by this verified artifact store.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns true when the committed stream required no retained artifacts.
    pub fn is_empty(&self) -> bool {
        self.requirements.is_empty()
    }

    /// Returns verified retained artifact bytes matching an event-derived requirement.
    pub fn artifact_for_requirement(
        &self,
        requirement: &EventArtifactRequirement,
    ) -> Option<&VerifiedRunArtifactBytes> {
        let key = (
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        );
        let artifact = self.artifacts.get(&key)?;
        validate_artifact_requirement_against_evidence(requirement, artifact.evidence())
            .ok()
            .map(|()| artifact)
    }

    /// Iterates verified retained artifacts by exact artifact authority key.
    pub fn artifacts(
        &self,
    ) -> impl Iterator<Item = (&ArtifactAuthorityKey, &VerifiedRunArtifactBytes)> {
        self.artifacts.iter()
    }

    /// Exports exact retained artifact-byte authority for projection rebuilds.
    pub fn artifact_byte_authority_map(&self) -> ArtifactByteAuthorityMap {
        self.artifacts
            .iter()
            .map(|(key, artifact)| {
                (
                    key.clone(),
                    (artifact.bytes().to_vec(), artifact.evidence().clone()),
                )
            })
            .collect()
    }

    /// Verifies this retained artifact store covers a committed run stream exactly.
    pub fn validate_committed_stream(&self, committed: &CommittedRunStream) -> Result<()> {
        if self.run_id != *committed.run_id() {
            return Err(StoreError::PersistedEventMismatch {
                field: "run_id",
                message: format!(
                    "retained artifact store for {} cannot verify run {}",
                    self.run_id,
                    committed.run_id()
                ),
            });
        }
        if self.requirements != committed.artifact_requirements() {
            return Err(StoreError::ProjectionConflict {
                key: "retained_artifacts:requirements".to_owned(),
                message: "retained artifact store requirements do not match committed run stream"
                    .to_owned(),
            });
        }
        self.validate_requirements(committed.artifact_requirements())
    }

    fn validate_requirements(&self, requirements: &[EventArtifactRequirement]) -> Result<()> {
        for requirement in requirements {
            let key = (
                requirement.artifact_id.clone(),
                requirement.evidence_hash.clone(),
            );
            let Some(artifact) = self.artifacts.get(&key) else {
                return Err(StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            validate_artifact_requirement_against_evidence(requirement, artifact.evidence())?;
        }
        Ok(())
    }
}

fn insert_verified_run_artifact(
    artifacts: &mut BTreeMap<ArtifactAuthorityKey, VerifiedRunArtifactBytes>,
    artifact: VerifiedRunArtifactBytes,
) -> Result<()> {
    let key = artifact_authority_key(artifact.evidence())?;
    if let Some(existing) = artifacts.get(&key) {
        if existing != &artifact {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: artifact.evidence().artifact_id.clone(),
                field: "retained_artifact",
            });
        }
        return Ok(());
    }
    artifacts.insert(key, artifact);
    Ok(())
}

fn committed_run_stream_commits(events: &[KernelEventEnvelope]) -> Vec<CommittedRunStreamCommit> {
    let mut commits = Vec::<CommittedRunStreamCommit>::new();
    for event in events {
        if let Some(current) = commits.last_mut() {
            if current.seq == event.seq() && &current.commit_key == event.commit_key() {
                current.events.push(event.clone());
                continue;
            }
        }
        commits.push(CommittedRunStreamCommit {
            seq: event.seq(),
            commit_key: event.commit_key().clone(),
            events: vec![event.clone()],
        });
    }
    commits
}

fn next_seq_after_committed_stream(events: &[KernelEventEnvelope]) -> Result<StreamSeq> {
    let Some(event) = events.last() else {
        return Ok(StreamSeq::FIRST);
    };
    event.seq().checked_next()
}

use self::saga::{derive_saga_projection, forward_ledgers_quiescent};

mod saga;

fn validate_run_stream_order(events: &[KernelEventEnvelope]) -> Result<()> {
    let mut stream_run_id: Option<RunId> = None;
    let mut current_seq: Option<StreamSeq> = None;
    let mut current_commit_key: Option<CommitKey> = None;
    let mut current_store_commit_order: Option<StoreCommitOrder> = None;
    let mut expected_ordinal = 0_u32;

    for event in events {
        match &stream_run_id {
            Some(run_id) if event.run_id() != run_id => {
                return Err(StoreError::PersistedEventMismatch {
                    field: "run_id",
                    message: "persisted run stream contains events for multiple runs".to_owned(),
                });
            }
            Some(_) => {}
            None => stream_run_id = Some(event.run_id().clone()),
        }

        match current_seq {
            Some(seq) if event.seq() == seq => {
                if current_commit_key.as_ref() != Some(event.commit_key()) {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "commit_key",
                        message: format!(
                            "persisted run stream seq {} contains multiple commit keys",
                            event.seq()
                        ),
                    });
                }
                if current_store_commit_order != Some(event.store_commit_order()) {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: format!(
                            "persisted run stream seq {} contains multiple store append coordinates",
                            event.seq()
                        ),
                    });
                }
            }
            Some(seq) => {
                let expected_next = seq.checked_next()?;
                if event.seq() != expected_next {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "seq",
                        message: format!(
                            "persisted run stream expected seq {expected_next} but found {}",
                            event.seq()
                        ),
                    });
                }
                let previous_store_commit_order = current_store_commit_order.ok_or_else(|| {
                    StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: "persisted run stream has no coordinate for the previous commit"
                            .to_owned(),
                    }
                })?;
                if event.store_commit_order() <= previous_store_commit_order {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: format!(
                            "persisted run stream store append coordinate {} is not greater than the previous commit",
                            event.store_commit_order().as_u64()
                        ),
                    });
                }
                current_seq = Some(expected_next);
                current_commit_key = Some(event.commit_key().clone());
                current_store_commit_order = Some(event.store_commit_order());
                expected_ordinal = 0;
            }
            None => {
                if event.seq() != StreamSeq::FIRST {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "seq",
                        message: format!(
                            "persisted run stream expected first seq {} but found {}",
                            StreamSeq::FIRST,
                            event.seq()
                        ),
                    });
                }
                if event.store_commit_order() < StoreCommitOrder::FIRST {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "store_commit_order",
                        message: "persisted run stream first commit has a non-positive store append coordinate"
                            .to_owned(),
                    });
                }
                current_seq = Some(StreamSeq::FIRST);
                current_commit_key = Some(event.commit_key().clone());
                current_store_commit_order = Some(event.store_commit_order());
            }
        }

        if event.ordinal().as_u32() != expected_ordinal {
            return Err(StoreError::PersistedEventMismatch {
                field: "ordinal",
                message: format!(
                    "persisted run stream expected ordinal {expected_ordinal} for seq {} but found {}",
                    event.seq(),
                    event.ordinal()
                ),
            });
        }
        expected_ordinal = expected_ordinal
            .checked_add(1)
            .ok_or(StoreError::SequenceOverflow)?;
    }

    Ok(())
}

#[cfg(test)]
mod stream_order_tests {
    use super::*;

    fn event(run_id: &RunId, seq: u64, store_commit_order: u64) -> KernelEventEnvelope {
        test_support::persisted_kernel_event_envelope_for_test(
            run_id,
            seq,
            store_commit_order,
            CommitKey::new(format!("order-{seq}")).expect("commit key"),
            KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: test_support::fixed_spec_hash_for_test(1),
                node_id: test_support::fixed_node_id_for_test(2),
                attempt_id: test_support::fixed_attempt_id_for_test(3),
                attempt_no: 1,
                state_kind: test_support::fixed_state_kind_for_test(4),
                state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
            }),
        )
    }

    #[test]
    fn committed_stream_coordinates_are_positive_and_strictly_increasing() {
        let run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            test_support::fixed_digest_bytes_for_test(5),
        );

        let first = event(&run_id, 1, 2);
        let repeated = event(&run_id, 2, 2);
        let error = validate_run_stream_order(&[first, repeated])
            .expect_err("repeated store coordinate must reject");
        assert!(error.to_string().contains("not greater"), "{error}");

        let non_positive = event(&run_id, 1, 0);
        let error = validate_run_stream_order(&[non_positive])
            .expect_err("non-positive first store coordinate must reject");
        assert!(error.to_string().contains("non-positive"), "{error}");
    }
}

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

/// Authoritative state needed to validate and stage one absent commit-key append.
///
/// Durable stores load this from their run stream, artifact table, logical-key table, and
/// rebuilt projections before calling [`stage_prepared_commit_plan`]. Commit-key lookup remains
/// the storage implementation's responsibility because the RFC requires that lookup to precede
/// stale `expected_next_seq` checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitBase {
    /// Exact artifact evidence recorded before event commit.
    pub artifacts: ArtifactAuthorityMap,
    /// Exact retained or same-commit artifact bytes available for projection validation.
    pub artifact_bytes: ArtifactByteAuthorityMap,
    /// Logical keys already present in the run stream.
    pub logical_keys: LogicalKeySet,
    /// Unique logical keys and their current payload hash.
    ///
    /// Most unique logical keys are immutable after their first write. The side-effect
    /// `submission_result` key is the one recoverable slot: `SubmissionUnknown` may be
    /// refreshed or superseded in projection by later recovery evidence for the same invocation
    /// epoch.
    pub unique_logical_payloads: UniqueLogicalPayloads,
    /// Current projections derived from the authoritative run stream.
    pub projections: ProjectionSnapshot,
    /// Lane-local transition authority folded from committed resource-lane history.
    pub resource_lane_authority: ResourceLaneAuthoritySet,
    /// Store-owned next sequence for the run being committed.
    pub actual_next_seq: StreamSeq,
    /// Store-owned append coordinate assigned to this atomic commit.
    pub store_commit_order: StoreCommitOrder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterializedActiveLane {
    holder: SideEffectPairLedgerRef,
    ledger_purpose: events::SideEffectLedgerPurpose,
    node_id: NodeId,
    attempt_id: AttemptId,
    invocation_epoch: u32,
    claim_id: events::ResourceLaneClaimId,
    claim_fencing_token: u64,
    lane_transition_seq: u64,
}

/// Staged result of validating a typed commit against a [`CommitBase`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedCommit {
    batch: CommittedBatch,
    logical_keys: LogicalKeySet,
    unique_logical_payloads: UniqueLogicalPayloads,
    projections: ProjectionSnapshot,
    resource_lane_authority: ResourceLaneAuthoritySet,
}

impl StagedCommit {
    /// Store-owned committed batch.
    pub fn batch(&self) -> &CommittedBatch {
        &self.batch
    }

    /// Staged projection snapshot after this commit.
    pub fn projections(&self) -> &ProjectionSnapshot {
        &self.projections
    }

    /// Consumes this staged commit into owned parts.
    pub fn into_parts(
        self,
    ) -> (
        CommittedBatch,
        LogicalKeySet,
        UniqueLogicalPayloads,
        ProjectionSnapshot,
        ResourceLaneAuthoritySet,
    ) {
        (
            self.batch,
            self.logical_keys,
            self.unique_logical_payloads,
            self.projections,
            self.resource_lane_authority,
        )
    }
}

/// Result of staging an absent commit key before durable insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedCommitOutcome {
    /// The commit staged successfully and is ready for durable insertion.
    Staged(Box<StagedCommit>),
    /// FIFO admission was blocked; no domain authority rows should be persisted.
    AdmissionBlocked(Box<WaitFifoAdmissionBlock>),
}

struct CommitStagingVerifier<'a> {
    artifacts: &'a ArtifactAuthorityMap,
    logical_keys: &'a BTreeSet<(RunId, LogicalEventKey)>,
    projections: &'a ProjectionSnapshot,
}

impl CommitStagingVerifier<'_> {
    fn validate_artifact_evidence(&self, evidence: &ArtifactEvidenceRef) -> Result<()> {
        let key = artifact_authority_key(evidence)?;
        let Some(stored) = self.artifacts.get(&key) else {
            return Err(StoreError::MissingArtifact {
                artifact_id: evidence.artifact_id.clone(),
            });
        };
        compare_artifact_field(
            &evidence.artifact_id,
            "digest",
            stored.digest.as_str(),
            evidence.digest.as_str(),
        )?;
        compare_artifact_field(
            &evidence.artifact_id,
            "byte_len",
            stored.byte_len,
            evidence.byte_len,
        )?;
        compare_artifact_field(
            &evidence.artifact_id,
            "media_type",
            stored.media_type.as_str(),
            evidence.media_type.as_str(),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "schema_id",
            stored.schema_id.as_ref().map(SchemaId::as_str),
            evidence.schema_id.as_ref().map(SchemaId::as_str),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "semantic_type_id",
            stored.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
            evidence
                .semantic_type_id
                .as_ref()
                .map(SemanticTypeId::as_str),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "producer_node_id",
            stored.producer_node_id.as_ref().map(NodeId::as_str),
            evidence.producer_node_id.as_ref().map(NodeId::as_str),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "producer_seed_id",
            stored.producer_seed_id.as_ref().map(SeedId::as_str),
            evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        )?;
        compare_artifact_field(
            &evidence.artifact_id,
            "artifact_role",
            stored.artifact_role.as_str(),
            evidence.artifact_role.as_str(),
        )
    }

    fn validate_artifact_requirement(&self, requirement: &EventArtifactRequirement) -> Result<()> {
        let key = (
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        );
        let Some(evidence) = self.artifacts.get(&key) else {
            return Err(StoreError::MissingArtifact {
                artifact_id: requirement.artifact_id.clone(),
            });
        };
        validate_artifact_requirement_against_evidence(requirement, evidence)
    }

    fn validate_preconditions(&self, request: &CommitRequest) -> Result<()> {
        let actual_run_state = self.projections.run_state(&request.run_id);
        let run_state_ok = match request.preconditions.required_run_state {
            RequiredRunState::Any => true,
            RequiredRunState::Absent => actual_run_state == RunState::Absent,
            RequiredRunState::Started => actual_run_state != RunState::Absent,
            RequiredRunState::NotCompleted => actual_run_state == RunState::Started,
            RequiredRunState::Completed => actual_run_state == RunState::Completed,
        };
        if !run_state_ok {
            return Err(StoreError::RunStatePreconditionFailed {
                required: request.preconditions.required_run_state,
                actual: actual_run_state,
            });
        }

        for key in &request.preconditions.required_absent_logical_keys {
            if self
                .logical_keys
                .contains(&(request.run_id.clone(), key.clone()))
            {
                return Err(StoreError::LogicalKeyPreconditionFailed {
                    logical_key: key.clone(),
                    message: "key must be absent".to_owned(),
                });
            }
        }
        for key in &request.preconditions.required_present_logical_keys {
            if !self
                .logical_keys
                .contains(&(request.run_id.clone(), key.clone()))
            {
                return Err(StoreError::LogicalKeyPreconditionFailed {
                    logical_key: key.clone(),
                    message: "key must be present".to_owned(),
                });
            }
        }
        for precondition in &request.preconditions.required_cell_states {
            let actual = self
                .projections
                .cell_terminal_for_run(&request.run_id, &precondition.cell_id);
            let ok = match precondition.required {
                RequiredCellState::Absent => actual.is_none(),
                RequiredCellState::Produced => {
                    matches!(actual, Some(CellTerminalProjection::Produced { .. }))
                }
                RequiredCellState::Skipped => {
                    matches!(actual, Some(CellTerminalProjection::Skipped { .. }))
                }
                RequiredCellState::Terminal => actual.is_some(),
            };
            if !ok {
                return Err(StoreError::CellStatePreconditionFailed {
                    cell_id: precondition.cell_id.clone(),
                    required: precondition.required,
                });
            }
        }
        for precondition in &request.preconditions.required_side_effect_states {
            let actual = self
                .projections
                .side_effect_for_pair(&request.run_id, &precondition.pair_id);
            if !side_effect_precondition_matches(actual, precondition.required)? {
                return Err(StoreError::SideEffectStatePreconditionFailed {
                    pair_id: precondition.pair_id.clone(),
                    required: precondition.required,
                });
            }
        }
        if request.preconditions.required_public_output_absent
            && self.projections.has_public_output(&request.run_id)
        {
            return Err(StoreError::PublicOutputPreconditionFailed);
        }
        if request
            .payloads
            .iter()
            .any(|payload| matches!(payload, KernelEventPayload::FactRecorded(_)))
            && request.preconditions.certified_run_authority.is_none()
        {
            return Err(StoreError::ProjectionConflict {
                key: "fact:certified_run_authority".to_owned(),
                message: "fact recording requires certified run authority".to_owned(),
            });
        }
        Ok(())
    }
}

/// Private state behind the async typed store test backend.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone)]
struct RunMemoryCore {
    store_scope_id: StoreScopeId,
    next_store_commit_order: StoreCommitOrder,
    streams: BTreeMap<RunId, Vec<CommittedBatch>>,
    commit_keys: BTreeMap<(RunId, CommitKey), CommitKeyRecord>,
    artifacts: ArtifactAuthorityMap,
    artifact_bytes: BTreeMap<(ArtifactId, ContentDigest), (Vec<u8>, ArtifactEvidenceRef)>,
    logical_keys: BTreeSet<(RunId, LogicalEventKey)>,
    unique_logical_payloads: BTreeMap<(RunId, LogicalEventKey), ContentDigest>,
    projections: ProjectionSnapshot,
    resource_lane_authority: ResourceLaneAuthoritySet,
    execution_claims: BTreeMap<AdmissionLaneKey, MemoryExecutionClaim>,
}

#[cfg(any(test, feature = "test-support"))]
impl Default for RunMemoryCore {
    fn default() -> Self {
        Self {
            store_scope_id: StoreScopeId::new(
                "mfm.store_scope.v1:00000000000000000000000000000000",
            )
            .expect("test store scope"),
            next_store_commit_order: StoreCommitOrder::FIRST,
            streams: BTreeMap::new(),
            commit_keys: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            artifact_bytes: BTreeMap::new(),
            logical_keys: BTreeSet::new(),
            unique_logical_payloads: BTreeMap::new(),
            projections: ProjectionSnapshot::default(),
            resource_lane_authority: ResourceLaneAuthoritySet::default(),
            execution_claims: BTreeMap::new(),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryExecutionClaim {
    lane: ExecutionClaimAdmissionLane,
    holder_run_id: RunId,
    token: AdmissionToken,
    lease_expires_at_unix_ms: i64,
}

#[cfg(any(test, feature = "test-support"))]
impl RunMemoryCore {
    fn stream_events(&self, run_id: &RunId) -> Vec<KernelEventEnvelope> {
        self.streams
            .get(run_id)
            .into_iter()
            .flat_map(|batches| batches.iter())
            .flat_map(|batch| batch.events.iter().cloned())
            .collect()
    }

    fn acquire_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: AdmissionToken,
    ) -> Result<NowaitSkipAdmissionResult> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let lane_key = lane.erased_key();
        if let Some(holder) = self.execution_claims.get(&lane_key) {
            return Ok(NowaitSkipAdmissionResult::Busy(NowaitSkipAdmissionBusy {
                lane,
                holder: Some(execution_claim_lease(holder)),
            }));
        }
        let lease = AdmissionLease {
            lane: lane_key.clone(),
            holder_run_id: holder_run_id.clone(),
            token,
            lease_expires_at_unix_ms: execution_claim_expiry_unix_ms()?,
        };
        self.execution_claims.insert(
            lane_key,
            MemoryExecutionClaim {
                lane,
                holder_run_id: holder_run_id.clone(),
                token: lease.token.clone(),
                lease_expires_at_unix_ms: lease.lease_expires_at_unix_ms,
            },
        );
        Ok(NowaitSkipAdmissionResult::Admitted(lease))
    }

    fn execution_claim_status(&self, scope: &ExecutionClaimScope) -> Result<ExecutionClaimStatus> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let Some(holder) = self.execution_claims.get(&lane.erased_key()) else {
            return Ok(ExecutionClaimStatus::Unclaimed);
        };
        let lease = execution_claim_lease(holder);
        if holder.lease_expires_at_unix_ms <= unix_time_ms()? {
            Ok(ExecutionClaimStatus::Expired(lease))
        } else {
            Ok(ExecutionClaimStatus::Live(lease))
        }
    }

    fn renew_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: &AdmissionToken,
    ) -> Result<Option<AdmissionLease>> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let Some(holder) = self.execution_claims.get_mut(&lane.erased_key()) else {
            return Ok(None);
        };
        if &holder.holder_run_id != holder_run_id || &holder.token != token {
            return Ok(None);
        }
        holder.lease_expires_at_unix_ms = execution_claim_expiry_unix_ms()?;
        Ok(Some(execution_claim_lease(holder)))
    }

    fn release_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: &AdmissionToken,
    ) -> Result<bool> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let lane_key = lane.erased_key();
        let matches = self
            .execution_claims
            .get(&lane_key)
            .map(|holder| &holder.holder_run_id == holder_run_id && &holder.token == token)
            .unwrap_or(false);
        if matches {
            self.execution_claims.remove(&lane_key);
        }
        Ok(matches)
    }

    fn expired_execution_claims(&self) -> Result<Vec<ExpiredExecutionClaim>> {
        let now = unix_time_ms()?;
        Ok(self
            .execution_claims
            .iter()
            .filter(|(_, holder)| holder.lease_expires_at_unix_ms <= now)
            .map(|(_, holder)| ExpiredExecutionClaim {
                run_id: holder.holder_run_id.clone(),
                lane: holder.lane.clone(),
                lease: execution_claim_lease(holder),
            })
            .collect())
    }

    fn reap_expired_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: &AdmissionToken,
    ) -> Result<bool> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let lane_key = lane.erased_key();
        let now = unix_time_ms()?;
        let matches = self
            .execution_claims
            .get(&lane_key)
            .map(|holder| {
                &holder.holder_run_id == holder_run_id
                    && &holder.token == token
                    && holder.lease_expires_at_unix_ms <= now
            })
            .unwrap_or(false);
        if matches {
            self.execution_claims.remove(&lane_key);
        }
        Ok(matches)
    }
}

#[cfg(any(test, feature = "test-support"))]
fn execution_claim_lease(holder: &MemoryExecutionClaim) -> AdmissionLease {
    AdmissionLease {
        lane: holder.lane.erased_key(),
        holder_run_id: holder.holder_run_id.clone(),
        token: holder.token.clone(),
        lease_expires_at_unix_ms: holder.lease_expires_at_unix_ms,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn execution_claim_expiry_unix_ms() -> Result<i64> {
    let ttl_ms = i64::try_from(EXECUTION_CLAIM_LEASE_TTL_SECS)
        .ok()
        .and_then(|secs| secs.checked_mul(1000))
        .ok_or(StoreError::SequenceOverflow)?;
    unix_time_ms()?
        .checked_add(ttl_ms)
        .ok_or(StoreError::SequenceOverflow)
}

#[cfg(any(test, feature = "test-support"))]
fn unix_time_ms() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StoreError::Event("system time before unix epoch".to_owned()))?;
    i64::try_from(duration.as_millis()).map_err(|_| StoreError::SequenceOverflow)
}

/// Validates and stages a purpose-specific prepared commit plan after commit-key idempotency handling.
///
/// The returned batch fingerprint covers the full prepared mutation, including the artifact
/// evidence admitted atomically with the event payloads.
pub fn stage_prepared_commit_plan(
    base: &CommitBase,
    plan: &PreparedCommitPlan,
) -> Result<StagedCommitOutcome> {
    let fingerprint = prepared_commit_plan_fingerprint(plan)?;
    match materialize_resource_lane_intents(base, plan.request(), &fingerprint)? {
        ResourceLaneMaterialization::Materialized {
            request,
            resource_lane_authority,
        } => {
            stage_run_commit_with_fingerprint(base, &request, fingerprint, resource_lane_authority)
                .map(|staged| StagedCommitOutcome::Staged(Box::new(staged)))
        }
        ResourceLaneMaterialization::Blocked(block) => {
            Ok(StagedCommitOutcome::AdmissionBlocked(block))
        }
    }
}

enum ResourceLaneMaterialization {
    Materialized {
        request: Box<CommitRequest>,
        resource_lane_authority: ResourceLaneAuthoritySet,
    },
    Blocked(Box<WaitFifoAdmissionBlock>),
}

fn stage_run_commit_with_fingerprint(
    base: &CommitBase,
    request: &CommitRequest,
    fingerprint: CommitFingerprint,
    staged_resource_lane_authority: ResourceLaneAuthoritySet,
) -> Result<StagedCommit> {
    if request.expected_next_seq != base.actual_next_seq {
        return Err(StoreError::StaleExpectedNextSeq {
            expected: request.expected_next_seq,
            actual: base.actual_next_seq,
        });
    }

    validate_terminal_attempt_cell_pairs(&request.payloads)?;
    validate_terminal_side_effect_evidence_pairs(&request.payloads)?;
    validate_side_effect_attempt_failures_have_terminal_evidence(
        &base.projections,
        &request.payloads,
    )?;
    validate_retention_manifest_pairs(&request.payloads)?;
    validate_payload_public_diagnostics(&request.payloads)?;

    let verifier = CommitStagingVerifier {
        artifacts: &base.artifacts,
        logical_keys: &base.logical_keys,
        projections: &base.projections,
    };
    verifier.validate_preconditions(request)?;

    for evidence in &request.required_artifacts {
        verifier.validate_artifact_evidence(evidence)?;
    }
    for payload in &request.payloads {
        for requirement in event_artifact_requirements(payload) {
            verifier.validate_artifact_requirement(&requirement)?;
        }
    }

    let mut staged_projections = base.projections.clone();
    let mut staged_logical_keys = base.logical_keys.clone();
    let mut staged_unique_payloads = base.unique_logical_payloads.clone();
    let mut commit_unique_keys = BTreeSet::new();
    let mut events = Vec::with_capacity(request.payloads.len());
    for (index, payload) in request.payloads.iter().cloned().enumerate() {
        let canonical_payload = payload_canonical_json(&payload)?;
        let payload_hash = canonical_payload.content_digest();
        let schema_id = payload.event_schema_id()?;
        let ordinal = CommitOrdinal::from_index(index)?;
        let logical_key = derive_logical_key(
            &request.run_id,
            request.expected_next_seq,
            ordinal,
            &payload,
            &payload_hash,
        )?;
        let event_id = derive_event_id(
            &request.run_id,
            request.expected_next_seq,
            ordinal,
            &schema_id,
            &payload_hash,
        )?;
        let spec_hash = payload_spec_hash(&payload);
        let envelope = KernelEventEnvelope {
            event_id,
            event_schema_id: schema_id,
            run_id: request.run_id.clone(),
            seq: request.expected_next_seq,
            store_commit_order: base.store_commit_order,
            ordinal,
            spec_hash,
            commit_key: request.commit_key.clone(),
            logical_key,
            payload_hash,
            payload,
        };

        let key = (request.run_id.clone(), envelope.logical_key.clone());
        if is_unique_logical_key(&envelope.logical_key) {
            if !commit_unique_keys.insert(key.clone()) {
                return Err(StoreError::DuplicateLogicalKey {
                    logical_key: envelope.logical_key.clone(),
                });
            }
            if let Some(existing_hash) = staged_unique_payloads.get(&key) {
                if existing_hash == &envelope.payload_hash {
                    return Err(StoreError::DuplicateLogicalKey {
                        logical_key: envelope.logical_key.clone(),
                    });
                }
                if !unique_logical_key_rewrite_allowed(
                    base,
                    &request.run_id,
                    &envelope.logical_key,
                    &envelope.payload,
                    &staged_projections,
                )? {
                    return Err(StoreError::LogicalKeyConflict {
                        logical_key: envelope.logical_key.clone(),
                    });
                }
            }
            staged_unique_payloads.insert(key.clone(), envelope.payload_hash.clone());
        }
        staged_logical_keys.insert(key);
        require_admission_preconditions(
            &staged_projections,
            &request.run_id,
            &envelope.payload,
            request.preconditions.certified_run_authority.as_ref(),
        )?;
        projection::apply_projection(&mut staged_projections, &envelope, &base.artifact_bytes)?;
        events.push(envelope);
    }

    Ok(StagedCommit {
        batch: CommittedBatch {
            run_id: request.run_id.clone(),
            commit_key: request.commit_key.clone(),
            fingerprint,
            seq: request.expected_next_seq,
            store_commit_order: base.store_commit_order,
            events,
        },
        logical_keys: staged_logical_keys,
        unique_logical_payloads: staged_unique_payloads,
        projections: staged_projections,
        resource_lane_authority: staged_resource_lane_authority,
    })
}

fn materialize_resource_lane_intents(
    base: &CommitBase,
    request: &CommitRequest,
    fingerprint: &CommitFingerprint,
) -> Result<ResourceLaneMaterialization> {
    let mut resource_lane_authority = base.resource_lane_authority.clone();
    let mut active_lanes = materialized_active_resource_lanes(&base.projections);
    let mut materialized_payloads = Vec::with_capacity(request.payloads.len());

    for payload in &request.payloads {
        match payload {
            KernelEventPayload::ResourceLaneClaimIntent(intent) => {
                require_side_effect_pair_role(
                    &intent.ledger_key,
                    &intent.ledger_purpose,
                    intent.pair_role,
                    events::SideEffectPairRole::Submit,
                )?;
                let lane_key = ResourceLaneKey::from_evidence(&intent.resource_key);
                let holder =
                    SideEffectPairLedgerRef::new(request.run_id.clone(), intent.pair_id.clone());
                if let Some(existing_key) =
                    materialized_lane_key_for_pair(&active_lanes, &request.run_id, &intent.pair_id)
                {
                    let existing = active_lanes
                        .get(&existing_key)
                        .expect("resource lane key was found from pair materialization");
                    if existing.holder != holder {
                        return Err(StoreError::ProjectionConflict {
                            key: format!("resource_lane_pair:{}", intent.pair_id),
                            message: "side-effect pair already has active resource lane".to_owned(),
                        });
                    }
                }
                if let Some((existing_key, _)) = active_lanes
                    .iter()
                    .find(|(_, active)| active.holder == holder)
                {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("resource_lane_pair:{}", intent.pair_id),
                        message: format!(
                            "side-effect holder already has active resource lane {}:{}",
                            existing_key.namespace, existing_key.key
                        ),
                    });
                }
                if let Some(existing) = active_lanes.get(&lane_key) {
                    if existing.holder != holder {
                        return Ok(ResourceLaneMaterialization::Blocked(Box::new(
                            WaitFifoAdmissionBlock {
                                resource_lane_key: lane_key,
                                holder: Some(existing.holder.clone()),
                                waiter: None,
                            },
                        )));
                    }
                }

                let authority = resource_lane_authority.entry(lane_key.clone()).or_default();
                let lane_transition_seq = checked_lane_increment(
                    authority.last_transition_seq,
                    "resource lane transition sequence",
                )?;
                let claim_fencing_token = checked_lane_increment(
                    authority.last_claim_fencing_token,
                    "resource lane fencing token",
                )?;
                let claim_id = derive_resource_lane_claim_id(
                    &request.run_id,
                    &lane_key,
                    intent,
                    claim_fencing_token,
                    lane_transition_seq,
                    fingerprint,
                )?;
                let claimed = events::ResourceLaneClaimed {
                    spec_hash: intent.spec_hash.clone(),
                    node_id: intent.node_id.clone(),
                    attempt_id: intent.attempt_id.clone(),
                    ledger_key: intent.ledger_key.clone(),
                    ledger_purpose: intent.ledger_purpose.clone(),
                    pair_id: intent.pair_id.clone(),
                    pair_role: intent.pair_role,
                    invocation_epoch: intent.invocation_epoch,
                    resource_key: intent.resource_key.clone(),
                    requirement_digest: intent.requirement_digest.clone(),
                    resolved_by_capability_impl: intent.resolved_by_capability_impl.clone(),
                    claim_id,
                    claim_fencing_token,
                    lane_transition_seq,
                };
                authority.last_transition_seq = lane_transition_seq;
                authority.last_claim_fencing_token = claim_fencing_token;
                active_lanes.insert(
                    lane_key,
                    MaterializedActiveLane::from_claimed(&request.run_id, &claimed),
                );
                materialized_payloads.push(KernelEventPayload::ResourceLaneClaimed(claimed));
            }
            KernelEventPayload::ResourceLaneReleaseIntent(intent) => {
                require_side_effect_pair_role(
                    &intent.ledger_key,
                    &intent.ledger_purpose,
                    intent.pair_role,
                    events::SideEffectPairRole::Verify,
                )?;
                let (lane_key, active) = resolve_active_resource_lane_release(
                    &active_lanes,
                    ResourceLaneReleaseMatch {
                        run_id: &request.run_id,
                        ledger_key: &intent.ledger_key,
                        ledger_purpose: &intent.ledger_purpose,
                        pair_id: &intent.pair_id,
                        invocation_epoch: intent.invocation_epoch,
                        claim_id: &intent.claim_id,
                        mismatch_message:
                            "resource lane release intent does not match active claim",
                    },
                )?;
                let lane_key = lane_key.clone();
                let active = active.clone();
                let authority = resource_lane_authority.get_mut(&lane_key).ok_or_else(|| {
                    StoreError::ProjectionConflict {
                        key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
                        message: "resource lane authority missing active claim history".to_owned(),
                    }
                })?;
                if authority.last_transition_seq < active.lane_transition_seq
                    || authority.last_claim_fencing_token < active.claim_fencing_token
                {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
                        message: "resource lane authority is behind active claim".to_owned(),
                    });
                }
                let lane_transition_seq = checked_lane_increment(
                    authority.last_transition_seq,
                    "resource lane transition sequence",
                )?;
                let release_id = derive_resource_lane_release_id(
                    &request.run_id,
                    &lane_key,
                    intent,
                    active.claim_fencing_token,
                    lane_transition_seq,
                    fingerprint,
                )?;
                let released = events::ResourceLaneReleased {
                    spec_hash: intent.spec_hash.clone(),
                    ledger_key: intent.ledger_key.clone(),
                    ledger_purpose: intent.ledger_purpose.clone(),
                    pair_id: intent.pair_id.clone(),
                    pair_role: intent.pair_role,
                    invocation_epoch: intent.invocation_epoch,
                    claim_id: intent.claim_id.clone(),
                    release_id,
                    claim_fencing_token: active.claim_fencing_token,
                    release_authority: intent.release_authority,
                    release_reason: intent.release_reason.clone(),
                    lane_transition_seq,
                };
                authority.last_transition_seq = lane_transition_seq;
                active_lanes.remove(&lane_key);
                materialized_payloads.push(KernelEventPayload::ResourceLaneReleased(released));
            }
            KernelEventPayload::ResourceLaneClaimed(_)
            | KernelEventPayload::ResourceLaneReleased(_) => {
                return Err(StoreError::Event(
                    "prepared commits must use resource-lane intents, not store-filled lane events"
                        .to_owned(),
                ));
            }
            _ => materialized_payloads.push(payload.clone()),
        }
    }

    let request = CommitRequest::from_payloads(
        request.run_id.clone(),
        request.expected_next_seq,
        request.commit_key.clone(),
        materialized_payloads,
        request.required_artifacts.clone(),
        request.preconditions.clone(),
    )?;
    Ok(ResourceLaneMaterialization::Materialized {
        request: Box::new(request),
        resource_lane_authority,
    })
}

impl MaterializedActiveLane {
    fn from_projection(projection: &ResourceLaneProjection) -> Self {
        Self {
            holder: projection.holder.clone(),
            ledger_purpose: projection.ledger_purpose.clone(),
            node_id: projection.node_id.clone(),
            attempt_id: projection.attempt_id.clone(),
            invocation_epoch: projection.invocation_epoch,
            claim_id: projection.claim_id.clone(),
            claim_fencing_token: projection.claim_fencing_token,
            lane_transition_seq: projection.lane_transition_seq,
        }
    }

    fn from_claimed(run_id: &RunId, payload: &events::ResourceLaneClaimed) -> Self {
        Self {
            holder: SideEffectPairLedgerRef::new(run_id.clone(), payload.pair_id.clone()),
            ledger_purpose: payload.ledger_purpose.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            claim_id: payload.claim_id.clone(),
            claim_fencing_token: payload.claim_fencing_token,
            lane_transition_seq: payload.lane_transition_seq,
        }
    }
}

fn materialized_active_resource_lanes(
    projections: &ProjectionSnapshot,
) -> BTreeMap<ResourceLaneKey, MaterializedActiveLane> {
    projections
        .resource_lanes()
        .map(|(lane_key, projection)| {
            (
                lane_key.clone(),
                MaterializedActiveLane::from_projection(projection),
            )
        })
        .collect()
}

fn materialized_lane_key_for_pair(
    active_lanes: &BTreeMap<ResourceLaneKey, MaterializedActiveLane>,
    run_id: &RunId,
    pair_id: &SideEffectPairId,
) -> Option<ResourceLaneKey> {
    active_lanes.iter().find_map(|(key, active)| {
        (active.holder.run_id == *run_id && active.holder.pair_id == *pair_id).then(|| key.clone())
    })
}

fn checked_lane_increment(value: u64, label: &'static str) -> Result<u64> {
    value
        .checked_add(1)
        .filter(|value| *value > 0)
        .ok_or_else(|| StoreError::Event(format!("{label} overflow")))
}

fn derive_resource_lane_claim_id(
    run_id: &RunId,
    lane_key: &ResourceLaneKey,
    intent: &events::ResourceLaneClaimIntent,
    claim_fencing_token: u64,
    lane_transition_seq: u64,
    fingerprint: &CommitFingerprint,
) -> Result<events::ResourceLaneClaimId> {
    let digest = canonical_json(serde_json::json!({
        "attempt_id": intent.attempt_id.as_str(),
        "claim_fencing_token": claim_fencing_token,
        "commit_fingerprint": fingerprint.as_digest().as_str(),
        "invocation_epoch": intent.invocation_epoch,
        "lane_key": {
            "key": lane_key.key.as_str(),
            "namespace": lane_key.namespace.as_str(),
        },
        "lane_transition_seq": lane_transition_seq,
        "ledger_key": intent.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&intent.ledger_purpose),
        "node_id": intent.node_id.as_str(),
        "pair_id": intent.pair_id.as_str(),
        "pair_role": intent.pair_role.as_str(),
        "run_id": run_id.as_str(),
    }))?
    .content_digest();
    Ok(events::ResourceLaneClaimId::new(format!(
        "mfm.store.lane.claim.{}",
        short_stable_id_fragment(digest.as_str(), 32)
    ))?)
}

fn derive_resource_lane_release_id(
    run_id: &RunId,
    lane_key: &ResourceLaneKey,
    intent: &events::ResourceLaneReleaseIntent,
    claim_fencing_token: u64,
    lane_transition_seq: u64,
    fingerprint: &CommitFingerprint,
) -> Result<events::ResourceLaneReleaseId> {
    let digest = canonical_json(serde_json::json!({
        "claim_fencing_token": claim_fencing_token,
        "claim_id": intent.claim_id.as_str(),
        "commit_fingerprint": fingerprint.as_digest().as_str(),
        "invocation_epoch": intent.invocation_epoch,
        "lane_key": {
            "key": lane_key.key.as_str(),
            "namespace": lane_key.namespace.as_str(),
        },
        "lane_transition_seq": lane_transition_seq,
        "ledger_key": intent.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&intent.ledger_purpose),
        "pair_id": intent.pair_id.as_str(),
        "pair_role": intent.pair_role.as_str(),
        "release_authority": intent.release_authority.as_str(),
        "release_reason": intent.release_reason.as_str(),
        "run_id": run_id.as_str(),
    }))?
    .content_digest();
    Ok(events::ResourceLaneReleaseId::new(format!(
        "mfm.store.lane.release.{}",
        short_stable_id_fragment(digest.as_str(), 32)
    ))?)
}

#[cfg(any(test, feature = "test-support"))]
impl RunMemoryCore {
    fn projection_snapshot(&self) -> &ProjectionSnapshot {
        &self.projections
    }

    fn append_prepared_commit_bundle(
        &mut self,
        bundle: PreparedCommitBundle,
    ) -> Result<CommitOutcome> {
        let plan = bundle.plan();
        let request = plan.request();
        let fingerprint = prepared_commit_plan_fingerprint(plan)?;

        if let Some(record) = self
            .commit_keys
            .get(&(request.run_id.clone(), request.commit_key.clone()))
        {
            if record.fingerprint == fingerprint {
                return Ok(CommitOutcome::Idempotent(record.batch.clone()));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key.clone(),
            });
        }

        let mut artifacts = self.artifacts.clone();
        verify_existing_artifact_admissions(&artifacts, &bundle)?;
        admit_artifact_evidence(&mut artifacts, bundle.admitted_artifacts())?;
        let artifact_bytes = artifact_byte_authority_for_bundle(&self.artifact_bytes, &bundle)?;
        let base = CommitBase {
            artifacts,
            artifact_bytes: artifact_bytes.clone(),
            logical_keys: self.logical_keys.clone(),
            unique_logical_payloads: self.unique_logical_payloads.clone(),
            projections: self.projections.clone(),
            resource_lane_authority: self.resource_lane_authority.clone(),
            actual_next_seq: self.expected_next_seq(&request.run_id),
            store_commit_order: self.next_store_commit_order,
        };
        let staged = match stage_prepared_commit_plan(&base, plan)? {
            StagedCommitOutcome::Staged(staged) => *staged,
            StagedCommitOutcome::AdmissionBlocked(block) => {
                return Ok(CommitOutcome::AdmissionBlocked(block));
            }
        };
        if let Some(claim) = bundle.execution_claim() {
            match self.acquire_execution_claim(
                &claim.scope,
                &claim.holder_run_id,
                claim.token.clone(),
            )? {
                NowaitSkipAdmissionResult::Admitted(_) => {}
                NowaitSkipAdmissionResult::Busy(busy) => {
                    return Ok(CommitOutcome::ExecutionClaimBusy(Box::new(busy)));
                }
            }
        }
        let (
            batch,
            staged_logical_keys,
            staged_unique_payloads,
            staged_projections,
            staged_resource_lane_authority,
        ) = staged.into_parts();
        self.next_store_commit_order = batch
            .store_commit_order()
            .checked_next()
            .ok_or(StoreError::SequenceOverflow)?;
        self.artifact_bytes = artifact_bytes;
        self.streams
            .entry(request.run_id.clone())
            .or_default()
            .push(batch.clone());
        self.commit_keys.insert(
            (request.run_id.clone(), request.commit_key.clone()),
            CommitKeyRecord {
                fingerprint,
                batch: batch.clone(),
            },
        );
        self.artifacts = base.artifacts;
        self.logical_keys = staged_logical_keys;
        self.unique_logical_payloads = staged_unique_payloads;
        self.projections = staged_projections;
        self.resource_lane_authority = staged_resource_lane_authority;
        Ok(CommitOutcome::Appended(batch))
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<KernelEventEnvelope> {
        self.stream_events(run_id)
    }

    fn expected_next_seq(&self, run_id: &RunId) -> StreamSeq {
        let Some(batches) = self.streams.get(run_id) else {
            return StreamSeq::FIRST;
        };
        batches
            .last()
            .map(|batch| batch.seq.checked_next().unwrap_or(StreamSeq(u64::MAX)))
            .unwrap_or(StreamSeq::FIRST)
    }
}

/// Non-durable async wrapper around a private in-memory run store core.
///
fn admit_artifact_evidence(
    artifacts: &mut ArtifactAuthorityMap,
    admitted_artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    for evidence in admitted_artifacts {
        let key = artifact_authority_key(evidence)?;
        if let Some(existing) = artifacts.get(&key) {
            if existing != evidence {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: evidence.artifact_id.clone(),
                    field: "artifact",
                });
            }
            continue;
        }
        artifacts.insert(key, evidence.clone());
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
fn verify_existing_artifact_admissions(
    artifacts: &ArtifactAuthorityMap,
    bundle: &PreparedCommitBundle,
) -> Result<()> {
    for existing in bundle.existing_artifacts() {
        let evidence =
            admitted_artifact_evidence(bundle, existing.artifact_id(), existing.evidence_hash())?;
        let key = artifact_authority_key(evidence)?;
        match artifacts.get(&key) {
            Some(stored) if stored == evidence => {}
            Some(_) => {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: existing.artifact_id().clone(),
                    field: "artifact",
                });
            }
            None => {
                return Err(StoreError::MissingArtifact {
                    artifact_id: existing.artifact_id().clone(),
                });
            }
        }
    }
    Ok(())
}

fn admitted_artifact_evidence<'a>(
    bundle: &'a PreparedCommitBundle,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<&'a ArtifactEvidenceRef> {
    for evidence in bundle.admitted_artifacts() {
        if &evidence.artifact_id == artifact_id && evidence.evidence_hash()? == *evidence_hash {
            return Ok(evidence);
        }
    }
    Err(StoreError::MissingPreparedArtifactBytes {
        artifact_id: artifact_id.clone(),
    })
}

/// Returns artifact-byte projection authority after applying a prepared bundle.
pub fn artifact_byte_authority_for_bundle(
    existing: &ArtifactByteAuthorityMap,
    bundle: &PreparedCommitBundle,
) -> Result<ArtifactByteAuthorityMap> {
    let mut authority = existing.clone();
    for artifact in bundle.artifact_bytes() {
        let verified =
            PreparedArtifactBytes::new(artifact.bytes().to_vec(), artifact.evidence().clone())?;
        let (bytes, evidence, evidence_hash) = verified.into_parts();
        let key = (evidence.artifact_id.clone(), evidence_hash);
        match authority.get(&key) {
            Some((stored_bytes, stored_evidence))
                if stored_bytes == &bytes && stored_evidence == &evidence => {}
            Some((_, stored_evidence)) => {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: stored_evidence.artifact_id.clone(),
                    field: "artifact",
                });
            }
            None => {
                authority.insert(key, (bytes, evidence));
            }
        }
    }
    Ok(authority)
}

fn artifact_authority_key(evidence: &ArtifactEvidenceRef) -> Result<ArtifactAuthorityKey> {
    Ok((evidence.artifact_id.clone(), evidence.evidence_hash()?))
}

fn compare_artifact_field<T: PartialEq>(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: T,
    expected: T,
) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact_id.clone(),
            field,
        })
    }
}

fn compare_artifact_option(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: Option<&str>,
    expected: Option<&str>,
) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact_id.clone(),
            field,
        })
    }
}

fn side_effect_precondition_matches(
    projection: Option<&SideEffectProjection>,
    required: RequiredSideEffectState,
) -> Result<bool> {
    let state = projection
        .map(SideEffectProjection::ledger_state)
        .transpose()?;
    match required {
        RequiredSideEffectState::Absent => Ok(state.is_none()),
        RequiredSideEffectState::IntentPersisted => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::IntentPersisted { .. })
        )),
        RequiredSideEffectState::Claimed => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Claimed { .. })
        )),
        RequiredSideEffectState::InvocationPrepared => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Prepared { .. })
        )),
        RequiredSideEffectState::InvocationStarted => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Started { .. })
        )),
        RequiredSideEffectState::SubmissionResult => Ok(matches!(
            state.map(|state| state.phase()),
            Some(
                SideEffectLedgerPhase::SubmissionKnown { .. }
                    | SideEffectLedgerPhase::Ambiguous { .. }
            )
        )),
        RequiredSideEffectState::ReceiptObserved => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::ReceiptObserved { .. })
        )),
        RequiredSideEffectState::ConfirmationObserved => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Confirmed { .. })
        )),
        RequiredSideEffectState::Ambiguous => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Ambiguous { .. })
        )),
        RequiredSideEffectState::Failed => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Failed { .. })
        )),
    }
}

use self::admission::require_admission_preconditions;

mod admission;

mod private {
    pub trait Sealed {}
}

fn validate_unique_artifact_evidence(
    field: &'static str,
    artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    let mut by_artifact = BTreeMap::<ArtifactAuthorityKey, &ArtifactEvidenceRef>::new();
    for artifact in artifacts {
        let key = artifact_authority_key(artifact)?;
        if let Some(existing) = by_artifact.insert(key, artifact) {
            if existing != artifact {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: artifact.artifact_id.clone(),
                    field,
                });
            }
        }
    }
    Ok(())
}

fn validate_required_artifacts_cover_payload_references(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    for payload in &request.payloads {
        for requirement in event_artifact_requirements(payload) {
            let Some(evidence) = request.required_artifacts.iter().find(|evidence| {
                evidence.artifact_id == requirement.artifact_id
                    && evidence.evidence_hash().ok().as_ref() == Some(&requirement.evidence_hash)
            }) else {
                return Err(invalid_prepared_commit_purpose(
                    purpose,
                    format!(
                        "missing required artifact evidence for {}",
                        requirement.artifact_id
                    ),
                ));
            };
            validate_required_artifact_requirement(purpose, &requirement, evidence)?;
        }
    }
    Ok(())
}

fn reject_store_materialized_resource_lane_payloads(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    if request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::ResourceLaneClaimed(_)
                | KernelEventPayload::ResourceLaneReleased(_)
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            purpose,
            "prepared commits must use resource-lane intents, not store-filled lane events",
        ));
    }
    Ok(())
}

fn validate_required_artifact_requirement(
    purpose: &'static str,
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    validate_artifact_requirement_against_evidence(requirement, evidence).map_err(|error| {
        if let StoreError::ArtifactEvidenceMismatch { field, .. } = error {
            return invalid_prepared_commit_purpose(
                purpose,
                format!(
                    "required artifact {} field {field} does not satisfy payload reference",
                    requirement.artifact_id
                ),
            );
        }
        error
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactRequirementValidationMode {
    Strict,
    RetentionMetadata,
}

fn artifact_evidence_mismatch(artifact_id: &ArtifactId, field: &'static str) -> StoreError {
    StoreError::ArtifactEvidenceMismatch {
        artifact_id: artifact_id.clone(),
        field,
    }
}

fn require_artifact_option_present(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: Option<&str>,
) -> Result<()> {
    if actual.is_some() {
        Ok(())
    } else {
        Err(artifact_evidence_mismatch(artifact_id, field))
    }
}

fn require_artifact_option_absent(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: Option<&str>,
) -> Result<()> {
    if actual.is_none() {
        Ok(())
    } else {
        Err(artifact_evidence_mismatch(artifact_id, field))
    }
}

fn validate_artifact_requirement_exact_fields(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if let Some(schema_id) = &requirement.schema_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "schema_id",
            evidence.schema_id.as_ref().map(SchemaId::as_str),
            Some(schema_id.as_str()),
        )?;
    }
    if let Some(semantic_type_id) = &requirement.semantic_type_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "semantic_type_id",
            evidence
                .semantic_type_id
                .as_ref()
                .map(SemanticTypeId::as_str),
            Some(semantic_type_id.as_str()),
        )?;
    }
    if let Some(producer_node_id) = &requirement.producer_node_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_node_id",
            evidence.producer_node_id.as_ref().map(NodeId::as_str),
            Some(producer_node_id.as_str()),
        )?;
    }
    if let Some(producer_seed_id) = &requirement.producer_seed_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_seed_id",
            evidence.producer_seed_id.as_ref().map(SeedId::as_str),
            Some(producer_seed_id.as_str()),
        )?;
    }
    Ok(())
}

fn validate_artifact_schema_policy(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    policy: events::ArtifactSchemaPolicy,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let actual = evidence.schema_id.as_ref().map(SchemaId::as_str);
    match policy {
        events::ArtifactSchemaPolicy::ExactSeedSchema
        | events::ArtifactSchemaPolicy::ExactValueSchema
        | events::ArtifactSchemaPolicy::ExactEvidenceSchema
        | events::ArtifactSchemaPolicy::ExactFactDescriptorSchema
        | events::ArtifactSchemaPolicy::ExactFactQueryEvidenceSchema
        | events::ArtifactSchemaPolicy::ExactPublicSchema
        | events::ArtifactSchemaPolicy::ExactDiagnosticSchema => {
            if let Some(schema_id) = &requirement.schema_id {
                compare_artifact_option(
                    &requirement.artifact_id,
                    "schema_id",
                    actual,
                    Some(schema_id.as_str()),
                )?;
            } else if mode == ArtifactRequirementValidationMode::RetentionMetadata {
                require_artifact_option_present(&requirement.artifact_id, "schema_id", actual)?;
            } else {
                return Err(artifact_evidence_mismatch(
                    &requirement.artifact_id,
                    "schema_id",
                ));
            }
        }
        events::ArtifactSchemaPolicy::Absent => {
            if requirement.schema_id.is_some() {
                return Err(artifact_evidence_mismatch(
                    &requirement.artifact_id,
                    "schema_id",
                ));
            }
            require_artifact_option_absent(&requirement.artifact_id, "schema_id", actual)?;
        }
    }
    Ok(())
}

fn validate_artifact_semantic_policy(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    policy: events::ArtifactSemanticPolicy,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let actual = evidence
        .semantic_type_id
        .as_ref()
        .map(SemanticTypeId::as_str);
    match policy {
        events::ArtifactSemanticPolicy::ExactSeedSemantic
        | events::ArtifactSemanticPolicy::ExactValueSemantic => {
            if let Some(semantic_type_id) = &requirement.semantic_type_id {
                compare_artifact_option(
                    &requirement.artifact_id,
                    "semantic_type_id",
                    actual,
                    Some(semantic_type_id.as_str()),
                )?;
            } else if mode == ArtifactRequirementValidationMode::RetentionMetadata {
                require_artifact_option_present(
                    &requirement.artifact_id,
                    "semantic_type_id",
                    actual,
                )?;
            } else {
                return Err(artifact_evidence_mismatch(
                    &requirement.artifact_id,
                    "semantic_type_id",
                ));
            }
        }
        events::ArtifactSemanticPolicy::Absent => {
            if requirement.semantic_type_id.is_some() {
                return Err(artifact_evidence_mismatch(
                    &requirement.artifact_id,
                    "semantic_type_id",
                ));
            }
            require_artifact_option_absent(&requirement.artifact_id, "semantic_type_id", actual)?;
        }
    }
    Ok(())
}

fn require_producer_node_absent(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if requirement.producer_node_id.is_some() {
        return Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_node_id",
        ));
    }
    require_artifact_option_absent(
        &requirement.artifact_id,
        "producer_node_id",
        evidence.producer_node_id.as_ref().map(NodeId::as_str),
    )
}

fn require_producer_seed_absent(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if requirement.producer_seed_id.is_some() {
        return Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_seed_id",
        ));
    }
    require_artifact_option_absent(
        &requirement.artifact_id,
        "producer_seed_id",
        evidence.producer_seed_id.as_ref().map(SeedId::as_str),
    )
}

fn require_producer_node_exact_or_present(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let actual = evidence.producer_node_id.as_ref().map(NodeId::as_str);
    if let Some(producer_node_id) = &requirement.producer_node_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_node_id",
            actual,
            Some(producer_node_id.as_str()),
        )
    } else if mode == ArtifactRequirementValidationMode::RetentionMetadata {
        require_artifact_option_present(&requirement.artifact_id, "producer_node_id", actual)
    } else {
        Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_node_id",
        ))
    }
}

fn require_producer_seed_exact_or_present(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let actual = evidence.producer_seed_id.as_ref().map(SeedId::as_str);
    if let Some(producer_seed_id) = &requirement.producer_seed_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_seed_id",
            actual,
            Some(producer_seed_id.as_str()),
        )
    } else if mode == ArtifactRequirementValidationMode::RetentionMetadata {
        require_artifact_option_present(&requirement.artifact_id, "producer_seed_id", actual)
    } else {
        Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_seed_id",
        ))
    }
}

fn validate_optional_producer_node_no_seed(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    require_producer_seed_absent(requirement, evidence)?;
    if let Some(producer_node_id) = &requirement.producer_node_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_node_id",
            evidence.producer_node_id.as_ref().map(NodeId::as_str),
            Some(producer_node_id.as_str()),
        )?;
    }
    Ok(())
}

fn validate_artifact_producer_policy(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    policy: events::ArtifactProducerScope,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    match policy {
        events::ArtifactProducerScope::LaunchOrGlobalNoSeed
        | events::ArtifactProducerScope::DiagnosticOptionalNodeNoSeed => {
            validate_optional_producer_node_no_seed(requirement, evidence)?;
        }
        events::ArtifactProducerScope::SeedRequired => {
            require_producer_node_absent(requirement, evidence)?;
            require_producer_seed_exact_or_present(requirement, evidence, mode)?;
        }
        events::ArtifactProducerScope::NodeRequired => {
            require_producer_seed_absent(requirement, evidence)?;
            require_producer_node_exact_or_present(requirement, evidence, mode)?;
        }
        events::ArtifactProducerScope::GlobalNoSeed
        | events::ArtifactProducerScope::MiddlewareNoSeed => {
            require_producer_node_absent(requirement, evidence)?;
            require_producer_seed_absent(requirement, evidence)?;
        }
    }
    Ok(())
}

fn validate_artifact_role_contract(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    role: ArtifactRole,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let contract = role.contract();
    validate_artifact_schema_policy(requirement, evidence, contract.schema, mode)?;
    validate_artifact_semantic_policy(requirement, evidence, contract.semantic, mode)?;
    validate_artifact_producer_policy(requirement, evidence, contract.producer, mode)
}

/// Validates a typed event artifact requirement against retained artifact evidence.
///
/// This applies the closed [`events::ArtifactRole`] contract for role-bearing requirements and
/// exact carried-field matching for schema-only requirements.
pub fn validate_artifact_requirement_against_evidence(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    let mode = if requirement.source.is_retention() {
        ArtifactRequirementValidationMode::RetentionMetadata
    } else {
        ArtifactRequirementValidationMode::Strict
    };
    compare_artifact_field(
        &requirement.artifact_id,
        "artifact_id",
        evidence.artifact_id.as_str(),
        requirement.artifact_id.as_str(),
    )?;
    compare_artifact_field(
        &requirement.artifact_id,
        "evidence_hash",
        evidence.evidence_hash()?.as_str(),
        requirement.evidence_hash.as_str(),
    )?;
    if let Some(digest) = &requirement.digest {
        compare_artifact_field(
            &requirement.artifact_id,
            "digest",
            evidence.digest.as_str(),
            digest.as_str(),
        )?;
    }
    if let Some(byte_len) = requirement.byte_len {
        compare_artifact_field(
            &requirement.artifact_id,
            "byte_len",
            evidence.byte_len,
            byte_len,
        )?;
    }
    if let Some(media_type) = &requirement.media_type {
        compare_artifact_field(
            &requirement.artifact_id,
            "media_type",
            evidence.media_type.as_str(),
            media_type.as_str(),
        )?;
    }
    if let Some(role) = requirement.artifact_role {
        compare_artifact_field(
            &requirement.artifact_id,
            "artifact_role",
            evidence.artifact_role.as_str(),
            role.as_str(),
        )?;
        validate_artifact_role_contract(requirement, evidence, role, mode)?;
    } else {
        validate_artifact_requirement_exact_fields(requirement, evidence)?;
    }
    Ok(())
}

pub(super) fn verify_retained_artifact_bytes(
    bytes: &[u8],
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.digest != digest
        || evidence.artifact_id != artifact_id
        || evidence.byte_len != bytes.len() as u64
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "bytes",
        });
    }
    Ok(())
}

fn validate_payload_public_diagnostics(payloads: &[KernelEventPayload]) -> Result<()> {
    for payload in payloads {
        match payload {
            KernelEventPayload::PublicOutputRenderFailed(payload) => payload.error.validate()?,
            KernelEventPayload::StateAttemptFailed(payload) => payload.error.validate()?,
            KernelEventPayload::SideEffectFailed(payload) => payload.error.validate()?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_run_start_commit(request: &CommitRequest) -> Result<()> {
    if request.payloads.len() != 1
        || !matches!(
            request.payloads.first(),
            Some(KernelEventPayload::RunAdmitted(_))
        )
    {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "run-admission commits must contain exactly one RunAdmitted payload",
        ));
    }
    let Some(KernelEventPayload::RunAdmitted(payload)) = request.payloads.first() else {
        unreachable!("run-admission payload shape was checked above");
    };
    let authority = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                RunAdmission::NAME,
                "run admission requires certified run store authority",
            )
        })?;
    if authority.run_id() != request.run_id() || authority.spec_hash() != &payload.spec_hash {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "certified run store authority does not match RunAdmitted",
        ));
    }
    validate_run_admitted_identity_for_request(request.run_id(), payload)?;
    if request.preconditions.required_run_state != RequiredRunState::Absent {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "run admission requires absent-run precondition",
        ));
    }
    Ok(())
}

fn validate_run_admitted_identity_for_request(
    run_id: &RunId,
    payload: &events::RunAdmitted,
) -> Result<()> {
    if payload.run_id != *run_id {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted run id does not match commit run id",
        ));
    }
    if payload.identity_material.certified_spec_hash != payload.spec_hash {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted identity material spec hash does not match event spec hash",
        ));
    }
    let derived = payload.identity_material.derive_run_id().map_err(|_| {
        invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted identity material is invalid",
        )
    })?;
    if derived != payload.run_id {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted run id does not match identity material",
        ));
    }
    Ok(())
}

fn validate_state_attempt_started_commit(request: &CommitRequest) -> Result<()> {
    if request.payloads.len() != 1
        || !matches!(
            request.payloads.first(),
            Some(KernelEventPayload::StateAttemptStarted(_))
        )
    {
        return Err(invalid_prepared_commit_purpose(
            StateAttemptStarted::NAME,
            "state-attempt-start commits must contain exactly one StateAttemptStarted payload",
        ));
    }
    if request.preconditions.required_run_state != RequiredRunState::NotCompleted {
        return Err(invalid_prepared_commit_purpose(
            StateAttemptStarted::NAME,
            "state-attempt-start requires not-completed run precondition",
        ));
    }
    Ok(())
}

fn validate_attempt_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        AttemptTerminal::NAME,
        request,
        is_attempt_terminal_commit_payload,
        "attempt-terminal commits cannot contain non-attempt-terminal payloads",
    )?;
    require_purpose_payload(
        AttemptTerminal::NAME,
        request,
        is_attempt_terminal_payload,
        "missing attempt-terminal payload",
    )?;
    validate_attempt_terminal_resource_lane_release_batch(request)?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

fn validate_side_effect_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SideEffectTerminal::NAME,
        request,
        is_side_effect_terminal_commit_payload,
        "side-effect terminal commits cannot contain non-side-effect-terminal payloads",
    )?;
    require_purpose_payload(
        SideEffectTerminal::NAME,
        request,
        is_side_effect_terminal_disposition_payload,
        "missing side-effect terminal payload",
    )?;
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SideEffectTerminal::NAME,
            "side-effect terminal commits require certified run authority",
        ));
    }
    validate_side_effect_terminal_resource_lane_release_batch(request)?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

fn validate_side_effect_progress_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SideEffectProgress::NAME,
        request,
        is_side_effect_progress_commit_payload,
        "side-effect progress commits cannot contain non-side-effect-progress payloads",
    )?;
    require_purpose_payload(
        SideEffectProgress::NAME,
        request,
        is_side_effect_payload,
        "missing side-effect payload",
    )?;
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SideEffectProgress::NAME,
            "side-effect progress commits require certified run authority",
        ));
    }
    Ok(())
}

fn validate_retention_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        Retention::NAME,
        request,
        is_retention_commit_payload,
        "retention commits cannot contain non-retention payloads",
    )?;
    require_purpose_payload(
        Retention::NAME,
        request,
        is_retention_payload,
        "missing retention payload",
    )?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

fn validate_manual_resolution_commit(request: &CommitRequest) -> Result<()> {
    let manual_resolution_count = request
        .payloads
        .iter()
        .filter(|payload| matches!(payload, KernelEventPayload::ManualResolutionRecorded(_)))
        .count();
    if manual_resolution_count != 1 {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution commits must contain exactly one ManualResolutionRecorded payload",
        ));
    }
    if !matches!(
        request.payloads.last(),
        Some(KernelEventPayload::ManualResolutionRecorded(_))
    ) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution release intents must precede ManualResolutionRecorded",
        ));
    }
    if request.payloads.iter().any(|payload| {
        !matches!(
            payload,
            KernelEventPayload::ManualResolutionRecorded(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_)
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution commits may only contain resource lane release intents and ManualResolutionRecorded",
        ));
    }
    if request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
                release_authority,
                ..
            }) if *release_authority != events::ResourceLaneReleaseAuthority::ManualResolution
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution release intents require manual resolution release authority",
        ));
    }
    if request.preconditions.required_run_state != RequiredRunState::NotCompleted {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires not-completed run precondition",
        ));
    }
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires certified run authority",
        ));
    }
    Ok(())
}

fn validate_manual_resolution_commit_with_proof(
    request: &CommitRequest,
    proof: &VerifiedManualResolutionForPrefix,
) -> Result<()> {
    validate_manual_resolution_commit(request)?;
    let manual_resolutions = request
        .payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if manual_resolutions.len() != 1 {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires exactly one ManualResolutionRecorded payload",
        ));
    }
    let payload = manual_resolutions[0];
    let token = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "manual resolution requires certified run authority",
            )
        })?;
    let prefix = proof.prefix();
    if prefix.run_id() != request.run_id()
        || prefix.run_id() != &payload.run_id
        || prefix.run_id() != token.run_id()
    {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof run id does not match manual resolution request",
        ));
    }
    if prefix.spec_hash() != &payload.spec_hash || prefix.spec_hash() != token.spec_hash() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof spec hash does not match manual resolution request",
        ));
    }
    if prefix.expected_next_seq() != request.expected_next_seq().as_u64() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof prefix expected_next_seq does not match manual resolution request",
        ));
    }
    if proof.outcome() != payload.outcome {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "ManualResolutionRecorded outcome does not match manual proof",
        ));
    }
    if proof.evidence().schema_id != payload.evidence_schema_id
        || proof.evidence().content_hash != payload.evidence_hash
        || proof.evidence().artifact_id != payload.evidence_artifact_id
        || proof.authorization().schema_id != payload.authorization_schema_id
        || proof.authorization().content_hash != payload.authorization_hash
        || proof.authorization().artifact_id != payload.authorization_artifact_id
    {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "ManualResolutionRecorded artifact refs do not match manual proof",
        ));
    }
    let block_reason = manual_block_reason_from_auth(prefix.manual_block_reason());
    let certified_manual_policy = manual_policy_for_block_reason(token.saga_policy(), block_reason)
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "certified run authority policy does not permit manual proof block reason",
            )
        })?;
    if certified_manual_policy != prefix.manual_policy() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof policy does not match certified run authority",
        ));
    }
    Ok(())
}

fn validate_saga_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SagaTerminal::NAME,
        request,
        is_saga_terminal_commit_payload,
        "saga terminal commits cannot contain non-terminal payloads",
    )?;
    if request
        .payloads
        .iter()
        .filter(|payload| is_run_completed_payload(payload))
        .count()
        != 1
    {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution commits must contain exactly one RunCompleted payload",
        ));
    }
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution requires certified run authority",
        ));
    }
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

fn validate_saga_terminal_commit_with_proof(
    request: &CommitRequest,
    proof: &SagaTerminalProof,
) -> Result<()> {
    validate_saga_terminal_commit(request)?;
    let completed = request
        .payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if completed.len() != 1 {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution requires exactly one RunCompleted payload",
        ));
    }
    let payload = completed[0];
    let token = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "saga terminal resolution requires certified run authority",
            )
        })?;
    if proof.run_id() != request.run_id()
        || proof.run_id() != &payload.run_id
        || proof.run_id() != token.run_id()
    {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof run id does not match terminal request",
        ));
    }
    if proof.prefix_next_seq() != request.expected_next_seq() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof prefix does not match terminal request expected next sequence",
        ));
    }
    if proof.saga_policy_digest() != token.saga_policy_digest() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof saga policy digest does not match certified run authority",
        ));
    }
    let proof_outcome = proof.outcome();
    if payload.outcome != proof_outcome {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "RunCompleted outcome does not match SagaTerminalProof",
        ));
    }
    if matches!(payload.outcome, events::RunCompletionOutcome::Completed(_)) {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "completed public-output terminal must use CompleteRun authority",
        ));
    }
    if let Some(spec_hash) = proof.manual_spec_hash() {
        if spec_hash != &payload.spec_hash {
            return Err(invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "manual proof spec hash does not match RunCompleted payload",
            ));
        }
    }
    Ok(())
}

fn require_purpose_payload(
    purpose: &'static str,
    request: &CommitRequest,
    predicate: impl Fn(&KernelEventPayload) -> bool,
    message: &'static str,
) -> Result<()> {
    if request.payloads.iter().any(predicate) {
        Ok(())
    } else {
        Err(invalid_prepared_commit_purpose(purpose, message))
    }
}

fn reject_wrong_purpose_payloads(
    purpose: &'static str,
    request: &CommitRequest,
    predicate: impl Fn(&KernelEventPayload) -> bool,
    message: &'static str,
) -> Result<()> {
    if let Some(payload) = request.payloads.iter().find(|payload| !predicate(payload)) {
        Err(invalid_prepared_commit_purpose(
            purpose,
            format!("{message}: {:?}", payload.event_schema_id()),
        ))
    } else {
        Ok(())
    }
}

fn validate_attempt_terminal_resource_lane_release_batch(request: &CommitRequest) -> Result<()> {
    reject_terminal_resource_lane_claims(AttemptTerminal::NAME, request)?;
    for (release_index, release) in request
        .payloads
        .iter()
        .enumerate()
        .filter(|(_, payload)| is_resource_lane_release_payload(payload))
    {
        let Some(release_ref) = release.resource_lane_authority_ref() else {
            continue;
        };
        let matched_terminal =
            request
                .payloads
                .iter()
                .enumerate()
                .any(|(terminal_index, terminal)| {
                    terminal_index > release_index
                        && terminal_payload_matches_resource_lane_release(
                            terminal,
                            &release_ref,
                            TerminalReleaseMatchKind::AttemptTerminal,
                            None,
                        )
                });
        if !matched_terminal {
            return Err(invalid_prepared_commit_purpose(
                AttemptTerminal::NAME,
                "resource lane release must precede a matching terminal attempt payload",
            ));
        }
    }
    Ok(())
}

fn validate_side_effect_terminal_resource_lane_release_batch(
    request: &CommitRequest,
) -> Result<()> {
    reject_terminal_resource_lane_claims(SideEffectTerminal::NAME, request)?;
    let authority = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                SideEffectTerminal::NAME,
                "side-effect terminal resource-lane release requires certified run authority",
            )
        })?;
    for (release_index, release) in request
        .payloads
        .iter()
        .enumerate()
        .filter(|(_, payload)| is_resource_lane_release_payload(payload))
    {
        let Some(release_ref) = release.resource_lane_authority_ref() else {
            continue;
        };
        let matched_terminal =
            request
                .payloads
                .iter()
                .enumerate()
                .any(|(terminal_index, terminal)| {
                    terminal_index > release_index
                        && terminal_payload_matches_resource_lane_release(
                            terminal,
                            &release_ref,
                            TerminalReleaseMatchKind::SideEffectTerminal,
                            Some(authority),
                        )
                });
        if !matched_terminal {
            return Err(invalid_prepared_commit_purpose(
                SideEffectTerminal::NAME,
                "resource lane release must precede a matching side-effect terminal payload",
            ));
        }
    }
    Ok(())
}

fn reject_terminal_resource_lane_claims(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    if request.payloads.iter().any(is_resource_lane_claim_payload) {
        return Err(invalid_prepared_commit_purpose(
            purpose,
            "terminal commits cannot acquire resource lanes",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalReleaseMatchKind {
    AttemptTerminal,
    SideEffectTerminal,
}

fn terminal_payload_matches_resource_lane_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
    kind: TerminalReleaseMatchKind,
    authority: Option<&CertifiedRunStoreAuthority>,
) -> bool {
    match kind {
        TerminalReleaseMatchKind::AttemptTerminal => {
            attempt_terminal_payload_matches_release(terminal, release)
        }
        TerminalReleaseMatchKind::SideEffectTerminal => {
            let Some(authority) = authority else {
                return false;
            };
            side_effect_terminal_payload_matches_release(terminal, release, authority)
        }
    }
}

fn attempt_terminal_payload_matches_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
) -> bool {
    let Some(emitter) = release.emitter else {
        return false;
    };
    match terminal {
        KernelEventPayload::StateAttemptCompleted(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        _ => false,
    }
}

fn side_effect_terminal_payload_matches_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
    authority: &CertifiedRunStoreAuthority,
) -> bool {
    if release.release_authority != Some(events::ResourceLaneReleaseAuthority::VerifyTerminal) {
        return false;
    }
    if !is_side_effect_terminal_disposition_payload(terminal) {
        return false;
    }
    let Some(terminal) = terminal.side_effect_ledger_ref() else {
        return false;
    };
    terminal.ledger_key == release.ledger.ledger_key
        && terminal.ledger_purpose == release.ledger.ledger_purpose
        && terminal.pair_id == release.ledger.pair_id
        && terminal.invocation_epoch == release.ledger.invocation_epoch
        && side_effect_terminal_release_role_allowed(
            terminal.kind,
            terminal.pair_role,
            release.ledger.pair_role,
        )
        && side_effect_terminal_release_policy_allowed(terminal.kind, terminal.pair_id, authority)
}

fn side_effect_terminal_release_policy_allowed(
    terminal_kind: events::SideEffectEventKind,
    pair_id: &SideEffectPairId,
    authority: &CertifiedRunStoreAuthority,
) -> bool {
    let Ok(pair) = authority.side_effect_pair(pair_id) else {
        return false;
    };
    match terminal_kind {
        events::SideEffectEventKind::ReceiptObserved => {
            pair.terminal_policy == SideEffectTerminalPolicy::Receipt
        }
        events::SideEffectEventKind::ConfirmationObserved => true,
        events::SideEffectEventKind::NotSubmittedProven | events::SideEffectEventKind::Failed => {
            true
        }
        _ => false,
    }
}

fn side_effect_terminal_release_role_allowed(
    terminal_kind: events::SideEffectEventKind,
    terminal_role: events::SideEffectPairRole,
    release_role: events::SideEffectPairRole,
) -> bool {
    if release_role != events::SideEffectPairRole::Verify {
        return false;
    }
    match terminal_kind {
        events::SideEffectEventKind::NotSubmittedProven => {
            matches!(
                terminal_role,
                events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
            )
        }
        events::SideEffectEventKind::ReceiptObserved
        | events::SideEffectEventKind::ConfirmationObserved => {
            terminal_role == events::SideEffectPairRole::Verify
        }
        events::SideEffectEventKind::Failed => matches!(
            terminal_role,
            events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
        ),
        _ => false,
    }
}

fn is_attempt_terminal_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::StateAttemptCompleted(_)
            | KernelEventPayload::StateAttemptInterrupted(_)
            | KernelEventPayload::StateAttemptFailed(_)
            | KernelEventPayload::CellProduced(_)
            | KernelEventPayload::CellSkipped(_)
            | KernelEventPayload::FactRecorded(_)
            | KernelEventPayload::ArtifactReferenced(_)
            | KernelEventPayload::PublicOutputProduced(_)
            | KernelEventPayload::PublicOutputRenderFailed(_)
            | KernelEventPayload::RunCompleted(_)
    )
}

fn is_attempt_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_attempt_terminal_payload(payload)
        || matches!(
            payload,
            KernelEventPayload::ResourceLaneReleased(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_)
        )
        || is_retention_ref_payload(payload)
}

fn is_side_effect_terminal_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::SideEffectNotSubmittedProven(_)
            | KernelEventPayload::SideEffectSubmissionObserved(_)
            | KernelEventPayload::SideEffectSubmissionUnknown(_)
            | KernelEventPayload::SideEffectReceiptObserved(_)
            | KernelEventPayload::SideEffectConfirmationObserved(_)
            | KernelEventPayload::SideEffectAmbiguous(_)
            | KernelEventPayload::SideEffectFailed(_)
            | KernelEventPayload::ResourceLaneReleased(_)
            | KernelEventPayload::ResourceLaneReleaseIntent(_)
    )
}

fn is_side_effect_terminal_disposition_payload(payload: &KernelEventPayload) -> bool {
    is_side_effect_terminal_payload(payload) && !is_resource_lane_release_payload(payload)
}

fn is_side_effect_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_side_effect_payload(payload)
        || is_attempt_terminal_payload(payload)
        || is_retention_ref_payload(payload)
}

fn is_side_effect_progress_commit_payload(payload: &KernelEventPayload) -> bool {
    (is_side_effect_payload(payload) && !is_side_effect_terminal_payload(payload))
        || is_retention_ref_payload(payload)
}

fn is_side_effect_payload(payload: &KernelEventPayload) -> bool {
    payload.side_effect_ledger_ref().is_some()
}

fn is_resource_lane_claim_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::ResourceLaneClaimed(_) | KernelEventPayload::ResourceLaneClaimIntent(_)
    )
}

fn is_resource_lane_release_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::ResourceLaneReleased(_)
            | KernelEventPayload::ResourceLaneReleaseIntent(_)
    )
}

fn is_retention_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RetentionRefsAppended(_)
            | KernelEventPayload::RetentionManifestProjected(_)
    )
}

fn is_retention_ref_payload(payload: &KernelEventPayload) -> bool {
    matches!(payload, KernelEventPayload::RetentionRefsAppended(_))
}

fn is_run_completed_payload(payload: &KernelEventPayload) -> bool {
    matches!(payload, KernelEventPayload::RunCompleted(_))
}

fn is_completed_run_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RunCompleted(events::RunCompleted {
            outcome: events::RunCompletionOutcome::Completed(_),
            ..
        })
    )
}

fn is_non_run_completed_attempt_terminal_payload(payload: &KernelEventPayload) -> bool {
    is_attempt_terminal_payload(payload) && !is_run_completed_payload(payload)
}

fn is_retention_commit_payload(payload: &KernelEventPayload) -> bool {
    is_retention_payload(payload)
        || is_non_run_completed_attempt_terminal_payload(payload)
        || is_completed_run_payload(payload)
}

fn is_saga_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_run_completed_payload(payload)
        || is_non_run_completed_attempt_terminal_payload(payload)
        || is_retention_ref_payload(payload)
}

fn request_contains_saga_terminal_outcome(request: &CommitRequest) -> bool {
    request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::RunCompleted(events::RunCompleted {
                outcome: events::RunCompletionOutcome::Compensated
                    | events::RunCompletionOutcome::ManuallyResolved
                    | events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                ..
            })
        )
    })
}

fn request_contains_manual_resolution(request: &CommitRequest) -> bool {
    request
        .payloads
        .iter()
        .any(|payload| matches!(payload, KernelEventPayload::ManualResolutionRecorded(_)))
}

fn invalid_prepared_commit_purpose(
    purpose: &'static str,
    message: impl Into<String>,
) -> StoreError {
    StoreError::InvalidPreparedCommitPurpose {
        purpose,
        message: message.into(),
    }
}

fn validate_payload_run_and_spec(run_id: &RunId, payloads: &[KernelEventPayload]) -> Result<()> {
    let Some(first) = payloads.first() else {
        return Err(StoreError::EmptyCommit);
    };
    let expected_spec_hash = payload_spec_hash(first);
    for payload in payloads {
        if let Some(payload_run_id) = payload_run_id(payload) {
            if payload_run_id != *run_id {
                return Err(StoreError::PayloadRunMismatch {
                    expected: Box::new(run_id.clone()),
                    actual: Box::new(payload_run_id),
                });
            }
        }
        let actual_spec_hash = payload_spec_hash(payload);
        if actual_spec_hash != expected_spec_hash {
            return Err(StoreError::PayloadSpecHashMismatch {
                expected: Box::new(expected_spec_hash),
                actual: Box::new(actual_spec_hash),
            });
        }
    }
    Ok(())
}

fn validate_terminal_attempt_cell_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    let mut public_outputs = BTreeSet::new();

    for payload in payloads {
        match payload {
            KernelEventPayload::StateAttemptCompleted(payload) => {
                completions.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            KernelEventPayload::CellProduced(payload) => {
                terminal_cells.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            KernelEventPayload::CellSkipped(payload) => {
                terminal_cells.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            KernelEventPayload::PublicOutputProduced(payload) => {
                public_outputs.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.receipt_cell_id.clone(),
                ));
            }
            _ => {}
        }
    }

    for (node_id, attempt_id, output_cell_id) in &completions {
        if !terminal_cells.contains(&(node_id.clone(), attempt_id.clone(), output_cell_id.clone()))
        {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message: "attempt completion requires matching terminal cell in same commit"
                    .to_owned(),
            });
        }
    }
    for (node_id, attempt_id, cell_id) in &terminal_cells {
        if !completions.contains(&(node_id.clone(), attempt_id.clone(), cell_id.clone())) {
            return Err(StoreError::ProjectionConflict {
                key: format!("cell:{cell_id}:terminal"),
                message: "terminal cell requires matching attempt completion in same commit"
                    .to_owned(),
            });
        }
    }
    for (node_id, attempt_id, receipt_cell_id) in &public_outputs {
        let terminal = (node_id.clone(), attempt_id.clone(), receipt_cell_id.clone());
        if !terminal_cells.contains(&terminal) || !completions.contains(&terminal) {
            return Err(StoreError::ProjectionConflict {
                key: format!("public_output:{node_id}:{attempt_id}"),
                message: "public output requires matching render receipt terminal in same commit"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_supported_stream_model(events: &[KernelEventEnvelope]) -> Result<()> {
    let mut started_attempts = BTreeMap::<(NodeId, AttemptId), StreamSeq>::new();
    for event in events {
        if let KernelEventPayload::StateAttemptStarted(payload) = event.payload() {
            started_attempts.insert(
                (payload.node_id.clone(), payload.attempt_id.clone()),
                event.seq(),
            );
        }
    }
    for event in events {
        for (node_id, attempt_id) in payload_required_started_attempts(event.payload()) {
            let key = (node_id.clone(), attempt_id.clone());
            let Some(start_seq) = started_attempts.get(&key) else {
                return Err(StoreError::ProjectionConflict {
                    key: format!("stream_model:{node_id}:{attempt_id}"),
                    message: "invalid run stream model: attempt-bound payload is not preceded by a StateAttemptStarted commit".to_owned(),
                });
            };
            if *start_seq >= event.seq() {
                return Err(StoreError::ProjectionConflict {
                    key: format!("stream_model:{node_id}:{attempt_id}"),
                    message: "invalid run stream model: StateAttemptStarted must be committed before attempt-bound terminal payloads".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn payload_required_started_attempts(payload: &KernelEventPayload) -> Vec<(NodeId, AttemptId)> {
    match payload {
        KernelEventPayload::StateAttemptCompleted(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::CellProduced(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::CellSkipped(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::FactRecorded(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::PublicOutputProduced(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        payload => payload
            .side_effect_ref()
            .map(|side_effect| vec![(side_effect.node_id.clone(), side_effect.attempt_id.clone())])
            .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalSideEffectEvidencePair {
    node_id: NodeId,
    attempt_id: AttemptId,
    retryable: bool,
    kind: TerminalSideEffectEvidenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalSideEffectEvidenceKind {
    Ambiguous,
    Failed,
}

impl TerminalSideEffectEvidencePair {
    fn ambiguous(payload: &side_effect::Ambiguous) -> Self {
        Self {
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            retryable: false,
            kind: TerminalSideEffectEvidenceKind::Ambiguous,
        }
    }

    fn failed(payload: &side_effect::Failed) -> Self {
        Self {
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            retryable: payload.retryable,
            kind: TerminalSideEffectEvidenceKind::Failed,
        }
    }

    fn node_attempt_key(&self) -> (NodeId, AttemptId) {
        (self.node_id.clone(), self.attempt_id.clone())
    }

    fn label(&self) -> &'static str {
        match self.kind {
            TerminalSideEffectEvidenceKind::Ambiguous => "ambiguity",
            TerminalSideEffectEvidenceKind::Failed => "failure",
        }
    }
}

fn validate_terminal_side_effect_evidence_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
    let mut side_effect_terminals = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for payload in payloads {
        match payload {
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                insert_terminal_side_effect_pair(
                    &mut side_effect_terminals,
                    TerminalSideEffectEvidencePair::ambiguous(payload),
                )?;
            }
            KernelEventPayload::SideEffectFailed(payload) => {
                insert_terminal_side_effect_pair(
                    &mut side_effect_terminals,
                    TerminalSideEffectEvidencePair::failed(payload),
                )?;
            }
            KernelEventPayload::StateAttemptFailed(payload) => {
                attempt_failures.insert(
                    (payload.node_id.clone(), payload.attempt_id.clone()),
                    payload.retryable,
                );
            }
            _ => {}
        }
    }
    for pair in side_effect_terminals.values() {
        let key = pair.node_attempt_key();
        match attempt_failures.get(&key) {
            Some(attempt_retryable) if *attempt_retryable == pair.retryable => {}
            Some(_) => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "sidefx:{}:{}:{}",
                        pair.node_id,
                        pair.attempt_id,
                        pair.label()
                    ),
                    message: format!(
                        "side-effect {} retryability must match attempt failure",
                        pair.label()
                    ),
                });
            }
            None => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "sidefx:{}:{}:{}",
                        pair.node_id,
                        pair.attempt_id,
                        pair.label()
                    ),
                    message: format!(
                        "side-effect {} requires matching StateAttemptFailed in same commit",
                        pair.label()
                    ),
                });
            }
        }
    }
    Ok(())
}

fn validate_side_effect_attempt_failures_have_terminal_evidence(
    projections: &ProjectionSnapshot,
    payloads: &[KernelEventPayload],
) -> Result<()> {
    let mut current_side_effect_authority = BTreeSet::new();
    let mut terminal_side_effect_evidence = BTreeSet::new();
    let mut attempt_failures = Vec::new();
    for payload in payloads {
        if let Some(side_effect) = payload.side_effect_ref() {
            let key = (side_effect.node_id.clone(), side_effect.attempt_id.clone());
            current_side_effect_authority.insert(key.clone());
            if matches!(
                side_effect.kind,
                events::SideEffectEventKind::Ambiguous | events::SideEffectEventKind::Failed
            ) {
                terminal_side_effect_evidence.insert(key);
            }
        }
        if let KernelEventPayload::StateAttemptFailed(payload) = payload {
            attempt_failures.push((payload.node_id.clone(), payload.attempt_id.clone()));
        }
    }

    for (node_id, attempt_id) in attempt_failures {
        let key = (node_id.clone(), attempt_id.clone());
        if terminal_side_effect_evidence.contains(&key) {
            continue;
        }
        let prior_side_effect = projections.side_effects().find_map(|(_, projection)| {
            if projection.intent.node_id == node_id && projection.intent.attempt_id == attempt_id {
                Some(projection)
            } else {
                None
            }
        });
        if prior_side_effect
            .map(|projection| projection.ledger_state().map(|state| state.is_confirmed()))
            .transpose()?
            .unwrap_or(false)
        {
            continue;
        }
        if prior_side_effect.is_some() || current_side_effect_authority.contains(&key) {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message:
                    "side-effect attempt failure requires terminal side-effect evidence in the same commit"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

fn insert_terminal_side_effect_pair(
    pairs: &mut BTreeMap<(NodeId, AttemptId), TerminalSideEffectEvidencePair>,
    pair: TerminalSideEffectEvidencePair,
) -> Result<()> {
    let key = pair.node_attempt_key();
    if pairs.insert(key.clone(), pair).is_some() {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{}:{}:terminal", key.0, key.1),
            message: "terminal side-effect evidence must be unique per node attempt in one commit"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_retention_manifest_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
    let retained_manifest_refs = payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::RetentionRefsAppended(payload) => Some(&payload.refs),
            _ => None,
        })
        .flatten()
        .filter(|retention_ref| retention_ref.role == ArtifactRole::RetentionManifest)
        .map(|retention_ref| {
            (
                retention_ref.artifact_id.clone(),
                retention_ref.content_digest.clone(),
            )
        })
        .collect::<BTreeSet<_>>();

    for payload in payloads {
        let KernelEventPayload::RetentionManifestProjected(payload) = payload else {
            continue;
        };
        if !retained_manifest_refs.contains(&(
            payload.manifest_artifact_id.clone(),
            payload.manifest_digest.clone(),
        )) {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "retention:{}:manifest:{}",
                    payload.run_id, payload.manifest_seq
                ),
                message:
                    "retention manifest projection requires matching retention ref in same commit"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

fn payload_run_id(payload: &KernelEventPayload) -> Option<RunId> {
    payload.run_id().cloned()
}

fn payload_spec_hash(payload: &KernelEventPayload) -> SpecHash {
    payload.spec_hash().clone()
}

/// Returns canonical JSON bytes for a typed event payload.
pub fn payload_canonical_json(payload: &KernelEventPayload) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(payload_json(payload))
}

fn kernel_event_envelope_json(envelope: &KernelEventEnvelope) -> serde_json::Value {
    serde_json::json!({
        "commit_key": envelope.commit_key().as_str(),
        "event_id": envelope.event_id().as_str(),
        "event_schema_id": envelope.event_schema_id().as_str(),
        "logical_key": envelope.logical_key().as_str(),
        "ordinal": envelope.ordinal().as_u32(),
        "payload": payload_json(envelope.payload()),
        "payload_hash": envelope.payload_hash().as_str(),
        "run_id": envelope.run_id().as_str(),
        "seq": envelope.seq().as_u64(),
        "store_commit_order": envelope.store_commit_order().as_u64(),
        "spec_hash": envelope.spec_hash().as_str(),
    })
}

fn parse_kernel_event_envelope(json: &serde_json::Value) -> Result<KernelEventEnvelope> {
    let seq = StreamSeq::new(required_u64(json, "seq")?)?;
    let store_commit_order = StoreCommitOrder::new(required_u64(json, "store_commit_order")?);
    let ordinal = CommitOrdinal::new(required_u32(json, "ordinal")?);
    let payload = payload_from_json_value(required_obj(json, "payload")?)?;
    KernelEventEnvelope::from_persisted_record(PersistedKernelEventRecord {
        event_id: parse_identity(required_str(json, "event_id")?)?,
        event_schema_id: parse_identity(required_str(json, "event_schema_id")?)?,
        run_id: parse_identity(required_str(json, "run_id")?)?,
        seq,
        store_commit_order,
        ordinal,
        spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
        commit_key: CommitKey::new(required_str(json, "commit_key")?)?,
        logical_key: LogicalEventKey::new(required_str(json, "logical_key")?)?,
        payload_hash: parse_identity(required_str(json, "payload_hash")?)?,
        payload,
    })
}

/// Computes the canonical idempotency fingerprint for a purpose-specific prepared commit plan.
///
/// The fingerprint intentionally excludes `expected_next_seq`, so idempotent retries can be
/// recognized before stale sequence checks as required by the store contract. It covers the
/// artifact evidence admitted atomically with the commit.
pub fn prepared_commit_plan_fingerprint(plan: &PreparedCommitPlan) -> Result<CommitFingerprint> {
    let request = plan.request();
    let canonical = canonical_json(serde_json::json!({
        "admitted_artifacts": sorted_store_artifacts_json(plan.admitted_artifacts()),
        "commit_key": request.commit_key.as_str(),
        "payloads": request.payloads.iter().map(payload_json).collect::<Vec<_>>(),
        "preconditions": preconditions_json(&request.preconditions),
        "required_artifacts": sorted_store_artifacts_json(&request.required_artifacts),
        "run_id": request.run_id.as_str(),
    }))?;
    Ok(CommitFingerprint(canonical.content_digest()))
}

fn sorted_store_artifacts_json(artifacts: &[ArtifactEvidenceRef]) -> Vec<serde_json::Value> {
    let mut artifacts = artifacts
        .iter()
        .map(store_artifact_json)
        .collect::<Vec<_>>();
    artifacts.sort_by_key(serde_json::Value::to_string);
    artifacts
}

fn derive_event_id(
    run_id: &RunId,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    event_schema_id: &SchemaId,
    payload_hash: &ContentDigest,
) -> Result<EventId> {
    let canonical = canonical_json(serde_json::json!({
        "event_schema_id": event_schema_id.as_str(),
        "ordinal": ordinal.as_u32(),
        "payload_hash": payload_hash.as_str(),
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?;
    Ok(EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

fn derive_logical_key(
    run_id: &RunId,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    payload: &KernelEventPayload,
    payload_hash: &ContentDigest,
) -> Result<LogicalEventKey> {
    let key = match payload {
        KernelEventPayload::RunAdmitted(_) => "run:admission".to_owned(),
        KernelEventPayload::ManualResolutionRecorded(_) => "run:manual_resolution".to_owned(),
        KernelEventPayload::RunCompleted(_) => "run:complete".to_owned(),
        KernelEventPayload::StateAttemptStarted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptCompleted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::FactRecorded(_) => {
            let claim_id =
                mfm_facts::derive_fact_claim_id(run_id.clone(), seq.as_u64(), ordinal.as_u32())
                    .map_err(|error| StoreError::Event(error.to_string()))?;
            fact_claim_projection_key("fact", &claim_id)
        }
        KernelEventPayload::ArtifactReferenced(payload) => {
            format!("artifact:{}:ref", payload.artifact_ref.artifact_id)
        }
        KernelEventPayload::CellProduced(payload) => {
            format!("cell:{}:terminal", payload.cell_id)
        }
        KernelEventPayload::CellSkipped(payload) => {
            format!("cell:{}:terminal", payload.cell_id)
        }
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            format!(
                "sidefx:{}:{}:intent",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.pair_id
            )
        }
        KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx:{}:{}:claim:{}:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx:{}:{}:claim:{}:{}:taken_over",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::ResourceLaneClaimed(payload) => format!(
            "resource_lane:{}:{}:claim:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.claim_id
        ),
        KernelEventPayload::ResourceLaneClaimIntent(_)
        | KernelEventPayload::ResourceLaneReleaseIntent(_) => {
            return Err(StoreError::Event(
                "resource-lane intent payload reached persisted logical-key derivation".to_owned(),
            ));
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx:{}:{}:invocation:{}:prepared:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx:{}:{}:invocation:{}:started",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:receipt",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:confirmation",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!(
                "sidefx:{}:{}:ambiguous",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.pair_id
            )
        }
        KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx:{}:{}:invocation:{}:failure",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::ResourceLaneReleased(payload) => format!(
            "resource_lane:{}:{}:release:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.release_id
        ),
        KernelEventPayload::PublicOutputProduced(payload) => {
            format!("public_output:{}", payload.public_schema_id)
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            format!(
                "public_output_failed:{}:{}:{}",
                payload.public_schema_id, payload.node_id, payload.attempt_id
            )
        }
        KernelEventPayload::RetentionRefsAppended(payload) => {
            format!("retention:{}:refs:{}", payload.run_id, payload_hash)
        }
        KernelEventPayload::RetentionManifestProjected(payload) => {
            format!(
                "retention:{}:manifest:{}",
                payload.run_id, payload.manifest_seq
            )
        }
    };
    LogicalEventKey::new(key)
}

fn is_unique_logical_key(key: &LogicalEventKey) -> bool {
    let key = key.as_str();
    !key.starts_with("attempt:")
}

fn unique_logical_key_rewrite_allowed(
    base: &CommitBase,
    run_id: &RunId,
    logical_key: &LogicalEventKey,
    payload: &KernelEventPayload,
    projections: &ProjectionSnapshot,
) -> Result<bool> {
    if !base
        .logical_keys
        .contains(&(run_id.clone(), logical_key.clone()))
    {
        return Ok(false);
    }
    let Some((pair_id, invocation_epoch)) = recoverable_submission_result_payload(payload) else {
        return Ok(false);
    };
    Ok(matches!(
        projections.side_effect_state_for_pair(run_id, pair_id)?,
        Some(state)
            if matches!(
                state.phase(),
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::Unknown,
                } if claim.invocation_epoch == invocation_epoch
            )
    ))
}

fn recoverable_submission_result_payload(
    payload: &KernelEventPayload,
) -> Option<(&SideEffectPairId, u32)> {
    match payload {
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        _ => None,
    }
}

fn side_effect_ledger_purpose_key(purpose: &events::SideEffectLedgerPurpose) -> String {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => "forward".to_owned(),
        events::SideEffectLedgerPurpose::Remediation {
            forward_pair_id, ..
        } => {
            format!("remediation:{forward_pair_id}")
        }
    }
}

pub use mfm_events::v1::{EventArtifactReferenceSource, EventArtifactRequirement};

pub use self::artifact_refs::event_artifact_requirements;
use self::artifact_refs::referenced_artifact_ids;

mod artifact_refs;

mod projection;

pub use self::resource_lanes::resource_lane_release_intent_resolution;
use self::resource_lanes::{
    acquire_resource_lane, release_resource_lane, require_no_resource_lane_for_holder,
    require_no_resource_lanes_for_run, resolve_active_resource_lane_release,
    ResourceLaneReleaseMatch,
};
use self::side_effects::{
    note_saga_engagement, prepared_invocation_projection, require_active_attempt_for_side_effect,
    require_forward_fence_open, require_remediation_intent_admissible,
    require_side_effect_pair_consistent, require_side_effect_pair_role, require_side_effect_phase,
    require_side_effect_purpose, side_effect_projection_error, transition_side_effect_epoch_only,
    transition_side_effect_failure, EpochOnlyTransition, PairRoleRequirement,
};

mod resource_lanes;

mod side_effects;

fn payload_json(payload: &KernelEventPayload) -> serde_json::Value {
    match payload {
        KernelEventPayload::RunAdmitted(payload) => serde_json::json!({
            "admitted_binding_digest": payload.admitted_binding_digest.as_str(),
            "canonicalizer_identity": payload.canonicalizer_identity.as_str(),
            "certificate_artifact": run_artifact_json(&payload.certificate_artifact),
            "config_artifacts": payload.config_artifacts.iter().map(run_artifact_json).collect::<Vec<_>>(),
            "descriptor_identities": payload.descriptor_identities.iter().map(descriptor_identity_json).collect::<Vec<_>>(),
            "fact_descriptor_artifacts": payload.fact_descriptor_artifacts.iter().map(run_artifact_json).collect::<Vec<_>>(),
            "adapter_executables": payload.adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
            "entry_point": entry_point_launch_evidence_json(&payload.entry_point),
            "identity_material": run_identity_material_json(&payload.identity_material),
            "lowering_version": payload.lowering_version.as_str(),
            "public_output_schema_id": payload.public_output_schema_id.as_str(),
            "run_id": payload.run_id.as_str(),
            "runner_executables": payload.runner_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
            "saga_policy_digest": payload.saga_policy_digest.as_str(),
            "seed_cells": payload.seed_cells.iter().map(seed_cell_ref_json).collect::<Vec<_>>(),
            "spec_artifact": run_artifact_json(&payload.spec_artifact),
            "spec_hash": payload.spec_hash.as_str(),
            "spec_version": payload.spec_version.as_str(),
            "variant": "RunAdmitted",
        }),
        KernelEventPayload::StateAttemptStarted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "attempt_no": payload.attempt_no,
            "node_id": payload.node_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "state_kind": payload.state_kind.as_str(),
            "state_version": payload.state_version.as_str(),
            "variant": "StateAttemptStarted",
        }),
        KernelEventPayload::FactRecorded(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim": fact_claim_json(&payload.claim),
            "node_id": payload.node_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "FactRecorded",
        }),
        KernelEventPayload::ArtifactReferenced(payload) => serde_json::json!({
            "artifact_ref": event_artifact_json(&payload.artifact_ref),
            "attempt_id": payload.attempt_id.as_ref().map(AttemptId::as_str),
            "node_id": payload.node_id.as_ref().map(NodeId::as_str),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ArtifactReferenced",
        }),
        KernelEventPayload::CellProduced(payload) => serde_json::json!({
            "artifact_id": payload.artifact_id.as_str(),
            "attempt_id": payload.attempt_id.as_str(),
            "cell_id": payload.cell_id.as_str(),
            "context": cell_context_json(&payload.context),
            "content_digest": payload.content_digest.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "node_id": payload.node_id.as_str(),
            "producer_state_kind": payload.producer_state_kind.as_ref().map(|value| value.as_str()),
            "producer_state_version": payload.producer_state_version.as_ref().map(|value| value.as_str()),
            "schema_id": payload.schema_id.as_str(),
            "scope_id": payload.scope_id.as_str(),
            "semantic_type_id": payload.semantic_type_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "value_lineage": value_lineage_json(&payload.value_lineage),
            "variant": "CellProduced",
        }),
        KernelEventPayload::CellSkipped(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "cell_id": payload.cell_id.as_str(),
            "context": cell_context_json(&payload.context),
            "node_id": payload.node_id.as_str(),
            "schema_id": payload.schema_id.as_str(),
            "scope_id": payload.scope_id.as_str(),
            "semantic_type_id": payload.semantic_type_id.as_str(),
            "skip_reason": skip_reason_json(&payload.skip_reason),
            "spec_hash": payload.spec_hash.as_str(),
            "value_lineage": value_lineage_json(&payload.value_lineage),
            "variant": "CellSkipped",
        }),
        KernelEventPayload::SideEffectIntentPersisted(payload) => serde_json::json!({
            "adapter_kind": payload.adapter_kind.as_str(),
            "adapter_version": payload.adapter_version.as_str(),
            "attempt_id": payload.attempt_id.as_str(),
            "capability_kind": payload.capability_kind.as_str(),
            "capability_version": payload.capability_version.as_str(),
            "idempotency_input_hash": payload.idempotency_input_hash.as_str(),
            "idempotency_input_schema_id": payload.idempotency_input_schema_id.as_str(),
            "idempotency_key": payload.idempotency_key.as_str(),
            "intent_artifact_evidence_hash": payload.intent_artifact_evidence_hash.as_str(),
            "intent_artifact_id": payload.intent_artifact_id.as_str(),
            "intent_hash": payload.intent_hash.as_str(),
            "intent_schema_id": payload.intent_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "scope_id": payload.scope_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectIntentPersisted",
        }),
        KernelEventPayload::SideEffectClaimed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "claim_owner": payload.claim_owner.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectClaimed",
        }),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "new_claim_owner": payload.new_claim_owner.as_str(),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "previous_claim_generation": payload.previous_claim_generation,
            "previous_claim_owner": payload.previous_claim_owner.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectClaimTakenOver",
        }),
        KernelEventPayload::ResourceLaneClaimed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token,
            "claim_id": payload.claim_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "lane_transition_seq": payload.lane_transition_seq,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "requirement_digest": payload.requirement_digest.as_str(),
            "resolved_by_capability_impl": payload.resolved_by_capability_impl.as_str(),
            "resource_key": resource_key_evidence_json(&payload.resource_key),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneClaimed",
        }),
        KernelEventPayload::ResourceLaneClaimIntent(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "requirement_digest": payload.requirement_digest.as_str(),
            "resolved_by_capability_impl": payload.resolved_by_capability_impl.as_str(),
            "resource_key": resource_key_evidence_json(&payload.resource_key),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneClaimIntent",
        }),
        KernelEventPayload::SideEffectInvocationPrepared(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "resource_key": payload.resource_key.as_ref().map(resource_key_evidence_json),
            "prepared_artifact_evidence_hash": payload
                .prepared_artifact_evidence_hash
                .as_ref()
                .map(ContentDigest::as_str),
            "prepared_artifact_id": payload.prepared_artifact_id.as_ref().map(ArtifactId::as_str),
            "prepared_hash": payload.prepared_hash.as_ref().map(ContentDigest::as_str),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectInvocationPrepared",
        }),
        KernelEventPayload::SideEffectInvocationStarted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "claim_owner": payload.claim_owner.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectInvocationStarted",
        }),
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "proof_artifact_evidence_hash": payload.proof_artifact_evidence_hash.as_str(),
            "proof_artifact_id": payload.proof_artifact_id.as_str(),
            "proof_hash": payload.proof_hash.as_str(),
            "proof_schema_id": payload.proof_schema_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectNotSubmittedProven",
        }),
        KernelEventPayload::SideEffectSubmissionObserved(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "submission_artifact_evidence_hash": payload.submission_artifact_evidence_hash.as_str(),
            "submission_artifact_id": payload.submission_artifact_id.as_str(),
            "submission_hash": payload.submission_hash.as_str(),
            "submission_schema_id": payload.submission_schema_id.as_str(),
            "variant": "SideEffectSubmissionObserved",
        }),
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "evidence_artifact_evidence_hash": payload.evidence_artifact_evidence_hash.as_str(),
            "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "evidence_schema_id": payload.evidence_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectSubmissionUnknown",
        }),
        KernelEventPayload::SideEffectReceiptObserved(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "receipt_artifact_evidence_hash": payload.receipt_artifact_evidence_hash.as_str(),
            "receipt_artifact_id": payload.receipt_artifact_id.as_str(),
            "receipt_hash": payload.receipt_hash.as_str(),
            "receipt_schema_id": payload.receipt_schema_id.as_str(),
            "replay_verifier_id": payload.replay_verifier_id.as_str(),
            "resource_touched_set": payload.resource_touched_set.as_ref().map(resource_touched_set_evidence_json),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectReceiptObserved",
        }),
        KernelEventPayload::SideEffectConfirmationObserved(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "confirmation_artifact_evidence_hash": payload
                .confirmation_artifact_evidence_hash
                .as_str(),
            "confirmation_artifact_id": payload.confirmation_artifact_id.as_str(),
            "confirmation_hash": payload.confirmation_hash.as_str(),
            "confirmation_schema_id": payload.confirmation_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "replay_verifier_id": payload.replay_verifier_id.as_str(),
            "resource_touched_set": payload.resource_touched_set.as_ref().map(resource_touched_set_evidence_json),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectConfirmationObserved",
        }),
        KernelEventPayload::SideEffectAmbiguous(payload) => serde_json::json!({
            "ambiguity_code": payload.ambiguity_code.as_str(),
            "attempt_id": payload.attempt_id.as_str(),
            "evidence_artifact_evidence_hash": payload.evidence_artifact_evidence_hash.as_str(),
            "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "evidence_schema_id": payload.evidence_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectAmbiguous",
        }),
        KernelEventPayload::SideEffectFailed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "error": error_info_json(&payload.error),
            "failure_phase": failure_phase_str(payload.failure_phase),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "retryable": payload.retryable,
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectFailed",
        }),
        KernelEventPayload::ResourceLaneReleased(payload) => serde_json::json!({
            "claim_fencing_token": payload.claim_fencing_token,
            "claim_id": payload.claim_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "lane_transition_seq": payload.lane_transition_seq,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "release_authority": payload.release_authority.as_str(),
            "release_id": payload.release_id.as_str(),
            "release_reason": payload.release_reason.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneReleased",
        }),
        KernelEventPayload::ResourceLaneReleaseIntent(payload) => serde_json::json!({
            "claim_id": payload.claim_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "release_authority": payload.release_authority.as_str(),
            "release_reason": payload.release_reason.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneReleaseIntent",
        }),
        KernelEventPayload::PublicOutputProduced(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "cells": payload.cells.iter().map(named_cell_ref_json).collect::<Vec<_>>(),
            "node_id": payload.node_id.as_str(),
            "output_spec_digest": payload.output_spec_digest.as_str(),
            "public_schema_id": payload.public_schema_id.as_str(),
            "receipt_cell_id": payload.receipt_cell_id.as_str(),
            "rendered_artifact_evidence_hash": payload
                .rendered_artifact_evidence_hash
                .as_ref()
                .map(ContentDigest::as_str),
            "rendered_artifact_id": payload.rendered_artifact_id.as_ref().map(ArtifactId::as_str),
            "rendered_digest": payload.rendered_digest.as_str(),
            "renderer_descriptor_id": payload.renderer_descriptor_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "PublicOutputProduced",
        }),
        KernelEventPayload::PublicOutputRenderFailed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "error": error_info_json(&payload.error),
            "node_id": payload.node_id.as_str(),
            "public_schema_id": payload.public_schema_id.as_str(),
            "renderer_descriptor_id": payload.renderer_descriptor_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "PublicOutputRenderFailed",
        }),
        KernelEventPayload::StateAttemptCompleted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "node_id": payload.node_id.as_str(),
            "output_cell_id": payload.output_cell_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "StateAttemptCompleted",
        }),
        KernelEventPayload::StateAttemptInterrupted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "node_id": payload.node_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "StateAttemptInterrupted",
        }),
        KernelEventPayload::StateAttemptFailed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "error": error_info_json(&payload.error),
            "node_id": payload.node_id.as_str(),
            "retryable": payload.retryable,
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "StateAttemptFailed",
        }),
        KernelEventPayload::ManualResolutionRecorded(payload) => serde_json::json!({
            "authorization_artifact_evidence_hash": payload
                .authorization_artifact_evidence_hash
                .as_str(),
            "authorization_artifact_id": payload.authorization_artifact_id.as_str(),
            "authorization_hash": payload.authorization_hash.as_str(),
            "authorization_schema_id": payload.authorization_schema_id.as_str(),
            "evidence_artifact_evidence_hash": payload.evidence_artifact_evidence_hash.as_str(),
            "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "evidence_schema_id": payload.evidence_schema_id.as_str(),
            "note": payload.note.as_ref().map(manual_resolution_note_json),
            "outcome": manual_resolution_outcome_str(payload.outcome),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ManualResolutionRecorded",
        }),
        KernelEventPayload::RunCompleted(payload) => serde_json::json!({
            "outcome": run_completion_outcome_json(&payload.outcome),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "RunCompleted",
        }),
        KernelEventPayload::RetentionRefsAppended(payload) => serde_json::json!({
            "reason": retention_reason_str(payload.reason),
            "refs": payload.refs.iter().map(retention_ref_json).collect::<Vec<_>>(),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "RetentionRefsAppended",
        }),
        KernelEventPayload::RetentionManifestProjected(payload) => serde_json::json!({
            "manifest_artifact_evidence_hash": payload.manifest_artifact_evidence_hash.as_str(),
            "manifest_artifact_id": payload.manifest_artifact_id.as_str(),
            "manifest_digest": payload.manifest_digest.as_str(),
            "manifest_seq": payload.manifest_seq,
            "previous_manifest_digest": payload.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "RetentionManifestProjected",
        }),
    }
}

/// Parses a typed kernel event payload from its store canonical JSON shape.
pub fn payload_from_json_value(json: &serde_json::Value) -> Result<KernelEventPayload> {
    match required_str(json, "variant")? {
        "RunAdmitted" => Ok(KernelEventPayload::RunAdmitted(Box::new(
            events::RunAdmitted {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                identity_material: parse_run_identity_material(required_obj(
                    json,
                    "identity_material",
                )?)?,
                entry_point: parse_entry_point_launch_evidence(required_obj(json, "entry_point")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                spec_artifact: parse_run_artifact(required_obj(json, "spec_artifact")?)?,
                certificate_artifact: parse_run_artifact(required_obj(
                    json,
                    "certificate_artifact",
                )?)?,
                config_artifacts: parse_vec(json, "config_artifacts", parse_run_artifact)?,
                fact_descriptor_artifacts: parse_vec(
                    json,
                    "fact_descriptor_artifacts",
                    parse_run_artifact,
                )?,
                spec_version: parse_identity(required_str(json, "spec_version")?)?,
                lowering_version: parse_identity(required_str(json, "lowering_version")?)?,
                public_output_schema_id: parse_identity(required_str(
                    json,
                    "public_output_schema_id",
                )?)?,
                saga_policy_digest: parse_identity(required_str(json, "saga_policy_digest")?)?,
                descriptor_identities: parse_vec(json, "descriptor_identities", |item| {
                    parse_descriptor_identity(item)
                })?,
                runner_executables: parse_vec(json, "runner_executables", parse_executable)?,
                adapter_executables: parse_vec(json, "adapter_executables", parse_executable)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
                admitted_binding_digest: parse_identity(required_str(
                    json,
                    "admitted_binding_digest",
                )?)?,
                seed_cells: parse_vec(json, "seed_cells", parse_seed_cell_ref)?,
            },
        ))),
        "StateAttemptStarted" => Ok(KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                attempt_no: required_u32(json, "attempt_no")?,
                state_kind: parse_identity(required_str(json, "state_kind")?)?,
                state_version: parse_identity(required_str(json, "state_version")?)?,
            },
        )),
        "FactRecorded" => Ok(KernelEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            claim: parse_fact_claim(required_obj(json, "claim")?)?,
        })),
        "ArtifactReferenced" => Ok(KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: optional_str(json, "node_id")?
                    .map(parse_identity)
                    .transpose()?,
                attempt_id: optional_str(json, "attempt_id")?
                    .map(parse_identity)
                    .transpose()?,
                artifact_ref: parse_event_artifact(required_obj(json, "artifact_ref")?)?,
            },
        )),
        "CellProduced" => Ok(KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            cell_id: parse_identity(required_str(json, "cell_id")?)?,
            scope_id: parse_identity(required_str(json, "scope_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
            context: parse_cell_context(required_obj(json, "context")?)?,
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
            producer_state_kind: optional_str(json, "producer_state_kind")?
                .map(parse_identity)
                .transpose()?,
            producer_state_version: optional_str(json, "producer_state_version")?
                .map(parse_identity)
                .transpose()?,
        })),
        "CellSkipped" => Ok(KernelEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            cell_id: parse_identity(required_str(json, "cell_id")?)?,
            scope_id: parse_identity(required_str(json, "scope_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
            context: parse_cell_context(required_obj(json, "context")?)?,
            skip_reason: parse_skip_reason(required_obj(json, "skip_reason")?)?,
        })),
        "SideEffectIntentPersisted" => Ok(KernelEventPayload::SideEffectIntentPersisted(
            side_effect::IntentPersisted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                scope_id: parse_identity(required_str(json, "scope_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                intent_schema_id: parse_identity(required_str(json, "intent_schema_id")?)?,
                intent_hash: parse_identity(required_str(json, "intent_hash")?)?,
                intent_artifact_id: parse_identity(required_str(json, "intent_artifact_id")?)?,
                intent_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "intent_artifact_evidence_hash",
                )?)?,
                idempotency_input_schema_id: parse_identity(required_str(
                    json,
                    "idempotency_input_schema_id",
                )?)?,
                idempotency_input_hash: parse_identity(required_str(
                    json,
                    "idempotency_input_hash",
                )?)?,
                idempotency_key: events::IdempotencyKeyRef::new(required_str(
                    json,
                    "idempotency_key",
                )?)?,
                capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
                capability_version: parse_identity(required_str(json, "capability_version")?)?,
                adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
                adapter_version: parse_identity(required_str(json, "adapter_version")?)?,
            },
        )),
        "SideEffectClaimed" => Ok(KernelEventPayload::SideEffectClaimed(
            side_effect::Claimed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "SideEffectClaimTakenOver" => Ok(KernelEventPayload::SideEffectClaimTakenOver(
            side_effect::ClaimTakenOver {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                previous_claim_owner: events::RunnerInvocationId::new(required_str(
                    json,
                    "previous_claim_owner",
                )?)?,
                new_claim_owner: events::RunnerInvocationId::new(required_str(
                    json,
                    "new_claim_owner",
                )?)?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                previous_claim_generation: required_u32(json, "previous_claim_generation")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "ResourceLaneClaimed" => Ok(KernelEventPayload::ResourceLaneClaimed(
            events::ResourceLaneClaimed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                resource_key: parse_resource_key_evidence(required_obj(json, "resource_key")?)?,
                requirement_digest: parse_identity(required_str(json, "requirement_digest")?)?,
                resolved_by_capability_impl: events::RunnerFactoryId::new(required_str(
                    json,
                    "resolved_by_capability_impl",
                )?)?,
                claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
                claim_fencing_token: required_u64(json, "claim_fencing_token")?,
                lane_transition_seq: required_u64(json, "lane_transition_seq")?,
            },
        )),
        "SideEffectInvocationPrepared" => Ok(KernelEventPayload::SideEffectInvocationPrepared(
            side_effect::InvocationPrepared {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
                resource_key: optional_obj(json, "resource_key")?
                    .map(parse_resource_key_evidence)
                    .transpose()?,
                prepared_artifact_id: optional_str(json, "prepared_artifact_id")?
                    .map(parse_identity)
                    .transpose()?,
                prepared_hash: optional_str(json, "prepared_hash")?
                    .map(parse_identity)
                    .transpose()?,
                prepared_artifact_evidence_hash: optional_str(
                    json,
                    "prepared_artifact_evidence_hash",
                )?
                .map(parse_identity)
                .transpose()?,
            },
        )),
        "SideEffectInvocationStarted" => Ok(KernelEventPayload::SideEffectInvocationStarted(
            side_effect::InvocationStarted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "SideEffectNotSubmittedProven" => Ok(KernelEventPayload::SideEffectNotSubmittedProven(
            side_effect::NotSubmittedProven {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                proof_schema_id: parse_identity(required_str(json, "proof_schema_id")?)?,
                proof_hash: parse_identity(required_str(json, "proof_hash")?)?,
                proof_artifact_id: parse_identity(required_str(json, "proof_artifact_id")?)?,
                proof_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "proof_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectSubmissionObserved" => Ok(KernelEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                submission_schema_id: parse_identity(required_str(json, "submission_schema_id")?)?,
                submission_hash: parse_identity(required_str(json, "submission_hash")?)?,
                submission_artifact_id: parse_identity(required_str(
                    json,
                    "submission_artifact_id",
                )?)?,
                submission_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "submission_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectSubmissionUnknown" => Ok(KernelEventPayload::SideEffectSubmissionUnknown(
            side_effect::SubmissionUnknown {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectReceiptObserved" => Ok(KernelEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                receipt_schema_id: parse_identity(required_str(json, "receipt_schema_id")?)?,
                receipt_hash: parse_identity(required_str(json, "receipt_hash")?)?,
                receipt_artifact_id: parse_identity(required_str(json, "receipt_artifact_id")?)?,
                receipt_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "receipt_artifact_evidence_hash",
                )?)?,
                replay_verifier_id: events::ReplayVerifierId::new(required_str(
                    json,
                    "replay_verifier_id",
                )?)?,
                resource_touched_set: optional_obj(json, "resource_touched_set")?
                    .map(parse_resource_touched_set_evidence)
                    .transpose()?,
            },
        )),
        "SideEffectConfirmationObserved" => Ok(KernelEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                confirmation_schema_id: parse_identity(required_str(
                    json,
                    "confirmation_schema_id",
                )?)?,
                confirmation_hash: parse_identity(required_str(json, "confirmation_hash")?)?,
                confirmation_artifact_id: parse_identity(required_str(
                    json,
                    "confirmation_artifact_id",
                )?)?,
                confirmation_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "confirmation_artifact_evidence_hash",
                )?)?,
                replay_verifier_id: events::ReplayVerifierId::new(required_str(
                    json,
                    "replay_verifier_id",
                )?)?,
                resource_touched_set: optional_obj(json, "resource_touched_set")?
                    .map(parse_resource_touched_set_evidence)
                    .transpose()?,
            },
        )),
        "SideEffectAmbiguous" => Ok(KernelEventPayload::SideEffectAmbiguous(
            side_effect::Ambiguous {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                ambiguity_code: events::AmbiguityCode::new(required_str(json, "ambiguity_code")?)?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectFailed" => Ok(KernelEventPayload::SideEffectFailed(side_effect::Failed {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
            ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                json,
                "ledger_purpose",
            )?)?,
            pair_id: parse_identity(required_str(json, "pair_id")?)?,
            pair_role: parse_side_effect_pair_role(json, "pair_role")?,
            invocation_epoch: required_u32(json, "invocation_epoch")?,
            failure_phase: parse_failure_phase(required_str(json, "failure_phase")?)?,
            retryable: required_bool(json, "retryable")?,
            error: parse_error_info(required_obj(json, "error")?)?,
        })),
        "ResourceLaneReleased" => Ok(KernelEventPayload::ResourceLaneReleased(
            events::ResourceLaneReleased {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
                release_id: events::ResourceLaneReleaseId::new(required_str(json, "release_id")?)?,
                claim_fencing_token: required_u64(json, "claim_fencing_token")?,
                release_authority: parse_resource_lane_release_authority(required_str(
                    json,
                    "release_authority",
                )?)?,
                release_reason: events::ResourceLaneReleaseReason::new(required_str(
                    json,
                    "release_reason",
                )?)?,
                lane_transition_seq: required_u64(json, "lane_transition_seq")?,
            },
        )),
        "PublicOutputProduced" => Ok(KernelEventPayload::PublicOutputProduced(
            events::PublicOutputProduced {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                receipt_cell_id: parse_identity(required_str(json, "receipt_cell_id")?)?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                output_spec_digest: parse_identity(required_str(json, "output_spec_digest")?)?,
                cells: parse_vec(json, "cells", parse_named_cell_ref)?,
                rendered_digest: parse_identity(required_str(json, "rendered_digest")?)?,
                rendered_artifact_id: optional_str(json, "rendered_artifact_id")?
                    .map(parse_identity)
                    .transpose()?,
                rendered_artifact_evidence_hash: optional_str(
                    json,
                    "rendered_artifact_evidence_hash",
                )?
                .map(parse_identity)
                .transpose()?,
                renderer_descriptor_id: parse_identity(required_str(
                    json,
                    "renderer_descriptor_id",
                )?)?,
            },
        )),
        "PublicOutputRenderFailed" => Ok(KernelEventPayload::PublicOutputRenderFailed(
            events::PublicOutputRenderFailed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                renderer_descriptor_id: parse_identity(required_str(
                    json,
                    "renderer_descriptor_id",
                )?)?,
                error: parse_error_info(required_obj(json, "error")?)?,
            },
        )),
        "StateAttemptCompleted" => Ok(KernelEventPayload::StateAttemptCompleted(
            events::StateAttemptCompleted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                output_cell_id: parse_identity(required_str(json, "output_cell_id")?)?,
            },
        )),
        "StateAttemptInterrupted" => Ok(KernelEventPayload::StateAttemptInterrupted(
            events::StateAttemptInterrupted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            },
        )),
        "StateAttemptFailed" => Ok(KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                retryable: required_bool(json, "retryable")?,
                error: parse_error_info(required_obj(json, "error")?)?,
            },
        )),
        "ManualResolutionRecorded" => Ok(KernelEventPayload::ManualResolutionRecorded(
            events::ManualResolutionRecorded {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                outcome: parse_manual_resolution_outcome(required_str(json, "outcome")?)?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
                authorization_schema_id: parse_identity(required_str(
                    json,
                    "authorization_schema_id",
                )?)?,
                authorization_hash: parse_identity(required_str(json, "authorization_hash")?)?,
                authorization_artifact_id: parse_identity(required_str(
                    json,
                    "authorization_artifact_id",
                )?)?,
                authorization_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "authorization_artifact_evidence_hash",
                )?)?,
                note: optional_obj(json, "note")?
                    .map(parse_manual_resolution_note)
                    .transpose()?,
            },
        )),
        "RunCompleted" => Ok(KernelEventPayload::RunCompleted(events::RunCompleted {
            run_id: parse_identity(required_str(json, "run_id")?)?,
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            outcome: parse_run_completion_outcome(required_obj(json, "outcome")?)?,
        })),
        "RetentionRefsAppended" => Ok(KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                refs: parse_vec(json, "refs", parse_retention_ref)?,
                reason: parse_retention_reason(required_str(json, "reason")?)?,
            },
        )),
        "RetentionManifestProjected" => Ok(KernelEventPayload::RetentionManifestProjected(
            events::RetentionManifestProjected {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                manifest_seq: required_u64(json, "manifest_seq")?,
                manifest_digest: parse_identity(required_str(json, "manifest_digest")?)?,
                previous_manifest_digest: optional_str(json, "previous_manifest_digest")?
                    .map(parse_identity)
                    .transpose()?,
                manifest_artifact_id: parse_identity(required_str(json, "manifest_artifact_id")?)?,
                manifest_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "manifest_artifact_evidence_hash",
                )?)?,
            },
        )),
        other => Err(StoreError::Event(format!(
            "unknown event payload variant {other}"
        ))),
    }
}

fn parse_descriptor_identity(json: &serde_json::Value) -> Result<DescriptorIdentity> {
    match required_str(json, "descriptor_family")? {
        "state" => Ok(DescriptorIdentity::State(Box::new(
            StateDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                name: required_str(json, "name")?.to_owned(),
                state_kind: parse_identity(required_str(json, "state_kind")?)?,
                state_version: parse_identity(required_str(json, "state_version")?)?,
                context: parse_state_context_descriptor(required_obj(json, "context")?)?,
                input_context: parse_state_input_context_contract(required_obj(
                    json,
                    "input_context",
                )?)?,
                output_context: parse_state_output_context_contract(required_obj(
                    json,
                    "output_context",
                )?)?,
                config_schema_id: parse_identity(required_str(json, "config_schema_id")?)?,
                input_schema_id: parse_identity(required_str(json, "input_schema_id")?)?,
                output_schema_id: parse_identity(required_str(json, "output_schema_id")?)?,
                output_semantic_type_id: parse_identity(required_str(
                    json,
                    "output_semantic_type_id",
                )?)?,
                effect_kind: parse_identity(required_str(json, "effect_kind")?)?,
                effect_class: required_str(json, "effect_class")?.to_owned(),
                effect_name: required_str(json, "effect_name")?.to_owned(),
                effect_version: parse_identity(required_str(json, "effect_version")?)?,
                capabilities: parse_capability_set(required_obj(json, "capabilities")?)?,
                emitted_fact_descriptors: parse_vec(
                    json,
                    "emitted_fact_descriptors",
                    parse_fact_descriptor_ref,
                )?,
                runner: required_str(json, "runner")?.to_owned(),
                side_effect_contract_digest: optional_str(json, "side_effect_contract_digest")?
                    .map(parse_identity)
                    .transpose()?,
            },
        ))),
        "operation" => Ok(DescriptorIdentity::Operation(Box::new(
            OperationDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                name: required_str(json, "name")?.to_owned(),
                operation_kind: parse_identity(required_str(json, "operation_kind")?)?,
                operation_version: parse_identity(required_str(json, "operation_version")?)?,
                config_schema_id: parse_identity(required_str(json, "config_schema_id")?)?,
                input_schema_id: parse_identity(required_str(json, "input_schema_id")?)?,
                output_schema_id: parse_identity(required_str(json, "output_schema_id")?)?,
                expansion_abi: required_str(json, "expansion_abi")?.to_owned(),
            },
        ))),
        "renderer" => Ok(DescriptorIdentity::Renderer(Box::new(
            RendererDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                renderer_kind: RendererKind::new(required_str(json, "renderer_kind")?)
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                renderer_version: RendererVersion::new(required_str(json, "renderer_version")?)
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            },
        ))),
        other => Err(StoreError::Identity(format!(
            "unknown descriptor identity family {other}"
        ))),
    }
}

fn parse_state_context_descriptor(
    json: &serde_json::Value,
) -> Result<spec::StateContextDescriptorSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateContextDescriptorSpec::no_context()),
        "required" => Ok(spec::StateContextDescriptorSpec::Required(Box::new(
            spec::StateContextDescriptorRequirementSpec {
                context_descriptor_id: parse_identity(required_str(
                    json,
                    "context_descriptor_id",
                )?)?,
                schema_id: parse_identity(required_str(json, "schema_id")?)?,
                semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            },
        ))),
        kind => Err(StoreError::Event(format!(
            "unknown state context descriptor kind {kind}"
        ))),
    }
}

fn parse_context_producer(json: &serde_json::Value) -> Result<spec::ContextProducerSpec> {
    Ok(spec::ContextProducerSpec {
        producer_descriptor_ids: required_array(json, "producer_descriptor_ids")?
            .iter()
            .map(|value| {
                let raw = value.as_str().ok_or_else(|| {
                    StoreError::Event("producer_descriptor_ids entries must be strings".to_owned())
                })?;
                parse_identity(raw).map_err(StoreError::from)
            })
            .collect::<Result<Vec<_>>>()?,
        seed_producers_allowed: required_bool(json, "seed_producers_allowed")?,
    })
}

fn parse_state_input_context_contract(
    json: &serde_json::Value,
) -> Result<spec::StateInputContextContractSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateInputContextContractSpec::no_context()),
        "required" => Ok(spec::StateInputContextContractSpec::Required {
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required_obj(json, "producer")?)?),
        }),
        kind => Err(StoreError::Event(format!(
            "unknown state input context contract kind {kind}"
        ))),
    }
}

fn parse_state_output_context_contract(
    json: &serde_json::Value,
) -> Result<spec::StateOutputContextContractSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateOutputContextContractSpec::no_context()),
        "produces" => Ok(spec::StateOutputContextContractSpec::Produces {
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
        }),
        kind => Err(StoreError::Event(format!(
            "unknown state output context contract kind {kind}"
        ))),
    }
}

fn parse_cell_context(json: &serde_json::Value) -> Result<spec::CellContextSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::CellContextSpec::no_context()),
        "bound" => Ok(spec::CellContextSpec::Bound {
            context_ref: parse_identity(required_str(json, "context_ref")?)?,
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required_obj(json, "producer")?)?),
        }),
        kind => Err(StoreError::Event(format!(
            "unknown cell context kind {kind}"
        ))),
    }
}

fn parse_capability_set(json: &serde_json::Value) -> Result<CapabilitySetDescriptor> {
    let capabilities = required_array(json, "capabilities")?
        .iter()
        .map(parse_capability)
        .collect::<Result<Vec<_>>>()?;
    Ok(CapabilitySetDescriptor::new(capabilities)?)
}

fn parse_capability(json: &serde_json::Value) -> Result<CapabilityDescriptor> {
    Ok(CapabilityDescriptor::new(
        parse_identity::<CapabilityKind>(required_str(json, "kind")?)?,
        parse_identity::<CapabilityVersion>(required_str(json, "version")?)?,
        parse_capability_role(required_str(json, "role")?)?,
        required_str(json, "name")?.to_owned(),
    )?)
}

fn parse_executable(json: &serde_json::Value) -> Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id: events::RunnerFactoryId::new(required_str(json, "factory_id")?)?,
        cargo_package_digest: parse_identity(required_str(json, "cargo_package_digest")?)?,
        binary_digest: parse_identity(required_str(json, "binary_digest")?)?,
        nix_derivation_hash: optional_str(json, "nix_derivation_hash")?
            .map(events::NixDerivationHash::new)
            .transpose()?,
        nix_output_hash: optional_str(json, "nix_output_hash")?
            .map(events::NixOutputHash::new)
            .transpose()?,
    })
}

fn parse_seed_cell_ref(json: &serde_json::Value) -> Result<events::SeedCellRef> {
    Ok(events::SeedCellRef {
        seed_id: parse_identity(required_str(json, "seed_id")?)?,
        cell_id: parse_identity(required_str(json, "cell_id")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        digest: parse_identity(required_str(json, "digest")?)?,
        seed_artifact: parse_event_artifact(required_obj(json, "seed_artifact")?)?,
    })
}

fn parse_named_cell_ref(json: &serde_json::Value) -> Result<events::NamedTypedCellRef> {
    Ok(events::NamedTypedCellRef {
        public_field_path: PublicFieldPath::new(required_str(json, "public_field_path")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
        cell_id: parse_identity(required_str(json, "cell_id")?)?,
        producer: parse_cell_producer(required_obj(json, "producer")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
    })
}

fn parse_cell_producer(json: &serde_json::Value) -> Result<CellProducer> {
    match required_str(json, "kind")? {
        "node" => Ok(CellProducer::Node(parse_identity(required_str(
            json, "node_id",
        )?)?)),
        "seed" => Ok(CellProducer::Seed(parse_identity(required_str(
            json, "seed_id",
        )?)?)),
        other => Err(StoreError::Identity(format!(
            "unknown cell producer {other}"
        ))),
    }
}

fn parse_value_lineage(json: &serde_json::Value) -> Result<ValueLineageRef> {
    Ok(ValueLineageRef {
        lineage_digest: parse_identity(required_str(json, "lineage_digest")?)?,
    })
}

/// Parses an artifact evidence reference from canonical JSON.
pub fn parse_event_artifact(json: &serde_json::Value) -> CodecResult<events::ArtifactEvidenceRef> {
    Ok(events::ArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        semantic_type_id: optional_str(json, "semantic_type_id")?
            .map(parse_identity)
            .transpose()?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
    })
}

fn parse_fact_claim(json: &serde_json::Value) -> Result<mfm_facts::FactClaim> {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: parse_fact_visibility(required_obj(json, "visibility")?)?,
        fact_kind: mfm_facts::FactKind::new(required_str(json, "fact_kind")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
        fact_descriptor_hash: parse_identity(required_str(json, "fact_descriptor_hash")?)?,
        subject: parse_fact_subject_evidence(required_obj(json, "subject")?)?,
        observed_at: optional_str(json, "observed_at")?.map(str::to_owned),
        request: optional_obj(json, "request")?
            .map(parse_fact_request_evidence)
            .transpose()?,
        response: parse_fact_response_evidence(required_obj(json, "response")?)?,
        producer: parse_fact_producer_provenance(required_obj(json, "producer")?)?,
    })
    .map_err(|error| StoreError::Identity(error.to_string()))
}

fn parse_fact_visibility(json: &serde_json::Value) -> Result<mfm_facts::FactVisibility> {
    match required_str(json, "kind")? {
        "run_private" => Ok(mfm_facts::FactVisibility::RunPrivate),
        "indexed" => Ok(mfm_facts::FactVisibility::Indexed {
            audience: parse_fact_audience(required_str(json, "audience")?)?,
            scope: parse_fact_visibility_scope(required_str(json, "scope")?)?,
        }),
        other => Err(StoreError::Identity(format!(
            "unknown fact visibility {other}"
        ))),
    }
}

fn parse_fact_audience(value: &str) -> Result<mfm_facts::FactAudience> {
    value
        .parse::<mfm_facts::FactAudience>()
        .map_err(|_| StoreError::Identity(format!("unknown fact audience {value}")))
}

fn parse_fact_visibility_scope(value: &str) -> Result<mfm_facts::FactVisibilityScope> {
    value
        .parse::<mfm_facts::FactVisibilityScope>()
        .map_err(|_| StoreError::Identity(format!("unknown fact visibility scope {value}")))
}

fn parse_fact_subject_evidence(json: &serde_json::Value) -> Result<mfm_facts::FactSubjectEvidence> {
    mfm_facts::FactSubjectEvidence::new(
        parse_identity(required_str(json, "fact_subject_namespace_hash")?)?,
        PlainCanonicalJsonBytes::from_canonical_json_slice(
            required_str(json, "subject_material")?.as_bytes(),
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?,
        parse_identity(required_str(json, "subject_material_hash")?)?,
        mfm_facts::FactKey::from_digest(parse_identity(required_str(json, "fact_key")?)?),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))
}

fn parse_fact_request_evidence(json: &serde_json::Value) -> Result<mfm_facts::FactRequestEvidence> {
    Ok(mfm_facts::FactRequestEvidence::new(
        parse_identity(required_str(json, "request_schema_id")?)?,
        parse_identity(required_str(json, "request_hash")?)?,
    ))
}

fn parse_fact_response_evidence(
    json: &serde_json::Value,
) -> Result<mfm_facts::FactResponseEvidence> {
    Ok(mfm_facts::FactResponseEvidence::new(
        parse_identity(required_str(json, "response_schema_id")?)?,
        parse_identity(required_str(json, "response_hash")?)?,
        parse_identity(required_str(json, "artifact_id")?)?,
        parse_identity(required_str(json, "artifact_evidence_hash")?)?,
    ))
}

fn parse_fact_producer_provenance(
    json: &serde_json::Value,
) -> Result<mfm_facts::FactProducerProvenance> {
    Ok(mfm_facts::FactProducerProvenance::new(
        parse_identity(required_str(json, "capability_kind")?)?,
        required_str(json, "capability_version")?.parse()?,
        parse_identity(required_str(json, "adapter_kind")?)?,
        required_str(json, "adapter_version")?.parse()?,
    ))
}

fn parse_run_artifact(json: &serde_json::Value) -> Result<events::RunArtifactEvidenceRef> {
    Ok(events::RunArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        schema_id: optional_str(json, "schema_id")?
            .map(parse_identity)
            .transpose()?,
        semantic_type_id: optional_str(json, "semantic_type_id")?
            .map(parse_identity)
            .transpose()?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
    })
}

fn parse_entry_point_launch_evidence(
    json: &serde_json::Value,
) -> Result<events::EntryPointLaunchEvidence> {
    Ok(events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new(required_str(json, "resolved_op_id")?)?,
        entry_point_registry_digest: parse_identity(required_str(
            json,
            "entry_point_registry_digest",
        )?)?,
    })
}

fn parse_run_identity_material(json: &serde_json::Value) -> Result<events::RunIdentityMaterialV1> {
    Ok(events::RunIdentityMaterialV1 {
        certified_spec_hash: parse_identity(required_str(json, "certified_spec_hash")?)?,
        store_scope_id: StoreScopeId::new(required_str(json, "store_scope_id")?)?,
        invocation_key_digest: parse_identity(required_str(json, "invocation_key_digest")?)?,
    })
}

/// Parses a cell skip reason from canonical JSON.
pub fn parse_skip_reason(json: &serde_json::Value) -> CodecResult<events::SkipReason> {
    Ok(events::SkipReason {
        code: events::ErrorCode::new(required_str(json, "code")?)?,
        safe_message: required_str(json, "safe_message")?.to_owned(),
    })
}

/// Parses a side-effect ledger purpose from canonical JSON.
pub fn parse_side_effect_ledger_purpose(
    json: &serde_json::Value,
) -> CodecResult<events::SideEffectLedgerPurpose> {
    match required_str(json, "kind")? {
        "forward" => Ok(events::SideEffectLedgerPurpose::Forward),
        "remediation" => Ok(events::SideEffectLedgerPurpose::Remediation {
            forward_pair_id: parse_identity(required_str(json, "forward_pair_id")?)?,
        }),
        other => Err(CodecError::Identity(format!(
            "unknown side-effect ledger purpose {other}"
        ))),
    }
}

fn parse_side_effect_pair_role(
    json: &serde_json::Value,
    field: &'static str,
) -> CodecResult<events::SideEffectPairRole> {
    let value = required_str(json, field)?;
    events::SideEffectPairRole::parse(value)
        .ok_or_else(|| CodecError::Identity(format!("unknown side-effect pair role {value}")))
}

/// Parses exclusive resource-key evidence from canonical JSON.
pub fn parse_resource_key_evidence(
    json: &serde_json::Value,
) -> CodecResult<events::ResourceKeyEvidence> {
    Ok(events::ResourceKeyEvidence {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        key_schema_id: parse_identity(required_str(json, "key_schema_id")?)?,
        key: events::ResourceKey::new(required_str(json, "key")?)?,
    })
}

/// Parses touched-set resource evidence from canonical JSON.
pub fn parse_resource_touched_set_evidence(
    json: &serde_json::Value,
) -> CodecResult<events::ResourceTouchedSetEvidence> {
    Ok(events::ResourceTouchedSetEvidence {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
        evidence_artifact_evidence_hash: parse_identity(required_str(
            json,
            "evidence_artifact_evidence_hash",
        )?)?,
    })
}

/// Parses a manual resolution outcome tag.
pub fn parse_manual_resolution_outcome(
    value: &str,
) -> CodecResult<events::ManualResolutionOutcome> {
    match value {
        "confirm_remediated" => Ok(events::ManualResolutionOutcome::ConfirmRemediated),
        "fail_without_acdc_claim" => Ok(events::ManualResolutionOutcome::FailWithoutAcdcClaim),
        other => Err(CodecError::Identity(format!(
            "unknown manual resolution outcome {other}"
        ))),
    }
}

/// Parses a manual resolution note from canonical JSON.
pub fn parse_manual_resolution_note(
    json: &serde_json::Value,
) -> CodecResult<events::ManualResolutionNote> {
    Ok(events::ManualResolutionNote::new(required_str(
        json, "text",
    )?)?)
}

/// Parses structured error info from canonical JSON.
pub fn parse_error_info(json: &serde_json::Value) -> CodecResult<events::MfmErrorInfo> {
    let public_details = optional_obj(json, "public_details")?
        .map(|details| {
            Ok::<events::RedactedJson, CodecError>(events::RedactedJson::new(parse_identity(
                required_str(details, "content_digest")?,
            )?))
        })
        .transpose()?;
    let diagnostic_ref = optional_obj(json, "diagnostic_ref")?
        .map(parse_event_artifact)
        .transpose()?;
    let mut error = events::MfmErrorInfo::new(
        events::ErrorCode::new(required_str(json, "code")?)?,
        parse_error_category(required_str(json, "category")?)?,
        required_bool(json, "retryable")?,
        required_str(json, "safe_message")?,
    )?;
    if let Some(public_details) = public_details {
        error = error.with_public_details(public_details)?;
    }
    if let Some(diagnostic_ref) = diagnostic_ref {
        error = error.with_diagnostic_ref(diagnostic_ref)?;
    }
    Ok(error)
}

/// Parses a run completion outcome from canonical JSON.
pub fn parse_run_completion_outcome(
    json: &serde_json::Value,
) -> CodecResult<events::RunCompletionOutcome> {
    match required_str(json, "kind")? {
        "completed" => {
            let evidence = required_obj(json, "public_output")?;
            Ok(events::RunCompletionOutcome::Completed(Box::new(
                events::PublicOutputCompletionEvidence {
                    public_output_schema_id: parse_identity(required_str(
                        evidence,
                        "public_output_schema_id",
                    )?)?,
                    public_output_event_id: parse_identity(required_str(
                        evidence,
                        "public_output_event_id",
                    )?)?,
                },
            )))
        }
        "compensated" => Ok(events::RunCompletionOutcome::Compensated),
        "manually_resolved" => Ok(events::RunCompletionOutcome::ManuallyResolved),
        "failed_without_acdc_claim" => Ok(events::RunCompletionOutcome::FailedWithoutAcdcClaim),
        other => Err(CodecError::Identity(format!(
            "unknown run completion outcome {other}"
        ))),
    }
}

fn parse_fact_descriptor_ref(json: &serde_json::Value) -> Result<spec::FactDescriptorRef> {
    Ok(spec::FactDescriptorRef {
        descriptor_hash: parse_identity(required_str(json, "descriptor_hash")?)?,
    })
}

fn parse_retention_ref(json: &serde_json::Value) -> Result<events::RetentionRef> {
    Ok(events::RetentionRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
    })
}

fn decode_artifact_role_tag(value: &str) -> CodecResult<ArtifactRole> {
    ArtifactRole::parse(value)
        .ok_or_else(|| CodecError::Identity(format!("unknown artifact role {value}")))
}

fn parse_capability_role(value: &str) -> Result<CapabilityRole> {
    match value {
        "read_external" => Ok(CapabilityRole::ReadExternal),
        "managed_platform_write" => Ok(CapabilityRole::ManagedPlatformWrite),
        "support" => Ok(CapabilityRole::Support),
        "external_mutation_authority" => Ok(CapabilityRole::ExternalMutationAuthority),
        other => Err(StoreError::Identity(format!(
            "unknown capability role {other}"
        ))),
    }
}

/// Parses an error category tag.
pub fn parse_error_category(value: &str) -> CodecResult<events::ErrorCategory> {
    match value {
        "planning" => Ok(events::ErrorCategory::Planning),
        "validation" => Ok(events::ErrorCategory::Validation),
        "capability" => Ok(events::ErrorCategory::Capability),
        "side_effect" => Ok(events::ErrorCategory::SideEffect),
        "runtime" => Ok(events::ErrorCategory::Runtime),
        "storage" => Ok(events::ErrorCategory::Storage),
        "cancelled" => Ok(events::ErrorCategory::Cancelled),
        other => Err(CodecError::Identity(format!(
            "unknown error category {other}"
        ))),
    }
}

/// Parses a side-effect failure phase tag.
pub fn parse_failure_phase(value: &str) -> CodecResult<side_effect::FailurePhase> {
    match value {
        "before_invocation_started" => Ok(side_effect::FailurePhase::BeforeInvocationStarted),
        "after_not_submitted_proven" => Ok(side_effect::FailurePhase::AfterNotSubmittedProven),
        other => Err(CodecError::Identity(format!(
            "unknown failure phase {other}"
        ))),
    }
}

fn parse_retention_reason(value: &str) -> Result<events::RetentionReason> {
    match value {
        "run_admitted" => Ok(events::RetentionReason::RunAdmitted),
        "runtime_evidence" => Ok(events::RetentionReason::RuntimeEvidence),
        "public_output" => Ok(events::RetentionReason::PublicOutput),
        "manifest_projection" => Ok(events::RetentionReason::ManifestProjection),
        other => Err(StoreError::Identity(format!(
            "unknown retention reason {other}"
        ))),
    }
}

fn parse_resource_lane_release_authority(
    value: &str,
) -> Result<events::ResourceLaneReleaseAuthority> {
    events::ResourceLaneReleaseAuthority::parse(value).map_err(|error| {
        StoreError::Identity(format!("invalid resource lane release authority: {error}"))
    })
}

fn parse_vec<T>(
    json: &serde_json::Value,
    field: &'static str,
    parser: impl Fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    required_array(json, field)?.iter().map(parser).collect()
}

fn required_array<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> Result<&'a [serde_json::Value]> {
    json.get(field)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| StoreError::Event(format!("missing array field {field}")))
}

fn preconditions_json(preconditions: &CommitPreconditions) -> serde_json::Value {
    let mut absent = preconditions
        .required_absent_logical_keys
        .iter()
        .map(|key| key.as_str())
        .collect::<Vec<_>>();
    absent.sort_unstable();
    let mut present = preconditions
        .required_present_logical_keys
        .iter()
        .map(|key| key.as_str())
        .collect::<Vec<_>>();
    present.sort_unstable();
    let mut cells = preconditions
        .required_cell_states
        .iter()
        .map(|precondition| {
            serde_json::json!({
                "cell_id": precondition.cell_id.as_str(),
                "required": required_cell_state_str(precondition.required),
            })
        })
        .collect::<Vec<_>>();
    cells.sort_by_key(serde_json::Value::to_string);
    let mut side_effects = preconditions
        .required_side_effect_states
        .iter()
        .map(|precondition| {
            serde_json::json!({
                "pair_id": precondition.pair_id.as_str(),
                "required": required_side_effect_state_str(precondition.required),
            })
        })
        .collect::<Vec<_>>();
    side_effects.sort_by_key(serde_json::Value::to_string);
    serde_json::json!({
        "required_absent_logical_keys": absent,
        "required_cell_states": cells,
        "required_present_logical_keys": present,
        "required_public_output_absent": preconditions.required_public_output_absent,
        "required_run_state": required_run_state_str(preconditions.required_run_state),
        "required_side_effect_states": side_effects,
        "certified_run_authority": preconditions.certified_run_authority.as_ref().map(certified_run_authority_json),
    })
}

fn certified_run_authority_json(token: &CertifiedRunStoreAuthority) -> serde_json::Value {
    serde_json::json!({
        "run_id": token.run_id().as_str(),
        "saga_policy_digest": token.saga_policy_digest().as_str(),
        "spec_hash": token.spec_hash().as_str(),
    })
}

fn store_artifact_json(evidence: &ArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "artifact_role": evidence.artifact_role.as_str(),
        "byte_len": evidence.byte_len,
        "digest": evidence.digest.as_str(),
        "media_type": evidence.media_type.as_str(),
        "producer_node_id": evidence.producer_node_id.as_ref().map(NodeId::as_str),
        "producer_seed_id": evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        "schema_id": evidence.schema_id.as_ref().map(SchemaId::as_str),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

/// Encodes an artifact evidence reference as canonical JSON.
pub fn event_artifact_json(evidence: &events::ArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "byte_len": evidence.byte_len,
        "content_digest": evidence.content_digest.as_str(),
        "evidence_hash": evidence.evidence_hash.as_str(),
        "media_type": evidence.media_type.as_str(),
        "role": evidence.role.as_str(),
        "schema_id": evidence.schema_id.as_str(),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

fn fact_claim_json(claim: &mfm_facts::FactClaim) -> serde_json::Value {
    serde_json::json!({
        "visibility": fact_visibility_json(claim.visibility()),
        "fact_kind": claim.fact_kind().as_str(),
        "fact_descriptor_hash": claim.fact_descriptor_hash().as_str(),
        "subject": fact_subject_evidence_json(claim.subject()),
        "observed_at": claim.observed_at(),
        "request": claim.request().map(fact_request_evidence_json),
        "response": fact_response_evidence_json(claim.response()),
        "producer": fact_producer_provenance_json(claim.producer()),
    })
}

fn fact_visibility_json(visibility: &mfm_facts::FactVisibility) -> serde_json::Value {
    match visibility {
        mfm_facts::FactVisibility::RunPrivate => serde_json::json!({
            "kind": "run_private",
        }),
        mfm_facts::FactVisibility::Indexed { audience, scope } => serde_json::json!({
            "kind": "indexed",
            "audience": audience.as_str(),
            "scope": scope.as_str(),
        }),
    }
}

fn fact_subject_evidence_json(evidence: &mfm_facts::FactSubjectEvidence) -> serde_json::Value {
    serde_json::json!({
        "fact_subject_namespace_hash": evidence.fact_subject_namespace_hash().as_str(),
        "subject_material": evidence.subject_material().as_str(),
        "subject_material_hash": evidence.subject_material_hash().as_str(),
        "fact_key": evidence.fact_key().as_str(),
    })
}

fn fact_request_evidence_json(evidence: &mfm_facts::FactRequestEvidence) -> serde_json::Value {
    serde_json::json!({
        "request_schema_id": evidence.request_schema_id().as_str(),
        "request_hash": evidence.request_hash().as_str(),
    })
}

fn fact_response_evidence_json(evidence: &mfm_facts::FactResponseEvidence) -> serde_json::Value {
    serde_json::json!({
        "response_schema_id": evidence.response_schema_id().as_str(),
        "response_hash": evidence.response_hash().as_str(),
        "artifact_id": evidence.artifact_id().as_str(),
        "artifact_evidence_hash": evidence.artifact_evidence_hash().as_str(),
    })
}

fn fact_producer_provenance_json(
    provenance: &mfm_facts::FactProducerProvenance,
) -> serde_json::Value {
    serde_json::json!({
        "capability_kind": provenance.capability_kind().as_str(),
        "capability_version": provenance.capability_version().as_str(),
        "adapter_kind": provenance.adapter_kind().as_str(),
        "adapter_version": provenance.adapter_version().as_str(),
    })
}

fn run_artifact_json(evidence: &events::RunArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "byte_len": evidence.byte_len,
        "content_digest": evidence.content_digest.as_str(),
        "evidence_hash": evidence.evidence_hash.as_str(),
        "media_type": evidence.media_type.as_str(),
        "role": evidence.role.as_str(),
        "schema_id": evidence.schema_id.as_ref().map(SchemaId::as_str),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

fn seed_cell_ref_json(seed: &events::SeedCellRef) -> serde_json::Value {
    serde_json::json!({
        "cell_id": seed.cell_id.as_str(),
        "digest": seed.digest.as_str(),
        "schema_id": seed.schema_id.as_str(),
        "scope_id": seed.scope_id.as_str(),
        "seed_artifact": event_artifact_json(&seed.seed_artifact),
        "seed_id": seed.seed_id.as_str(),
        "semantic_type_id": seed.semantic_type_id.as_str(),
    })
}

fn entry_point_launch_evidence_json(
    evidence: &events::EntryPointLaunchEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "entry_point_registry_digest": evidence.entry_point_registry_digest.as_str(),
        "resolved_op_id": evidence.resolved_op_id.as_str(),
    })
}

fn run_identity_material_json(material: &events::RunIdentityMaterialV1) -> serde_json::Value {
    serde_json::json!({
        "certified_spec_hash": material.certified_spec_hash.as_str(),
        "invocation_key_digest": material.invocation_key_digest.as_str(),
        "store_scope_id": material.store_scope_id.as_str(),
    })
}

fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "cargo_package_digest": identity.cargo_package_digest.as_str(),
        "factory_id": identity.factory_id.as_str(),
        "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(|value| value.as_str()),
        "nix_output_hash": identity.nix_output_hash.as_ref().map(|value| value.as_str()),
    })
}

fn descriptor_identity_json(identity: &DescriptorIdentity) -> serde_json::Value {
    match identity {
        DescriptorIdentity::State(identity) => serde_json::json!({
            "capabilities": capability_set_json(&identity.capabilities),
            "config_schema_id": identity.config_schema_id.as_str(),
            "context": state_context_descriptor_json(&identity.context),
            "descriptor_family": "state",
            "descriptor_id": identity.descriptor_id.as_str(),
            "effect_class": identity.effect_class.as_str(),
            "effect_kind": identity.effect_kind.as_str(),
            "effect_name": identity.effect_name.as_str(),
            "effect_version": identity.effect_version.as_str(),
            "emitted_fact_descriptors": fact_descriptor_refs_json(&identity.emitted_fact_descriptors),
            "input_context": state_input_context_contract_json(&identity.input_context),
            "input_schema_id": identity.input_schema_id.as_str(),
            "name": identity.name.as_str(),
            "output_context": state_output_context_contract_json(&identity.output_context),
            "output_schema_id": identity.output_schema_id.as_str(),
            "output_semantic_type_id": identity.output_semantic_type_id.as_str(),
            "runner": identity.runner.as_str(),
            "side_effect_contract_digest": identity.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
            "state_kind": identity.state_kind.as_str(),
            "state_version": identity.state_version.as_str(),
        }),
        DescriptorIdentity::Operation(identity) => serde_json::json!({
            "config_schema_id": identity.config_schema_id.as_str(),
            "descriptor_family": "operation",
            "descriptor_id": identity.descriptor_id.as_str(),
            "expansion_abi": identity.expansion_abi.as_str(),
            "input_schema_id": identity.input_schema_id.as_str(),
            "name": identity.name.as_str(),
            "operation_kind": identity.operation_kind.as_str(),
            "operation_version": identity.operation_version.as_str(),
            "output_schema_id": identity.output_schema_id.as_str(),
        }),
        DescriptorIdentity::Renderer(identity) => renderer_descriptor_json(identity),
    }
}

fn state_context_descriptor_json(context: &spec::StateContextDescriptorSpec) -> serde_json::Value {
    match context {
        spec::StateContextDescriptorSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateContextDescriptorSpec::Required(requirement) => {
            let spec::StateContextDescriptorRequirementSpec {
                context_descriptor_id,
                schema_id,
                semantic_type_id,
                canonicalizer_identity,
            } = requirement.as_ref();
            serde_json::json!({
                "canonicalizer_identity": canonicalizer_identity.as_str(),
                "context_descriptor_id": context_descriptor_id.as_str(),
                "kind": "required",
                "schema_id": schema_id.as_str(),
                "semantic_type_id": semantic_type_id.as_str(),
            })
        }
    }
}

fn context_producer_json(producer: &spec::ContextProducerSpec) -> serde_json::Value {
    serde_json::json!({
        "producer_descriptor_ids": producer
            .producer_descriptor_ids
            .iter()
            .map(DescriptorId::as_str)
            .collect::<Vec<_>>(),
        "seed_producers_allowed": producer.seed_producers_allowed,
    })
}

fn state_input_context_contract_json(
    contract: &spec::StateInputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateInputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateInputContextContractSpec::Required {
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "kind": "required",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn state_output_context_contract_json(
    contract: &spec::StateOutputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateOutputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateOutputContextContractSpec::Produces {
            resource_kind,
            stage,
        } => serde_json::json!({
            "kind": "produces",
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn cell_context_json(context: &spec::CellContextSpec) -> serde_json::Value {
    match context {
        spec::CellContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "bound",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn fact_descriptor_refs_json(refs: &[spec::FactDescriptorRef]) -> Vec<serde_json::Value> {
    refs.iter()
        .map(|reference| {
            serde_json::json!({
                "descriptor_hash": reference.descriptor_hash.as_str(),
            })
        })
        .collect()
}

fn renderer_descriptor_json(
    identity: &mfm_spec::v1::RendererDescriptorIdentity,
) -> serde_json::Value {
    serde_json::json!({
        "canonicalizer_identity": identity.canonicalizer_identity.as_str(),
        "descriptor_family": "renderer",
        "descriptor_id": identity.descriptor_id.as_str(),
        "public_schema_id": identity.public_schema_id.as_str(),
        "renderer_kind": identity.renderer_kind.as_str(),
        "renderer_version": identity.renderer_version.as_str(),
    })
}

fn capability_set_json(
    descriptor: &mfm_capabilities::CapabilitySetDescriptor,
) -> serde_json::Value {
    serde_json::json!({
        "capabilities": descriptor.capabilities.iter().map(|capability| {
            serde_json::json!({
                "kind": capability.kind.as_str(),
                "name": capability.name.as_str(),
                "role": capability.role.as_str(),
                "version": capability.version.as_str(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn named_cell_ref_json(cell: &events::NamedTypedCellRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": cell.artifact_id.as_str(),
        "cell_id": cell.cell_id.as_str(),
        "content_digest": cell.content_digest.as_str(),
        "evidence_hash": cell.evidence_hash.as_str(),
        "producer": cell_producer_json(&cell.producer),
        "public_field_path": cell.public_field_path.as_str(),
        "schema_id": cell.schema_id.as_str(),
        "scope_id": cell.scope_id.as_str(),
        "semantic_type_id": cell.semantic_type_id.as_str(),
        "value_lineage": value_lineage_json(&cell.value_lineage),
    })
}

fn cell_producer_json(producer: &CellProducer) -> serde_json::Value {
    match producer {
        CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
        CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
    }
}

fn value_lineage_json(lineage: &ValueLineageRef) -> serde_json::Value {
    serde_json::json!({
        "lineage_digest": lineage.lineage_digest.as_str(),
    })
}

/// Encodes a cell skip reason as canonical JSON.
pub fn skip_reason_json(reason: &events::SkipReason) -> serde_json::Value {
    serde_json::json!({
        "code": reason.code.as_str(),
        "safe_message": reason.safe_message.as_str(),
    })
}

/// Encodes a side-effect ledger purpose as canonical JSON.
pub fn side_effect_ledger_purpose_json(
    purpose: &events::SideEffectLedgerPurpose,
) -> serde_json::Value {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => serde_json::json!({
            "kind": "forward",
        }),
        events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
            serde_json::json!({
                "forward_pair_id": forward_pair_id.as_str(),
                "kind": "remediation",
            })
        }
    }
}

/// Encodes exclusive resource-key evidence as canonical JSON.
pub fn resource_key_evidence_json(evidence: &events::ResourceKeyEvidence) -> serde_json::Value {
    serde_json::json!({
        "key": evidence.key.as_str(),
        "key_schema_id": evidence.key_schema_id.as_str(),
        "namespace": evidence.namespace.as_str(),
    })
}

/// Encodes touched-set resource evidence as canonical JSON.
pub fn resource_touched_set_evidence_json(
    evidence: &events::ResourceTouchedSetEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "evidence_artifact_evidence_hash": evidence.evidence_artifact_evidence_hash.as_str(),
        "evidence_artifact_id": evidence.evidence_artifact_id.as_str(),
        "evidence_hash": evidence.evidence_hash.as_str(),
        "evidence_schema_id": evidence.evidence_schema_id.as_str(),
        "namespace": evidence.namespace.as_str(),
    })
}

/// Returns the canonical tag for a manual resolution outcome.
pub fn manual_resolution_outcome_str(outcome: events::ManualResolutionOutcome) -> &'static str {
    outcome.as_str()
}

/// Returns the canonical tag for a run-completion outcome.
pub fn run_completion_outcome_str(outcome: &events::RunCompletionOutcome) -> &'static str {
    outcome.kind()
}

/// Returns the public terminal-claim tag represented by a run-completion outcome.
pub fn run_completion_claim_str(outcome: &events::RunCompletionOutcome) -> &'static str {
    match outcome {
        events::RunCompletionOutcome::Completed(_) => "public_output",
        events::RunCompletionOutcome::Compensated => "compensation",
        events::RunCompletionOutcome::ManuallyResolved => "manual_resolution",
        events::RunCompletionOutcome::FailedWithoutAcdcClaim => "no_acdc_claim",
    }
}

/// Encodes a manual resolution note as canonical JSON.
pub fn manual_resolution_note_json(note: &events::ManualResolutionNote) -> serde_json::Value {
    serde_json::json!({
        "text": note.as_str(),
    })
}

/// Encodes structured error info as canonical JSON.
pub fn error_info_json(error: &events::MfmErrorInfo) -> serde_json::Value {
    serde_json::json!({
        "category": error_category_str(error.category),
        "code": error.code.as_str(),
        "diagnostic_ref": error.diagnostic_ref.as_ref().map(event_artifact_json),
        "public_details": error.public_details.as_ref().map(|details| {
            serde_json::json!({
                "content_digest": details.content_digest.as_str(),
            })
        }),
        "retryable": error.retryable,
        "safe_message": error.safe_message.as_str(),
    })
}

/// Encodes a run completion outcome as canonical JSON.
pub fn run_completion_outcome_json(outcome: &events::RunCompletionOutcome) -> serde_json::Value {
    match outcome {
        events::RunCompletionOutcome::Completed(evidence) => serde_json::json!({
            "kind": outcome.kind(),
            "public_output": {
                "public_output_event_id": evidence.public_output_event_id.as_str(),
                "public_output_schema_id": evidence.public_output_schema_id.as_str(),
            },
        }),
        events::RunCompletionOutcome::Compensated => serde_json::json!({
            "kind": outcome.kind(),
        }),
        events::RunCompletionOutcome::ManuallyResolved => serde_json::json!({
            "kind": outcome.kind(),
        }),
        events::RunCompletionOutcome::FailedWithoutAcdcClaim => serde_json::json!({
            "kind": outcome.kind(),
        }),
    }
}

/// Encodes a run-completion projection as JSON.
pub fn run_completion_projection_json(
    run_id: &RunId,
    projection: &RunCompletionProjection,
) -> serde_json::Value {
    serde_json::json!({
        "event_id": projection.event_id.as_str(),
        "outcome": run_completion_outcome_json(&projection.outcome),
        "run_id": run_id.as_str(),
    })
}

/// Parses a run-completion projection from JSON.
pub fn parse_run_completion_projection(
    json: &serde_json::Value,
) -> CodecResult<(RunId, RunCompletionProjection)> {
    Ok((
        parse_identity(required_str(json, "run_id")?)?,
        RunCompletionProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            outcome: parse_run_completion_outcome(required_obj(json, "outcome")?)?,
        },
    ))
}

/// Encodes a saga-engagement projection as JSON.
pub fn saga_engagement_projection_json(
    run_id: &RunId,
    projection: &SagaEngagementProjection,
) -> serde_json::Value {
    serde_json::json!({
        "event_id": projection.event_id.as_str(),
        "reason": saga_engagement_reason_json(&projection.reason),
        "run_id": run_id.as_str(),
    })
}

/// Parses a saga-engagement projection from JSON.
pub fn parse_saga_engagement_projection(
    json: &serde_json::Value,
) -> CodecResult<(RunId, SagaEngagementProjection)> {
    Ok((
        parse_identity(required_str(json, "run_id")?)?,
        SagaEngagementProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            reason: parse_saga_engagement_reason(required_obj(json, "reason")?)?,
        },
    ))
}

fn saga_engagement_reason_json(reason: &SagaEngagementReason) -> serde_json::Value {
    match reason {
        SagaEngagementReason::NonRetryableFailure {
            node_id,
            attempt_id,
        } => serde_json::json!({
            "attempt_id": attempt_id.as_str(),
            "kind": "non_retryable_failure",
            "node_id": node_id.as_str(),
        }),
        SagaEngagementReason::ForwardAmbiguous { pair_id } => serde_json::json!({
            "kind": "forward_ambiguous",
            "pair_id": pair_id.as_str(),
        }),
    }
}

fn parse_saga_engagement_reason(json: &serde_json::Value) -> CodecResult<SagaEngagementReason> {
    match required_str(json, "kind")? {
        "non_retryable_failure" => Ok(SagaEngagementReason::NonRetryableFailure {
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        }),
        "forward_ambiguous" => Ok(SagaEngagementReason::ForwardAmbiguous {
            pair_id: parse_identity(required_str(json, "pair_id")?)?,
        }),
        other => Err(CodecError::Identity(format!(
            "unknown saga engagement reason {other}"
        ))),
    }
}

/// Encodes a manual-resolution projection as JSON.
pub fn manual_resolution_projection_json(
    run_id: &RunId,
    projection: &ManualResolutionProjection,
) -> serde_json::Value {
    serde_json::json!({
        "authorization_artifact_id": projection.authorization_artifact_id.as_str(),
        "authorization_hash": projection.authorization_hash.as_str(),
        "authorization_schema_id": projection.authorization_schema_id.as_str(),
        "event_id": projection.event_id.as_str(),
        "evidence_artifact_id": projection.evidence_artifact_id.as_str(),
        "evidence_hash": projection.evidence_hash.as_str(),
        "evidence_schema_id": projection.evidence_schema_id.as_str(),
        "note": projection.note.as_ref().map(manual_resolution_note_json),
        "outcome": manual_resolution_outcome_str(projection.outcome),
        "run_id": run_id.as_str(),
    })
}

/// Parses a manual-resolution projection from JSON.
pub fn parse_manual_resolution_projection(
    json: &serde_json::Value,
) -> CodecResult<(RunId, ManualResolutionProjection)> {
    Ok((
        parse_identity(required_str(json, "run_id")?)?,
        ManualResolutionProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            outcome: parse_manual_resolution_outcome(required_str(json, "outcome")?)?,
            evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
            evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
            authorization_schema_id: parse_identity(required_str(
                json,
                "authorization_schema_id",
            )?)?,
            authorization_hash: parse_identity(required_str(json, "authorization_hash")?)?,
            authorization_artifact_id: parse_identity(required_str(
                json,
                "authorization_artifact_id",
            )?)?,
            note: optional_obj(json, "note")?
                .map(parse_manual_resolution_note)
                .transpose()?,
        },
    ))
}

/// Encodes an attempt projection as JSON.
pub fn attempt_projection_json(projection: &AttemptProjection) -> serde_json::Value {
    let status = match &projection.status {
        AttemptStatus::Started {
            attempt_no,
            state_kind,
            state_version,
        } => serde_json::json!({
            "variant": "started",
            "attempt_no": attempt_no,
            "state_kind": state_kind.as_str(),
            "state_version": state_version.as_str(),
        }),
        AttemptStatus::Completed { output_cell_id } => serde_json::json!({
            "variant": "completed",
            "output_cell_id": output_cell_id.as_str(),
        }),
        AttemptStatus::Failed { retryable, error } => serde_json::json!({
            "variant": "failed",
            "retryable": retryable,
            "error": error_info_json(error),
        }),
        AttemptStatus::Interrupted => serde_json::json!({
            "variant": "interrupted",
        }),
    };
    serde_json::json!({
        "attempt_id": projection.attempt_id.as_str(),
        "event_id": projection.event_id.as_str(),
        "node_id": projection.node_id.as_str(),
        "run_id": projection.run_id.as_str(),
        "status": status,
    })
}

/// Parses an attempt projection from JSON.
pub fn parse_attempt_projection(json: &serde_json::Value) -> CodecResult<AttemptProjection> {
    let status_json = required_obj(json, "status")?;
    let status = match required_str(status_json, "variant")? {
        "started" => AttemptStatus::Started {
            attempt_no: required_u64(status_json, "attempt_no")?
                .try_into()
                .map_err(|_| CodecError::Field("attempt_no overflow".into()))?,
            state_kind: parse_identity(required_str(status_json, "state_kind")?)?,
            state_version: required_str(status_json, "state_version")?.parse()?,
        },
        "completed" => AttemptStatus::Completed {
            output_cell_id: parse_identity(required_str(status_json, "output_cell_id")?)?,
        },
        "failed" => AttemptStatus::Failed {
            retryable: required_bool(status_json, "retryable")?,
            error: Box::new(parse_error_info(required_obj(status_json, "error")?)?),
        },
        "interrupted" => AttemptStatus::Interrupted,
        other => {
            return Err(CodecError::Identity(format!(
                "unknown attempt status {other}"
            )));
        }
    };
    Ok(AttemptProjection {
        run_id: parse_identity(required_str(json, "run_id")?)?,
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        event_id: parse_identity(required_str(json, "event_id")?)?,
        status,
    })
}

/// Encodes a cell terminal projection as JSON.
pub fn cell_projection_json(
    run_id: &RunId,
    cell_id: &CellId,
    projection: &CellTerminalProjection,
) -> serde_json::Value {
    match projection {
        CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            evidence_hash,
        } => serde_json::json!({
            "variant": "produced",
            "run_id": run_id.as_str(),
            "cell_id": cell_id.as_str(),
            "event_id": event_id.as_str(),
            "node_id": node_id.as_str(),
            "attempt_id": attempt_id.as_str(),
            "schema_id": schema_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
            "artifact_id": artifact_id.as_str(),
            "content_digest": content_digest.as_str(),
            "evidence_hash": evidence_hash.as_str(),
        }),
        CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        } => serde_json::json!({
            "variant": "skipped",
            "run_id": run_id.as_str(),
            "cell_id": cell_id.as_str(),
            "event_id": event_id.as_str(),
            "node_id": node_id.as_str(),
            "attempt_id": attempt_id.as_str(),
            "schema_id": schema_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
            "skip_reason": skip_reason_json(skip_reason),
        }),
    }
}

/// Parses a cell terminal projection from JSON.
pub fn parse_cell_projection(
    json: &serde_json::Value,
) -> CodecResult<((RunId, CellId), CellTerminalProjection)> {
    let run_id = parse_identity(required_str(json, "run_id")?)?;
    let cell_id = parse_identity(required_str(json, "cell_id")?)?;
    let projection = match required_str(json, "variant")? {
        "produced" => CellTerminalProjection::Produced {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        },
        "skipped" => CellTerminalProjection::Skipped {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            skip_reason: parse_skip_reason(required_obj(json, "skip_reason")?)?,
        },
        other => {
            return Err(CodecError::Identity(format!(
                "unknown cell projection {other}"
            )));
        }
    };
    Ok(((run_id, cell_id), projection))
}

/// Encodes a side-effect projection as JSON.
pub fn side_effect_projection_json(projection: &SideEffectProjection) -> serde_json::Value {
    serde_json::json!({
        "claim": projection.claim.as_ref().map(side_effect_claim_json),
        "confirmation": projection.confirmation.as_ref().map(side_effect_artifact_json),
        "event_id": projection.event_id.as_str(),
        "intent": side_effect_intent_json(&projection.intent),
        "ledger_key": projection.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&projection.ledger_purpose),
        "pair_id": projection.pair_id.as_str(),
        "phase": side_effect_phase_json(&projection.phase),
        "prepared_invocation": projection.prepared_invocation.as_ref().map(side_effect_artifact_json),
        "receipt": projection.receipt.as_ref().map(side_effect_artifact_json),
        "resource_key": projection.resource_key.as_ref().map(resource_key_evidence_json),
        "resource_touched_set": projection.resource_touched_set.as_ref().map(resource_touched_set_evidence_json),
        "run_id": projection.run_id.as_str(),
        "submission": projection.submission.as_ref().map(side_effect_artifact_json),
    })
}

/// Parses a side-effect projection from JSON.
pub fn parse_side_effect_projection(json: &serde_json::Value) -> CodecResult<SideEffectProjection> {
    Ok(SideEffectProjection {
        run_id: parse_identity(required_str(json, "run_id")?)?,
        ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
        ledger_purpose: parse_side_effect_ledger_purpose(required_obj(json, "ledger_purpose")?)?,
        pair_id: parse_identity(required_str(json, "pair_id")?)?,
        event_id: parse_identity(required_str(json, "event_id")?)?,
        intent: parse_side_effect_intent(required_obj(json, "intent")?)?,
        prepared_invocation: optional_obj(json, "prepared_invocation")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        resource_key: optional_obj(json, "resource_key")?
            .map(parse_resource_key_evidence)
            .transpose()?,
        submission: optional_obj(json, "submission")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        receipt: optional_obj(json, "receipt")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        confirmation: optional_obj(json, "confirmation")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        resource_touched_set: optional_obj(json, "resource_touched_set")?
            .map(parse_resource_touched_set_evidence)
            .transpose()?,
        claim: optional_obj(json, "claim")?
            .map(parse_side_effect_claim)
            .transpose()?,
        phase: parse_side_effect_phase(required_obj(json, "phase")?)?,
    })
}

fn side_effect_artifact_json(artifact: &SideEffectArtifactProjection) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": artifact.artifact_id.as_str(),
        "content_digest": artifact.content_digest.as_str(),
        "evidence_hash": artifact.evidence_hash.as_str(),
        "schema_id": artifact.schema_id.as_ref().map(SchemaId::as_str),
    })
}

fn parse_side_effect_artifact(
    json: &serde_json::Value,
) -> CodecResult<SideEffectArtifactProjection> {
    Ok(SideEffectArtifactProjection {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        schema_id: optional_str(json, "schema_id")?
            .map(parse_identity)
            .transpose()?,
    })
}

/// Encodes a resource-lane projection as JSON.
pub fn resource_lane_projection_json(
    lane_key: &ResourceLaneKey,
    projection: &ResourceLaneProjection,
) -> serde_json::Value {
    serde_json::json!({
        "attempt_id": projection.attempt_id.as_str(),
        "claim_fencing_token": projection.claim_fencing_token,
        "claim_id": projection.claim_id.as_str(),
        "event_id": projection.event_id.as_str(),
        "invocation_epoch": projection.invocation_epoch,
        "key": lane_key.key.as_str(),
        "key_schema_id": lane_key.key_schema_id.as_str(),
        "lane_transition_seq": projection.lane_transition_seq,
        "ledger_key": projection.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&projection.ledger_purpose),
        "namespace": lane_key.namespace.as_str(),
        "node_id": projection.node_id.as_str(),
        "pair_id": projection.holder.pair_id.as_str(),
        "run_id": projection.holder.run_id.as_str(),
    })
}

/// Parses a resource-lane projection from JSON.
pub fn parse_resource_lane_projection(
    json: &serde_json::Value,
) -> CodecResult<(ResourceLaneKey, ResourceLaneProjection)> {
    let lane_key = ResourceLaneKey {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        key_schema_id: parse_identity(required_str(json, "key_schema_id")?)?,
        key: events::ResourceKey::new(required_str(json, "key")?)?,
    };
    let projection = ResourceLaneProjection {
        event_id: parse_identity(required_str(json, "event_id")?)?,
        holder: SideEffectPairLedgerRef::new(
            parse_identity(required_str(json, "run_id")?)?,
            parse_identity(required_str(json, "pair_id")?)?,
        ),
        ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
        ledger_purpose: parse_side_effect_ledger_purpose(required_obj(json, "ledger_purpose")?)?,
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
        claim_fencing_token: required_u64(json, "claim_fencing_token")?,
        lane_transition_seq: required_u64(json, "lane_transition_seq")?,
    };
    Ok((lane_key, projection))
}

fn side_effect_intent_json(intent: &SideEffectIntentProjection) -> serde_json::Value {
    serde_json::json!({
        "adapter_kind": intent.adapter_kind.as_str(),
        "adapter_version": intent.adapter_version.as_str(),
        "attempt_id": intent.attempt_id.as_str(),
        "capability_kind": intent.capability_kind.as_str(),
        "capability_version": intent.capability_version.as_str(),
        "idempotency_input_hash": intent.idempotency_input_hash.as_str(),
        "idempotency_input_schema_id": intent.idempotency_input_schema_id.as_str(),
        "idempotency_key": intent.idempotency_key.as_str(),
        "intent_artifact_id": intent.intent_artifact_id.as_str(),
        "intent_hash": intent.intent_hash.as_str(),
        "intent_schema_id": intent.intent_schema_id.as_str(),
        "invocation_epoch": intent.invocation_epoch,
        "node_id": intent.node_id.as_str(),
        "scope_id": intent.scope_id.as_str(),
    })
}

fn parse_side_effect_intent(json: &serde_json::Value) -> CodecResult<SideEffectIntentProjection> {
    Ok(SideEffectIntentProjection {
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        intent_schema_id: parse_identity(required_str(json, "intent_schema_id")?)?,
        intent_hash: parse_identity(required_str(json, "intent_hash")?)?,
        intent_artifact_id: parse_identity(required_str(json, "intent_artifact_id")?)?,
        idempotency_input_schema_id: parse_identity(required_str(
            json,
            "idempotency_input_schema_id",
        )?)?,
        idempotency_input_hash: parse_identity(required_str(json, "idempotency_input_hash")?)?,
        idempotency_key: events::IdempotencyKeyRef::new(required_str(json, "idempotency_key")?)?,
        capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
        capability_version: required_str(json, "capability_version")?.parse()?,
        adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
        adapter_version: required_str(json, "adapter_version")?.parse()?,
    })
}

fn side_effect_claim_json(claim: &SideEffectClaimProjection) -> serde_json::Value {
    serde_json::json!({
        "attempt_id": claim.attempt_id.as_str(),
        "claim_fencing_token": claim.claim_fencing_token.as_str(),
        "claim_generation": claim.claim_generation,
        "claim_owner": claim.claim_owner.as_str(),
        "invocation_epoch": claim.invocation_epoch,
        "node_id": claim.node_id.as_str(),
    })
}

fn parse_side_effect_claim(json: &serde_json::Value) -> CodecResult<SideEffectClaimProjection> {
    Ok(SideEffectClaimProjection {
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        claim_generation: required_u32(json, "claim_generation")?,
        claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
            json,
            "claim_fencing_token",
        )?)?,
    })
}

fn side_effect_phase_json(phase: &SideEffectPhase) -> serde_json::Value {
    match phase {
        SideEffectPhase::IntentPersisted { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Claimed {
            claim_owner,
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "claim_owner": claim_owner.as_str(),
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::InvocationPrepared {
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::InvocationStarted {
            claim_owner,
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "claim_owner": claim_owner.as_str(),
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::SubmissionObserved { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::NotSubmittedProven { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::SubmissionUnknown { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::ReceiptObserved { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::ConfirmationObserved { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Ambiguous { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Failed {
            invocation_epoch,
            failure_phase,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "failure_phase": failure_phase_str(*failure_phase),
            }),
        ),
    }
}

fn phase_json(
    variant: &'static str,
    invocation_epoch: u32,
    mut extra: serde_json::Value,
) -> serde_json::Value {
    extra["variant"] = serde_json::json!(variant);
    extra["invocation_epoch"] = serde_json::json!(invocation_epoch);
    extra
}

fn parse_side_effect_phase(json: &serde_json::Value) -> CodecResult<SideEffectPhase> {
    let invocation_epoch = required_u32(json, "invocation_epoch")?;
    match required_str(json, "variant")? {
        "intent_persisted" => Ok(SideEffectPhase::IntentPersisted { invocation_epoch }),
        "claimed" => Ok(SideEffectPhase::Claimed {
            claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "invocation_prepared" => Ok(SideEffectPhase::InvocationPrepared {
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "invocation_started" => Ok(SideEffectPhase::InvocationStarted {
            claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "submission_observed" => Ok(SideEffectPhase::SubmissionObserved { invocation_epoch }),
        "not_submitted_proven" => Ok(SideEffectPhase::NotSubmittedProven { invocation_epoch }),
        "submission_unknown" => Ok(SideEffectPhase::SubmissionUnknown { invocation_epoch }),
        "receipt_observed" => Ok(SideEffectPhase::ReceiptObserved { invocation_epoch }),
        "confirmation_observed" => Ok(SideEffectPhase::ConfirmationObserved { invocation_epoch }),
        "ambiguous" => Ok(SideEffectPhase::Ambiguous { invocation_epoch }),
        "failed" => Ok(SideEffectPhase::Failed {
            invocation_epoch,
            failure_phase: parse_failure_phase(required_str(json, "failure_phase")?)?,
        }),
        other => Err(CodecError::Identity(format!(
            "unknown side-effect phase {other}"
        ))),
    }
}

/// Encodes a public-output projection as JSON.
pub fn public_output_projection_json(
    run_id: &RunId,
    schema_id: &SchemaId,
    projection: &PublicOutputProjection,
) -> serde_json::Value {
    match projection {
        PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        } => serde_json::json!({
            "variant": "produced",
            "run_id": run_id.as_str(),
            "public_schema_id": schema_id.as_str(),
            "event_id": event_id.as_str(),
            "rendered_digest": rendered_digest.as_str(),
            "rendered_artifact_id": rendered_artifact_id.as_ref().map(ArtifactId::as_str),
        }),
        PublicOutputProjection::RenderFailed { event_id, error } => serde_json::json!({
            "variant": "render_failed",
            "run_id": run_id.as_str(),
            "public_schema_id": schema_id.as_str(),
            "event_id": event_id.as_str(),
            "error": error_info_json(error),
        }),
    }
}

/// Parses a public-output projection from JSON.
pub fn parse_public_output_projection(
    json: &serde_json::Value,
) -> CodecResult<((RunId, SchemaId), PublicOutputProjection)> {
    let run_id = parse_identity(required_str(json, "run_id")?)?;
    let schema_id = parse_identity(required_str(json, "public_schema_id")?)?;
    let projection = match required_str(json, "variant")? {
        "produced" => PublicOutputProjection::Produced {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            rendered_digest: parse_identity(required_str(json, "rendered_digest")?)?,
            rendered_artifact_id: optional_str(json, "rendered_artifact_id")?
                .map(parse_identity)
                .transpose()?,
        },
        "render_failed" => PublicOutputProjection::RenderFailed {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            error: Box::new(parse_error_info(required_obj(json, "error")?)?),
        },
        other => {
            return Err(CodecError::Identity(format!(
                "unknown public output projection {other}"
            )));
        }
    };
    Ok(((run_id, schema_id), projection))
}

/// Encodes a retention manifest projection as JSON.
pub fn retention_manifest_projection_json(
    manifest: &RetentionManifestProjection,
) -> serde_json::Value {
    serde_json::json!({
        "manifest_artifact_id": manifest.manifest_artifact_id.as_str(),
        "manifest_digest": manifest.manifest_digest.as_str(),
        "previous_manifest_digest": manifest.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
        "manifest_seq": manifest.manifest_seq,
    })
}

/// Parses a retention manifest projection from JSON.
pub fn parse_retention_manifest_projection(
    json: &serde_json::Value,
) -> CodecResult<RetentionManifestProjection> {
    Ok(RetentionManifestProjection {
        manifest_seq: required_u64(json, "manifest_seq")?,
        manifest_digest: parse_identity(required_str(json, "manifest_digest")?)?,
        previous_manifest_digest: optional_str(json, "previous_manifest_digest")?
            .map(parse_identity)
            .transpose()?,
        manifest_artifact_id: parse_identity(required_str(json, "manifest_artifact_id")?)?,
    })
}

/// Returns the canonical run-state projection tag.
pub fn run_state_str(state: RunState) -> &'static str {
    match state {
        RunState::Absent => "absent",
        RunState::Started => "started",
        RunState::Completed => "completed",
    }
}

/// Parses a run-state projection tag.
pub fn parse_run_state(value: &str) -> CodecResult<RunState> {
    match value {
        "absent" => Ok(RunState::Absent),
        "started" => Ok(RunState::Started),
        "completed" => Ok(RunState::Completed),
        other => Err(CodecError::Identity(format!("unknown run state {other}"))),
    }
}

/// Encodes a retention reference as canonical JSON.
pub fn retention_ref_json(retention_ref: &events::RetentionRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": retention_ref.artifact_id.as_str(),
        "content_digest": retention_ref.content_digest.as_str(),
        "evidence_hash": retention_ref.evidence_hash.as_str(),
        "role": retention_ref.role.as_str(),
    })
}

fn required_run_state_str(state: RequiredRunState) -> &'static str {
    match state {
        RequiredRunState::Any => "any",
        RequiredRunState::Absent => "absent",
        RequiredRunState::Started => "started",
        RequiredRunState::NotCompleted => "not_completed",
        RequiredRunState::Completed => "completed",
    }
}

fn required_cell_state_str(state: RequiredCellState) -> &'static str {
    match state {
        RequiredCellState::Absent => "absent",
        RequiredCellState::Produced => "produced",
        RequiredCellState::Skipped => "skipped",
        RequiredCellState::Terminal => "terminal",
    }
}

fn required_side_effect_state_str(state: RequiredSideEffectState) -> &'static str {
    match state {
        RequiredSideEffectState::Absent => "absent",
        RequiredSideEffectState::IntentPersisted => "intent_persisted",
        RequiredSideEffectState::Claimed => "claimed",
        RequiredSideEffectState::InvocationPrepared => "invocation_prepared",
        RequiredSideEffectState::InvocationStarted => "invocation_started",
        RequiredSideEffectState::SubmissionResult => "submission_result",
        RequiredSideEffectState::ReceiptObserved => "receipt_observed",
        RequiredSideEffectState::ConfirmationObserved => "confirmation_observed",
        RequiredSideEffectState::Ambiguous => "ambiguous",
        RequiredSideEffectState::Failed => "failed",
    }
}

/// Returns the canonical tag for an error category.
pub fn error_category_str(category: events::ErrorCategory) -> &'static str {
    match category {
        events::ErrorCategory::Planning => "planning",
        events::ErrorCategory::Validation => "validation",
        events::ErrorCategory::Capability => "capability",
        events::ErrorCategory::SideEffect => "side_effect",
        events::ErrorCategory::Runtime => "runtime",
        events::ErrorCategory::Storage => "storage",
        events::ErrorCategory::Cancelled => "cancelled",
    }
}

/// Returns the canonical tag for a side-effect failure phase.
pub fn failure_phase_str(phase: side_effect::FailurePhase) -> &'static str {
    match phase {
        side_effect::FailurePhase::BeforeInvocationStarted => "before_invocation_started",
        side_effect::FailurePhase::AfterNotSubmittedProven => "after_not_submitted_proven",
    }
}

fn retention_reason_str(reason: events::RetentionReason) -> &'static str {
    match reason {
        events::RetentionReason::RunAdmitted => "run_admitted",
        events::RetentionReason::RuntimeEvidence => "runtime_evidence",
        events::RetentionReason::PublicOutput => "public_output",
        events::RetentionReason::ManifestProjection => "manifest_projection",
    }
}
