use super::*;

/// Error returned by typed store contract validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// No committed journal exists for the requested run.
    #[error("committed journal not found for run {run_id}")]
    RunNotFound {
        /// Requested run id.
        run_id: RunId,
    },
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
