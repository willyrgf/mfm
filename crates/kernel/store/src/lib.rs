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

    use mfm_canonical::PlainCanonicalJsonBytes;
    use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
        CellId, ContentDigest, DigestAlgorithm, EventId, IdentityError, NodeId, RunId, SchemaId,
        ScopeId, SeedId, SemanticTypeId, SpecHash, StateKind, StateVersion,
    };
    use mfm_spec::v1::{CellProducer, DescriptorIdentity, MediaType, ValueLineageRef};

    /// Result type for typed store helpers.
    pub type Result<T> = std::result::Result<T, StoreError>;

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
                Self::DuplicateLogicalKey { logical_key } => {
                    write!(f, "duplicate logical key {logical_key}")
                }
                Self::LogicalKeyConflict { logical_key } => {
                    write!(f, "logical key conflict {logical_key}")
                }
                Self::ProjectionConflict { key, message } => {
                    write!(f, "projection conflict for {key}: {message}")
                }
                Self::Identity(message) => write!(f, "identity error: {message}"),
                Self::Serialize(message) => write!(f, "store JSON serialization error: {message}"),
                Self::Canonical(message) => write!(f, "store canonicalization error: {message}"),
                Self::Event(message) => write!(f, "event contract error: {message}"),
            }
        }
    }

    impl std::error::Error for StoreError {}

    impl From<IdentityError> for StoreError {
        fn from(error: IdentityError) -> Self {
            Self::Identity(error.to_string())
        }
    }

    impl From<mfm_events::EventError> for StoreError {
        fn from(error: mfm_events::EventError) -> Self {
            Self::Event(error.to_string())
        }
    }

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

    impl KernelEventEnvelope {
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
        /// Run has completed, failed, or been cancelled.
        Completed,
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
        /// Ledger key.
        pub ledger_key: events::SideEffectLedgerKey,
        /// Last event id that updated this projection.
        pub event_id: EventId,
        /// Intent evidence that opened this ledger key.
        pub intent: SideEffectIntentProjection,
        /// Active claim, when one exists.
        pub claim: Option<SideEffectClaimProjection>,
        /// Current projected phase.
        pub phase: SideEffectPhase,
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
        /// Manifest artifact id.
        pub manifest_artifact_id: ArtifactId,
    }

    /// Store-owned projection snapshot derived from authoritative run streams.
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct ProjectionSnapshot {
        run_states: BTreeMap<RunId, RunState>,
        attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
        cells: BTreeMap<CellId, CellTerminalProjection>,
        side_effects: BTreeMap<events::SideEffectLedgerKey, SideEffectProjection>,
        public_outputs: BTreeMap<SchemaId, PublicOutputProjection>,
        retentions: BTreeMap<RunId, RetentionProjection>,
    }

    impl ProjectionSnapshot {
        /// Rebuilds projections from store-owned event envelopes.
        pub fn rebuild_from_run_stream(events: &[KernelEventEnvelope]) -> Result<Self> {
            let mut snapshot = Self::default();
            for event in events {
                apply_projection(&mut snapshot, event)?;
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

        /// Returns a cell terminal projection.
        pub fn cell_terminal(&self, cell_id: &CellId) -> Option<&CellTerminalProjection> {
            self.cells.get(cell_id)
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
            self.side_effects.get(ledger_key)
        }

        /// Returns a public-output projection.
        pub fn public_output(&self, schema_id: &SchemaId) -> Option<&PublicOutputProjection> {
            self.public_outputs.get(schema_id)
        }

        /// Returns a retention projection.
        pub fn retention(&self, run_id: &RunId) -> Option<&RetentionProjection> {
            self.retentions.get(run_id)
        }

        /// Returns whether any public output is projected.
        pub fn has_public_output(&self) -> bool {
            !self.public_outputs.is_empty()
        }
    }

    /// Read-only access to store-owned projections.
    pub trait TypedProjectionRead {
        /// Returns the current projection snapshot.
        fn projection_snapshot(&self) -> &ProjectionSnapshot;
    }

    /// Typed run event store commit contract.
    pub trait TypedRunEventStore: TypedProjectionRead {
        /// Records artifact evidence before events reference that artifact.
        fn record_artifact_evidence(&mut self, evidence: ArtifactEvidenceRef) -> Result<()>;

        /// Appends one typed run commit or returns an idempotent previous batch.
        fn append_typed_run_commit(&mut self, request: TypedCommitRequest)
            -> Result<CommitOutcome>;

        /// Loads the authoritative run stream.
        fn load_run_stream(&self, run_id: &RunId) -> Vec<KernelEventEnvelope>;

        /// Returns the next store-owned stream sequence for a run.
        fn expected_next_seq(&self, run_id: &RunId) -> StreamSeq;
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct CommitKeyRecord {
        fingerprint: CommitFingerprint,
        batch: CommittedBatch,
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

        fn validate_artifact_requirement(&self, requirement: &ArtifactRequirement) -> Result<()> {
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
                let actual = self.projections.side_effect(&precondition.ledger_key);
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

    impl TypedProjectionRead for InMemoryTypedRunStore {
        fn projection_snapshot(&self) -> &ProjectionSnapshot {
            &self.projections
        }
    }

    impl TypedRunEventStore for InMemoryTypedRunStore {
        fn record_artifact_evidence(&mut self, evidence: ArtifactEvidenceRef) -> Result<()> {
            if let Some(existing) = self.artifacts.get(&evidence.artifact_id) {
                if existing != &evidence {
                    return Err(StoreError::ArtifactEvidenceMismatch {
                        artifact_id: evidence.artifact_id,
                        field: "artifact",
                    });
                }
                return Ok(());
            }
            self.artifacts
                .insert(evidence.artifact_id.clone(), evidence);
            Ok(())
        }

        fn append_typed_run_commit(
            &mut self,
            request: TypedCommitRequest,
        ) -> Result<CommitOutcome> {
            let fingerprint = commit_fingerprint(&request)?;

            if let Some(record) = self
                .commit_keys
                .get(&(request.run_id.clone(), request.commit_key.clone()))
            {
                if record.fingerprint == fingerprint {
                    return Ok(CommitOutcome::Idempotent(record.batch.clone()));
                }
                return Err(StoreError::CommitConflict {
                    commit_key: request.commit_key,
                });
            }

            let actual_next_seq = self.expected_next_seq(&request.run_id);
            if request.expected_next_seq != actual_next_seq {
                return Err(StoreError::StaleExpectedNextSeq {
                    expected: request.expected_next_seq,
                    actual: actual_next_seq,
                });
            }

            validate_payload_run_and_spec(&request.run_id, &request.payloads)?;
            validate_terminal_attempt_cell_pairs(&request.payloads)?;
            self.validate_preconditions(&request)?;

            for evidence in &request.required_artifacts {
                self.validate_artifact_evidence(evidence)?;
            }
            for payload in &request.payloads {
                for requirement in artifact_requirements(payload) {
                    self.validate_artifact_requirement(&requirement)?;
                }
            }

            let mut staged_projections = self.projections.clone();
            let mut staged_logical_keys = self.logical_keys.clone();
            let mut staged_unique_payloads = self.unique_logical_payloads.clone();
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
                    if let Some(existing_hash) = staged_unique_payloads.get(&key) {
                        if existing_hash == &envelope.payload_hash {
                            return Err(StoreError::DuplicateLogicalKey {
                                logical_key: envelope.logical_key.clone(),
                            });
                        }
                        return Err(StoreError::LogicalKeyConflict {
                            logical_key: envelope.logical_key.clone(),
                        });
                    }
                    staged_unique_payloads.insert(key.clone(), envelope.payload_hash.clone());
                }
                staged_logical_keys.insert(key);
                apply_projection(&mut staged_projections, &envelope)?;
                events.push(envelope);
            }

            let batch = CommittedBatch {
                run_id: request.run_id.clone(),
                commit_key: request.commit_key.clone(),
                fingerprint: fingerprint.clone(),
                seq: request.expected_next_seq,
                events,
            };
            self.streams
                .entry(request.run_id.clone())
                .or_default()
                .push(batch.clone());
            self.commit_keys.insert(
                (request.run_id, request.commit_key),
                CommitKeyRecord {
                    fingerprint,
                    batch: batch.clone(),
                },
            );
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
        Ok(())
    }

    fn payload_run_id(payload: &KernelEventPayload) -> Option<RunId> {
        match payload {
            KernelEventPayload::RunStarted(payload) => Some(payload.run_id.clone()),
            KernelEventPayload::RunCompleted(payload) => Some(payload.run_id.clone()),
            KernelEventPayload::RetentionRefsAppended(payload) => Some(payload.run_id.clone()),
            KernelEventPayload::RetentionManifestProjected(payload) => Some(payload.run_id.clone()),
            _ => None,
        }
    }

    fn payload_spec_hash(payload: &KernelEventPayload) -> SpecHash {
        match payload {
            KernelEventPayload::RunStarted(payload) => payload.spec_hash.clone(),
            KernelEventPayload::StateAttemptStarted(payload) => payload.spec_hash.clone(),
            KernelEventPayload::FactRecorded(payload) => payload.spec_hash.clone(),
            KernelEventPayload::ArtifactReferenced(payload) => payload.spec_hash.clone(),
            KernelEventPayload::CellProduced(payload) => payload.spec_hash.clone(),
            KernelEventPayload::CellSkipped(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectIntentPersisted(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectClaimed(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectClaimTakenOver(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectInvocationPrepared(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectInvocationStarted(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectSubmissionObserved(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectReceiptObserved(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                payload.spec_hash.clone()
            }
            KernelEventPayload::SideEffectAmbiguous(payload) => payload.spec_hash.clone(),
            KernelEventPayload::SideEffectFailed(payload) => payload.spec_hash.clone(),
            KernelEventPayload::PublicOutputProduced(payload) => payload.spec_hash.clone(),
            KernelEventPayload::PublicOutputRenderFailed(payload) => payload.spec_hash.clone(),
            KernelEventPayload::StateAttemptCompleted(payload) => payload.spec_hash.clone(),
            KernelEventPayload::StateAttemptFailed(payload) => payload.spec_hash.clone(),
            KernelEventPayload::RunCompleted(payload) => payload.spec_hash.clone(),
            KernelEventPayload::RetentionRefsAppended(payload) => payload.spec_hash.clone(),
            KernelEventPayload::RetentionManifestProjected(payload) => payload.spec_hash.clone(),
        }
    }

    /// Returns canonical JSON bytes for a typed event payload.
    pub fn payload_canonical_json(payload: &KernelEventPayload) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(payload_json(payload))
    }

    /// Returns the canonical payload hash for a typed event payload.
    pub fn payload_hash(payload: &KernelEventPayload) -> Result<ContentDigest> {
        Ok(payload_canonical_json(payload)?.content_digest())
    }

    fn commit_fingerprint(request: &TypedCommitRequest) -> Result<CommitFingerprint> {
        let mut required_artifacts = request
            .required_artifacts
            .iter()
            .map(store_artifact_json)
            .collect::<Vec<_>>();
        required_artifacts.sort_by_key(serde_json::Value::to_string);

        let canonical = canonical_json(serde_json::json!({
            "commit_key": request.commit_key.as_str(),
            "payloads": request.payloads.iter().map(payload_json).collect::<Vec<_>>(),
            "preconditions": preconditions_json(&request.preconditions),
            "required_artifacts": required_artifacts,
            "run_id": request.run_id.as_str(),
        }))?;
        Ok(CommitFingerprint(canonical.content_digest()))
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
                format!("sidefx:{}:intent", payload.ledger_key)
            }
            KernelEventPayload::SideEffectClaimed(payload) => format!(
                "sidefx:{}:claim:{}:{}",
                payload.ledger_key, payload.invocation_epoch, payload.claim_generation
            ),
            KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
                "sidefx:{}:claim:{}:{}:taken_over",
                payload.ledger_key, payload.invocation_epoch, payload.claim_generation
            ),
            KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
                "sidefx:{}:invocation:{}:prepared",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
                "sidefx:{}:invocation:{}:started",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
                "sidefx:{}:invocation:{}:submission_result",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
                "sidefx:{}:invocation:{}:submission_result",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
                "sidefx:{}:invocation:{}:submission_result",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
                "sidefx:{}:invocation:{}:receipt",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
                "sidefx:{}:invocation:{}:confirmation",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                format!("sidefx:{}:ambiguous", payload.ledger_key)
            }
            KernelEventPayload::SideEffectFailed(payload) => format!(
                "sidefx:{}:invocation:{}:failure",
                payload.ledger_key, payload.invocation_epoch
            ),
            KernelEventPayload::PublicOutputProduced(payload) => {
                format!("public_output:{}", payload.public_schema_id)
            }
            KernelEventPayload::PublicOutputRenderFailed(payload) => {
                format!("public_output:{}", payload.public_schema_id)
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ArtifactRequirement {
        artifact_id: ArtifactId,
        digest: Option<ContentDigest>,
        byte_len: Option<u64>,
        media_type: Option<MediaType>,
        schema_id: Option<SchemaId>,
        semantic_type_id: Option<SemanticTypeId>,
        producer_node_id: Option<NodeId>,
        producer_seed_id: Option<SeedId>,
        artifact_role: Option<ArtifactRole>,
    }

    fn artifact_requirements(payload: &KernelEventPayload) -> Vec<ArtifactRequirement> {
        let mut requirements = Vec::new();
        match payload {
            KernelEventPayload::RunStarted(payload) => {
                for seed in &payload.seed_cells {
                    push_event_artifact(
                        &mut requirements,
                        &seed.seed_artifact,
                        None,
                        Some(seed.seed_id.clone()),
                    );
                }
            }
            KernelEventPayload::FactRecorded(payload) => requirements.push(ArtifactRequirement {
                artifact_id: payload.artifact_id.clone(),
                digest: Some(payload.response_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.response_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::FactResponse),
            }),
            KernelEventPayload::ArtifactReferenced(payload) => {
                push_event_artifact(
                    &mut requirements,
                    &payload.artifact_ref,
                    payload.node_id.clone(),
                    None,
                );
            }
            KernelEventPayload::CellProduced(payload) => requirements.push(ArtifactRequirement {
                artifact_id: payload.artifact_id.clone(),
                digest: Some(payload.content_digest.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.schema_id.clone()),
                semantic_type_id: Some(payload.semantic_type_id.clone()),
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::StateOutput),
            }),
            KernelEventPayload::PublicOutputProduced(payload) => {
                for cell in &payload.cells {
                    let (producer_node_id, producer_seed_id) =
                        cell_producer_artifact_owner(&cell.producer);
                    requirements.push(ArtifactRequirement {
                        artifact_id: cell.artifact_id.clone(),
                        digest: Some(cell.content_digest.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: Some(cell.schema_id.clone()),
                        semantic_type_id: Some(cell.semantic_type_id.clone()),
                        producer_node_id,
                        producer_seed_id,
                        artifact_role: None,
                    });
                }
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    requirements.push(ArtifactRequirement {
                        artifact_id: artifact_id.clone(),
                        digest: Some(payload.rendered_digest.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: Some(payload.public_schema_id.clone()),
                        semantic_type_id: None,
                        producer_node_id: Some(payload.node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: Some(ArtifactRole::PublicOutput),
                    });
                }
            }
            KernelEventPayload::PublicOutputRenderFailed(payload) => {
                if let Some(ref evidence) = payload.error.diagnostic_ref {
                    push_event_artifact(
                        &mut requirements,
                        evidence,
                        Some(payload.node_id.clone()),
                        None,
                    );
                }
            }
            KernelEventPayload::StateAttemptFailed(payload) => {
                if let Some(ref evidence) = payload.error.diagnostic_ref {
                    push_event_artifact(
                        &mut requirements,
                        evidence,
                        Some(payload.node_id.clone()),
                        None,
                    );
                }
            }
            KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
                events::RunCompletionOutcome::Failed(error)
                | events::RunCompletionOutcome::Cancelled(error) => {
                    if let Some(ref evidence) = error.diagnostic_ref {
                        push_event_artifact(&mut requirements, evidence, None, None);
                    }
                }
                events::RunCompletionOutcome::Completed(_) => {}
            },
            KernelEventPayload::SideEffectIntentPersisted(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.intent_artifact_id.clone(),
                    digest: Some(payload.intent_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.intent_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::SideEffectIntent),
                });
            }
            KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                if let (Some(artifact_id), Some(hash)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    requirements.push(ArtifactRequirement {
                        artifact_id: artifact_id.clone(),
                        digest: Some(hash.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: None,
                        semantic_type_id: None,
                        producer_node_id: Some(payload.node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: Some(ArtifactRole::PreparedInvocation),
                    });
                }
            }
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.proof_artifact_id.clone(),
                    digest: Some(payload.proof_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.proof_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::NotSubmittedProof),
                });
            }
            KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.submission_artifact_id.clone(),
                    digest: Some(payload.submission_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.submission_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::Submission),
                });
            }
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.evidence_artifact_id.clone(),
                    digest: Some(payload.evidence_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.evidence_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::SubmissionUnknownEvidence),
                });
            }
            KernelEventPayload::SideEffectReceiptObserved(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.receipt_artifact_id.clone(),
                    digest: Some(payload.receipt_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.receipt_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::Receipt),
                });
            }
            KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.confirmation_artifact_id.clone(),
                    digest: Some(payload.confirmation_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.confirmation_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::Confirmation),
                });
            }
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.evidence_artifact_id.clone(),
                    digest: Some(payload.evidence_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.evidence_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::AmbiguityEvidence),
                });
            }
            KernelEventPayload::SideEffectFailed(payload) => {
                if let Some(ref evidence) = payload.error.diagnostic_ref {
                    push_event_artifact(
                        &mut requirements,
                        evidence,
                        Some(payload.node_id.clone()),
                        None,
                    );
                }
            }
            KernelEventPayload::RetentionRefsAppended(payload) => {
                for retention_ref in &payload.refs {
                    requirements.push(ArtifactRequirement {
                        artifact_id: retention_ref.artifact_id.clone(),
                        digest: Some(retention_ref.content_digest.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: None,
                        semantic_type_id: None,
                        producer_node_id: None,
                        producer_seed_id: None,
                        artifact_role: Some(retention_ref.role),
                    });
                }
            }
            KernelEventPayload::RetentionManifestProjected(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.manifest_artifact_id.clone(),
                    digest: Some(payload.manifest_digest.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::RetentionManifest),
                });
            }
            KernelEventPayload::StateAttemptStarted(_)
            | KernelEventPayload::CellSkipped(_)
            | KernelEventPayload::SideEffectClaimed(_)
            | KernelEventPayload::SideEffectClaimTakenOver(_)
            | KernelEventPayload::SideEffectInvocationStarted(_)
            | KernelEventPayload::StateAttemptCompleted(_) => {}
        }
        requirements
    }

    fn push_event_artifact(
        requirements: &mut Vec<ArtifactRequirement>,
        evidence: &events::ArtifactEvidenceRef,
        producer_node_id: Option<NodeId>,
        producer_seed_id: Option<SeedId>,
    ) {
        requirements.push(ArtifactRequirement {
            artifact_id: evidence.artifact_id.clone(),
            digest: Some(evidence.content_digest.clone()),
            byte_len: Some(evidence.byte_len),
            media_type: Some(evidence.media_type.clone()),
            schema_id: Some(evidence.schema_id.clone()),
            semantic_type_id: evidence.semantic_type_id.clone(),
            producer_node_id,
            producer_seed_id,
            artifact_role: Some(evidence.role),
        });
    }

    fn cell_producer_artifact_owner(producer: &CellProducer) -> (Option<NodeId>, Option<SeedId>) {
        match producer {
            CellProducer::Node(node_id) => (Some(node_id.clone()), None),
            CellProducer::Seed(seed_id) => (None, Some(seed_id.clone())),
        }
    }

    fn apply_projection(
        projections: &mut ProjectionSnapshot,
        envelope: &KernelEventEnvelope,
    ) -> Result<()> {
        match envelope.payload() {
            KernelEventPayload::RunStarted(payload) => {
                let state = projections.run_state(&payload.run_id);
                if state != RunState::Absent {
                    return Err(StoreError::ProjectionConflict {
                        key: "run:start".to_owned(),
                        message: "run already started".to_owned(),
                    });
                }
                projections
                    .run_states
                    .insert(payload.run_id.clone(), RunState::Started);
            }
            KernelEventPayload::RunCompleted(payload) => {
                let state = projections.run_state(&payload.run_id);
                if state != RunState::Started {
                    return Err(StoreError::ProjectionConflict {
                        key: "run:complete".to_owned(),
                        message: "run must be started and not completed".to_owned(),
                    });
                }
                projections
                    .run_states
                    .insert(payload.run_id.clone(), RunState::Completed);
            }
            KernelEventPayload::StateAttemptStarted(payload) => {
                let key = (payload.node_id.clone(), payload.attempt_id.clone());
                if projections.attempts.contains_key(&key) {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("attempt:{}:{}", payload.node_id, payload.attempt_id),
                        message: "attempt already started".to_owned(),
                    });
                }
                projections.attempts.insert(
                    key,
                    AttemptProjection {
                        node_id: payload.node_id.clone(),
                        attempt_id: payload.attempt_id.clone(),
                        event_id: envelope.event_id.clone(),
                        status: AttemptStatus::Started {
                            attempt_no: payload.attempt_no,
                            state_kind: payload.state_kind.clone(),
                            state_version: payload.state_version.clone(),
                        },
                    },
                );
            }
            KernelEventPayload::StateAttemptCompleted(payload) => {
                update_attempt_terminal_projection(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    envelope.event_id.clone(),
                    AttemptStatus::Completed {
                        output_cell_id: payload.output_cell_id.clone(),
                    },
                )?;
            }
            KernelEventPayload::StateAttemptFailed(payload) => {
                update_attempt_terminal_projection(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    envelope.event_id.clone(),
                    AttemptStatus::Failed {
                        retryable: payload.retryable,
                        error: Box::new(payload.error.clone()),
                    },
                )?;
            }
            KernelEventPayload::CellProduced(payload) => {
                if projections.cells.contains_key(&payload.cell_id) {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("cell:{}:terminal", payload.cell_id),
                        message: "cell already terminal".to_owned(),
                    });
                }
                projections.cells.insert(
                    payload.cell_id.clone(),
                    CellTerminalProjection::Produced {
                        event_id: envelope.event_id.clone(),
                        node_id: payload.node_id.clone(),
                        attempt_id: payload.attempt_id.clone(),
                        schema_id: payload.schema_id.clone(),
                        semantic_type_id: payload.semantic_type_id.clone(),
                        artifact_id: payload.artifact_id.clone(),
                        content_digest: payload.content_digest.clone(),
                    },
                );
            }
            KernelEventPayload::CellSkipped(payload) => {
                if projections.cells.contains_key(&payload.cell_id) {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("cell:{}:terminal", payload.cell_id),
                        message: "cell already terminal".to_owned(),
                    });
                }
                projections.cells.insert(
                    payload.cell_id.clone(),
                    CellTerminalProjection::Skipped {
                        event_id: envelope.event_id.clone(),
                        node_id: payload.node_id.clone(),
                        attempt_id: payload.attempt_id.clone(),
                        schema_id: payload.schema_id.clone(),
                        semantic_type_id: payload.semantic_type_id.clone(),
                        skip_reason: payload.skip_reason.clone(),
                    },
                );
            }
            KernelEventPayload::SideEffectIntentPersisted(payload) => {
                if projections.side_effects.contains_key(&payload.ledger_key) {
                    return Err(side_effect_projection_error(
                        &payload.ledger_key,
                        "intent already persisted",
                    ));
                }
                let intent = SideEffectIntentProjection {
                    node_id: payload.node_id.clone(),
                    attempt_id: payload.attempt_id.clone(),
                    scope_id: payload.scope_id.clone(),
                    invocation_epoch: payload.invocation_epoch,
                    intent_schema_id: payload.intent_schema_id.clone(),
                    intent_hash: payload.intent_hash.clone(),
                    intent_artifact_id: payload.intent_artifact_id.clone(),
                    idempotency_input_schema_id: payload.idempotency_input_schema_id.clone(),
                    idempotency_input_hash: payload.idempotency_input_hash.clone(),
                    idempotency_key: payload.idempotency_key.clone(),
                    capability_kind: payload.capability_kind.clone(),
                    capability_version: payload.capability_version.clone(),
                    adapter_kind: payload.adapter_kind.clone(),
                    adapter_version: payload.adapter_version.clone(),
                };
                projections.side_effects.insert(
                    payload.ledger_key.clone(),
                    SideEffectProjection {
                        ledger_key: payload.ledger_key.clone(),
                        event_id: envelope.event_id.clone(),
                        intent,
                        claim: None,
                        phase: SideEffectPhase::IntentPersisted {
                            invocation_epoch: payload.invocation_epoch,
                        },
                    },
                );
            }
            KernelEventPayload::SideEffectClaimed(payload) => {
                let previous = require_side_effect_phase(
                    projections,
                    &payload.ledger_key,
                    "intent",
                    |projection| {
                        matches!(projection.phase, SideEffectPhase::IntentPersisted { .. })
                    },
                )?;
                require_intent_context(
                    previous,
                    &payload.node_id,
                    &payload.attempt_id,
                    payload.invocation_epoch,
                )?;
                let intent = previous.intent.clone();
                let claim = SideEffectClaimProjection {
                    node_id: payload.node_id.clone(),
                    attempt_id: payload.attempt_id.clone(),
                    claim_owner: payload.claim_owner.clone(),
                    invocation_epoch: payload.invocation_epoch,
                    claim_generation: payload.claim_generation,
                    claim_fencing_token: payload.claim_fencing_token.clone(),
                };
                projections.side_effects.insert(
                    payload.ledger_key.clone(),
                    SideEffectProjection {
                        ledger_key: payload.ledger_key.clone(),
                        event_id: envelope.event_id.clone(),
                        intent,
                        claim: Some(claim),
                        phase: SideEffectPhase::Claimed {
                            claim_owner: payload.claim_owner.clone(),
                            invocation_epoch: payload.invocation_epoch,
                            claim_generation: payload.claim_generation,
                            claim_fencing_token: payload.claim_fencing_token.clone(),
                        },
                    },
                );
            }
            KernelEventPayload::SideEffectClaimTakenOver(payload) => {
                let previous = require_side_effect_phase(
                    projections,
                    &payload.ledger_key,
                    "claim or prepared",
                    |projection| {
                        matches!(
                            projection.phase,
                            SideEffectPhase::Claimed { .. }
                                | SideEffectPhase::InvocationPrepared { .. }
                        )
                    },
                )?;
                let old_claim = previous_claim(previous, &payload.ledger_key)?;
                require_claim_takeover_matches(&payload.ledger_key, old_claim, payload)?;
                let intent = previous.intent.clone();
                let claim = SideEffectClaimProjection {
                    node_id: payload.node_id.clone(),
                    attempt_id: payload.attempt_id.clone(),
                    claim_owner: payload.new_claim_owner.clone(),
                    invocation_epoch: payload.invocation_epoch,
                    claim_generation: payload.claim_generation,
                    claim_fencing_token: payload.claim_fencing_token.clone(),
                };
                projections.side_effects.insert(
                    payload.ledger_key.clone(),
                    SideEffectProjection {
                        ledger_key: payload.ledger_key.clone(),
                        event_id: envelope.event_id.clone(),
                        intent,
                        claim: Some(claim),
                        phase: SideEffectPhase::Claimed {
                            claim_owner: payload.new_claim_owner.clone(),
                            invocation_epoch: payload.invocation_epoch,
                            claim_generation: payload.claim_generation,
                            claim_fencing_token: payload.claim_fencing_token.clone(),
                        },
                    },
                );
            }
            KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                let previous = require_side_effect_phase(
                    projections,
                    &payload.ledger_key,
                    "claim",
                    |projection| matches!(projection.phase, SideEffectPhase::Claimed { .. }),
                )?;
                let claim = previous_claim(previous, &payload.ledger_key)?;
                require_claim_context(
                    &payload.ledger_key,
                    claim,
                    ExpectedClaimContext {
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        invocation_epoch: payload.invocation_epoch,
                        claim_generation: payload.claim_generation,
                        claim_fencing_token: &payload.claim_fencing_token,
                        claim_owner: None,
                    },
                )?;
                let intent = previous.intent.clone();
                projections.side_effects.insert(
                    payload.ledger_key.clone(),
                    SideEffectProjection {
                        ledger_key: payload.ledger_key.clone(),
                        event_id: envelope.event_id.clone(),
                        intent,
                        claim: Some(claim.clone()),
                        phase: SideEffectPhase::InvocationPrepared {
                            invocation_epoch: payload.invocation_epoch,
                            claim_generation: payload.claim_generation,
                            claim_fencing_token: payload.claim_fencing_token.clone(),
                        },
                    },
                );
            }
            KernelEventPayload::SideEffectInvocationStarted(payload) => {
                let previous = require_side_effect_phase(
                    projections,
                    &payload.ledger_key,
                    "prepared",
                    |projection| {
                        matches!(projection.phase, SideEffectPhase::InvocationPrepared { .. })
                    },
                )?;
                let claim = previous_claim(previous, &payload.ledger_key)?;
                require_claim_context(
                    &payload.ledger_key,
                    claim,
                    ExpectedClaimContext {
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        invocation_epoch: payload.invocation_epoch,
                        claim_generation: payload.claim_generation,
                        claim_fencing_token: &payload.claim_fencing_token,
                        claim_owner: Some(&payload.claim_owner),
                    },
                )?;
                let intent = previous.intent.clone();
                projections.side_effects.insert(
                    payload.ledger_key.clone(),
                    SideEffectProjection {
                        ledger_key: payload.ledger_key.clone(),
                        event_id: envelope.event_id.clone(),
                        intent,
                        claim: Some(claim.clone()),
                        phase: SideEffectPhase::InvocationStarted {
                            claim_owner: payload.claim_owner.clone(),
                            invocation_epoch: payload.invocation_epoch,
                            claim_generation: payload.claim_generation,
                            claim_fencing_token: payload.claim_fencing_token.clone(),
                        },
                    },
                );
            }
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "started",
                    },
                    |epoch| SideEffectPhase::NotSubmittedProven {
                        invocation_epoch: epoch,
                    },
                )?;
            }
            KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "started",
                    },
                    |epoch| SideEffectPhase::SubmissionObserved {
                        invocation_epoch: epoch,
                    },
                )?;
            }
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "started",
                    },
                    |epoch| SideEffectPhase::SubmissionUnknown {
                        invocation_epoch: epoch,
                    },
                )?;
            }
            KernelEventPayload::SideEffectReceiptObserved(payload) => {
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "submission_observed",
                    },
                    |epoch| SideEffectPhase::ReceiptObserved {
                        invocation_epoch: epoch,
                    },
                )?;
            }
            KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "receipt",
                    },
                    |epoch| SideEffectPhase::ConfirmationObserved {
                        invocation_epoch: epoch,
                    },
                )?;
            }
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "started",
                    },
                    |epoch| SideEffectPhase::Ambiguous {
                        invocation_epoch: epoch,
                    },
                )?;
            }
            KernelEventPayload::SideEffectFailed(payload) => {
                transition_side_effect_failure(projections, payload, envelope.event_id.clone())?;
            }
            KernelEventPayload::PublicOutputProduced(payload) => {
                if projections
                    .public_outputs
                    .contains_key(&payload.public_schema_id)
                {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("public_output:{}", payload.public_schema_id),
                        message: "public output already projected".to_owned(),
                    });
                }
                projections.public_outputs.insert(
                    payload.public_schema_id.clone(),
                    PublicOutputProjection::Produced {
                        event_id: envelope.event_id.clone(),
                        rendered_digest: payload.rendered_digest.clone(),
                        rendered_artifact_id: payload.rendered_artifact_id.clone(),
                    },
                );
            }
            KernelEventPayload::PublicOutputRenderFailed(payload) => {
                if projections
                    .public_outputs
                    .contains_key(&payload.public_schema_id)
                {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("public_output:{}", payload.public_schema_id),
                        message: "public output already projected".to_owned(),
                    });
                }
                projections.public_outputs.insert(
                    payload.public_schema_id.clone(),
                    PublicOutputProjection::RenderFailed {
                        event_id: envelope.event_id.clone(),
                        error: Box::new(payload.error.clone()),
                    },
                );
            }
            KernelEventPayload::RetentionRefsAppended(payload) => {
                let retention = projections
                    .retentions
                    .entry(payload.run_id.clone())
                    .or_default();
                for retention_ref in &payload.refs {
                    retention
                        .refs
                        .insert(retention_ref.artifact_id.clone(), retention_ref.clone());
                }
            }
            KernelEventPayload::RetentionManifestProjected(payload) => {
                let retention = projections
                    .retentions
                    .entry(payload.run_id.clone())
                    .or_default();
                if let Some(previous) = &retention.manifest {
                    if payload.manifest_seq <= previous.manifest_seq {
                        return Err(StoreError::ProjectionConflict {
                            key: format!("retention:{}:manifest", payload.run_id),
                            message: "manifest sequence must increase".to_owned(),
                        });
                    }
                }
                retention.manifest = Some(RetentionManifestProjection {
                    manifest_seq: payload.manifest_seq,
                    manifest_digest: payload.manifest_digest.clone(),
                    manifest_artifact_id: payload.manifest_artifact_id.clone(),
                });
            }
            KernelEventPayload::FactRecorded(_) | KernelEventPayload::ArtifactReferenced(_) => {}
        }
        Ok(())
    }

    fn update_attempt_terminal_projection(
        projections: &mut ProjectionSnapshot,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        event_id: EventId,
        status: AttemptStatus,
    ) -> Result<()> {
        let key = (node_id.clone(), attempt_id.clone());
        let Some(projection) = projections.attempts.get_mut(&key) else {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message: "attempt terminal event requires a started attempt".to_owned(),
            });
        };
        if !matches!(projection.status, AttemptStatus::Started { .. }) {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message: "attempt already terminal".to_owned(),
            });
        }
        projection.event_id = event_id;
        projection.status = status;
        Ok(())
    }

    fn side_effect_projection_error(
        ledger_key: &events::SideEffectLedgerKey,
        message: impl Into<String>,
    ) -> StoreError {
        StoreError::ProjectionConflict {
            key: format!("sidefx:{ledger_key}"),
            message: message.into(),
        }
    }

    fn require_side_effect_phase<'a>(
        projections: &'a ProjectionSnapshot,
        ledger_key: &events::SideEffectLedgerKey,
        expected: &'static str,
        predicate: impl FnOnce(&SideEffectProjection) -> bool,
    ) -> Result<&'a SideEffectProjection> {
        let Some(projection) = projections.side_effects.get(ledger_key) else {
            return Err(side_effect_projection_error(
                ledger_key,
                format!("missing side-effect projection; expected {expected}"),
            ));
        };
        if predicate(projection) {
            Ok(projection)
        } else {
            Err(side_effect_projection_error(
                ledger_key,
                format!("illegal side-effect transition; expected {expected}"),
            ))
        }
    }

    fn previous_claim<'a>(
        projection: &'a SideEffectProjection,
        ledger_key: &events::SideEffectLedgerKey,
    ) -> Result<&'a SideEffectClaimProjection> {
        projection.claim.as_ref().ok_or_else(|| {
            side_effect_projection_error(ledger_key, "side-effect transition requires active claim")
        })
    }

    fn require_intent_context(
        projection: &SideEffectProjection,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        invocation_epoch: u32,
    ) -> Result<()> {
        let ledger_key = &projection.ledger_key;
        if projection.intent.node_id != *node_id {
            return Err(side_effect_projection_error(
                ledger_key,
                "node id does not match intent projection",
            ));
        }
        if projection.intent.attempt_id != *attempt_id {
            return Err(side_effect_projection_error(
                ledger_key,
                "attempt id does not match intent projection",
            ));
        }
        if projection.intent.invocation_epoch != invocation_epoch {
            return Err(side_effect_projection_error(
                ledger_key,
                "invocation epoch does not match intent projection",
            ));
        }
        Ok(())
    }

    struct ExpectedClaimContext<'a> {
        node_id: &'a NodeId,
        attempt_id: &'a AttemptId,
        invocation_epoch: u32,
        claim_generation: u32,
        claim_fencing_token: &'a side_effect::ClaimFencingToken,
        claim_owner: Option<&'a events::RunnerInvocationId>,
    }

    fn require_claim_context(
        ledger_key: &events::SideEffectLedgerKey,
        claim: &SideEffectClaimProjection,
        expected: ExpectedClaimContext<'_>,
    ) -> Result<()> {
        if claim.node_id != *expected.node_id {
            return Err(side_effect_projection_error(
                ledger_key,
                "node id does not match active claim",
            ));
        }
        if claim.attempt_id != *expected.attempt_id {
            return Err(side_effect_projection_error(
                ledger_key,
                "attempt id does not match active claim",
            ));
        }
        if claim.invocation_epoch != expected.invocation_epoch {
            return Err(side_effect_projection_error(
                ledger_key,
                "invocation epoch does not match active claim",
            ));
        }
        if claim.claim_generation != expected.claim_generation {
            return Err(side_effect_projection_error(
                ledger_key,
                "claim generation does not match active claim",
            ));
        }
        if claim.claim_fencing_token != *expected.claim_fencing_token {
            return Err(side_effect_projection_error(
                ledger_key,
                "claim fencing token does not match active claim",
            ));
        }
        if let Some(claim_owner) = expected.claim_owner {
            if claim.claim_owner != *claim_owner {
                return Err(side_effect_projection_error(
                    ledger_key,
                    "claim owner does not match active claim",
                ));
            }
        }
        Ok(())
    }

    fn require_claim_takeover_matches(
        ledger_key: &events::SideEffectLedgerKey,
        claim: &SideEffectClaimProjection,
        payload: &side_effect::ClaimTakenOver,
    ) -> Result<()> {
        if claim.node_id != payload.node_id {
            return Err(side_effect_projection_error(
                ledger_key,
                "node id does not match active claim",
            ));
        }
        if claim.attempt_id != payload.attempt_id {
            return Err(side_effect_projection_error(
                ledger_key,
                "attempt id does not match active claim",
            ));
        }
        if claim.claim_owner != payload.previous_claim_owner {
            return Err(side_effect_projection_error(
                ledger_key,
                "previous claim owner does not match active claim",
            ));
        }
        if claim.claim_generation != payload.previous_claim_generation {
            return Err(side_effect_projection_error(
                ledger_key,
                "previous claim generation does not match active claim",
            ));
        }
        if payload.claim_generation <= payload.previous_claim_generation {
            return Err(side_effect_projection_error(
                ledger_key,
                "takeover claim generation must increase",
            ));
        }
        if claim.invocation_epoch != payload.invocation_epoch {
            return Err(side_effect_projection_error(
                ledger_key,
                "invocation epoch does not match active claim",
            ));
        }
        Ok(())
    }

    fn phase_matches_expected(phase: &SideEffectPhase, expected: &'static str) -> bool {
        match expected {
            "started" => matches!(phase, SideEffectPhase::InvocationStarted { .. }),
            "submission_observed" => matches!(phase, SideEffectPhase::SubmissionObserved { .. }),
            "receipt" => matches!(phase, SideEffectPhase::ReceiptObserved { .. }),
            "not_submitted" => matches!(phase, SideEffectPhase::NotSubmittedProven { .. }),
            _ => false,
        }
    }

    struct EpochOnlyTransition<'a> {
        ledger_key: &'a events::SideEffectLedgerKey,
        node_id: &'a NodeId,
        attempt_id: &'a AttemptId,
        event_id: EventId,
        invocation_epoch: u32,
        required_previous: &'static str,
    }

    fn transition_side_effect_epoch_only(
        projections: &mut ProjectionSnapshot,
        transition: EpochOnlyTransition<'_>,
        next_phase: impl FnOnce(u32) -> SideEffectPhase,
    ) -> Result<()> {
        let (intent, claim) = {
            let previous = require_side_effect_phase(
                projections,
                transition.ledger_key,
                transition.required_previous,
                |projection| {
                    phase_matches_expected(&projection.phase, transition.required_previous)
                },
            )?;
            let claim = previous_claim(previous, transition.ledger_key)?;
            require_claim_context(
                transition.ledger_key,
                claim,
                ExpectedClaimContext {
                    node_id: transition.node_id,
                    attempt_id: transition.attempt_id,
                    invocation_epoch: transition.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: &claim.claim_fencing_token,
                    claim_owner: Some(&claim.claim_owner),
                },
            )?;
            (previous.intent.clone(), claim.clone())
        };
        projections.side_effects.insert(
            transition.ledger_key.clone(),
            SideEffectProjection {
                ledger_key: transition.ledger_key.clone(),
                event_id: transition.event_id,
                intent,
                claim: Some(claim),
                phase: next_phase(transition.invocation_epoch),
            },
        );
        Ok(())
    }

    fn transition_side_effect_failure(
        projections: &mut ProjectionSnapshot,
        payload: &side_effect::Failed,
        event_id: EventId,
    ) -> Result<()> {
        let (intent, claim) = {
            let Some(previous) = projections.side_effects.get(&payload.ledger_key) else {
                return Err(side_effect_projection_error(
                    &payload.ledger_key,
                    "missing side-effect projection",
                ));
            };
            match payload.failure_phase {
                side_effect::FailurePhase::BeforeInvocationStarted => {
                    if !matches!(
                        previous.phase,
                        SideEffectPhase::IntentPersisted { .. }
                            | SideEffectPhase::Claimed { .. }
                            | SideEffectPhase::InvocationPrepared { .. }
                    ) {
                        return Err(side_effect_projection_error(
                            &payload.ledger_key,
                            "before-start failure requires intent, claim, or prepared phase",
                        ));
                    }
                    require_intent_context(
                        previous,
                        &payload.node_id,
                        &payload.attempt_id,
                        payload.invocation_epoch,
                    )?;
                    if let Some(claim) = &previous.claim {
                        if claim.invocation_epoch != payload.invocation_epoch {
                            return Err(side_effect_projection_error(
                                &payload.ledger_key,
                                "failure invocation epoch does not match active claim",
                            ));
                        }
                    }
                }
                side_effect::FailurePhase::AfterNotSubmittedProven => {
                    if !phase_matches_expected(&previous.phase, "not_submitted") {
                        return Err(side_effect_projection_error(
                            &payload.ledger_key,
                            "after-not-submitted failure requires not-submitted phase",
                        ));
                    }
                    let claim = previous_claim(previous, &payload.ledger_key)?;
                    require_claim_context(
                        &payload.ledger_key,
                        claim,
                        ExpectedClaimContext {
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                            claim_generation: claim.claim_generation,
                            claim_fencing_token: &claim.claim_fencing_token,
                            claim_owner: Some(&claim.claim_owner),
                        },
                    )?;
                }
            }
            (previous.intent.clone(), previous.claim.clone())
        };
        projections.side_effects.insert(
            payload.ledger_key.clone(),
            SideEffectProjection {
                ledger_key: payload.ledger_key.clone(),
                event_id,
                intent,
                claim,
                phase: SideEffectPhase::Failed {
                    invocation_epoch: payload.invocation_epoch,
                    failure_phase: payload.failure_phase,
                },
            },
        );
        Ok(())
    }

    fn payload_json(payload: &KernelEventPayload) -> serde_json::Value {
        match payload {
            KernelEventPayload::RunStarted(payload) => serde_json::json!({
                "canonicalizer_identity": payload.canonicalizer_identity.as_str(),
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
                "node_id": payload.node_id.as_str(),
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
                "node_id": payload.node_id.as_str(),
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "SideEffectInvocationStarted",
            }),
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
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
                "node_id": payload.node_id.as_str(),
                "spec_hash": payload.spec_hash.as_str(),
                "variant": "SideEffectSubmissionUnknown",
            }),
            KernelEventPayload::SideEffectReceiptObserved(payload) => serde_json::json!({
                "attempt_id": payload.attempt_id.as_str(),
                "invocation_epoch": payload.invocation_epoch,
                "ledger_key": payload.ledger_key.as_str(),
                "node_id": payload.node_id.as_str(),
                "receipt_artifact_id": payload.receipt_artifact_id.as_str(),
                "receipt_hash": payload.receipt_hash.as_str(),
                "receipt_schema_id": payload.receipt_schema_id.as_str(),
                "replay_verifier_id": payload.replay_verifier_id.as_str(),
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
                "node_id": payload.node_id.as_str(),
                "replay_verifier_id": payload.replay_verifier_id.as_str(),
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

    fn event_artifact_json(evidence: &events::ArtifactEvidenceRef) -> serde_json::Value {
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
                    "name": capability.name,
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

    fn skip_reason_json(reason: &events::SkipReason) -> serde_json::Value {
        serde_json::json!({
            "code": reason.code.as_str(),
            "safe_message": reason.safe_message.as_str(),
        })
    }

    fn error_info_json(error: &events::MfmErrorInfo) -> serde_json::Value {
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

    fn run_completion_outcome_json(outcome: &events::RunCompletionOutcome) -> serde_json::Value {
        match outcome {
            events::RunCompletionOutcome::Completed(evidence) => serde_json::json!({
                "kind": "completed",
                "public_output": {
                    "public_output_event_id": evidence.public_output_event_id.as_str(),
                    "public_output_schema_id": evidence.public_output_schema_id.as_str(),
                },
            }),
            events::RunCompletionOutcome::Failed(error) => serde_json::json!({
                "kind": "failed",
                "terminal_error": error_info_json(error),
            }),
            events::RunCompletionOutcome::Cancelled(error) => serde_json::json!({
                "kind": "cancelled",
                "terminal_error": error_info_json(error),
            }),
        }
    }

    fn retention_ref_json(retention_ref: &events::RetentionRef) -> serde_json::Value {
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

    fn artifact_role_str(role: ArtifactRole) -> &'static str {
        match role {
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
            ArtifactRole::PublicOutput => "public_output",
            ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
            ArtifactRole::RetentionManifest => "retention_manifest",
        }
    }

    fn error_category_str(category: events::ErrorCategory) -> &'static str {
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

    fn failure_phase_str(phase: side_effect::FailurePhase) -> &'static str {
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
