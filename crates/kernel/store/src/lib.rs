#![warn(missing_docs)]
//! Typed kernel store contracts for MFM.
//!
//! The store commit surface accepts typed event payload batches plus typed
//! preconditions. Stores own envelopes, stream sequence numbers, ordinals,
//! event ids, logical keys, commit-key idempotency entries, and projection
//! writes.
//!
//! ```compile_fail
//! use mfm_store::v1::KernelEventEnvelope;
//!
//! // Envelopes are store-owned. Callers cannot construct forged event ids,
//! // stream sequence numbers, ordinals, logical keys, or projection writes.
//! let _forged = KernelEventEnvelope {};
//! ```

/// Versioned v1 typed store contract.
pub mod v1 {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt;
    use std::future::Future;
    use std::pin::Pin;

    use mfm_canonical::PlainCanonicalJsonBytes;
    use mfm_capabilities::{CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor};
    use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
        CellId, ContentDigest, DigestAlgorithm, EventId, IdentityError, NodeId, RunId, SchemaId,
        ScopeId, SeedId, SemanticTypeId, SpecHash, StateKind, StateVersion,
    };
    use mfm_spec::v1::{
        CanonicalizerIdentity, CellProducer, DescriptorIdentity, ManualResolutionEvidenceSpec,
        MediaType, OperationDescriptorIdentity, PublicFieldPath, RemediationUnresolvedSpec,
        RendererDescriptorIdentity, RendererKind, RendererVersion, ResourceNamespace,
        SagaPolicySpec, StateDescriptorIdentity, ValueLineageRef,
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
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum StoreError {
        /// A commit contained no payloads.
        EmptyCommit,
        /// Stream sequence arithmetic overflowed.
        SequenceOverflow,
        /// A payload was bound to a different run.
        PayloadRunMismatch {
            /// Run supplied to the commit API.
            expected: Box<RunId>,
            /// Run found inside the payload.
            actual: Box<RunId>,
        },
        /// Payloads in one commit carried different certified spec hashes.
        PayloadSpecHashMismatch {
            /// First payload spec hash.
            expected: Box<SpecHash>,
            /// Later payload spec hash.
            actual: Box<SpecHash>,
        },
        /// The commit key was reused for a different canonical commit.
        CommitConflict {
            /// Reused commit key.
            commit_key: CommitKey,
        },
        /// The caller's expected next sequence is stale.
        StaleExpectedNextSeq {
            /// Expected sequence supplied by the caller.
            expected: StreamSeq,
            /// Actual next sequence owned by the store.
            actual: StreamSeq,
        },
        /// A required logical key precondition failed.
        LogicalKeyPreconditionFailed {
            /// Logical key that failed the precondition.
            logical_key: LogicalEventKey,
            /// Stable diagnostic.
            message: String,
        },
        /// A run-state precondition failed.
        RunStatePreconditionFailed {
            /// Required run state.
            required: RequiredRunState,
            /// Actual run state.
            actual: RunState,
        },
        /// A cell-state precondition failed.
        CellStatePreconditionFailed {
            /// Cell id that failed the precondition.
            cell_id: CellId,
            /// Required cell state.
            required: RequiredCellState,
        },
        /// A side-effect-state precondition failed.
        SideEffectStatePreconditionFailed {
            /// Ledger key that failed the precondition.
            ledger_key: events::SideEffectLedgerKey,
            /// Required side-effect state.
            required: RequiredSideEffectState,
        },
        /// A public output was already projected when absence was required.
        PublicOutputPreconditionFailed,
        /// A referenced artifact is missing from the store-owned artifact evidence table.
        MissingArtifact {
            /// Missing artifact id.
            artifact_id: ArtifactId,
        },
        /// Stored artifact evidence does not match a required artifact reference.
        ArtifactEvidenceMismatch {
            /// Artifact id with mismatched evidence.
            artifact_id: ArtifactId,
            /// Mismatched field label.
            field: &'static str,
        },
        /// A prepared commit tried to admit artifact evidence that no event in the commit
        /// references.
        UnreferencedArtifactEvidence {
            /// Unreferenced artifact id.
            artifact_id: ArtifactId,
        },
        /// A logical key that must be unique already exists.
        DuplicateLogicalKey {
            /// Duplicate logical key.
            logical_key: LogicalEventKey,
        },
        /// A logical key conflict would corrupt an existing projection.
        LogicalKeyConflict {
            /// Conflicting logical key.
            logical_key: LogicalEventKey,
        },
        /// A projection transition would corrupt an existing projection.
        ProjectionConflict {
            /// Projection key.
            key: String,
            /// Stable diagnostic.
            message: String,
        },
        /// An exclusive resource lane is held by a different run-scoped ledger.
        ResourceLaneBlocked {
            /// Blocked resource lane.
            lane_key: Box<ResourceLaneKey>,
            /// Current holder of the lane.
            holder: Box<SideEffectLedgerRef>,
        },
        /// A persisted event row disagrees with store-derived typed event fields.
        PersistedEventMismatch {
            /// Mismatched field label.
            field: &'static str,
            /// Stable diagnostic.
            message: String,
        },
        /// Identity construction failed.
        Identity(String),
        /// JSON serialization failed before canonicalization.
        Serialize(String),
        /// Canonical JSON construction failed.
        Canonical(String),
        /// Event schema id construction failed.
        Event(String),
    }

    impl fmt::Display for StoreError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::EmptyCommit => f.write_str("typed commit cannot be empty"),
                Self::SequenceOverflow => f.write_str("typed stream sequence overflowed"),
                Self::PayloadRunMismatch { expected, actual } => {
                    write!(f, "payload run mismatch: expected {expected}, got {actual}")
                }
                Self::PayloadSpecHashMismatch { expected, actual } => {
                    write!(
                        f,
                        "payload spec hash mismatch: expected {expected}, got {actual}"
                    )
                }
                Self::CommitConflict { commit_key } => {
                    write!(f, "commit key reused for different payloads: {commit_key}")
                }
                Self::StaleExpectedNextSeq { expected, actual } => write!(
                    f,
                    "stale expected_next_seq: expected {expected}, actual {actual}"
                ),
                Self::LogicalKeyPreconditionFailed {
                    logical_key,
                    message,
                } => write!(
                    f,
                    "logical key precondition failed for {logical_key}: {message}"
                ),
                Self::RunStatePreconditionFailed { required, actual } => {
                    write!(
                        f,
                        "run state precondition failed: required {required:?}, actual {actual:?}"
                    )
                }
                Self::CellStatePreconditionFailed { cell_id, required } => write!(
                    f,
                    "cell state precondition failed for {cell_id}: required {required:?}"
                ),
                Self::SideEffectStatePreconditionFailed {
                    ledger_key,
                    required,
                } => write!(
                    f,
                    "side-effect state precondition failed for {ledger_key}: required {required:?}"
                ),
                Self::PublicOutputPreconditionFailed => {
                    f.write_str("public output absence precondition failed")
                }
                Self::MissingArtifact { artifact_id } => {
                    write!(f, "missing artifact evidence for {artifact_id}")
                }
                Self::ArtifactEvidenceMismatch { artifact_id, field } => {
                    write!(
                        f,
                        "artifact evidence mismatch for {artifact_id} field {field}"
                    )
                }
                Self::UnreferencedArtifactEvidence { artifact_id } => {
                    write!(
                        f,
                        "prepared commit admitted unreferenced artifact evidence {artifact_id}"
                    )
                }
                Self::DuplicateLogicalKey { logical_key } => {
                    write!(f, "duplicate logical key {logical_key}")
                }
                Self::LogicalKeyConflict { logical_key } => {
                    write!(f, "logical key conflict {logical_key}")
                }
                Self::ProjectionConflict { key, message } => {
                    write!(f, "projection conflict for {key}: {message}")
                }
                Self::ResourceLaneBlocked { lane_key, holder } => write!(
                    f,
                    "resource lane {}:{} is held by run {} ledger {}",
                    lane_key.namespace, lane_key.key, holder.run_id, holder.ledger_key
                ),
                Self::PersistedEventMismatch { field, message } => {
                    write!(f, "persisted event mismatch for {field}: {message}")
                }
                Self::Identity(message) => write!(f, "identity error: {message}"),
                Self::Serialize(message) => write!(f, "store JSON serialization error: {message}"),
                Self::Canonical(message) => write!(f, "store canonicalization error: {message}"),
                Self::Event(message) => write!(f, "event contract error: {message}"),
            }
        }
    }

    impl std::error::Error for StoreError {}

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
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum CodecError {
        /// A required JSON field was missing or had the wrong shape.
        Field(String),
        /// A typed identity, digest, or enum tag failed to parse.
        Identity(String),
    }

    /// Shared canonical-JSON codec for kernel events, projections, and saga types.
    ///
    /// This module exists so the in-memory store and the Postgres adapter share one
    /// implementation of the JSON parse/encode logic instead of maintaining parallel copies kept
    /// in lockstep by parity tests. Functions return the backend-neutral [`CodecError`], which each
    /// store maps into its own error type via `From`.
    pub mod codec;

    fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
        let json = serde_json::to_string(&value)
            .map_err(|error| StoreError::Serialize(error.to_string()))?;
        PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| StoreError::Canonical(error.to_string()))
    }

    fn checked_ascii_key(field: &'static str, value: impl AsRef<str>) -> Result<String> {
        let value = value.as_ref();
        if value.is_empty()
            || value.len() > 512
            || !value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
        {
            return Err(StoreError::Identity(format!(
                "{field} must be 1..=512 visible ASCII bytes"
            )));
        }
        Ok(value.to_owned())
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
    pub struct CommitKey(String);

    impl CommitKey {
        /// Creates a checked commit key.
        pub fn new(value: impl AsRef<str>) -> Result<Self> {
            checked_ascii_key("commit key", value).map(Self)
        }

        /// Returns the persisted commit key string.
        pub fn as_str(&self) -> &str {
            &self.0
        }
    }

    impl fmt::Display for CommitKey {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.as_str())
        }
    }

    /// Store-derived logical event key used for duplicate/conflict checks.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct LogicalEventKey(String);

    impl LogicalEventKey {
        /// Creates a checked logical key for typed preconditions.
        pub fn new(value: impl AsRef<str>) -> Result<Self> {
            checked_ascii_key("logical event key", value).map(Self)
        }

        /// Returns the persisted logical key string.
        pub fn as_str(&self) -> &str {
            &self.0
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
    }

    impl fmt::Display for CommitFingerprint {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    /// Store-owned audit metadata carried beside an event payload.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct KernelEventAudit {
        store_contract_version: &'static str,
        payload_canonical_byte_len: u64,
    }

    impl KernelEventAudit {
        fn new(payload_canonical_byte_len: u64) -> Self {
            Self {
                store_contract_version: "mfm.store.v1",
                payload_canonical_byte_len,
            }
        }

        /// Returns the store contract version that produced the envelope.
        pub const fn store_contract_version(&self) -> &'static str {
            self.store_contract_version
        }

        /// Returns the canonical payload byte length used for hashing.
        pub const fn payload_canonical_byte_len(&self) -> u64 {
            self.payload_canonical_byte_len
        }
    }

    /// Store-owned typed event envelope.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct KernelEventEnvelope {
        event_id: EventId,
        event_schema_id: SchemaId,
        run_id: RunId,
        seq: StreamSeq,
        ordinal: CommitOrdinal,
        spec_hash: SpecHash,
        commit_key: CommitKey,
        logical_key: LogicalEventKey,
        payload_hash: ContentDigest,
        payload: KernelEventPayload,
        audit: KernelEventAudit,
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
        /// Persisted canonical payload byte length.
        pub payload_canonical_byte_len: u64,
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
            let derived_byte_len = canonical_payload.as_bytes().len() as u64;
            if derived_byte_len != record.payload_canonical_byte_len {
                return Err(StoreError::PersistedEventMismatch {
                    field: "payload_canonical_byte_len",
                    message: "persisted payload byte length does not match canonical payload"
                        .to_owned(),
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
            let derived_logical_key = derive_logical_key(&record.payload, &record.payload_hash)?;
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
                ordinal: record.ordinal,
                spec_hash: record.spec_hash,
                commit_key: record.commit_key,
                logical_key: record.logical_key,
                payload_hash: record.payload_hash,
                payload: record.payload,
                audit: KernelEventAudit::new(record.payload_canonical_byte_len),
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

        /// Store-owned audit metadata.
        pub fn audit(&self) -> &KernelEventAudit {
            &self.audit
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
                events::RunCompletionOutcome::FailedWithoutAcdcClaim => {
                    Self::FailedWithoutAcdcClaim
                }
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
        /// A forward side-effect ledger became ambiguous.
        ForwardAmbiguous {
            /// Forward ledger key.
            ledger_key: events::SideEffectLedgerKey,
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
        /// The ledger has not reached a quiescent classification yet.
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
        /// Derived forward-ledger obligation state.
        pub obligations: BTreeMap<events::SideEffectLedgerKey, SagaObligationProjection>,
        /// Manual resolution evidence recorded for the run, if any.
        pub manual_resolution: Option<ManualResolutionProjection>,
        /// Run completion recorded for the run, if any.
        pub run_completion: Option<RunCompletionProjection>,
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
        /// Side-effect ledger key checked by this precondition.
        pub ledger_key: events::SideEffectLedgerKey,
        /// Required side-effect phase.
        pub required: RequiredSideEffectState,
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
        /// Certified saga policy used by store admission for saga manual and terminal events.
        pub saga_policy: Option<SagaPolicySpec>,
    }

    /// Payload-level typed commit request.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TypedCommitRequest {
        /// Run id to append to.
        pub run_id: RunId,
        /// Caller's expected next store-owned stream sequence.
        pub expected_next_seq: StreamSeq,
        /// Commit key for idempotency.
        pub commit_key: CommitKey,
        /// Ordered typed event payloads.
        pub payloads: Vec<KernelEventPayload>,
        /// Artifact evidence refs required for the commit.
        pub required_artifacts: Vec<ArtifactEvidenceRef>,
        /// Atomic commit preconditions.
        pub preconditions: CommitPreconditions,
    }

    /// Runtime-prepared atomic store mutation for typed run streams.
    ///
    /// A prepared commit carries both the event payload batch and the artifact evidence that must
    /// become run authority with that batch. Stores admit the artifact evidence and append the
    /// referencing events in one atomic mutation.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PreparedTypedCommit {
        request: TypedCommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    }

    impl PreparedTypedCommit {
        /// Prepares an atomic typed commit from a validated payload request and artifact evidence.
        ///
        /// Every admitted artifact must be referenced by the commit request. The store still owns
        /// final validation against existing artifact evidence, logical keys, preconditions, and
        /// projection transitions.
        pub fn new(
            request: TypedCommitRequest,
            admitted_artifacts: Vec<ArtifactEvidenceRef>,
        ) -> Result<Self> {
            let referenced_artifacts = referenced_artifact_ids(&request);
            for evidence in &request.required_artifacts {
                if !referenced_artifacts.contains(&evidence.artifact_id) {
                    return Err(StoreError::UnreferencedArtifactEvidence {
                        artifact_id: evidence.artifact_id.clone(),
                    });
                }
            }
            let mut deduped = BTreeMap::<ArtifactId, ArtifactEvidenceRef>::new();
            for evidence in admitted_artifacts {
                if !referenced_artifacts.contains(&evidence.artifact_id) {
                    return Err(StoreError::UnreferencedArtifactEvidence {
                        artifact_id: evidence.artifact_id,
                    });
                }
                if let Some(existing) = deduped.get(&evidence.artifact_id) {
                    if existing != &evidence {
                        return Err(StoreError::ArtifactEvidenceMismatch {
                            artifact_id: evidence.artifact_id,
                            field: "artifact",
                        });
                    }
                    continue;
                }
                deduped.insert(evidence.artifact_id.clone(), evidence);
            }

            Ok(Self {
                request,
                admitted_artifacts: deduped.into_values().collect(),
            })
        }

        /// Returns the typed commit request bound to this prepared mutation.
        pub fn request(&self) -> &TypedCommitRequest {
            &self.request
        }

        /// Returns artifact evidence to admit atomically with the event batch.
        pub fn admitted_artifacts(&self) -> &[ArtifactEvidenceRef] {
            &self.admitted_artifacts
        }

        /// Consumes the prepared commit into owned parts.
        pub fn into_parts(self) -> (TypedCommitRequest, Vec<ArtifactEvidenceRef>) {
            (self.request, self.admitted_artifacts)
        }
    }

    /// Batch of events committed atomically by the store.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CommittedBatch {
        run_id: RunId,
        commit_key: CommitKey,
        fingerprint: CommitFingerprint,
        seq: StreamSeq,
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

        /// Store-owned envelopes created for this batch.
        pub fn events(&self) -> &[KernelEventEnvelope] {
            &self.events
        }
    }

    /// Result of appending a typed commit.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum CommitOutcome {
        /// A new batch was appended.
        Appended(CommittedBatch),
        /// The commit key had already appended the same canonical batch.
        Idempotent(CommittedBatch),
    }

    impl CommitOutcome {
        /// Returns the committed batch for either outcome.
        pub fn batch(&self) -> &CommittedBatch {
            match self {
                Self::Appended(batch) | Self::Idempotent(batch) => batch,
            }
        }
    }

    /// Cell terminal projection derived from committed run events.
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

    /// Side-effect projection derived from committed run events.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectProjection {
        /// Run id that owns this ledger.
        pub run_id: RunId,
        /// Ledger key.
        pub ledger_key: events::SideEffectLedgerKey,
        /// Ledger purpose.
        pub ledger_purpose: events::SideEffectLedgerPurpose,
        /// Last event id that updated this projection.
        pub event_id: EventId,
        /// Intent evidence that opened this ledger key.
        pub intent: SideEffectIntentProjection,
        /// Prepared invocation artifact evidence, when one has been recorded.
        pub prepared_invocation: Option<SideEffectArtifactProjection>,
        /// Exclusive resource key evidence recorded at invocation preparation.
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

    /// Durable identity for one side-effect ledger within a run.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SideEffectLedgerRef {
        /// Run id that owns the ledger.
        pub run_id: RunId,
        /// Run-local side-effect ledger key.
        pub ledger_key: events::SideEffectLedgerKey,
    }

    impl SideEffectLedgerRef {
        /// Creates a side-effect ledger reference.
        pub fn new(run_id: RunId, ledger_key: events::SideEffectLedgerKey) -> Self {
            Self { run_id, ledger_key }
        }
    }

    /// Cross-run resource lane key derived from resource key evidence.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ResourceLaneKey {
        /// Resource namespace.
        pub namespace: ResourceNamespace,
        /// Store-comparable resource key.
        pub key: events::ResourceKey,
    }

    impl ResourceLaneKey {
        /// Creates a lane key from typed resource key evidence.
        pub fn from_evidence(evidence: &events::ResourceKeyEvidence) -> Self {
            Self {
                namespace: evidence.namespace.clone(),
                key: evidence.key.clone(),
            }
        }
    }

    /// Active holder for an exclusive resource lane.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ResourceLaneProjection {
        /// Store event id that acquired or refreshed the lane.
        pub event_id: EventId,
        /// Run-scoped side-effect ledger holding the lane.
        pub holder: SideEffectLedgerRef,
        /// Ledger purpose.
        pub ledger_purpose: events::SideEffectLedgerPurpose,
        /// Node id that prepared the invocation.
        pub node_id: NodeId,
        /// Attempt id that prepared the invocation.
        pub attempt_id: AttemptId,
        /// Invocation epoch that prepared the invocation.
        pub invocation_epoch: u32,
    }

    /// Side-effect artifact evidence retained by the projection.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectArtifactProjection {
        /// Artifact id.
        pub artifact_id: ArtifactId,
        /// Canonical content digest.
        pub content_digest: ContentDigest,
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
    }

    /// Fact projection derived from committed read-fact events.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FactProjection {
        /// Store-owned event id that recorded the fact.
        pub event_id: EventId,
        /// Node id that requested the fact.
        pub node_id: NodeId,
        /// Attempt id that requested the fact.
        pub attempt_id: AttemptId,
        /// Stable fact key.
        pub fact_key: events::FactKey,
        /// Request schema id.
        pub request_schema_id: SchemaId,
        /// Canonical request hash.
        pub request_hash: ContentDigest,
        /// Response schema id.
        pub response_schema_id: SchemaId,
        /// Canonical response hash.
        pub response_hash: ContentDigest,
        /// Response artifact id.
        pub artifact_id: ArtifactId,
        /// Adapter capability kind.
        pub capability_kind: CapabilityKind,
        /// Adapter capability version.
        pub capability_version: CapabilityVersion,
        /// Adapter kind.
        pub adapter_kind: AdapterKind,
        /// Adapter version.
        pub adapter_version: AdapterVersion,
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
        /// Retained artifact refs by artifact id.
        pub refs: BTreeMap<ArtifactId, events::RetentionRef>,
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

    /// Retention projection verified by rebuilding from one contiguous authoritative run stream.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct VerifiedRetentionProjection {
        run_id: RunId,
        projection: RetentionProjection,
    }

    impl VerifiedRetentionProjection {
        /// Rebuilds retention projection state from raw committed run events for synthetic
        /// test, migration, or repair tooling.
        ///
        /// Production replay/read authority must validate the stream against certified runtime
        /// authority before trusting retention evidence.
        pub fn from_synthetic_run_stream(
            run_id: RunId,
            events: &[KernelEventEnvelope],
        ) -> Result<Self> {
            if let Some(event) = events.iter().find(|event| event.run_id() != &run_id) {
                return Err(StoreError::PersistedEventMismatch {
                    field: "run_id",
                    message: format!(
                        "retention projection requested run {} but stream contains {}",
                        run_id,
                        event.run_id()
                    ),
                });
            }
            let snapshot = ProjectionSnapshot::rebuild_from_run_stream(events)?;
            let projection = snapshot.retention(&run_id).cloned().unwrap_or_default();
            Ok(Self { run_id, projection })
        }

        /// Run id covered by this verified projection.
        pub fn run_id(&self) -> &RunId {
            &self.run_id
        }

        /// Verified retention projection.
        pub fn projection(&self) -> &RetentionProjection {
            &self.projection
        }

        /// Returns true when the artifact is retained with matching digest and role evidence.
        pub fn retains_artifact(&self, evidence: &ArtifactEvidenceRef) -> bool {
            retention_projection_retains_artifact(&self.projection, evidence)
        }
    }

    /// Complete verified retention projection set supplied to local artifact garbage collection.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct VerifiedRetentionProjectionSet {
        projections: BTreeMap<RunId, RetentionProjection>,
    }

    impl VerifiedRetentionProjectionSet {
        /// Rebuilds retention projections for all supplied raw run streams for synthetic test,
        /// migration, or repair tooling.
        ///
        /// Callers must pass the complete authoritative run-stream set for the artifact store
        /// scope. This helper verifies only store-level stream shape; production replay/read
        /// authority must validate each stream against certified runtime authority first.
        pub fn from_synthetic_run_streams<'a, I>(streams: I) -> Result<Self>
        where
            I: IntoIterator<Item = (RunId, &'a [KernelEventEnvelope])>,
        {
            let mut projections = BTreeMap::new();
            for (run_id, events) in streams {
                let verified =
                    VerifiedRetentionProjection::from_synthetic_run_stream(run_id.clone(), events)?;
                projections.insert(run_id, verified.projection);
            }
            if projections.is_empty() {
                return Err(StoreError::ProjectionConflict {
                    key: "retention:complete".to_owned(),
                    message: "verified retention projection set requires at least one run stream"
                        .to_owned(),
                });
            }
            Ok(Self { projections })
        }

        /// Iterates verified run retention projections.
        pub fn projections(&self) -> impl Iterator<Item = (&RunId, &RetentionProjection)> {
            self.projections.iter()
        }

        /// Returns true when any verified run retention projection retains the artifact.
        pub fn retains_artifact(&self, evidence: &ArtifactEvidenceRef) -> bool {
            self.projections
                .values()
                .any(|projection| retention_projection_retains_artifact(projection, evidence))
        }
    }

    fn retention_projection_retains_artifact(
        projection: &RetentionProjection,
        evidence: &ArtifactEvidenceRef,
    ) -> bool {
        projection
            .refs
            .get(&evidence.artifact_id)
            .is_some_and(|retained| {
                retained.content_digest == evidence.digest
                    && retained.role == evidence.artifact_role
            })
            || projection.manifests.values().any(|manifest| {
                evidence.artifact_role == ArtifactRole::RetentionManifest
                    && manifest.manifest_artifact_id == evidence.artifact_id
                    && manifest.manifest_digest == evidence.digest
            })
    }

    /// Store-owned projection snapshot derived from authoritative run streams.
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct ProjectionSnapshot {
        run_states: BTreeMap<RunId, RunState>,
        run_completions: BTreeMap<RunId, RunCompletionProjection>,
        saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
        manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
        attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
        cells: BTreeMap<CellId, CellTerminalProjection>,
        facts: BTreeMap<(NodeId, AttemptId, events::FactKey), FactProjection>,
        side_effects: BTreeMap<SideEffectLedgerRef, SideEffectProjection>,
        resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
        public_outputs: BTreeMap<SchemaId, PublicOutputProjection>,
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
        /// Terminal run completion projections.
        pub run_completions: BTreeMap<RunId, RunCompletionProjection>,
        /// First saga engagement per run.
        pub saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
        /// Manual resolution evidence per run.
        pub manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
        /// Attempt projections.
        pub attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
        /// Terminal cell projections.
        pub cells: BTreeMap<CellId, CellTerminalProjection>,
        /// Recorded fact projections.
        pub facts: BTreeMap<(NodeId, AttemptId, events::FactKey), FactProjection>,
        /// Side-effect ledger projections.
        pub side_effects: BTreeMap<SideEffectLedgerRef, SideEffectProjection>,
        /// Cross-run resource lane projections.
        pub resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
        /// Public output projections.
        pub public_outputs: BTreeMap<SchemaId, PublicOutputProjection>,
        /// Retention projections.
        pub retentions: BTreeMap<RunId, RetentionProjection>,
    }

    impl ProjectionSnapshot {
        /// Creates a projection snapshot from storage-owned projection maps.
        ///
        /// Callers populate only the projection families they hydrate and leave the rest empty via
        /// [`ProjectionSnapshotParts`]'s [`Default`].
        pub fn from_parts(parts: ProjectionSnapshotParts) -> Self {
            let ProjectionSnapshotParts {
                run_states,
                run_completions,
                saga_engagements,
                manual_resolutions,
                attempts,
                cells,
                facts,
                side_effects,
                resource_lanes,
                public_outputs,
                retentions,
            } = parts;
            Self {
                run_states,
                run_completions,
                saga_engagements,
                manual_resolutions,
                attempts,
                cells,
                facts,
                side_effects,
                resource_lanes,
                public_outputs,
                retentions,
            }
        }

        /// Validates that a loaded run stream is ordered and contiguous.
        pub fn validate_run_stream(events: &[KernelEventEnvelope]) -> Result<()> {
            validate_run_stream_order(events)
        }

        /// Rebuilds projections from store-owned event envelopes.
        pub fn rebuild_from_run_stream(events: &[KernelEventEnvelope]) -> Result<Self> {
            Self::validate_run_stream(events)?;
            let mut snapshot = Self::default();
            for event in events {
                projection::apply_projection(&mut snapshot, event)?;
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
            self.cells.get(cell_id)
        }

        /// Returns a recorded fact projection.
        pub fn fact(
            &self,
            node_id: &NodeId,
            attempt_id: &AttemptId,
            fact_key: &events::FactKey,
        ) -> Option<&FactProjection> {
            self.facts
                .get(&(node_id.clone(), attempt_id.clone(), fact_key.clone()))
        }

        /// Returns an attempt lifecycle projection.
        pub fn attempt(
            &self,
            node_id: &NodeId,
            attempt_id: &AttemptId,
        ) -> Option<&AttemptProjection> {
            self.attempts.get(&(node_id.clone(), attempt_id.clone()))
        }

        /// Returns a side-effect projection.
        pub fn side_effect(
            &self,
            ledger_key: &events::SideEffectLedgerKey,
        ) -> Option<&SideEffectProjection> {
            self.side_effects
                .values()
                .find(|projection| projection.ledger_key == *ledger_key)
        }

        /// Returns a side-effect projection for a run-scoped ledger.
        pub fn side_effect_for_run(
            &self,
            run_id: &RunId,
            ledger_key: &events::SideEffectLedgerKey,
        ) -> Option<&SideEffectProjection> {
            self.side_effects.get(&SideEffectLedgerRef::new(
                run_id.clone(),
                ledger_key.clone(),
            ))
        }

        /// Returns an active resource lane holder.
        pub fn resource_lane(&self, key: &ResourceLaneKey) -> Option<&ResourceLaneProjection> {
            self.resource_lanes.get(key)
        }

        /// Returns a public-output projection.
        pub fn public_output(&self, schema_id: &SchemaId) -> Option<&PublicOutputProjection> {
            self.public_outputs.get(schema_id)
        }

        /// Returns a retention projection.
        pub fn retention(&self, run_id: &RunId) -> Option<&RetentionProjection> {
            self.retentions.get(run_id)
        }

        /// Returns whether terminal public-output authority is projected.
        pub fn has_public_output(&self) -> bool {
            self.public_outputs
                .values()
                .any(|projection| matches!(projection, PublicOutputProjection::Produced { .. }))
        }

        /// Returns whether all past-boundary forward ledgers for the current projection are quiescent.
        pub fn forward_ledgers_quiescent(&self, run_id: &RunId) -> bool {
            forward_ledgers_quiescent(self, run_id)
        }

        /// Derives saga status from certified saga policy plus the current stream projection.
        pub fn derive_saga_projection(
            &self,
            run_id: &RunId,
            policy: &SagaPolicySpec,
        ) -> SagaProjection {
            derive_saga_projection(self, run_id, policy)
        }

        /// Requires that the current prefix derives a manual-blocked saga mode.
        pub fn require_manual_resolution_admissible(
            &self,
            run_id: &RunId,
            policy: &SagaPolicySpec,
        ) -> Result<()> {
            let saga = self.derive_saga_projection(run_id, policy);
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
        ) -> Result<events::RunCompletionOutcome> {
            let saga = self.derive_saga_projection(run_id, policy);
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

        /// Requires that a claimed saga terminal outcome matches the prefix-derived outcome.
        pub fn require_saga_terminal_outcome_admissible(
            &self,
            run_id: &RunId,
            policy: &SagaPolicySpec,
            claimed: &events::RunCompletionOutcome,
        ) -> Result<()> {
            let expected = self.saga_terminal_completion_outcome(run_id, policy)?;
            if claimed == &expected {
                Ok(())
            } else {
                Err(StoreError::ProjectionConflict {
                    key: format!("run:{run_id}:saga_terminal"),
                    message: "saga terminal outcome does not match prefix-derived run mode"
                        .to_owned(),
                })
            }
        }

        /// Iterates projected run states.
        pub fn run_states(&self) -> impl Iterator<Item = (&RunId, &RunState)> {
            self.run_states.iter()
        }

        /// Iterates run completion projections.
        pub fn run_completions(&self) -> impl Iterator<Item = (&RunId, &RunCompletionProjection)> {
            self.run_completions.iter()
        }

        /// Iterates saga engagement projections.
        pub fn saga_engagements(
            &self,
        ) -> impl Iterator<Item = (&RunId, &SagaEngagementProjection)> {
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
        pub fn cells(&self) -> impl Iterator<Item = (&CellId, &CellTerminalProjection)> {
            self.cells.iter()
        }

        /// Iterates fact projections.
        pub fn facts(
            &self,
        ) -> impl Iterator<Item = (&(NodeId, AttemptId, events::FactKey), &FactProjection)>
        {
            self.facts.iter()
        }

        /// Iterates side-effect projections.
        pub fn side_effects(
            &self,
        ) -> impl Iterator<Item = (&SideEffectLedgerRef, &SideEffectProjection)> {
            self.side_effects.iter()
        }

        /// Iterates active resource lane projections.
        pub fn resource_lanes(
            &self,
        ) -> impl Iterator<Item = (&ResourceLaneKey, &ResourceLaneProjection)> {
            self.resource_lanes.iter()
        }

        /// Iterates public-output projections.
        pub fn public_outputs(&self) -> impl Iterator<Item = (&SchemaId, &PublicOutputProjection)> {
            self.public_outputs.iter()
        }

        /// Iterates retention projections.
        pub fn retentions(&self) -> impl Iterator<Item = (&RunId, &RetentionProjection)> {
            self.retentions.iter()
        }
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

    /// Store-verified projection snapshot.
    ///
    /// The snapshot is produced by store-owned commit/staging or committed-stream validation.
    /// Consumers use this as read authority instead of reconstructing independent projection
    /// views from raw event vectors.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct VerifiedProjectionSnapshot {
        snapshot: ProjectionSnapshot,
    }

    impl VerifiedProjectionSnapshot {
        fn from_rebuilt(snapshot: ProjectionSnapshot) -> Self {
            Self { snapshot }
        }

        /// Returns the verified projection snapshot.
        pub fn snapshot(&self) -> &ProjectionSnapshot {
            &self.snapshot
        }

        /// Consumes this authority into the verified snapshot.
        pub fn into_snapshot(self) -> ProjectionSnapshot {
            self.snapshot
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
        projection: VerifiedProjectionSnapshot,
        next_seq: StreamSeq,
        artifact_requirements: Vec<EventArtifactRequirement>,
    }

    impl CommittedRunStream {
        /// Rebuilds store-owned stream authority from persisted event envelopes.
        pub fn from_events(run_id: RunId, events: Vec<KernelEventEnvelope>) -> Result<Self> {
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
            let projection = VerifiedProjectionSnapshot::from_rebuilt(
                ProjectionSnapshot::rebuild_from_run_stream(&events)?,
            );
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
            self.projection.snapshot()
        }

        /// Returns the verified projection authority for this stream.
        pub fn verified_projection(&self) -> &VerifiedProjectionSnapshot {
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

        /// Returns the saga engagement projection for this run, when any.
        pub fn saga_projection(&self) -> Option<&SagaEngagementProjection> {
            self.projection.snapshot().saga_engagement(&self.run_id)
        }

        /// Returns the side-effect projection for a run-scoped ledger.
        pub fn side_effect_projection(
            &self,
            ledger_key: &events::SideEffectLedgerKey,
        ) -> Option<&SideEffectProjection> {
            self.projection
                .snapshot()
                .side_effect_for_run(&self.run_id, ledger_key)
        }
    }

    fn committed_run_stream_commits(
        events: &[KernelEventEnvelope],
    ) -> Vec<CommittedRunStreamCommit> {
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
        let mut expected_ordinal = 0_u32;

        for event in events {
            match &stream_run_id {
                Some(run_id) if event.run_id() != run_id => {
                    return Err(StoreError::PersistedEventMismatch {
                        field: "run_id",
                        message: "persisted run stream contains events for multiple runs"
                            .to_owned(),
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
                    current_seq = Some(expected_next);
                    current_commit_key = Some(event.commit_key().clone());
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
                    current_seq = Some(StreamSeq::FIRST);
                    current_commit_key = Some(event.commit_key().clone());
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

    /// Read-only access to store-owned projections.
    pub trait TypedProjectionRead {
        /// Returns the current projection snapshot.
        fn projection_snapshot(&self) -> &ProjectionSnapshot;
    }

    /// Typed run event store commit contract.
    pub trait TypedRunEventStore: TypedProjectionRead {
        /// Atomically admits artifact evidence and appends one typed run commit, or returns an
        /// idempotent previous batch.
        fn append_prepared_typed_commit(
            &mut self,
            commit: PreparedTypedCommit,
        ) -> Result<CommitOutcome>;

        /// Loads the authoritative run stream.
        fn load_run_stream(&self, run_id: &RunId) -> Vec<KernelEventEnvelope>;

        /// Returns the next store-owned stream sequence for a run.
        fn expected_next_seq(&self, run_id: &RunId) -> StreamSeq;
    }

    /// Async typed run event store commit contract for durable stores.
    ///
    /// This is the same certified commit surface as [`TypedRunEventStore`] without exposing
    /// implementation-owned projection state to callers. Runtime code must derive read views from
    /// the authoritative stream returned by [`Self::load_run_stream`].
    pub trait AsyncTypedRunEventStore {
        /// Store-specific error type.
        type Error: StoreErrorInspection + fmt::Display + Send + Sync + 'static;

        /// Atomically admits artifact evidence and appends one typed run commit, or returns an
        /// idempotent previous batch.
        fn append_prepared_typed_commit<'a>(
            &'a self,
            commit: PreparedTypedCommit,
        ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error>;

        /// Loads the authoritative run stream.
        fn load_run_stream<'a>(
            &'a self,
            run_id: &'a RunId,
        ) -> AsyncStoreFuture<'a, Vec<KernelEventEnvelope>, Self::Error>;

        /// Returns the next store-owned stream sequence for a run.
        fn expected_next_seq<'a>(
            &'a self,
            run_id: &'a RunId,
        ) -> AsyncStoreFuture<'a, StreamSeq, Self::Error>;
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct CommitKeyRecord {
        fingerprint: CommitFingerprint,
        batch: CommittedBatch,
    }

    /// Set of logical keys already present for run streams.
    pub type LogicalKeySet = BTreeSet<(RunId, LogicalEventKey)>;

    /// Unique logical-key payload hashes already present for run streams.
    pub type UniqueLogicalPayloads = BTreeMap<(RunId, LogicalEventKey), ContentDigest>;

    /// Authoritative state needed to validate and stage one absent commit-key append.
    ///
    /// Durable stores load this from their run stream, artifact table, logical-key table, and
    /// rebuilt projections before calling [`stage_typed_run_commit`]. Commit-key lookup remains the
    /// storage implementation's responsibility because the RFC requires that lookup to precede stale
    /// `expected_next_seq` checks.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TypedCommitBase {
        /// Artifact evidence recorded before event commit.
        pub artifacts: BTreeMap<ArtifactId, ArtifactEvidenceRef>,
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
        /// Store-owned next sequence for the run being committed.
        pub actual_next_seq: StreamSeq,
    }

    /// Staged result of validating a typed commit against a [`TypedCommitBase`].
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct StagedTypedCommit {
        batch: CommittedBatch,
        logical_keys: LogicalKeySet,
        unique_logical_payloads: UniqueLogicalPayloads,
        projections: ProjectionSnapshot,
    }

    impl StagedTypedCommit {
        /// Store-owned committed batch.
        pub fn batch(&self) -> &CommittedBatch {
            &self.batch
        }

        /// Staged logical-key set after this commit.
        pub fn logical_keys(&self) -> &LogicalKeySet {
            &self.logical_keys
        }

        /// Staged unique logical-key payload map after this commit.
        pub fn unique_logical_payloads(&self) -> &UniqueLogicalPayloads {
            &self.unique_logical_payloads
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
        ) {
            (
                self.batch,
                self.logical_keys,
                self.unique_logical_payloads,
                self.projections,
            )
        }
    }

    /// In-memory implementation of the typed store contract for contract tests.
    #[derive(Debug, Clone, Default)]
    pub struct InMemoryTypedRunStore {
        streams: BTreeMap<RunId, Vec<CommittedBatch>>,
        commit_keys: BTreeMap<(RunId, CommitKey), CommitKeyRecord>,
        artifacts: BTreeMap<ArtifactId, ArtifactEvidenceRef>,
        logical_keys: BTreeSet<(RunId, LogicalEventKey)>,
        unique_logical_payloads: BTreeMap<(RunId, LogicalEventKey), ContentDigest>,
        projections: ProjectionSnapshot,
    }

    impl InMemoryTypedRunStore {
        /// Creates an empty in-memory typed run store.
        pub fn new() -> Self {
            Self::default()
        }

        fn stream_events(&self, run_id: &RunId) -> Vec<KernelEventEnvelope> {
            self.streams
                .get(run_id)
                .into_iter()
                .flat_map(|batches| batches.iter())
                .flat_map(|batch| batch.events.iter().cloned())
                .collect()
        }

        fn validate_artifact_evidence(&self, evidence: &ArtifactEvidenceRef) -> Result<()> {
            let Some(stored) = self.artifacts.get(&evidence.artifact_id) else {
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
                artifact_role_str(stored.artifact_role),
                artifact_role_str(evidence.artifact_role),
            )
        }

        fn validate_artifact_requirement(
            &self,
            requirement: &EventArtifactRequirement,
        ) -> Result<()> {
            let Some(stored) = self.artifacts.get(&requirement.artifact_id) else {
                return Err(StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            if let Some(digest) = &requirement.digest {
                compare_artifact_field(
                    &requirement.artifact_id,
                    "digest",
                    stored.digest.as_str(),
                    digest.as_str(),
                )?;
            }
            if let Some(byte_len) = requirement.byte_len {
                compare_artifact_field(
                    &requirement.artifact_id,
                    "byte_len",
                    stored.byte_len,
                    byte_len,
                )?;
            }
            if let Some(media_type) = &requirement.media_type {
                compare_artifact_field(
                    &requirement.artifact_id,
                    "media_type",
                    stored.media_type.as_str(),
                    media_type.as_str(),
                )?;
            }
            if let Some(schema_id) = &requirement.schema_id {
                compare_artifact_option(
                    &requirement.artifact_id,
                    "schema_id",
                    stored.schema_id.as_ref().map(SchemaId::as_str),
                    Some(schema_id.as_str()),
                )?;
            }
            if let Some(semantic_type_id) = &requirement.semantic_type_id {
                compare_artifact_option(
                    &requirement.artifact_id,
                    "semantic_type_id",
                    stored.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
                    Some(semantic_type_id.as_str()),
                )?;
            }
            if let Some(producer_node_id) = &requirement.producer_node_id {
                compare_artifact_option(
                    &requirement.artifact_id,
                    "producer_node_id",
                    stored.producer_node_id.as_ref().map(NodeId::as_str),
                    Some(producer_node_id.as_str()),
                )?;
            }
            if let Some(producer_seed_id) = &requirement.producer_seed_id {
                compare_artifact_option(
                    &requirement.artifact_id,
                    "producer_seed_id",
                    stored.producer_seed_id.as_ref().map(SeedId::as_str),
                    Some(producer_seed_id.as_str()),
                )?;
            }
            if let Some(role) = requirement.artifact_role {
                compare_artifact_field(
                    &requirement.artifact_id,
                    "artifact_role",
                    artifact_role_str(stored.artifact_role),
                    artifact_role_str(role),
                )?;
            }
            Ok(())
        }

        fn validate_preconditions(&self, request: &TypedCommitRequest) -> Result<()> {
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
                let actual = self.projections.cell_terminal(&precondition.cell_id);
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
                    .side_effect_for_run(&request.run_id, &precondition.ledger_key);
                if !side_effect_precondition_matches(actual, precondition.required) {
                    return Err(StoreError::SideEffectStatePreconditionFailed {
                        ledger_key: precondition.ledger_key.clone(),
                        required: precondition.required,
                    });
                }
            }
            if request.preconditions.required_public_output_absent
                && self.projections.has_public_output()
            {
                return Err(StoreError::PublicOutputPreconditionFailed);
            }
            Ok(())
        }
    }

    /// Validates and stages a typed commit after the caller has handled commit-key idempotency.
    ///
    /// This is the shared commit engine for in-memory and durable stores. It checks stale sequence,
    /// payload run/spec identity, logical-key preconditions, artifact evidence, and projection
    /// transitions, then returns the store-owned envelopes plus staged projection/logical-key state.
    pub fn stage_typed_run_commit(
        base: &TypedCommitBase,
        request: &TypedCommitRequest,
    ) -> Result<StagedTypedCommit> {
        let fingerprint = commit_fingerprint(request)?;
        stage_typed_run_commit_with_fingerprint(base, request, fingerprint)
    }

    /// Validates and stages a prepared typed commit after commit-key idempotency handling.
    ///
    /// The returned batch fingerprint covers the full prepared mutation, including the artifact
    /// evidence admitted atomically with the event payloads.
    pub fn stage_prepared_typed_run_commit(
        base: &TypedCommitBase,
        commit: &PreparedTypedCommit,
    ) -> Result<StagedTypedCommit> {
        let fingerprint = prepared_commit_fingerprint(commit)?;
        stage_typed_run_commit_with_fingerprint(base, commit.request(), fingerprint)
    }

    fn stage_typed_run_commit_with_fingerprint(
        base: &TypedCommitBase,
        request: &TypedCommitRequest,
        fingerprint: CommitFingerprint,
    ) -> Result<StagedTypedCommit> {
        if request.expected_next_seq != base.actual_next_seq {
            return Err(StoreError::StaleExpectedNextSeq {
                expected: request.expected_next_seq,
                actual: base.actual_next_seq,
            });
        }

        validate_payload_run_and_spec(&request.run_id, &request.payloads)?;
        validate_terminal_attempt_cell_pairs(&request.payloads)?;
        validate_terminal_side_effect_evidence_pairs(&request.payloads)?;
        validate_retention_manifest_pairs(&request.payloads)?;

        let verifier = InMemoryTypedRunStore {
            streams: BTreeMap::new(),
            commit_keys: BTreeMap::new(),
            artifacts: base.artifacts.clone(),
            logical_keys: base.logical_keys.clone(),
            unique_logical_payloads: base.unique_logical_payloads.clone(),
            projections: base.projections.clone(),
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
            let logical_key = derive_logical_key(&payload, &payload_hash)?;
            let ordinal = CommitOrdinal::from_index(index)?;
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
                ordinal,
                spec_hash,
                commit_key: request.commit_key.clone(),
                logical_key,
                payload_hash,
                payload,
                audit: KernelEventAudit::new(canonical_payload.as_bytes().len() as u64),
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
                    ) {
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
                &envelope.payload,
                request.preconditions.saga_policy.as_ref(),
            )?;
            projection::apply_projection(&mut staged_projections, &envelope)?;
            events.push(envelope);
        }

        Ok(StagedTypedCommit {
            batch: CommittedBatch {
                run_id: request.run_id.clone(),
                commit_key: request.commit_key.clone(),
                fingerprint,
                seq: request.expected_next_seq,
                events,
            },
            logical_keys: staged_logical_keys,
            unique_logical_payloads: staged_unique_payloads,
            projections: staged_projections,
        })
    }

    /// Builds a store-owned committed batch for an already validated request and persisted sequence.
    ///
    /// Durable stores use this after a same-fingerprint commit-key hit so the idempotent result can
    /// return the original sequence even when the caller's `expected_next_seq` is stale.
    pub fn build_committed_batch(
        request: &TypedCommitRequest,
        committed_seq: StreamSeq,
    ) -> Result<CommittedBatch> {
        let fingerprint = commit_fingerprint(request)?;
        build_committed_batch_with_fingerprint(request, committed_seq, fingerprint)
    }

    /// Builds a store-owned committed batch for an already persisted prepared commit.
    ///
    /// Durable stores use this after a same-fingerprint prepared commit-key hit so the idempotent
    /// result can return the original sequence even when the caller's `expected_next_seq` is stale.
    pub fn build_prepared_committed_batch(
        commit: &PreparedTypedCommit,
        committed_seq: StreamSeq,
    ) -> Result<CommittedBatch> {
        let fingerprint = prepared_commit_fingerprint(commit)?;
        build_committed_batch_with_fingerprint(commit.request(), committed_seq, fingerprint)
    }

    fn build_committed_batch_with_fingerprint(
        request: &TypedCommitRequest,
        committed_seq: StreamSeq,
        fingerprint: CommitFingerprint,
    ) -> Result<CommittedBatch> {
        validate_payload_run_and_spec(&request.run_id, &request.payloads)?;
        validate_terminal_attempt_cell_pairs(&request.payloads)?;
        validate_terminal_side_effect_evidence_pairs(&request.payloads)?;
        validate_retention_manifest_pairs(&request.payloads)?;

        let mut events = Vec::with_capacity(request.payloads.len());
        for (index, payload) in request.payloads.iter().cloned().enumerate() {
            let canonical_payload = payload_canonical_json(&payload)?;
            let payload_hash = canonical_payload.content_digest();
            let schema_id = payload.event_schema_id()?;
            let logical_key = derive_logical_key(&payload, &payload_hash)?;
            let ordinal = CommitOrdinal::from_index(index)?;
            let event_id = derive_event_id(
                &request.run_id,
                committed_seq,
                ordinal,
                &schema_id,
                &payload_hash,
            )?;
            let spec_hash = payload_spec_hash(&payload);
            events.push(KernelEventEnvelope {
                event_id,
                event_schema_id: schema_id,
                run_id: request.run_id.clone(),
                seq: committed_seq,
                ordinal,
                spec_hash,
                commit_key: request.commit_key.clone(),
                logical_key,
                payload_hash,
                payload,
                audit: KernelEventAudit::new(canonical_payload.as_bytes().len() as u64),
            });
        }

        Ok(CommittedBatch {
            run_id: request.run_id.clone(),
            commit_key: request.commit_key.clone(),
            fingerprint,
            seq: committed_seq,
            events,
        })
    }

    impl TypedProjectionRead for InMemoryTypedRunStore {
        fn projection_snapshot(&self) -> &ProjectionSnapshot {
            &self.projections
        }
    }

    impl TypedRunEventStore for InMemoryTypedRunStore {
        fn append_prepared_typed_commit(
            &mut self,
            commit: PreparedTypedCommit,
        ) -> Result<CommitOutcome> {
            let request = commit.request();
            let fingerprint = prepared_commit_fingerprint(&commit)?;

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
            admit_artifact_evidence(&mut artifacts, commit.admitted_artifacts())?;
            let base = TypedCommitBase {
                artifacts,
                logical_keys: self.logical_keys.clone(),
                unique_logical_payloads: self.unique_logical_payloads.clone(),
                projections: self.projections.clone(),
                actual_next_seq: self.expected_next_seq(&request.run_id),
            };
            let staged = stage_prepared_typed_run_commit(&base, &commit)?;
            let (batch, staged_logical_keys, staged_unique_payloads, staged_projections) =
                staged.into_parts();
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

    fn admit_artifact_evidence(
        artifacts: &mut BTreeMap<ArtifactId, ArtifactEvidenceRef>,
        admitted_artifacts: &[ArtifactEvidenceRef],
    ) -> Result<()> {
        for evidence in admitted_artifacts {
            if let Some(existing) = artifacts.get(&evidence.artifact_id) {
                if existing != evidence {
                    return Err(StoreError::ArtifactEvidenceMismatch {
                        artifact_id: evidence.artifact_id.clone(),
                        field: "artifact",
                    });
                }
                continue;
            }
            artifacts.insert(evidence.artifact_id.clone(), evidence.clone());
        }
        Ok(())
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
    ) -> bool {
        match required {
            RequiredSideEffectState::Absent => projection.is_none(),
            RequiredSideEffectState::IntentPersisted => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::IntentPersisted { .. })
            ),
            RequiredSideEffectState::Claimed => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::Claimed { .. })
            ),
            RequiredSideEffectState::InvocationPrepared => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::InvocationPrepared { .. })
            ),
            RequiredSideEffectState::InvocationStarted => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::InvocationStarted { .. })
            ),
            RequiredSideEffectState::SubmissionResult => matches!(
                projection.map(|projection| &projection.phase),
                Some(
                    SideEffectPhase::SubmissionObserved { .. }
                        | SideEffectPhase::NotSubmittedProven { .. }
                        | SideEffectPhase::SubmissionUnknown { .. }
                        | SideEffectPhase::Ambiguous { .. }
                )
            ),
            RequiredSideEffectState::ReceiptObserved => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::ReceiptObserved { .. })
            ),
            RequiredSideEffectState::ConfirmationObserved => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::ConfirmationObserved { .. })
            ),
            RequiredSideEffectState::Ambiguous => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::Ambiguous { .. })
            ),
            RequiredSideEffectState::Failed => matches!(
                projection.map(|projection| &projection.phase),
                Some(SideEffectPhase::Failed { .. })
            ),
        }
    }

    use self::admission::require_admission_preconditions;

    mod admission;

    fn validate_payload_run_and_spec(
        run_id: &RunId,
        payloads: &[KernelEventPayload],
    ) -> Result<()> {
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
            if !terminal_cells.contains(&(
                node_id.clone(),
                attempt_id.clone(),
                output_cell_id.clone(),
            )) {
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
                    message:
                        "public output requires matching render receipt terminal in same commit"
                            .to_owned(),
                });
            }
        }
        Ok(())
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

    fn insert_terminal_side_effect_pair(
        pairs: &mut BTreeMap<(NodeId, AttemptId), TerminalSideEffectEvidencePair>,
        pair: TerminalSideEffectEvidencePair,
    ) -> Result<()> {
        let key = pair.node_attempt_key();
        if pairs.insert(key.clone(), pair).is_some() {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}:{}:terminal", key.0, key.1),
                message:
                    "terminal side-effect evidence must be unique per node attempt in one commit"
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

    /// Returns the canonical payload hash for a typed event payload.
    pub fn payload_hash(payload: &KernelEventPayload) -> Result<ContentDigest> {
        Ok(payload_canonical_json(payload)?.content_digest())
    }

    /// Computes the canonical idempotency fingerprint for a typed commit request.
    ///
    /// The fingerprint intentionally excludes `expected_next_seq`, so idempotent retries can be
    /// recognized before stale sequence checks as required by the store contract.
    pub fn commit_fingerprint(request: &TypedCommitRequest) -> Result<CommitFingerprint> {
        let canonical = canonical_json(serde_json::json!({
            "commit_key": request.commit_key.as_str(),
            "payloads": request.payloads.iter().map(payload_json).collect::<Vec<_>>(),
            "preconditions": preconditions_json(&request.preconditions),
            "required_artifacts": sorted_store_artifacts_json(&request.required_artifacts),
            "run_id": request.run_id.as_str(),
        }))?;
        Ok(CommitFingerprint(canonical.content_digest()))
    }

    /// Computes the canonical idempotency fingerprint for a prepared typed commit.
    ///
    /// The fingerprint intentionally excludes `expected_next_seq`, so idempotent retries can be
    /// recognized before stale sequence checks as required by the store contract. Unlike
    /// [`commit_fingerprint`], this covers the artifact evidence admitted atomically with the commit.
    pub fn prepared_commit_fingerprint(commit: &PreparedTypedCommit) -> Result<CommitFingerprint> {
        let request = commit.request();
        let canonical = canonical_json(serde_json::json!({
            "admitted_artifacts": sorted_store_artifacts_json(commit.admitted_artifacts()),
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
        payload: &KernelEventPayload,
        payload_hash: &ContentDigest,
    ) -> Result<LogicalEventKey> {
        let key = match payload {
            KernelEventPayload::RunStarted(_) => "run:start".to_owned(),
            KernelEventPayload::ManualResolutionRecorded(_) => "run:manual_resolution".to_owned(),
            KernelEventPayload::RunCompleted(_) => "run:complete".to_owned(),
            KernelEventPayload::StateAttemptStarted(payload) => {
                format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
            }
            KernelEventPayload::StateAttemptCompleted(payload) => {
                format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
            }
            KernelEventPayload::StateAttemptFailed(payload) => {
                format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
            }
            KernelEventPayload::FactRecorded(payload) => format!(
                "fact:{}:{}:{}",
                payload.node_id, payload.attempt_id, payload.fact_key
            ),
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
                    payload.ledger_key
                )
            }
            KernelEventPayload::SideEffectClaimed(payload) => format!(
                "sidefx:{}:{}:claim:{}:{}",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch,
                payload.claim_generation
            ),
            KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
                "sidefx:{}:{}:claim:{}:{}:taken_over",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch,
                payload.claim_generation
            ),
            KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
                "sidefx:{}:{}:invocation:{}:prepared:{}",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch,
                payload.claim_generation
            ),
            KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
                "sidefx:{}:{}:invocation:{}:started",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
                "sidefx:{}:{}:invocation:{}:submission_result",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
                "sidefx:{}:{}:invocation:{}:submission_result",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
                "sidefx:{}:{}:invocation:{}:submission_result",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
                "sidefx:{}:{}:invocation:{}:receipt",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
                "sidefx:{}:{}:invocation:{}:confirmation",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                format!(
                    "sidefx:{}:{}:ambiguous",
                    side_effect_ledger_purpose_key(&payload.ledger_purpose),
                    payload.ledger_key
                )
            }
            KernelEventPayload::SideEffectFailed(payload) => format!(
                "sidefx:{}:{}:invocation:{}:failure",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.ledger_key,
                payload.invocation_epoch
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
        base: &TypedCommitBase,
        run_id: &RunId,
        logical_key: &LogicalEventKey,
        payload: &KernelEventPayload,
        projections: &ProjectionSnapshot,
    ) -> bool {
        if !base
            .logical_keys
            .contains(&(run_id.clone(), logical_key.clone()))
        {
            return false;
        }
        let Some((ledger_key, invocation_epoch)) = recoverable_submission_result_payload(payload)
        else {
            return false;
        };
        matches!(
            projections
                .side_effect_for_run(run_id, ledger_key)
                .map(|projection| &projection.phase),
            Some(SideEffectPhase::SubmissionUnknown {
                invocation_epoch: existing_epoch
            }) if *existing_epoch == invocation_epoch
        )
    }

    fn recoverable_submission_result_payload(
        payload: &KernelEventPayload,
    ) -> Option<(&events::SideEffectLedgerKey, u32)> {
        match payload {
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                Some((&payload.ledger_key, payload.invocation_epoch))
            }
            KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                Some((&payload.ledger_key, payload.invocation_epoch))
            }
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                Some((&payload.ledger_key, payload.invocation_epoch))
            }
            _ => None,
        }
    }

    fn side_effect_ledger_purpose_key(purpose: &events::SideEffectLedgerPurpose) -> String {
        match purpose {
            events::SideEffectLedgerPurpose::Forward => "forward".to_owned(),
            events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } => {
                format!("remediation:{forward_ledger_key}")
            }
        }
    }

    pub use mfm_events::v1::{EventArtifactReferenceSource, EventArtifactRequirement};

    pub use self::artifact_refs::event_artifact_requirements;
    use self::artifact_refs::referenced_artifact_ids;

    mod artifact_refs;

    mod projection;

    use self::resource_lanes::{
        acquire_resource_lane, release_resource_lane_for_holder, release_resource_lanes_for_run,
    };
    use self::side_effects::{
        note_saga_engagement, prepared_invocation_projection, previous_claim,
        require_active_attempt_for_side_effect, require_claim_context,
        require_claim_takeover_matches, require_forward_fence_open, require_forward_quiescence,
        require_intent_attempt_context, require_intent_context,
        require_remediation_intent_admissible, require_side_effect_phase,
        require_side_effect_purpose, side_effect_projection_error,
        transition_side_effect_epoch_only, transition_side_effect_failure, EpochOnlyTransition,
        ExpectedClaimContext,
    };

    mod resource_lanes;

    mod side_effects;

    fn payload_json(payload: &KernelEventPayload) -> serde_json::Value {
        match payload {
            KernelEventPayload::RunStarted(payload) => serde_json::json!({
                "canonicalizer_identity": payload.canonicalizer_identity.as_str(),
                "certificate_artifact_digest": payload.certificate_artifact_digest.as_str(),
                "certificate_artifact_id": payload.certificate_artifact_id.as_str(),
                "certificate_media_type": payload.certificate_media_type.as_str(),
                "descriptor_identities": payload.descriptor_identities.iter().map(descriptor_identity_json).collect::<Vec<_>>(),
                "adapter_executables": payload.adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
                "framework_version": payload.framework_version.as_str(),
                "lowering_version": payload.lowering_version.as_str(),
                "public_output_schema_id": payload.public_output_schema_id.as_str(),
                "run_id": payload.run_id.as_str(),
                "runner_executables": payload.runner_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
                "seed_cells": payload.seed_cells.iter().map(seed_cell_ref_json).collect::<Vec<_>>(),
                "source_revision": payload.source_revision.as_str(),
                "spec_artifact_id": payload.spec_artifact_id.as_str(),
                "spec_hash": payload.spec_hash.as_str(),
                "spec_media_type": payload.spec_media_type.as_str(),
                "spec_version": payload.spec_version.as_str(),
                "variant": "RunStarted",
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
                "adapter_kind": payload.adapter_kind.as_str(),
                "adapter_version": payload.adapter_version.as_str(),
                "artifact_id": payload.artifact_id.as_str(),
                "attempt_id": payload.attempt_id.as_str(),
                "capability_kind": payload.capability_kind.as_str(),
                "capability_version": payload.capability_version.as_str(),
                "fact_key": payload.fact_key.as_str(),
                "node_id": payload.node_id.as_str(),
                "request_hash": payload.request_hash.as_str(),
                "request_schema_id": payload.request_schema_id.as_str(),
                "response_hash": payload.response_hash.as_str(),
                "response_schema_id": payload.response_schema_id.as_str(),
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
                "content_digest": payload.content_digest.as_str(),
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
                "intent_artifact_id": payload.intent_artifact_id.as_str(),
                "intent_hash": payload.intent_hash.as_str(),
                "intent_schema_id": payload.intent_schema_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
                "node_id": payload.node_id.as_str(),
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
                "previous_claim_generation": payload.previous_claim_generation,
                "previous_claim_owner": payload.previous_claim_owner.as_str(),
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "SideEffectClaimTakenOver",
            }),
            KernelEventPayload::SideEffectInvocationPrepared(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "claim_fencing_token": payload.claim_fencing_token.as_str(),
                "claim_generation": payload.claim_generation,
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
                "node_id": payload.node_id.as_str(),
                "prepared_artifact_id": payload.prepared_artifact_id.as_ref().map(ArtifactId::as_str),
                "prepared_hash": payload.prepared_hash.as_ref().map(ContentDigest::as_str),
                "resource_key": payload.resource_key.as_ref().map(resource_key_evidence_json),
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
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "SideEffectInvocationStarted",
            }),
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
                "node_id": payload.node_id.as_str(),
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
                "spec_hash": payload.spec_hash.as_str(),
                "submission_artifact_id": payload.submission_artifact_id.as_str(),
                "submission_hash": payload.submission_hash.as_str(),
                "submission_schema_id": payload.submission_schema_id.as_str(),
                "variant": "SideEffectSubmissionObserved",
            }),
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
                "evidence_hash": payload.evidence_hash.as_str(),
                "evidence_schema_id": payload.evidence_schema_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
                "node_id": payload.node_id.as_str(),
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "SideEffectSubmissionUnknown",
            }),
            KernelEventPayload::SideEffectReceiptObserved(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
                "node_id": payload.node_id.as_str(),
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
                "confirmation_artifact_id": payload.confirmation_artifact_id.as_str(),
                "confirmation_hash": payload.confirmation_hash.as_str(),
                "confirmation_schema_id": payload.confirmation_schema_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
                "node_id": payload.node_id.as_str(),
                "replay_verifier_id": payload.replay_verifier_id.as_str(),
                "resource_touched_set": payload.resource_touched_set.as_ref().map(resource_touched_set_evidence_json),
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "SideEffectConfirmationObserved",
            }),
            KernelEventPayload::SideEffectAmbiguous(payload) => serde_json::json!({
                "ambiguity_code": payload.ambiguity_code.as_str(),
                "attempt_id": payload.attempt_id.as_str(),
                "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
                "evidence_hash": payload.evidence_hash.as_str(),
                "evidence_schema_id": payload.evidence_schema_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
                "node_id": payload.node_id.as_str(),
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
                "retryable": payload.retryable,
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "SideEffectFailed",
            }),
            KernelEventPayload::PublicOutputProduced(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "cells": payload.cells.iter().map(named_cell_ref_json).collect::<Vec<_>>(),
                "node_id": payload.node_id.as_str(),
                "output_spec_digest": payload.output_spec_digest.as_str(),
                "public_schema_id": payload.public_schema_id.as_str(),
                "receipt_cell_id": payload.receipt_cell_id.as_str(),
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
            KernelEventPayload::StateAttemptFailed(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "error": error_info_json(&payload.error),
                "node_id": payload.node_id.as_str(),
                "retryable": payload.retryable,
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "StateAttemptFailed",
            }),
            KernelEventPayload::ManualResolutionRecorded(payload) => serde_json::json!({
                "authorization_artifact_id": payload.authorization_artifact_id.as_str(),
                "authorization_hash": payload.authorization_hash.as_str(),
                "authorization_schema_id": payload.authorization_schema_id.as_str(),
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
            "RunStarted" => Ok(KernelEventPayload::RunStarted(events::RunStarted {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                spec_artifact_id: parse_identity(required_str(json, "spec_artifact_id")?)?,
                certificate_artifact_id: parse_identity(required_str(
                    json,
                    "certificate_artifact_id",
                )?)?,
                certificate_artifact_digest: parse_identity(required_str(
                    json,
                    "certificate_artifact_digest",
                )?)?,
                certificate_media_type: MediaType::new(required_str(
                    json,
                    "certificate_media_type",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
                spec_media_type: MediaType::new(required_str(json, "spec_media_type")?)
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                spec_version: parse_identity(required_str(json, "spec_version")?)?,
                lowering_version: parse_identity(required_str(json, "lowering_version")?)?,
                public_output_schema_id: parse_identity(required_str(
                    json,
                    "public_output_schema_id",
                )?)?,
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
                framework_version: events::FrameworkVersion::new(required_str(
                    json,
                    "framework_version",
                )?)?,
                source_revision: events::SourceRevision::new(required_str(
                    json,
                    "source_revision",
                )?)?,
                seed_cells: parse_vec(json, "seed_cells", parse_seed_cell_ref)?,
            })),
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
                capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
                capability_version: parse_identity(required_str(json, "capability_version")?)?,
                adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
                adapter_version: parse_identity(required_str(json, "adapter_version")?)?,
                request_schema_id: parse_identity(required_str(json, "request_schema_id")?)?,
                request_hash: parse_identity(required_str(json, "request_hash")?)?,
                response_schema_id: parse_identity(required_str(json, "response_schema_id")?)?,
                response_hash: parse_identity(required_str(json, "response_hash")?)?,
                fact_key: events::FactKey::new(required_str(json, "fact_key")?)?,
                artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
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
                artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
                content_digest: parse_identity(required_str(json, "content_digest")?)?,
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
                skip_reason: parse_skip_reason(required_obj(json, "skip_reason")?)?,
            })),
            "SideEffectIntentPersisted" => Ok(KernelEventPayload::SideEffectIntentPersisted(
                side_effect::IntentPersisted {
                    spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                    node_id: parse_identity(required_str(json, "node_id")?)?,
                    scope_id: parse_identity(required_str(json, "scope_id")?)?,
                    attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    intent_schema_id: parse_identity(required_str(json, "intent_schema_id")?)?,
                    intent_hash: parse_identity(required_str(json, "intent_hash")?)?,
                    intent_artifact_id: parse_identity(required_str(json, "intent_artifact_id")?)?,
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
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    claim_owner: events::RunnerInvocationId::new(required_str(
                        json,
                        "claim_owner",
                    )?)?,
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
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
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
            "SideEffectInvocationPrepared" => Ok(KernelEventPayload::SideEffectInvocationPrepared(
                side_effect::InvocationPrepared {
                    spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                    node_id: parse_identity(required_str(json, "node_id")?)?,
                    attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    claim_generation: required_u32(json, "claim_generation")?,
                    claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                        json,
                        "claim_fencing_token",
                    )?)?,
                    prepared_artifact_id: optional_str(json, "prepared_artifact_id")?
                        .map(parse_identity)
                        .transpose()?,
                    prepared_hash: optional_str(json, "prepared_hash")?
                        .map(parse_identity)
                        .transpose()?,
                    resource_key: optional_obj(json, "resource_key")?
                        .map(parse_resource_key_evidence)
                        .transpose()?,
                },
            )),
            "SideEffectInvocationStarted" => Ok(KernelEventPayload::SideEffectInvocationStarted(
                side_effect::InvocationStarted {
                    spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                    node_id: parse_identity(required_str(json, "node_id")?)?,
                    attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    claim_owner: events::RunnerInvocationId::new(required_str(
                        json,
                        "claim_owner",
                    )?)?,
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
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    proof_schema_id: parse_identity(required_str(json, "proof_schema_id")?)?,
                    proof_hash: parse_identity(required_str(json, "proof_hash")?)?,
                    proof_artifact_id: parse_identity(required_str(json, "proof_artifact_id")?)?,
                },
            )),
            "SideEffectSubmissionObserved" => Ok(KernelEventPayload::SideEffectSubmissionObserved(
                side_effect::SubmissionObserved {
                    spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                    node_id: parse_identity(required_str(json, "node_id")?)?,
                    attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    submission_schema_id: parse_identity(required_str(
                        json,
                        "submission_schema_id",
                    )?)?,
                    submission_hash: parse_identity(required_str(json, "submission_hash")?)?,
                    submission_artifact_id: parse_identity(required_str(
                        json,
                        "submission_artifact_id",
                    )?)?,
                },
            )),
            "SideEffectSubmissionUnknown" => Ok(KernelEventPayload::SideEffectSubmissionUnknown(
                side_effect::SubmissionUnknown {
                    spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                    node_id: parse_identity(required_str(json, "node_id")?)?,
                    attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                    evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                    evidence_artifact_id: parse_identity(required_str(
                        json,
                        "evidence_artifact_id",
                    )?)?,
                },
            )),
            "SideEffectReceiptObserved" => Ok(KernelEventPayload::SideEffectReceiptObserved(
                side_effect::ReceiptObserved {
                    spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                    node_id: parse_identity(required_str(json, "node_id")?)?,
                    attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    receipt_schema_id: parse_identity(required_str(json, "receipt_schema_id")?)?,
                    receipt_hash: parse_identity(required_str(json, "receipt_hash")?)?,
                    receipt_artifact_id: parse_identity(required_str(
                        json,
                        "receipt_artifact_id",
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
            "SideEffectConfirmationObserved" => {
                Ok(KernelEventPayload::SideEffectConfirmationObserved(
                    side_effect::ConfirmationObserved {
                        spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                        node_id: parse_identity(required_str(json, "node_id")?)?,
                        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                        ledger_key: events::SideEffectLedgerKey::new(required_str(
                            json,
                            "ledger_key",
                        )?)?,
                        ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                            json,
                            "ledger_purpose",
                        )?)?,
                        invocation_epoch: required_u32(json, "invocation_epoch")?,
                        confirmation_schema_id: parse_identity(required_str(
                            json,
                            "confirmation_schema_id",
                        )?)?,
                        confirmation_hash: parse_identity(required_str(
                            json,
                            "confirmation_hash",
                        )?)?,
                        confirmation_artifact_id: parse_identity(required_str(
                            json,
                            "confirmation_artifact_id",
                        )?)?,
                        replay_verifier_id: events::ReplayVerifierId::new(required_str(
                            json,
                            "replay_verifier_id",
                        )?)?,
                        resource_touched_set: optional_obj(json, "resource_touched_set")?
                            .map(parse_resource_touched_set_evidence)
                            .transpose()?,
                    },
                ))
            }
            "SideEffectAmbiguous" => Ok(KernelEventPayload::SideEffectAmbiguous(
                side_effect::Ambiguous {
                    spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                    node_id: parse_identity(required_str(json, "node_id")?)?,
                    attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                    ledger_key: events::SideEffectLedgerKey::new(required_str(
                        json,
                        "ledger_key",
                    )?)?,
                    ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                        json,
                        "ledger_purpose",
                    )?)?,
                    invocation_epoch: required_u32(json, "invocation_epoch")?,
                    ambiguity_code: events::AmbiguityCode::new(required_str(
                        json,
                        "ambiguity_code",
                    )?)?,
                    evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                    evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                    evidence_artifact_id: parse_identity(required_str(
                        json,
                        "evidence_artifact_id",
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
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                failure_phase: parse_failure_phase(required_str(json, "failure_phase")?)?,
                retryable: required_bool(json, "retryable")?,
                error: parse_error_info(required_obj(json, "error")?)?,
            })),
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
                    evidence_artifact_id: parse_identity(required_str(
                        json,
                        "evidence_artifact_id",
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
                    manifest_artifact_id: parse_identity(required_str(
                        json,
                        "manifest_artifact_id",
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
            source_revision: events::SourceRevision::new(required_str(json, "source_revision")?)?,
            cargo_package_name: events::PackageName::new(required_str(
                json,
                "cargo_package_name",
            )?)?,
            cargo_package_version: events::PackageVersion::new(required_str(
                json,
                "cargo_package_version",
            )?)?,
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
    pub fn parse_event_artifact(
        json: &serde_json::Value,
    ) -> CodecResult<events::ArtifactEvidenceRef> {
        Ok(events::ArtifactEvidenceRef {
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            role: parse_artifact_role(required_str(json, "role")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            semantic_type_id: optional_str(json, "semantic_type_id")?
                .map(parse_identity)
                .transpose()?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
            byte_len: required_u64(json, "byte_len")?,
            media_type: MediaType::new(required_str(json, "media_type")?)
                .map_err(|error| CodecError::Identity(error.to_string()))?,
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
                forward_ledger_key: events::SideEffectLedgerKey::new(required_str(
                    json,
                    "forward_ledger_key",
                )?)?,
            }),
            other => Err(CodecError::Identity(format!(
                "unknown side-effect ledger purpose {other}"
            ))),
        }
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
        Ok(events::MfmErrorInfo {
            code: events::ErrorCode::new(required_str(json, "code")?)?,
            category: parse_error_category(required_str(json, "category")?)?,
            retryable: required_bool(json, "retryable")?,
            safe_message: required_str(json, "safe_message")?.to_owned(),
            public_details: optional_obj(json, "public_details")?
                .map(|details| {
                    Ok::<events::RedactedJson, CodecError>(events::RedactedJson {
                        content_digest: parse_identity(required_str(details, "content_digest")?)?,
                    })
                })
                .transpose()?,
            diagnostic_ref: optional_obj(json, "diagnostic_ref")?
                .map(parse_event_artifact)
                .transpose()?,
        })
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

    fn parse_retention_ref(json: &serde_json::Value) -> Result<events::RetentionRef> {
        Ok(events::RetentionRef {
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            role: parse_artifact_role(required_str(json, "role")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
        })
    }

    /// Parses an artifact role tag.
    pub fn parse_artifact_role(value: &str) -> CodecResult<ArtifactRole> {
        match value {
            "typed_execution_spec" => Ok(ArtifactRole::TypedExecutionSpec),
            "typed_spec_certificate" => Ok(ArtifactRole::TypedSpecCertificate),
            "typed_config" => Ok(ArtifactRole::TypedConfig),
            "seed_input" => Ok(ArtifactRole::SeedInput),
            "state_output" => Ok(ArtifactRole::StateOutput),
            "fact_response" => Ok(ArtifactRole::FactResponse),
            "side_effect_intent" => Ok(ArtifactRole::SideEffectIntent),
            "prepared_invocation" => Ok(ArtifactRole::PreparedInvocation),
            "not_submitted_proof" => Ok(ArtifactRole::NotSubmittedProof),
            "submission" => Ok(ArtifactRole::Submission),
            "submission_unknown_evidence" => Ok(ArtifactRole::SubmissionUnknownEvidence),
            "receipt" => Ok(ArtifactRole::Receipt),
            "confirmation" => Ok(ArtifactRole::Confirmation),
            "ambiguity_evidence" => Ok(ArtifactRole::AmbiguityEvidence),
            "manual_resolution_evidence" => Ok(ArtifactRole::ManualResolutionEvidence),
            "manual_resolution_authorization" => Ok(ArtifactRole::ManualResolutionAuthorization),
            "public_output" => Ok(ArtifactRole::PublicOutput),
            "redacted_diagnostic" => Ok(ArtifactRole::RedactedDiagnostic),
            "retention_manifest" => Ok(ArtifactRole::RetentionManifest),
            other => Err(CodecError::Identity(format!(
                "unknown artifact role {other}"
            ))),
        }
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
            "run_started" => Ok(events::RetentionReason::RunStarted),
            "runtime_evidence" => Ok(events::RetentionReason::RuntimeEvidence),
            "public_output" => Ok(events::RetentionReason::PublicOutput),
            "manifest_projection" => Ok(events::RetentionReason::ManifestProjection),
            other => Err(StoreError::Identity(format!(
                "unknown retention reason {other}"
            ))),
        }
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
                    "ledger_key": precondition.ledger_key.as_str(),
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
            "saga_policy": preconditions.saga_policy.as_ref().map(saga_policy_json),
        })
    }

    fn saga_policy_json(policy: &SagaPolicySpec) -> serde_json::Value {
        match policy {
            SagaPolicySpec::NoSideEffects => serde_json::json!({
                "kind": "no_side_effects",
            }),
            SagaPolicySpec::FailWithoutAcdcClaim => serde_json::json!({
                "kind": "fail_without_acdc_claim",
            }),
            SagaPolicySpec::ManualResolution { manual } => serde_json::json!({
                "kind": "manual_resolution",
                "manual": manual_resolution_evidence_spec_json(manual),
            }),
            SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved,
            } => serde_json::json!({
                "kind": "compensate_completed",
                "on_remediation_unresolved": remediation_unresolved_spec_json(on_remediation_unresolved),
            }),
        }
    }

    fn remediation_unresolved_spec_json(policy: &RemediationUnresolvedSpec) -> serde_json::Value {
        match policy {
            RemediationUnresolvedSpec::ManualResolution { manual } => serde_json::json!({
                "kind": "manual_resolution",
                "manual": manual_resolution_evidence_spec_json(manual),
            }),
            RemediationUnresolvedSpec::FailWithoutAcdcClaim => serde_json::json!({
                "kind": "fail_without_acdc_claim",
            }),
        }
    }

    fn manual_resolution_evidence_spec_json(
        manual: &ManualResolutionEvidenceSpec,
    ) -> serde_json::Value {
        serde_json::json!({
            "authorization": manual_resolution_authorization_spec_json(&manual.authorization),
            "evidence_schema": manual.evidence_schema.as_str(),
        })
    }

    fn manual_resolution_authorization_spec_json(
        authorization: &mfm_spec::v1::ManualResolutionAuthorizationSpec,
    ) -> serde_json::Value {
        serde_json::json!({
            "authority": {
                "authority_id": authorization.authority.authority_id.as_str(),
                "operators": authorization
                    .authority
                    .operators
                    .iter()
                    .map(|operator| {
                        serde_json::json!({
                            "operator_id": operator.operator_id.as_str(),
                            "public_identity": operator.public_identity.as_str(),
                        })
                    })
                    .collect::<Vec<_>>(),
            },
            "quorum": {
                "kind": "threshold",
                "required_signatures": authorization.quorum.required_signatures(),
            },
            "signing_scheme": authorization.signing_scheme.as_str(),
            "verifier_id": authorization.verifier_id.as_str(),
        })
    }

    fn store_artifact_json(evidence: &ArtifactEvidenceRef) -> serde_json::Value {
        serde_json::json!({
            "artifact_id": evidence.artifact_id.as_str(),
            "artifact_role": artifact_role_str(evidence.artifact_role),
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
            "media_type": evidence.media_type.as_str(),
            "role": artifact_role_str(evidence.role),
            "schema_id": evidence.schema_id.as_str(),
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

    fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
        serde_json::json!({
            "binary_digest": identity.binary_digest.as_str(),
            "cargo_package_digest": identity.cargo_package_digest.as_str(),
            "cargo_package_name": identity.cargo_package_name.as_str(),
            "cargo_package_version": identity.cargo_package_version.as_str(),
            "factory_id": identity.factory_id.as_str(),
            "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(|value| value.as_str()),
            "nix_output_hash": identity.nix_output_hash.as_ref().map(|value| value.as_str()),
            "source_revision": identity.source_revision.as_str(),
        })
    }

    fn descriptor_identity_json(identity: &DescriptorIdentity) -> serde_json::Value {
        match identity {
            DescriptorIdentity::State(identity) => serde_json::json!({
                "capabilities": capability_set_json(&identity.capabilities),
                "config_schema_id": identity.config_schema_id.as_str(),
                "descriptor_family": "state",
                "descriptor_id": identity.descriptor_id.as_str(),
                "effect_class": identity.effect_class.as_str(),
                "effect_kind": identity.effect_kind.as_str(),
                "effect_name": identity.effect_name.as_str(),
                "effect_version": identity.effect_version.as_str(),
                "input_schema_id": identity.input_schema_id.as_str(),
                "name": identity.name.as_str(),
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
            events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } => {
                serde_json::json!({
                    "forward_ledger_key": forward_ledger_key.as_str(),
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
    pub fn run_completion_outcome_json(
        outcome: &events::RunCompletionOutcome,
    ) -> serde_json::Value {
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
            SagaEngagementReason::ForwardAmbiguous { ledger_key } => serde_json::json!({
                "kind": "forward_ambiguous",
                "ledger_key": ledger_key.as_str(),
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
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
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
        };
        serde_json::json!({
            "attempt_id": projection.attempt_id.as_str(),
            "event_id": projection.event_id.as_str(),
            "node_id": projection.node_id.as_str(),
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
            other => {
                return Err(CodecError::Identity(format!(
                    "unknown attempt status {other}"
                )));
            }
        };
        Ok(AttemptProjection {
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            event_id: parse_identity(required_str(json, "event_id")?)?,
            status,
        })
    }

    /// Encodes a cell terminal projection as JSON.
    pub fn cell_projection_json(
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
            } => serde_json::json!({
                "variant": "produced",
                "cell_id": cell_id.as_str(),
                "event_id": event_id.as_str(),
                "node_id": node_id.as_str(),
                "attempt_id": attempt_id.as_str(),
                "schema_id": schema_id.as_str(),
                "semantic_type_id": semantic_type_id.as_str(),
                "artifact_id": artifact_id.as_str(),
                "content_digest": content_digest.as_str(),
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
    ) -> CodecResult<(CellId, CellTerminalProjection)> {
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
        Ok((cell_id, projection))
    }

    /// Encodes a fact projection as JSON.
    pub fn fact_projection_json(projection: &FactProjection) -> serde_json::Value {
        serde_json::json!({
            "adapter_kind": projection.adapter_kind.as_str(),
            "adapter_version": projection.adapter_version.as_str(),
            "artifact_id": projection.artifact_id.as_str(),
            "attempt_id": projection.attempt_id.as_str(),
            "capability_kind": projection.capability_kind.as_str(),
            "capability_version": projection.capability_version.as_str(),
            "event_id": projection.event_id.as_str(),
            "fact_key": projection.fact_key.as_str(),
            "node_id": projection.node_id.as_str(),
            "request_hash": projection.request_hash.as_str(),
            "request_schema_id": projection.request_schema_id.as_str(),
            "response_hash": projection.response_hash.as_str(),
            "response_schema_id": projection.response_schema_id.as_str(),
        })
    }

    /// Parses a fact projection from JSON.
    pub fn parse_fact_projection(json: &serde_json::Value) -> CodecResult<FactProjection> {
        Ok(FactProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            fact_key: events::FactKey::new(required_str(json, "fact_key")?)?,
            request_schema_id: parse_identity(required_str(json, "request_schema_id")?)?,
            request_hash: parse_identity(required_str(json, "request_hash")?)?,
            response_schema_id: parse_identity(required_str(json, "response_schema_id")?)?,
            response_hash: parse_identity(required_str(json, "response_hash")?)?,
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
            capability_version: required_str(json, "capability_version")?.parse()?,
            adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
            adapter_version: required_str(json, "adapter_version")?.parse()?,
        })
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
    pub fn parse_side_effect_projection(
        json: &serde_json::Value,
    ) -> CodecResult<SideEffectProjection> {
        Ok(SideEffectProjection {
            run_id: parse_identity(required_str(json, "run_id")?)?,
            ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
            ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                json,
                "ledger_purpose",
            )?)?,
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
            "schema_id": artifact.schema_id.as_ref().map(SchemaId::as_str),
        })
    }

    fn parse_side_effect_artifact(
        json: &serde_json::Value,
    ) -> CodecResult<SideEffectArtifactProjection> {
        Ok(SideEffectArtifactProjection {
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
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
            "event_id": projection.event_id.as_str(),
            "invocation_epoch": projection.invocation_epoch,
            "key": lane_key.key.as_str(),
            "ledger_key": projection.holder.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&projection.ledger_purpose),
            "namespace": lane_key.namespace.as_str(),
            "node_id": projection.node_id.as_str(),
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
            key: events::ResourceKey::new(required_str(json, "key")?)?,
        };
        let projection = ResourceLaneProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            holder: SideEffectLedgerRef::new(
                parse_identity(required_str(json, "run_id")?)?,
                events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
            ),
            ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                json,
                "ledger_purpose",
            )?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            invocation_epoch: required_u32(json, "invocation_epoch")?,
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

    fn parse_side_effect_intent(
        json: &serde_json::Value,
    ) -> CodecResult<SideEffectIntentProjection> {
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
            idempotency_key: events::IdempotencyKeyRef::new(required_str(
                json,
                "idempotency_key",
            )?)?,
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
            "confirmation_observed" => {
                Ok(SideEffectPhase::ConfirmationObserved { invocation_epoch })
            }
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
                "public_schema_id": schema_id.as_str(),
                "event_id": event_id.as_str(),
                "rendered_digest": rendered_digest.as_str(),
                "rendered_artifact_id": rendered_artifact_id.as_ref().map(ArtifactId::as_str),
            }),
            PublicOutputProjection::RenderFailed { event_id, error } => serde_json::json!({
                "variant": "render_failed",
                "public_schema_id": schema_id.as_str(),
                "event_id": event_id.as_str(),
                "error": error_info_json(error),
            }),
        }
    }

    /// Parses a public-output projection from JSON.
    pub fn parse_public_output_projection(
        json: &serde_json::Value,
    ) -> CodecResult<(SchemaId, PublicOutputProjection)> {
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
        Ok((schema_id, projection))
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
            "role": artifact_role_str(retention_ref.role),
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

    /// Returns the canonical tag for an artifact role.
    pub fn artifact_role_str(role: ArtifactRole) -> &'static str {
        match role {
            ArtifactRole::TypedExecutionSpec => "typed_execution_spec",
            ArtifactRole::TypedSpecCertificate => "typed_spec_certificate",
            ArtifactRole::TypedConfig => "typed_config",
            ArtifactRole::SeedInput => "seed_input",
            ArtifactRole::StateOutput => "state_output",
            ArtifactRole::FactResponse => "fact_response",
            ArtifactRole::SideEffectIntent => "side_effect_intent",
            ArtifactRole::PreparedInvocation => "prepared_invocation",
            ArtifactRole::NotSubmittedProof => "not_submitted_proof",
            ArtifactRole::Submission => "submission",
            ArtifactRole::SubmissionUnknownEvidence => "submission_unknown_evidence",
            ArtifactRole::Receipt => "receipt",
            ArtifactRole::Confirmation => "confirmation",
            ArtifactRole::AmbiguityEvidence => "ambiguity_evidence",
            ArtifactRole::ManualResolutionEvidence => "manual_resolution_evidence",
            ArtifactRole::ManualResolutionAuthorization => "manual_resolution_authorization",
            ArtifactRole::PublicOutput => "public_output",
            ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
            ArtifactRole::RetentionManifest => "retention_manifest",
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
            events::RetentionReason::RunStarted => "run_started",
            events::RetentionReason::RuntimeEvidence => "runtime_evidence",
            events::RetentionReason::PublicOutput => "public_output",
            events::RetentionReason::ManifestProjection => "manifest_projection",
        }
    }
}
