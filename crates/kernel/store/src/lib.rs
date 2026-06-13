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
        CanonicalizerIdentity, CellProducer, DescriptorIdentity, MediaType,
        OperationDescriptorIdentity, PublicFieldPath, RemediationUnresolvedSpec,
        RendererDescriptorIdentity, RendererKind, RendererVersion, ResourceNamespace,
        SagaPolicySpec, StateDescriptorIdentity, ValueLineageRef,
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

    impl From<mfm_capabilities::CapabilityError> for StoreError {
        fn from(error: mfm_capabilities::CapabilityError) -> Self {
            Self::Identity(error.to_string())
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
        /// Operator identity reference schema id.
        pub operator_identity_ref_schema_id: SchemaId,
        /// Operator identity reference hash.
        pub operator_identity_ref_hash: ContentDigest,
        /// Operator identity reference artifact id.
        pub operator_identity_ref_artifact_id: ArtifactId,
        /// Evidence schema id.
        pub evidence_schema_id: SchemaId,
        /// Evidence hash.
        pub evidence_hash: ContentDigest,
        /// Evidence artifact id.
        pub evidence_artifact_id: ArtifactId,
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
        /// Run id holding the lane.
        pub run_id: RunId,
        /// Ledger holding the lane.
        pub ledger_key: events::SideEffectLedgerKey,
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
        side_effects: BTreeMap<events::SideEffectLedgerKey, SideEffectProjection>,
        resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
        public_outputs: BTreeMap<SchemaId, PublicOutputProjection>,
        retentions: BTreeMap<RunId, RetentionProjection>,
    }

    impl ProjectionSnapshot {
        /// Creates a projection snapshot from storage-owned projection maps.
        pub fn from_parts(
            run_states: BTreeMap<RunId, RunState>,
            attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
            cells: BTreeMap<CellId, CellTerminalProjection>,
            facts: BTreeMap<(NodeId, AttemptId, events::FactKey), FactProjection>,
            side_effects: BTreeMap<events::SideEffectLedgerKey, SideEffectProjection>,
            public_outputs: BTreeMap<SchemaId, PublicOutputProjection>,
            retentions: BTreeMap<RunId, RetentionProjection>,
        ) -> Self {
            Self {
                run_states,
                run_completions: BTreeMap::new(),
                saga_engagements: BTreeMap::new(),
                manual_resolutions: BTreeMap::new(),
                attempts,
                cells,
                facts,
                side_effects,
                resource_lanes: BTreeMap::new(),
                public_outputs,
                retentions,
            }
        }

        /// Creates a projection snapshot from storage-owned projection maps, including saga maps.
        pub fn from_parts_with_saga(
            run_states: BTreeMap<RunId, RunState>,
            run_completions: BTreeMap<RunId, RunCompletionProjection>,
            saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
            manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
            attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
            cells: BTreeMap<CellId, CellTerminalProjection>,
            facts: BTreeMap<(NodeId, AttemptId, events::FactKey), FactProjection>,
            side_effects: BTreeMap<events::SideEffectLedgerKey, SideEffectProjection>,
            public_outputs: BTreeMap<SchemaId, PublicOutputProjection>,
            retentions: BTreeMap<RunId, RetentionProjection>,
        ) -> Self {
            Self {
                run_states,
                run_completions,
                saga_engagements,
                manual_resolutions,
                attempts,
                cells,
                facts,
                side_effects,
                resource_lanes: BTreeMap::new(),
                public_outputs,
                retentions,
            }
        }

        /// Creates a projection snapshot from storage-owned maps, including resource lanes.
        #[allow(clippy::too_many_arguments)]
        pub fn from_parts_with_saga_and_resource_lanes(
            run_states: BTreeMap<RunId, RunState>,
            run_completions: BTreeMap<RunId, RunCompletionProjection>,
            saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
            manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
            attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
            cells: BTreeMap<CellId, CellTerminalProjection>,
            facts: BTreeMap<(NodeId, AttemptId, events::FactKey), FactProjection>,
            side_effects: BTreeMap<events::SideEffectLedgerKey, SideEffectProjection>,
            resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
            public_outputs: BTreeMap<SchemaId, PublicOutputProjection>,
            retentions: BTreeMap<RunId, RetentionProjection>,
        ) -> Self {
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
            self.side_effects.get(ledger_key)
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
        ) -> impl Iterator<Item = (&events::SideEffectLedgerKey, &SideEffectProjection)> {
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

    fn derive_saga_projection(
        projections: &ProjectionSnapshot,
        run_id: &RunId,
        policy: &SagaPolicySpec,
    ) -> SagaProjection {
        let obligations = derive_saga_obligations(projections, run_id);
        let run_completion = projections.run_completion(run_id).cloned();
        let manual_resolution = projections.manual_resolution(run_id).cloned();
        let engagement = projections.saga_engagement(run_id).cloned();
        let forward_quiescent = forward_ledgers_quiescent(projections, run_id);
        let has_forward_boundary = obligations
            .values()
            .any(|obligation| forward_ledger_crossed_boundary(&obligation.forward_phase));
        let owed_count = obligations
            .values()
            .filter(|obligation| obligation.classification == ForwardLedgerClassification::Owed)
            .count();
        let all_owed_closed = owed_count > 0
            && obligations.values().all(|obligation| {
                obligation.classification != ForwardLedgerClassification::Owed
                    || obligation
                        .remediation
                        .as_ref()
                        .map(|remediation| remediation.closed)
                        .unwrap_or(false)
            });
        let unresolved_reason = obligations.values().find_map(obligation_unresolved_reason);

        let (run_mode, manual_block_reason) = if let Some(completion) = run_completion.as_ref() {
            (run_mode_for_completion_outcome(&completion.outcome), None)
        } else if engagement.is_none() || !forward_quiescent {
            (RunMode::Forward, None)
        } else if !has_forward_boundary || (owed_count == 0 && unresolved_reason.is_none()) {
            (RunMode::FailedWithoutAcdcClaim, None)
        } else {
            run_mode_for_uncompleted_quiescent_saga(
                policy,
                manual_resolution.as_ref(),
                owed_count,
                all_owed_closed,
                unresolved_reason,
            )
        };

        SagaProjection {
            run_id: run_id.clone(),
            run_mode,
            engagement,
            forward_quiescent,
            manual_block_reason,
            obligations,
            manual_resolution,
            run_completion,
        }
    }

    fn derive_saga_obligations(
        projections: &ProjectionSnapshot,
        run_id: &RunId,
    ) -> BTreeMap<events::SideEffectLedgerKey, SagaObligationProjection> {
        projections
            .side_effects
            .values()
            .filter(|projection| {
                projection.run_id == *run_id
                    && matches!(
                        projection.ledger_purpose,
                        events::SideEffectLedgerPurpose::Forward
                    )
            })
            .map(|forward| {
                let remediation = remediation_for_forward(projections, run_id, &forward.ledger_key);
                (
                    forward.ledger_key.clone(),
                    SagaObligationProjection {
                        forward_ledger_key: forward.ledger_key.clone(),
                        forward_phase: forward.phase.clone(),
                        classification: forward_ledger_classification(&forward.phase),
                        remediation,
                    },
                )
            })
            .collect()
    }

    fn remediation_for_forward(
        projections: &ProjectionSnapshot,
        run_id: &RunId,
        forward_ledger_key: &events::SideEffectLedgerKey,
    ) -> Option<RemediationLedgerProjection> {
        projections
            .side_effects
            .values()
            .find(|projection| {
                projection.run_id == *run_id
                    && matches!(
                        &projection.ledger_purpose,
                        events::SideEffectLedgerPurpose::Remediation {
                            forward_ledger_key: linked
                        } if linked == forward_ledger_key
                    )
            })
            .map(|projection| {
                let unresolved = remediation_unresolved_reason(projections, projection);
                RemediationLedgerProjection {
                    ledger_key: projection.ledger_key.clone(),
                    phase: projection.phase.clone(),
                    closed: matches!(
                        projection.phase,
                        SideEffectPhase::ConfirmationObserved { .. }
                    ),
                    unresolved,
                }
            })
    }

    fn run_mode_for_completion_outcome(outcome: &events::RunCompletionOutcome) -> RunMode {
        match outcome {
            events::RunCompletionOutcome::Completed(_) => RunMode::Completed,
            events::RunCompletionOutcome::Compensated => RunMode::Compensated,
            events::RunCompletionOutcome::ManuallyResolved => RunMode::ManuallyResolved,
            events::RunCompletionOutcome::FailedWithoutAcdcClaim => RunMode::FailedWithoutAcdcClaim,
        }
    }

    fn run_mode_for_uncompleted_quiescent_saga(
        policy: &SagaPolicySpec,
        manual_resolution: Option<&ManualResolutionProjection>,
        owed_count: usize,
        all_owed_closed: bool,
        unresolved_reason: Option<ManualBlockReason>,
    ) -> (RunMode, Option<ManualBlockReason>) {
        if let Some(mode) = manual_resolution.map(manual_resolution_run_mode) {
            return (mode, None);
        }

        match policy {
            SagaPolicySpec::NoSideEffects | SagaPolicySpec::FailWithoutAcdcClaim => {
                (RunMode::FailedWithoutAcdcClaim, None)
            }
            SagaPolicySpec::ManualResolution { .. } => (
                RunMode::ManualBlocked,
                Some(ManualBlockReason::PolicyManualResolution),
            ),
            SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved,
            } => {
                if let Some(reason) = unresolved_reason {
                    match on_remediation_unresolved {
                        RemediationUnresolvedSpec::ManualResolution { .. } => {
                            (RunMode::ManualBlocked, Some(reason))
                        }
                        RemediationUnresolvedSpec::FailWithoutAcdcClaim => {
                            (RunMode::FailedWithoutAcdcClaim, None)
                        }
                    }
                } else if owed_count > 0 && all_owed_closed {
                    (RunMode::Compensated, None)
                } else if owed_count > 0 {
                    (RunMode::Remediating, None)
                } else {
                    (RunMode::FailedWithoutAcdcClaim, None)
                }
            }
        }
    }

    fn manual_resolution_run_mode(manual: &ManualResolutionProjection) -> RunMode {
        match manual.outcome {
            events::ManualResolutionOutcome::ConfirmRemediated => RunMode::ManuallyResolved,
            events::ManualResolutionOutcome::FailWithoutAcdcClaim => {
                RunMode::FailedWithoutAcdcClaim
            }
        }
    }

    fn obligation_unresolved_reason(
        obligation: &SagaObligationProjection,
    ) -> Option<ManualBlockReason> {
        if obligation.classification == ForwardLedgerClassification::Unresolvable {
            Some(ManualBlockReason::ForwardAmbiguous)
        } else {
            obligation
                .remediation
                .as_ref()
                .and_then(|remediation| remediation.unresolved)
        }
    }

    fn remediation_unresolved_reason(
        projections: &ProjectionSnapshot,
        projection: &SideEffectProjection,
    ) -> Option<ManualBlockReason> {
        match projection.phase {
            SideEffectPhase::Ambiguous { .. } => Some(ManualBlockReason::RemediationAmbiguous),
            SideEffectPhase::Failed { .. } => {
                if side_effect_failure_retryable(projections, projection) == Some(false) {
                    Some(ManualBlockReason::RemediationFailed)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn side_effect_failure_retryable(
        projections: &ProjectionSnapshot,
        projection: &SideEffectProjection,
    ) -> Option<bool> {
        match &projections
            .attempt(&projection.intent.node_id, &projection.intent.attempt_id)?
            .status
        {
            AttemptStatus::Failed { retryable, .. } => Some(*retryable),
            _ => None,
        }
    }

    fn forward_ledger_classification(phase: &SideEffectPhase) -> ForwardLedgerClassification {
        match phase {
            SideEffectPhase::IntentPersisted { .. }
            | SideEffectPhase::Claimed { .. }
            | SideEffectPhase::InvocationPrepared { .. }
            | SideEffectPhase::NotSubmittedProven { .. }
            | SideEffectPhase::Failed { .. } => ForwardLedgerClassification::NothingOwed,
            SideEffectPhase::InvocationStarted { .. }
            | SideEffectPhase::SubmissionObserved { .. }
            | SideEffectPhase::SubmissionUnknown { .. }
            | SideEffectPhase::ReceiptObserved { .. } => ForwardLedgerClassification::Pending,
            SideEffectPhase::ConfirmationObserved { .. } => ForwardLedgerClassification::Owed,
            SideEffectPhase::Ambiguous { .. } => ForwardLedgerClassification::Unresolvable,
        }
    }

    fn forward_ledgers_quiescent(projections: &ProjectionSnapshot, run_id: &RunId) -> bool {
        projections
            .side_effects
            .values()
            .filter(|projection| {
                projection.run_id == *run_id
                    && matches!(
                        projection.ledger_purpose,
                        events::SideEffectLedgerPurpose::Forward
                    )
            })
            .all(|projection| {
                !forward_ledger_crossed_boundary(&projection.phase)
                    || forward_ledger_phase_is_quiescent(&projection.phase)
            })
    }

    fn forward_ledger_crossed_boundary(phase: &SideEffectPhase) -> bool {
        matches!(
            phase,
            SideEffectPhase::InvocationStarted { .. }
                | SideEffectPhase::SubmissionObserved { .. }
                | SideEffectPhase::SubmissionUnknown { .. }
                | SideEffectPhase::ReceiptObserved { .. }
                | SideEffectPhase::ConfirmationObserved { .. }
                | SideEffectPhase::Ambiguous { .. }
                | SideEffectPhase::NotSubmittedProven { .. }
                | SideEffectPhase::Failed {
                    failure_phase: side_effect::FailurePhase::AfterNotSubmittedProven,
                    ..
                }
        )
    }

    fn forward_ledger_phase_is_quiescent(phase: &SideEffectPhase) -> bool {
        matches!(
            phase,
            SideEffectPhase::NotSubmittedProven { .. }
                | SideEffectPhase::ConfirmationObserved { .. }
                | SideEffectPhase::Ambiguous { .. }
                | SideEffectPhase::Failed { .. }
        )
    }

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
        type Error: fmt::Display + Send + Sync + 'static;

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
        validate_side_effect_attempt_failure_pairs(&request.payloads)?;
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
            for requirement in artifact_requirements(payload) {
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
            apply_projection(&mut staged_projections, &envelope)?;
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

    fn validate_side_effect_attempt_failure_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
        let mut side_effect_failures = BTreeMap::new();
        let mut attempt_failures = BTreeMap::new();
        for payload in payloads {
            match payload {
                KernelEventPayload::SideEffectFailed(payload) => {
                    side_effect_failures.insert(
                        (payload.node_id.clone(), payload.attempt_id.clone()),
                        payload.retryable,
                    );
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
        for (key, side_effect_retryable) in &side_effect_failures {
            match attempt_failures.get(key) {
                Some(attempt_retryable) if attempt_retryable == side_effect_retryable => {}
                Some(_) => {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("sidefx:{}:{}:failure", key.0, key.1),
                        message: "side-effect failure retryability must match attempt failure"
                            .to_owned(),
                    });
                }
                None => {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("sidefx:{}:{}:failure", key.0, key.1),
                        message: "side-effect failure requires matching StateAttemptFailed in same commit"
                            .to_owned(),
                    });
                }
            }
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
        match payload {
            KernelEventPayload::RunStarted(payload) => Some(payload.run_id.clone()),
            KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload.run_id.clone()),
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
            KernelEventPayload::ManualResolutionRecorded(payload) => payload.spec_hash.clone(),
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
            projections.side_effect(ledger_key).map(|projection| &projection.phase),
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
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.spec_artifact_id.clone(),
                    digest: Some(ContentDigest::from_digest(
                        payload.spec_hash.algorithm(),
                        *payload.spec_hash.digest(),
                    )),
                    byte_len: None,
                    media_type: Some(payload.spec_media_type.clone()),
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::TypedExecutionSpec),
                });
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.certificate_artifact_id.clone(),
                    digest: Some(payload.certificate_artifact_digest.clone()),
                    byte_len: None,
                    media_type: Some(payload.certificate_media_type.clone()),
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::TypedSpecCertificate),
                });
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
            KernelEventPayload::ManualResolutionRecorded(payload) => {
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.operator_identity_ref_artifact_id.clone(),
                    digest: Some(payload.operator_identity_ref_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.operator_identity_ref_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: None,
                });
                requirements.push(ArtifactRequirement {
                    artifact_id: payload.evidence_artifact_id.clone(),
                    digest: Some(payload.evidence_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.evidence_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: None,
                });
            }
            KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
                events::RunCompletionOutcome::Completed(_) => {}
                events::RunCompletionOutcome::Compensated
                | events::RunCompletionOutcome::ManuallyResolved
                | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {}
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
                if let Some(touched_set) = &payload.resource_touched_set {
                    push_resource_touched_set_requirement(&mut requirements, touched_set);
                }
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
                if let Some(touched_set) = &payload.resource_touched_set {
                    push_resource_touched_set_requirement(&mut requirements, touched_set);
                }
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

    fn referenced_artifact_ids(request: &TypedCommitRequest) -> BTreeSet<ArtifactId> {
        let mut artifact_ids = BTreeSet::new();
        for payload in &request.payloads {
            for requirement in artifact_requirements(payload) {
                artifact_ids.insert(requirement.artifact_id);
            }
        }
        artifact_ids
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

    fn push_resource_touched_set_requirement(
        requirements: &mut Vec<ArtifactRequirement>,
        evidence: &events::ResourceTouchedSetEvidence,
    ) {
        requirements.push(ArtifactRequirement {
            artifact_id: evidence.evidence_artifact_id.clone(),
            digest: Some(evidence.evidence_hash.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(evidence.evidence_schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: None,
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
                require_forward_quiescence(projections, &payload.run_id)?;
                projections
                    .run_states
                    .insert(payload.run_id.clone(), RunState::Completed);
                projections.run_completions.insert(
                    payload.run_id.clone(),
                    RunCompletionProjection {
                        event_id: envelope.event_id.clone(),
                        outcome: payload.outcome.clone(),
                    },
                );
                release_resource_lanes_for_run(projections, &payload.run_id);
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
                if !payload.retryable {
                    note_saga_engagement(
                        projections,
                        envelope.run_id(),
                        SagaEngagementProjection {
                            event_id: envelope.event_id.clone(),
                            reason: SagaEngagementReason::NonRetryableFailure {
                                node_id: payload.node_id.clone(),
                                attempt_id: payload.attempt_id.clone(),
                            },
                        },
                    );
                }
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
                require_forward_fence_open(
                    projections,
                    envelope.run_id(),
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                require_remediation_intent_admissible(
                    projections,
                    envelope.run_id(),
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
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
                        run_id: envelope.run_id().clone(),
                        ledger_key: payload.ledger_key.clone(),
                        ledger_purpose: payload.ledger_purpose.clone(),
                        event_id: envelope.event_id.clone(),
                        intent,
                        prepared_invocation: None,
                        resource_key: None,
                        submission: None,
                        receipt: None,
                        confirmation: None,
                        resource_touched_set: None,
                        claim: None,
                        phase: SideEffectPhase::IntentPersisted {
                            invocation_epoch: payload.invocation_epoch,
                        },
                    },
                );
            }
            KernelEventPayload::SideEffectClaimed(payload) => {
                require_forward_fence_open(
                    projections,
                    envelope.run_id(),
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                let previous = require_side_effect_phase(
                    projections,
                    &payload.ledger_key,
                    "intent or not-submitted",
                    |projection| {
                        matches!(
                            projection.phase,
                            SideEffectPhase::IntentPersisted { .. }
                                | SideEffectPhase::NotSubmittedProven { .. }
                        )
                    },
                )?;
                require_side_effect_purpose(
                    previous,
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                match previous.phase {
                    SideEffectPhase::IntentPersisted { invocation_epoch } => {
                        if payload.invocation_epoch != invocation_epoch {
                            return Err(side_effect_projection_error(
                                &payload.ledger_key,
                                "initial claim invocation epoch does not match intent",
                            ));
                        }
                        require_intent_context(
                            previous,
                            &payload.node_id,
                            &payload.attempt_id,
                            payload.invocation_epoch,
                        )?;
                    }
                    SideEffectPhase::NotSubmittedProven { invocation_epoch } => {
                        require_intent_attempt_context(
                            previous,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        let previous_claim = previous_claim(previous, &payload.ledger_key)?;
                        if payload.claim_generation <= previous_claim.claim_generation {
                            return Err(side_effect_projection_error(
                                &payload.ledger_key,
                                "retry claim generation must increase",
                            ));
                        }
                        if payload.claim_fencing_token == previous_claim.claim_fencing_token {
                            return Err(side_effect_projection_error(
                                &payload.ledger_key,
                                "retry claim fencing token must change",
                            ));
                        }
                        let next_epoch = invocation_epoch.checked_add(1).ok_or_else(|| {
                            side_effect_projection_error(
                                &payload.ledger_key,
                                "invocation epoch overflow",
                            )
                        })?;
                        if payload.invocation_epoch != next_epoch {
                            return Err(side_effect_projection_error(
                                &payload.ledger_key,
                                "retry claim must advance to the next invocation epoch",
                            ));
                        }
                    }
                    _ => unreachable!("phase predicate checked above"),
                }
                let intent = previous.intent.clone();
                let run_id = previous.run_id.clone();
                let ledger_purpose = previous.ledger_purpose.clone();
                let prepared_invocation = previous.prepared_invocation.clone();
                let resource_key = previous.resource_key.clone();
                let submission = previous.submission.clone();
                let receipt = previous.receipt.clone();
                let confirmation = previous.confirmation.clone();
                let resource_touched_set = previous.resource_touched_set.clone();
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
                        run_id,
                        ledger_key: payload.ledger_key.clone(),
                        ledger_purpose,
                        event_id: envelope.event_id.clone(),
                        intent,
                        prepared_invocation,
                        resource_key,
                        submission,
                        receipt,
                        confirmation,
                        resource_touched_set,
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
                require_forward_fence_open(
                    projections,
                    envelope.run_id(),
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
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
                require_side_effect_purpose(
                    previous,
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                let old_claim = previous_claim(previous, &payload.ledger_key)?;
                require_claim_takeover_matches(&payload.ledger_key, old_claim, payload)?;
                let intent = previous.intent.clone();
                let run_id = previous.run_id.clone();
                let ledger_purpose = previous.ledger_purpose.clone();
                let prepared_invocation = previous.prepared_invocation.clone();
                let resource_key = previous.resource_key.clone();
                let submission = previous.submission.clone();
                let receipt = previous.receipt.clone();
                let confirmation = previous.confirmation.clone();
                let resource_touched_set = previous.resource_touched_set.clone();
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
                        run_id,
                        ledger_key: payload.ledger_key.clone(),
                        ledger_purpose,
                        event_id: envelope.event_id.clone(),
                        intent,
                        prepared_invocation,
                        resource_key,
                        submission,
                        receipt,
                        confirmation,
                        resource_touched_set,
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
                require_forward_fence_open(
                    projections,
                    envelope.run_id(),
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                let previous = require_side_effect_phase(
                    projections,
                    &payload.ledger_key,
                    "claim",
                    |projection| matches!(projection.phase, SideEffectPhase::Claimed { .. }),
                )?;
                require_side_effect_purpose(
                    previous,
                    &payload.ledger_key,
                    &payload.ledger_purpose,
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
                let run_id = previous.run_id.clone();
                let ledger_purpose = previous.ledger_purpose.clone();
                let prepared_invocation = prepared_invocation_projection(
                    &payload.prepared_artifact_id,
                    &payload.prepared_hash,
                    &payload.ledger_key,
                )?
                .or_else(|| previous.prepared_invocation.clone());
                let resource_key = payload
                    .resource_key
                    .clone()
                    .or_else(|| previous.resource_key.clone());
                let submission = previous.submission.clone();
                let receipt = previous.receipt.clone();
                let confirmation = previous.confirmation.clone();
                let resource_touched_set = previous.resource_touched_set.clone();
                let claim = claim.clone();
                acquire_resource_lane(projections, envelope.run_id(), &envelope.event_id, payload)?;
                projections.side_effects.insert(
                    payload.ledger_key.clone(),
                    SideEffectProjection {
                        run_id,
                        ledger_key: payload.ledger_key.clone(),
                        ledger_purpose,
                        event_id: envelope.event_id.clone(),
                        intent,
                        prepared_invocation,
                        resource_key,
                        submission,
                        receipt,
                        confirmation,
                        resource_touched_set,
                        claim: Some(claim),
                        phase: SideEffectPhase::InvocationPrepared {
                            invocation_epoch: payload.invocation_epoch,
                            claim_generation: payload.claim_generation,
                            claim_fencing_token: payload.claim_fencing_token.clone(),
                        },
                    },
                );
            }
            KernelEventPayload::SideEffectInvocationStarted(payload) => {
                require_forward_fence_open(
                    projections,
                    envelope.run_id(),
                    &payload.ledger_key,
                    &payload.ledger_purpose,
                )?;
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                let previous = require_side_effect_phase(
                    projections,
                    &payload.ledger_key,
                    "prepared",
                    |projection| {
                        matches!(projection.phase, SideEffectPhase::InvocationPrepared { .. })
                    },
                )?;
                require_side_effect_purpose(
                    previous,
                    &payload.ledger_key,
                    &payload.ledger_purpose,
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
                let run_id = previous.run_id.clone();
                let ledger_purpose = previous.ledger_purpose.clone();
                let prepared_invocation = previous.prepared_invocation.clone();
                let resource_key = previous.resource_key.clone();
                let submission = previous.submission.clone();
                let receipt = previous.receipt.clone();
                let confirmation = previous.confirmation.clone();
                let resource_touched_set = previous.resource_touched_set.clone();
                projections.side_effects.insert(
                    payload.ledger_key.clone(),
                    SideEffectProjection {
                        run_id,
                        ledger_key: payload.ledger_key.clone(),
                        ledger_purpose,
                        event_id: envelope.event_id.clone(),
                        intent,
                        prepared_invocation,
                        resource_key,
                        submission,
                        receipt,
                        confirmation,
                        resource_touched_set,
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
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        ledger_purpose: &payload.ledger_purpose,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "submission_recovery",
                    },
                    |epoch| SideEffectPhase::NotSubmittedProven {
                        invocation_epoch: epoch,
                    },
                    |_| Ok(()),
                )?;
                release_resource_lane_for_ledger(projections, &payload.ledger_key);
            }
            KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        ledger_purpose: &payload.ledger_purpose,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "submission_recovery",
                    },
                    |epoch| SideEffectPhase::SubmissionObserved {
                        invocation_epoch: epoch,
                    },
                    |projection| {
                        projection.submission = Some(SideEffectArtifactProjection {
                            artifact_id: payload.submission_artifact_id.clone(),
                            content_digest: payload.submission_hash.clone(),
                            schema_id: Some(payload.submission_schema_id.clone()),
                        });
                        Ok(())
                    },
                )?;
            }
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        ledger_purpose: &payload.ledger_purpose,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "submission_recovery",
                    },
                    |epoch| SideEffectPhase::SubmissionUnknown {
                        invocation_epoch: epoch,
                    },
                    |_| Ok(()),
                )?;
            }
            KernelEventPayload::SideEffectReceiptObserved(payload) => {
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        ledger_purpose: &payload.ledger_purpose,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "submission_observed",
                    },
                    |epoch| SideEffectPhase::ReceiptObserved {
                        invocation_epoch: epoch,
                    },
                    |projection| {
                        projection.receipt = Some(SideEffectArtifactProjection {
                            artifact_id: payload.receipt_artifact_id.clone(),
                            content_digest: payload.receipt_hash.clone(),
                            schema_id: Some(payload.receipt_schema_id.clone()),
                        });
                        if let Some(touched_set) = payload.resource_touched_set.clone() {
                            projection.resource_touched_set = Some(touched_set);
                        }
                        Ok(())
                    },
                )?;
            }
            KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        ledger_purpose: &payload.ledger_purpose,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "receipt",
                    },
                    |epoch| SideEffectPhase::ConfirmationObserved {
                        invocation_epoch: epoch,
                    },
                    |projection| {
                        projection.confirmation = Some(SideEffectArtifactProjection {
                            artifact_id: payload.confirmation_artifact_id.clone(),
                            content_digest: payload.confirmation_hash.clone(),
                            schema_id: Some(payload.confirmation_schema_id.clone()),
                        });
                        if let Some(touched_set) = payload.resource_touched_set.clone() {
                            projection.resource_touched_set = Some(touched_set);
                        }
                        Ok(())
                    },
                )?;
                release_resource_lane_for_ledger(projections, &payload.ledger_key);
            }
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                transition_side_effect_epoch_only(
                    projections,
                    EpochOnlyTransition {
                        ledger_key: &payload.ledger_key,
                        ledger_purpose: &payload.ledger_purpose,
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        event_id: envelope.event_id.clone(),
                        invocation_epoch: payload.invocation_epoch,
                        required_previous: "ambiguity_source",
                    },
                    |epoch| SideEffectPhase::Ambiguous {
                        invocation_epoch: epoch,
                    },
                    |_| Ok(()),
                )?;
                if matches!(
                    payload.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                ) {
                    note_saga_engagement(
                        projections,
                        envelope.run_id(),
                        SagaEngagementProjection {
                            event_id: envelope.event_id.clone(),
                            reason: SagaEngagementReason::ForwardAmbiguous {
                                ledger_key: payload.ledger_key.clone(),
                            },
                        },
                    );
                }
            }
            KernelEventPayload::SideEffectFailed(payload) => {
                require_active_attempt_for_side_effect(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.ledger_key,
                )?;
                transition_side_effect_failure(projections, payload, envelope.event_id.clone())?;
                release_resource_lane_for_ledger(projections, &payload.ledger_key);
                if !payload.retryable {
                    note_saga_engagement(
                        projections,
                        envelope.run_id(),
                        SagaEngagementProjection {
                            event_id: envelope.event_id.clone(),
                            reason: SagaEngagementReason::NonRetryableFailure {
                                node_id: payload.node_id.clone(),
                                attempt_id: payload.attempt_id.clone(),
                            },
                        },
                    );
                }
            }
            KernelEventPayload::PublicOutputProduced(payload) => {
                if matches!(
                    projections.public_outputs.get(&payload.public_schema_id),
                    Some(PublicOutputProjection::Produced { .. })
                ) {
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
                if matches!(
                    projections.public_outputs.get(&payload.public_schema_id),
                    Some(PublicOutputProjection::Produced { .. })
                ) {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("public_output:{}", payload.public_schema_id),
                        message: "public output already produced".to_owned(),
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
            KernelEventPayload::ManualResolutionRecorded(payload) => {
                if projections.manual_resolutions.contains_key(&payload.run_id) {
                    return Err(StoreError::ProjectionConflict {
                        key: "run:manual_resolution".to_owned(),
                        message: "manual resolution already recorded".to_owned(),
                    });
                }
                require_forward_quiescence(projections, &payload.run_id)?;
                projections.manual_resolutions.insert(
                    payload.run_id.clone(),
                    ManualResolutionProjection {
                        event_id: envelope.event_id.clone(),
                        outcome: payload.outcome,
                        operator_identity_ref_schema_id: payload
                            .operator_identity_ref_schema_id
                            .clone(),
                        operator_identity_ref_hash: payload.operator_identity_ref_hash.clone(),
                        operator_identity_ref_artifact_id: payload
                            .operator_identity_ref_artifact_id
                            .clone(),
                        evidence_schema_id: payload.evidence_schema_id.clone(),
                        evidence_hash: payload.evidence_hash.clone(),
                        evidence_artifact_id: payload.evidence_artifact_id.clone(),
                        note: payload.note.clone(),
                    },
                );
                release_resource_lanes_for_run(projections, &payload.run_id);
            }
            KernelEventPayload::RetentionRefsAppended(payload) => {
                if projections.run_state(&payload.run_id) == RunState::Absent {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("retention:{}:refs", payload.run_id),
                        message: "retention refs require a started run".to_owned(),
                    });
                }
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
                if projections.run_state(&payload.run_id) == RunState::Absent {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("retention:{}:manifest", payload.run_id),
                        message: "retention manifest requires a started run".to_owned(),
                    });
                }
                let retention = projections
                    .retentions
                    .entry(payload.run_id.clone())
                    .or_default();
                if retention.manifests.contains_key(&payload.manifest_seq) {
                    return Err(StoreError::ProjectionConflict {
                        key: format!(
                            "retention:{}:manifest:{}",
                            payload.run_id, payload.manifest_seq
                        ),
                        message: "manifest sequence already projected".to_owned(),
                    });
                }
                match &retention.manifest {
                    Some(previous) => {
                        let expected_seq =
                            previous.manifest_seq.checked_add(1).ok_or_else(|| {
                                StoreError::ProjectionConflict {
                                    key: format!("retention:{}:manifest", payload.run_id),
                                    message: "manifest sequence overflow".to_owned(),
                                }
                            })?;
                        if payload.manifest_seq != expected_seq {
                            return Err(StoreError::ProjectionConflict {
                                key: format!("retention:{}:manifest", payload.run_id),
                                message: "manifest sequence must advance by one".to_owned(),
                            });
                        }
                        if payload.previous_manifest_digest.as_ref()
                            != Some(&previous.manifest_digest)
                        {
                            return Err(StoreError::ProjectionConflict {
                                key: format!("retention:{}:manifest", payload.run_id),
                                message:
                                    "manifest previous digest does not match latest projection"
                                        .to_owned(),
                            });
                        }
                    }
                    None => {
                        if payload.manifest_seq != 1 || payload.previous_manifest_digest.is_some() {
                            return Err(StoreError::ProjectionConflict {
                                key: format!("retention:{}:manifest", payload.run_id),
                                message:
                                    "first manifest must use sequence 1 and no previous digest"
                                        .to_owned(),
                            });
                        }
                    }
                }
                let projection = RetentionManifestProjection {
                    manifest_seq: payload.manifest_seq,
                    manifest_digest: payload.manifest_digest.clone(),
                    manifest_artifact_id: payload.manifest_artifact_id.clone(),
                    previous_manifest_digest: payload.previous_manifest_digest.clone(),
                };
                retention
                    .manifests
                    .insert(payload.manifest_seq, projection.clone());
                retention.manifest = Some(projection);
            }
            KernelEventPayload::FactRecorded(payload) => {
                match projections.attempt(&payload.node_id, &payload.attempt_id) {
                    Some(AttemptProjection {
                        status: AttemptStatus::Started { .. },
                        ..
                    }) => {}
                    Some(_) => {
                        return Err(StoreError::ProjectionConflict {
                            key: format!(
                                "fact:{}:{}:{}",
                                payload.node_id, payload.attempt_id, payload.fact_key
                            ),
                            message: "fact requires an active started attempt".to_owned(),
                        });
                    }
                    None => {
                        return Err(StoreError::ProjectionConflict {
                            key: format!(
                                "fact:{}:{}:{}",
                                payload.node_id, payload.attempt_id, payload.fact_key
                            ),
                            message: "fact requires a started attempt".to_owned(),
                        });
                    }
                }
                let key = (
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.fact_key.clone(),
                );
                if projections.facts.contains_key(&key) {
                    return Err(StoreError::ProjectionConflict {
                        key: format!(
                            "fact:{}:{}:{}",
                            payload.node_id, payload.attempt_id, payload.fact_key
                        ),
                        message: "fact already recorded for attempt".to_owned(),
                    });
                }
                projections.facts.insert(
                    key,
                    FactProjection {
                        event_id: envelope.event_id.clone(),
                        node_id: payload.node_id.clone(),
                        attempt_id: payload.attempt_id.clone(),
                        fact_key: payload.fact_key.clone(),
                        request_schema_id: payload.request_schema_id.clone(),
                        request_hash: payload.request_hash.clone(),
                        response_schema_id: payload.response_schema_id.clone(),
                        response_hash: payload.response_hash.clone(),
                        artifact_id: payload.artifact_id.clone(),
                        capability_kind: payload.capability_kind.clone(),
                        capability_version: payload.capability_version.clone(),
                        adapter_kind: payload.adapter_kind.clone(),
                        adapter_version: payload.adapter_version.clone(),
                    },
                );
            }
            KernelEventPayload::ArtifactReferenced(_) => {}
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

    fn note_saga_engagement(
        projections: &mut ProjectionSnapshot,
        run_id: &RunId,
        engagement: SagaEngagementProjection,
    ) {
        projections
            .saga_engagements
            .entry(run_id.clone())
            .or_insert(engagement);
    }

    fn require_forward_fence_open(
        projections: &ProjectionSnapshot,
        run_id: &RunId,
        ledger_key: &events::SideEffectLedgerKey,
        purpose: &events::SideEffectLedgerPurpose,
    ) -> Result<()> {
        if matches!(purpose, events::SideEffectLedgerPurpose::Forward)
            && projections.saga_engagement(run_id).is_some()
        {
            Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{ledger_key}"),
                message: "forward side-effect boundary event rejected after saga engagement"
                    .to_owned(),
            })
        } else {
            Ok(())
        }
    }

    fn require_forward_quiescence(projections: &ProjectionSnapshot, run_id: &RunId) -> Result<()> {
        if forward_ledgers_quiescent(projections, run_id) {
            Ok(())
        } else {
            Err(StoreError::ProjectionConflict {
                key: format!("run:{run_id}:quiescence"),
                message: "past-boundary forward side-effect ledgers must be quiescent".to_owned(),
            })
        }
    }

    fn require_remediation_intent_admissible(
        projections: &ProjectionSnapshot,
        run_id: &RunId,
        remediation_ledger_key: &events::SideEffectLedgerKey,
        purpose: &events::SideEffectLedgerPurpose,
    ) -> Result<()> {
        let events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } = purpose else {
            return Ok(());
        };
        if projections.saga_engagement(run_id).is_none() {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{remediation_ledger_key}"),
                message: "remediation ledger requires prior saga engagement".to_owned(),
            });
        }
        let Some(forward) = projections.side_effect(forward_ledger_key) else {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{remediation_ledger_key}"),
                message: "remediation ledger references missing forward ledger".to_owned(),
            });
        };
        if !matches!(
            forward.ledger_purpose,
            events::SideEffectLedgerPurpose::Forward
        ) {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{remediation_ledger_key}"),
                message: "remediation ledger references a non-forward ledger".to_owned(),
            });
        }
        if !matches!(forward.phase, SideEffectPhase::ConfirmationObserved { .. }) {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{remediation_ledger_key}"),
                message: "remediation ledger requires confirmed forward ledger".to_owned(),
            });
        }
        if projections.side_effects.values().any(|projection| {
            matches!(
                &projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Remediation {
                    forward_ledger_key: linked
                } if linked == forward_ledger_key
            )
        }) {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{remediation_ledger_key}"),
                message: "remediation ledger already exists for forward ledger".to_owned(),
            });
        }
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

    fn require_side_effect_purpose(
        projection: &SideEffectProjection,
        ledger_key: &events::SideEffectLedgerKey,
        expected: &events::SideEffectLedgerPurpose,
    ) -> Result<()> {
        if projection.ledger_purpose == *expected {
            Ok(())
        } else {
            Err(side_effect_projection_error(
                ledger_key,
                "side-effect ledger purpose changed",
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

    fn require_active_attempt_for_side_effect(
        projections: &ProjectionSnapshot,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        ledger_key: &events::SideEffectLedgerKey,
    ) -> Result<()> {
        match projections.attempt(node_id, attempt_id) {
            Some(AttemptProjection {
                status: AttemptStatus::Started { .. },
                ..
            }) => Ok(()),
            Some(_) => Err(side_effect_projection_error(
                ledger_key,
                "side-effect event requires an active started attempt",
            )),
            None => Err(side_effect_projection_error(
                ledger_key,
                "side-effect event requires a started attempt",
            )),
        }
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

    fn require_intent_attempt_context(
        projection: &SideEffectProjection,
        node_id: &NodeId,
        attempt_id: &AttemptId,
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
        if payload.claim_fencing_token == claim.claim_fencing_token {
            return Err(side_effect_projection_error(
                ledger_key,
                "takeover claim fencing token must change",
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

    fn prepared_invocation_projection(
        artifact_id: &Option<ArtifactId>,
        content_digest: &Option<ContentDigest>,
        ledger_key: &events::SideEffectLedgerKey,
    ) -> Result<Option<SideEffectArtifactProjection>> {
        match (artifact_id, content_digest) {
            (Some(artifact_id), Some(content_digest)) => Ok(Some(SideEffectArtifactProjection {
                artifact_id: artifact_id.clone(),
                content_digest: content_digest.clone(),
                schema_id: None,
            })),
            (None, None) => Ok(None),
            _ => Err(side_effect_projection_error(
                ledger_key,
                "prepared invocation artifact id and hash must be recorded together",
            )),
        }
    }

    fn acquire_resource_lane(
        projections: &mut ProjectionSnapshot,
        run_id: &RunId,
        event_id: &EventId,
        payload: &side_effect::InvocationPrepared,
    ) -> Result<()> {
        let Some(resource_key) = payload.resource_key.as_ref() else {
            if resource_lane_key_for_ledger(projections, &payload.ledger_key).is_some() {
                return Err(StoreError::ProjectionConflict {
                    key: format!("resource_lane:{}", payload.ledger_key),
                    message: "resource lane holder cannot refresh without key evidence".to_owned(),
                });
            }
            return Ok(());
        };

        let lane_key = ResourceLaneKey::from_evidence(resource_key);
        if let Some(existing_key) = resource_lane_key_for_ledger(projections, &payload.ledger_key) {
            if existing_key != lane_key {
                return Err(StoreError::ProjectionConflict {
                    key: format!("resource_lane:{}", payload.ledger_key),
                    message: "resource lane key changed for ledger".to_owned(),
                });
            }
        }
        if let Some(existing) = projections.resource_lanes.get(&lane_key) {
            if existing.ledger_key != payload.ledger_key {
                return Err(StoreError::ProjectionConflict {
                    key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
                    message: format!(
                        "resource lane already held by ledger {}",
                        existing.ledger_key
                    ),
                });
            }
        }

        projections.resource_lanes.insert(
            lane_key,
            ResourceLaneProjection {
                event_id: event_id.clone(),
                run_id: run_id.clone(),
                ledger_key: payload.ledger_key.clone(),
                ledger_purpose: payload.ledger_purpose.clone(),
                node_id: payload.node_id.clone(),
                attempt_id: payload.attempt_id.clone(),
                invocation_epoch: payload.invocation_epoch,
            },
        );
        Ok(())
    }

    fn resource_lane_key_for_ledger(
        projections: &ProjectionSnapshot,
        ledger_key: &events::SideEffectLedgerKey,
    ) -> Option<ResourceLaneKey> {
        projections
            .resource_lanes
            .iter()
            .find_map(|(key, projection)| {
                (projection.ledger_key == *ledger_key).then(|| key.clone())
            })
    }

    fn release_resource_lane_for_ledger(
        projections: &mut ProjectionSnapshot,
        ledger_key: &events::SideEffectLedgerKey,
    ) {
        if let Some(key) = resource_lane_key_for_ledger(projections, ledger_key) {
            projections.resource_lanes.remove(&key);
        }
    }

    fn release_resource_lanes_for_run(projections: &mut ProjectionSnapshot, run_id: &RunId) {
        projections
            .resource_lanes
            .retain(|_, projection| projection.run_id != *run_id);
    }

    fn phase_matches_expected(phase: &SideEffectPhase, expected: &'static str) -> bool {
        match expected {
            "started" => matches!(phase, SideEffectPhase::InvocationStarted { .. }),
            "submission_recovery" => matches!(
                phase,
                SideEffectPhase::InvocationStarted { .. }
                    | SideEffectPhase::SubmissionUnknown { .. }
            ),
            "submission_observed" => matches!(phase, SideEffectPhase::SubmissionObserved { .. }),
            "receipt" => matches!(phase, SideEffectPhase::ReceiptObserved { .. }),
            "not_submitted" => matches!(phase, SideEffectPhase::NotSubmittedProven { .. }),
            "ambiguity_source" => matches!(
                phase,
                SideEffectPhase::InvocationStarted { .. }
                    | SideEffectPhase::SubmissionUnknown { .. }
                    | SideEffectPhase::SubmissionObserved { .. }
                    | SideEffectPhase::ReceiptObserved { .. }
            ),
            _ => false,
        }
    }

    struct EpochOnlyTransition<'a> {
        ledger_key: &'a events::SideEffectLedgerKey,
        ledger_purpose: &'a events::SideEffectLedgerPurpose,
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
        update_projection: impl FnOnce(&mut SideEffectProjection) -> Result<()>,
    ) -> Result<()> {
        let (
            run_id,
            ledger_purpose,
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim,
        ) = {
            let previous = require_side_effect_phase(
                projections,
                transition.ledger_key,
                transition.required_previous,
                |projection| {
                    phase_matches_expected(&projection.phase, transition.required_previous)
                },
            )?;
            require_side_effect_purpose(
                previous,
                transition.ledger_key,
                transition.ledger_purpose,
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
            (
                previous.run_id.clone(),
                previous.ledger_purpose.clone(),
                previous.intent.clone(),
                previous.prepared_invocation.clone(),
                previous.resource_key.clone(),
                previous.submission.clone(),
                previous.receipt.clone(),
                previous.confirmation.clone(),
                previous.resource_touched_set.clone(),
                claim.clone(),
            )
        };
        let mut projection = SideEffectProjection {
            run_id,
            ledger_key: transition.ledger_key.clone(),
            ledger_purpose,
            event_id: transition.event_id,
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim: Some(claim),
            phase: next_phase(transition.invocation_epoch),
        };
        update_projection(&mut projection)?;
        projections
            .side_effects
            .insert(transition.ledger_key.clone(), projection);
        Ok(())
    }

    fn transition_side_effect_failure(
        projections: &mut ProjectionSnapshot,
        payload: &side_effect::Failed,
        event_id: EventId,
    ) -> Result<()> {
        let (
            run_id,
            ledger_purpose,
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim,
        ) = {
            let Some(previous) = projections.side_effects.get(&payload.ledger_key) else {
                return Err(side_effect_projection_error(
                    &payload.ledger_key,
                    "missing side-effect projection",
                ));
            };
            require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
            match payload.failure_phase {
                side_effect::FailurePhase::BeforeInvocationStarted => match previous.phase {
                    SideEffectPhase::IntentPersisted { invocation_epoch } => {
                        if payload.invocation_epoch != invocation_epoch {
                            return Err(side_effect_projection_error(
                                &payload.ledger_key,
                                "failure invocation epoch does not match intent",
                            ));
                        }
                        require_intent_context(
                            previous,
                            &payload.node_id,
                            &payload.attempt_id,
                            payload.invocation_epoch,
                        )?;
                    }
                    SideEffectPhase::Claimed { .. }
                    | SideEffectPhase::InvocationPrepared { .. } => {
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
                                claim_owner: None,
                            },
                        )?;
                    }
                    _ => {
                        return Err(side_effect_projection_error(
                            &payload.ledger_key,
                            "before-start failure requires intent, claim, or prepared phase",
                        ));
                    }
                },
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
            (
                previous.run_id.clone(),
                previous.ledger_purpose.clone(),
                previous.intent.clone(),
                previous.prepared_invocation.clone(),
                previous.resource_key.clone(),
                previous.submission.clone(),
                previous.receipt.clone(),
                previous.confirmation.clone(),
                previous.resource_touched_set.clone(),
                previous.claim.clone(),
            )
        };
        projections.side_effects.insert(
            payload.ledger_key.clone(),
            SideEffectProjection {
                run_id,
                ledger_key: payload.ledger_key.clone(),
                ledger_purpose,
                event_id,
                intent,
                prepared_invocation,
                resource_key,
                submission,
                receipt,
                confirmation,
                resource_touched_set,
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
                "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
                "evidence_hash": payload.evidence_hash.as_str(),
                "evidence_schema_id": payload.evidence_schema_id.as_str(),
                "note": payload.note.as_ref().map(manual_resolution_note_json),
                "operator_identity_ref_artifact_id": payload.operator_identity_ref_artifact_id.as_str(),
                "operator_identity_ref_hash": payload.operator_identity_ref_hash.as_str(),
                "operator_identity_ref_schema_id": payload.operator_identity_ref_schema_id.as_str(),
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
                    operator_identity_ref_schema_id: parse_identity(required_str(
                        json,
                        "operator_identity_ref_schema_id",
                    )?)?,
                    operator_identity_ref_hash: parse_identity(required_str(
                        json,
                        "operator_identity_ref_hash",
                    )?)?,
                    operator_identity_ref_artifact_id: parse_identity(required_str(
                        json,
                        "operator_identity_ref_artifact_id",
                    )?)?,
                    evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                    evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                    evidence_artifact_id: parse_identity(required_str(
                        json,
                        "evidence_artifact_id",
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

    fn parse_event_artifact(json: &serde_json::Value) -> Result<events::ArtifactEvidenceRef> {
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
                .map_err(|error| StoreError::Identity(error.to_string()))?,
        })
    }

    fn parse_skip_reason(json: &serde_json::Value) -> Result<events::SkipReason> {
        Ok(events::SkipReason {
            code: events::ErrorCode::new(required_str(json, "code")?)?,
            safe_message: required_str(json, "safe_message")?.to_owned(),
        })
    }

    fn parse_side_effect_ledger_purpose(
        json: &serde_json::Value,
    ) -> Result<events::SideEffectLedgerPurpose> {
        match required_str(json, "kind")? {
            "forward" => Ok(events::SideEffectLedgerPurpose::Forward),
            "remediation" => Ok(events::SideEffectLedgerPurpose::Remediation {
                forward_ledger_key: events::SideEffectLedgerKey::new(required_str(
                    json,
                    "forward_ledger_key",
                )?)?,
            }),
            other => Err(StoreError::Identity(format!(
                "unknown side-effect ledger purpose {other}"
            ))),
        }
    }

    fn parse_resource_key_evidence(
        json: &serde_json::Value,
    ) -> Result<events::ResourceKeyEvidence> {
        Ok(events::ResourceKeyEvidence {
            namespace: ResourceNamespace::new(required_str(json, "namespace")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            key_schema_id: parse_identity(required_str(json, "key_schema_id")?)?,
            key: events::ResourceKey::new(required_str(json, "key")?)?,
        })
    }

    fn parse_resource_touched_set_evidence(
        json: &serde_json::Value,
    ) -> Result<events::ResourceTouchedSetEvidence> {
        Ok(events::ResourceTouchedSetEvidence {
            namespace: ResourceNamespace::new(required_str(json, "namespace")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
            evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
        })
    }

    fn parse_manual_resolution_outcome(value: &str) -> Result<events::ManualResolutionOutcome> {
        match value {
            "confirm_remediated" => Ok(events::ManualResolutionOutcome::ConfirmRemediated),
            "fail_without_acdc_claim" => Ok(events::ManualResolutionOutcome::FailWithoutAcdcClaim),
            other => Err(StoreError::Identity(format!(
                "unknown manual resolution outcome {other}"
            ))),
        }
    }

    fn parse_manual_resolution_note(
        json: &serde_json::Value,
    ) -> Result<events::ManualResolutionNote> {
        Ok(events::ManualResolutionNote::new(required_str(
            json, "text",
        )?)?)
    }

    fn parse_error_info(json: &serde_json::Value) -> Result<events::MfmErrorInfo> {
        Ok(events::MfmErrorInfo {
            code: events::ErrorCode::new(required_str(json, "code")?)?,
            category: parse_error_category(required_str(json, "category")?)?,
            retryable: required_bool(json, "retryable")?,
            safe_message: required_str(json, "safe_message")?.to_owned(),
            public_details: optional_obj(json, "public_details")?
                .map(|details| {
                    Ok::<events::RedactedJson, StoreError>(events::RedactedJson {
                        content_digest: parse_identity(required_str(details, "content_digest")?)?,
                    })
                })
                .transpose()?,
            diagnostic_ref: optional_obj(json, "diagnostic_ref")?
                .map(parse_event_artifact)
                .transpose()?,
        })
    }

    fn parse_run_completion_outcome(
        json: &serde_json::Value,
    ) -> Result<events::RunCompletionOutcome> {
        match required_str(json, "kind")? {
            "completed" => {
                let evidence = required_obj(json, "public_output")?;
                Ok(events::RunCompletionOutcome::Completed(
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
                ))
            }
            "compensated" => Ok(events::RunCompletionOutcome::Compensated),
            "manually_resolved" => Ok(events::RunCompletionOutcome::ManuallyResolved),
            "failed_without_acdc_claim" => Ok(events::RunCompletionOutcome::FailedWithoutAcdcClaim),
            other => Err(StoreError::Identity(format!(
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

    fn parse_artifact_role(value: &str) -> Result<ArtifactRole> {
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
            "public_output" => Ok(ArtifactRole::PublicOutput),
            "redacted_diagnostic" => Ok(ArtifactRole::RedactedDiagnostic),
            "retention_manifest" => Ok(ArtifactRole::RetentionManifest),
            other => Err(StoreError::Identity(format!(
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

    fn parse_error_category(value: &str) -> Result<events::ErrorCategory> {
        match value {
            "planning" => Ok(events::ErrorCategory::Planning),
            "validation" => Ok(events::ErrorCategory::Validation),
            "capability" => Ok(events::ErrorCategory::Capability),
            "side_effect" => Ok(events::ErrorCategory::SideEffect),
            "runtime" => Ok(events::ErrorCategory::Runtime),
            "storage" => Ok(events::ErrorCategory::Storage),
            "cancelled" => Ok(events::ErrorCategory::Cancelled),
            other => Err(StoreError::Identity(format!(
                "unknown error category {other}"
            ))),
        }
    }

    fn parse_failure_phase(value: &str) -> Result<side_effect::FailurePhase> {
        match value {
            "before_invocation_started" => Ok(side_effect::FailurePhase::BeforeInvocationStarted),
            "after_not_submitted_proven" => Ok(side_effect::FailurePhase::AfterNotSubmittedProven),
            other => Err(StoreError::Identity(format!(
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

    fn parse_identity<T>(value: &str) -> Result<T>
    where
        T: std::str::FromStr<Err = IdentityError>,
    {
        value.parse().map_err(StoreError::from)
    }

    fn parse_vec<T>(
        json: &serde_json::Value,
        field: &'static str,
        parser: impl Fn(&serde_json::Value) -> Result<T>,
    ) -> Result<Vec<T>> {
        required_array(json, field)?.iter().map(parser).collect()
    }

    fn required_str<'a>(json: &'a serde_json::Value, field: &'static str) -> Result<&'a str> {
        json.get(field)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| StoreError::Event(format!("missing string field {field}")))
    }

    fn optional_str<'a>(
        json: &'a serde_json::Value,
        field: &'static str,
    ) -> Result<Option<&'a str>> {
        match json.get(field) {
            Some(serde_json::Value::Null) | None => Ok(None),
            Some(value) => value
                .as_str()
                .map(Some)
                .ok_or_else(|| StoreError::Event(format!("field {field} was not a string"))),
        }
    }

    fn required_obj<'a>(
        json: &'a serde_json::Value,
        field: &'static str,
    ) -> Result<&'a serde_json::Value> {
        let value = json
            .get(field)
            .ok_or_else(|| StoreError::Event(format!("missing object field {field}")))?;
        if value.is_object() {
            Ok(value)
        } else {
            Err(StoreError::Event(format!(
                "field {field} was not an object"
            )))
        }
    }

    fn optional_obj<'a>(
        json: &'a serde_json::Value,
        field: &'static str,
    ) -> Result<Option<&'a serde_json::Value>> {
        match json.get(field) {
            Some(serde_json::Value::Null) | None => Ok(None),
            Some(value) if value.is_object() => Ok(Some(value)),
            Some(_) => Err(StoreError::Event(format!(
                "field {field} was not an object"
            ))),
        }
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

    fn required_u64(json: &serde_json::Value, field: &'static str) -> Result<u64> {
        json.get(field)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| StoreError::Event(format!("missing u64 field {field}")))
    }

    fn required_u32(json: &serde_json::Value, field: &'static str) -> Result<u32> {
        required_u64(json, field)?
            .try_into()
            .map_err(|_| StoreError::Event(format!("{field} overflowed u32")))
    }

    fn required_bool(json: &serde_json::Value, field: &'static str) -> Result<bool> {
        json.get(field)
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| StoreError::Event(format!("missing bool field {field}")))
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

    fn skip_reason_json(reason: &events::SkipReason) -> serde_json::Value {
        serde_json::json!({
            "code": reason.code.as_str(),
            "safe_message": reason.safe_message.as_str(),
        })
    }

    fn side_effect_ledger_purpose_json(
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

    fn resource_key_evidence_json(evidence: &events::ResourceKeyEvidence) -> serde_json::Value {
        serde_json::json!({
            "key": evidence.key.as_str(),
            "key_schema_id": evidence.key_schema_id.as_str(),
            "namespace": evidence.namespace.as_str(),
        })
    }

    fn resource_touched_set_evidence_json(
        evidence: &events::ResourceTouchedSetEvidence,
    ) -> serde_json::Value {
        serde_json::json!({
            "evidence_artifact_id": evidence.evidence_artifact_id.as_str(),
            "evidence_hash": evidence.evidence_hash.as_str(),
            "evidence_schema_id": evidence.evidence_schema_id.as_str(),
            "namespace": evidence.namespace.as_str(),
        })
    }

    fn manual_resolution_outcome_str(outcome: events::ManualResolutionOutcome) -> &'static str {
        outcome.as_str()
    }

    fn manual_resolution_note_json(note: &events::ManualResolutionNote) -> serde_json::Value {
        serde_json::json!({
            "text": note.as_str(),
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
            events::RunCompletionOutcome::Compensated => serde_json::json!({
                "kind": "compensated",
            }),
            events::RunCompletionOutcome::ManuallyResolved => serde_json::json!({
                "kind": "manually_resolved",
            }),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim => serde_json::json!({
                "kind": "failed_without_acdc_claim",
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
