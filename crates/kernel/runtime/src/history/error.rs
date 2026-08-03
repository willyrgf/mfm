//! Closed redaction-safe history-port failures.

/// Stable redaction-safe structured history failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    /// The requested run does not exist.
    #[error("structured run was not found")]
    RunNotFound,
    /// Durable bytes or successor semantics are invalid.
    #[error("structured history is invalid")]
    InvalidHistory,
    /// A proposed semantic candidate was rejected before backend append.
    #[error("structured history candidate was rejected")]
    CandidateRejected,
    /// Persisted certification does not match the qualified registry.
    #[error("structured certification verification failed")]
    Certification,
    /// The locked backend head differs from the candidate predecessor.
    #[error("structured history head changed")]
    StaleHead,
    /// An append identity was reused for different content.
    #[error("structured append identity conflicts")]
    AppendConflict,
    /// The durable backend is unavailable.
    #[error("structured history backend is unavailable")]
    BackendUnavailable,
    /// Commit acknowledgement must be resolved before any rebase.
    #[error("structured append acknowledgement is unknown")]
    AcknowledgementUnknown,
}
