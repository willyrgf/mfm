use mfm_ids::{ArtifactId, RunId, SemanticDigest, TenantScopeId};

use mfm_journal::v1::JournalHead;

/// Error returned by the recoverability-v1 store contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// No committed journal exists for the requested authorized run.
    #[error("committed journal not found")]
    RunNotFound,
    /// A purpose authority did not belong to this exact store instance.
    #[error("store access denied for {purpose}")]
    AccessDenied {
        /// Reviewed purpose label.
        purpose: &'static str,
    },
    /// A policy-approved authority target violated its frozen grammar.
    #[error("invalid authority binding for {field}")]
    InvalidAuthorityBinding {
        /// Reviewed field label.
        field: &'static str,
    },
    /// An admission authority did not match the exact admitted root.
    #[error("admission authority does not match the exact root")]
    AdmissionAuthorityMismatch,
    /// The admission logical key already names different immutable root content.
    #[error("admission logical key conflicts with an existing root")]
    AdmissionConflict,
    /// An annex-backed journal value failed strict construction or projection.
    #[error("journal contract validation failed")]
    JournalContract,
    /// A prepared append violated the exhaustive legal batch contract.
    #[error("invalid {purpose} append: {message}")]
    InvalidPreparedAppend {
        /// Closed append purpose.
        purpose: &'static str,
        /// Reviewed invariant diagnostic.
        message: &'static str,
    },
    /// A loaded journal was absent or contained no admission root.
    #[error("committed journal is empty")]
    EmptyJournal,
    /// A loaded row disagreed with a store-derived field.
    #[error("persisted journal mismatch for {field}")]
    PersistedMismatch {
        /// Reviewed field label.
        field: &'static str,
    },
    /// A candidate named a stale physical predecessor.
    #[error("journal head compare-and-swap failed")]
    HeadMismatch {
        /// Candidate predecessor.
        expected: Box<JournalHead>,
        /// Current physical head.
        actual: Box<JournalHead>,
    },
    /// A non-admission append targeted an absent run.
    #[error("append targeted an absent run")]
    AppendRunNotFound {
        /// Absent run.
        run_id: RunId,
    },
    /// An append request id was reused with different predecessor or candidate bytes.
    #[error("append request id was reused with different content")]
    AppendRequestConflict,
    /// A per-run sequence would overflow.
    #[error("run sequence overflow")]
    SequenceOverflow,
    /// A tenant fact publication order would overflow.
    #[error("tenant fact publication order overflow")]
    FactOrderOverflow {
        /// Tenant whose dense order exhausted.
        tenant_scope_id: TenantScopeId,
    },
    /// A transition or authorization tried to extend a semantically closed run.
    #[error("semantic run is closed")]
    RunClosed,
    /// A closure was not adjacent to and inseparable from its terminal transition.
    #[error("invalid run closure")]
    InvalidClosure,
    /// One closed logical record slot was already occupied.
    #[error("duplicate logical record slot")]
    DuplicateLogicalRecord,
    /// An authorization was incompatible with the current structural node state.
    #[error("external access authorization is not structurally eligible")]
    AuthorizationNotEligible,
    /// An observation referenced no matching committed authorization.
    #[error("external access observation references an unknown authorization")]
    UnknownAuthorization,
    /// An authorization already has its one observation.
    #[error("external access authorization is already observed")]
    ObservationAlreadyCommitted,
    /// A post-closure observation did not reference an unmatched pre-closure authorization.
    #[error("external access observation is not a legal audit tail")]
    InvalidAuditTail,
    /// An access-audit page request violated its bounded head-fixed contract.
    #[error("invalid access-audit page for {field}")]
    InvalidAuditPage {
        /// Reviewed invalid request field.
        field: &'static str,
    },
    /// A transition-trace page or its supplied source-authority set violated its sealed contract.
    #[error("invalid transition-trace page for {field}")]
    InvalidTracePage {
        /// Reviewed invalid request or token field.
        field: &'static str,
    },
    /// A read retry changed its first frozen intent.
    #[error("read authorization conflicts with the frozen intent")]
    FrozenReadIntentConflict,
    /// A transition consumed an unavailable or incompatible observation.
    #[error("transition consumed an incompatible observation")]
    ObservationNotConsumable,
    /// A transition's before/after assertions disagree with the private fold.
    #[error("transition fold mismatch for {field}")]
    TransitionFoldMismatch {
        /// Reviewed field label.
        field: &'static str,
    },
    /// Object path bindings or admission intents were not canonical and exact.
    #[error("invalid exact object authority: {message}")]
    InvalidObjectAuthority {
        /// Reviewed invariant diagnostic.
        message: &'static str,
    },
    /// A required exact object did not have prior or same-batch authority.
    #[error("required exact object is missing")]
    MissingObjectAuthority {
        /// Missing artifact.
        artifact_id: ArtifactId,
    },
    /// Staged bytes did not match the exact bound content identity.
    #[error("staged object bytes do not match exact content identity")]
    ObjectContentMismatch {
        /// Mismatched artifact.
        artifact_id: ArtifactId,
    },
    /// Existing object authority disagreed with the supplied exact identity.
    #[error("object authority conflicts with existing immutable evidence")]
    ObjectAuthorityConflict {
        /// Conflicting artifact.
        artifact_id: ArtifactId,
    },
    /// A requested retained object is not reachable from the authorized journal.
    #[error("retained object is not reachable from the authorized journal")]
    ObjectNotReachable,
    /// A tenant fact coordinate disagreed with the legal batch or routing copies.
    #[error("invalid tenant fact coordinate")]
    InvalidFactCoordinate,
    /// Retained tenant publication/barrier structure was not a dense valid prefix.
    #[error("tenant fact history is corrupt")]
    CorruptFactHistory,
    /// A fact scan request exceeded the frozen response bound.
    #[error("fact selection limit exceeds 128")]
    FactSelectionLimitExceeded,
    /// A fact scan session was used with a different authorization or frontier.
    #[error("fact scan session binding mismatch")]
    FactScanBindingMismatch,
    /// A source closure crossed its admitted store or tenant scope.
    #[error("cross-run source scope mismatch")]
    SourceScopeMismatch,
    /// A source closure was incomplete, cyclic, or inconsistent.
    #[error("cross-run source closure is invalid")]
    InvalidSourceClosure,
    /// A digest derived from canonical retained material disagreed with its persisted identity.
    #[error("semantic digest mismatch")]
    DigestMismatch {
        /// Expected digest.
        expected: SemanticDigest,
        /// Derived digest.
        actual: SemanticDigest,
    },
    /// The in-memory backend lock was poisoned.
    #[error("in-memory store lock poisoned")]
    MemoryLockPoisoned,
    /// A second test-only commit failure was armed before the first was consumed.
    #[error("in-memory commit failure selector is already armed")]
    MemoryFailureSelectorAlreadyArmed,
    /// A test-only injected failure occurred before atomic publication.
    #[error("injected in-memory append failure at {point}")]
    InjectedFailure {
        /// Stable injection point.
        point: &'static str,
    },
}

impl From<mfm_journal::v1::JournalError> for StoreError {
    fn from(_: mfm_journal::v1::JournalError) -> Self {
        Self::JournalContract
    }
}

impl From<mfm_ids::IdentityError> for StoreError {
    fn from(_: mfm_ids::IdentityError) -> Self {
        Self::JournalContract
    }
}

impl From<mfm_ids::CheckedStringError> for StoreError {
    fn from(_: mfm_ids::CheckedStringError) -> Self {
        Self::JournalContract
    }
}

impl From<mfm_canonical::RecoverabilityError> for StoreError {
    fn from(_: mfm_canonical::RecoverabilityError) -> Self {
        Self::JournalContract
    }
}

impl From<mfm_spec::SpecError> for StoreError {
    fn from(_: mfm_spec::SpecError) -> Self {
        Self::PersistedMismatch {
            field: "certified_spec",
        }
    }
}

/// Exposes a wrapped typed store error without parsing display strings.
pub trait StoreErrorInspection {
    /// Returns the typed store error when this error wraps one.
    fn as_store_error(&self) -> Option<&StoreError>;
}

impl StoreErrorInspection for StoreError {
    fn as_store_error(&self) -> Option<&StoreError> {
        Some(self)
    }
}
