//! Closed runtime failure boundary.

/// Result type for stateless runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Failure to verify or execute one stateless runtime action.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The authoritative store rejected or could not complete an operation.
    #[error(transparent)]
    Store(#[from] mfm_store::StoreError),
    /// A backend failed without exposing a reviewed typed store error.
    #[error("journal store backend is unavailable")]
    StoreBackendUnavailable,
    /// Frozen journal construction or projection failed.
    #[error(transparent)]
    Journal(#[from] mfm_journal::v1::JournalError),
    /// A checked runtime-owned stable identity could not be derived.
    #[error(transparent)]
    Identity(#[from] mfm_ids::IdentityError),
    /// Canonical callback or verified retained bytes were malformed.
    #[error(transparent)]
    Canonical(#[from] mfm_canonical::CanonicalError),
    /// A typed program value violated its registered contract.
    #[error(transparent)]
    Program(#[from] mfm_program::ProgramError),
    /// The selected current candidate failed behind the redaction-safe
    /// certification boundary.
    #[error(transparent)]
    CandidateCertification(#[from] mfm_program::CandidateCertificationError),
    /// A reserved fact-selection request violated its frozen value contract.
    #[error(transparent)]
    Fact(#[from] mfm_facts::FactError),
    /// A certified descriptor or retained contract violated its frozen schema.
    #[error(transparent)]
    Spec(#[from] mfm_spec::SpecError),
    /// The recoverable executor rejected a request or retained proof.
    #[error(transparent)]
    Executor(#[from] mfm_executor::ExecutorError),
    /// The admitted executable or one selected qualified-registry entry is
    /// unavailable or mismatched.
    #[error("selected qualified program registry does not match the admitted executable")]
    CatalogSelection,
    /// A state callback returned a value outside its certified contract.
    #[error("state callback violated its certified value contract")]
    InvalidCallbackResult,
    /// A store acknowledgement cannot safely authorize another operation.
    #[error("journal append outcome is unknown")]
    OutcomeUnknown,
    /// A fresh affine append witness did not name the request being invoked.
    #[error("newly committed authorization does not match the frozen request")]
    AuthorityMismatch,
    /// The executor-derived immutable identity disagrees with the committed
    /// effect request.
    #[error("executor request identity does not match committed effect state")]
    EffectIdentityMismatch,
}

pub(crate) fn map_store_error(error: &impl mfm_store::StoreErrorInspection) -> RuntimeError {
    error
        .as_store_error()
        .cloned()
        .map(RuntimeError::Store)
        .unwrap_or(RuntimeError::StoreBackendUnavailable)
}
