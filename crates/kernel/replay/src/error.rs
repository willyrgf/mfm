use mfm_canonical::RecoverabilityError;
use mfm_journal::JournalError;
use mfm_store::{StoreError, StoreErrorInspection};

/// Result type for replay, inspection, and portable-export operations.
pub type Result<T> = std::result::Result<T, ReplayError>;

/// Redaction-safe failure returned by replay-owned algorithms.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// The frozen canonical contract rejected a replay or export value.
    #[error("recoverability-v3 canonical replay value is invalid")]
    Recoverability {
        /// Source-preserving codec failure.
        #[source]
        source: RecoverabilityError,
    },
    /// An annex-backed journal value could not be projected.
    #[error("recoverability-v3 journal value is invalid")]
    Journal {
        /// Source-preserving journal failure.
        #[source]
        source: JournalError,
    },
    /// Recorded history failed callback-free verification.
    #[error("recorded run history is invalid")]
    InvalidRecordedHistory,
    /// The sealed current candidate is absent or no longer available.
    #[error("the selected replay candidate is unavailable")]
    CandidateUnavailable,
    /// Capability-free candidate callbacks or certification failed.
    #[error("the selected replay candidate failed during capability-free execution")]
    CandidateExecutionFailed,
    /// A candidate result disagreed with the verified comparison inputs.
    #[error("candidate comparison failed integrity verification")]
    ComparisonIntegrityFailed,
    /// No committed journal exists for the authorized tenant-scoped run.
    #[error("committed run history was not found")]
    RunNotFound,
    /// A purpose authority did not bind the requested store, tenant, or run.
    #[error("run access authority does not match the requested replay operation")]
    AuthorityMismatch,
    /// A fixed-head page request violated the inspection contract.
    #[error("inspection page request is invalid")]
    InvalidPage,
    /// Portable-export material was incomplete or non-deterministic.
    #[error("portable run export is invalid")]
    InvalidExport,
    /// An explicit portable export stream read, write, flush, or rewind failed.
    #[error("portable run export stream I/O failed")]
    ExportStreamIo {
        /// Source-preserving I/O failure for internal diagnostics.
        #[source]
        source: std::io::Error,
    },
    /// A required source run was not authorized for export.
    #[error("a required source run is not authorized for export")]
    SourceRunExportDenied,
}

impl ReplayError {
    /// Returns the stable machine-readable category.
    pub const fn kind(&self) -> ReplayErrorKind {
        match self {
            Self::Recoverability { .. } | Self::Journal { .. } | Self::InvalidRecordedHistory => {
                ReplayErrorKind::InvalidRecordedHistory
            }
            Self::CandidateUnavailable => ReplayErrorKind::CandidateUnavailable,
            Self::CandidateExecutionFailed => ReplayErrorKind::CandidateExecutionFailed,
            Self::ComparisonIntegrityFailed => ReplayErrorKind::ComparisonIntegrityFailed,
            Self::RunNotFound => ReplayErrorKind::RunNotFound,
            Self::AuthorityMismatch => ReplayErrorKind::AuthorityMismatch,
            Self::InvalidPage => ReplayErrorKind::InvalidPage,
            Self::InvalidExport => ReplayErrorKind::InvalidExport,
            Self::ExportStreamIo { .. } => ReplayErrorKind::ExportStreamIo,
            Self::SourceRunExportDenied => ReplayErrorKind::SourceRunExportDenied,
        }
    }

    /// Returns the stable machine-readable error code.
    pub const fn code(&self) -> &'static str {
        self.kind().code()
    }
}

impl From<RecoverabilityError> for ReplayError {
    fn from(source: RecoverabilityError) -> Self {
        Self::Recoverability { source }
    }
}

impl From<JournalError> for ReplayError {
    fn from(source: JournalError) -> Self {
        Self::Journal { source }
    }
}

pub(crate) fn store_error(error: &impl StoreErrorInspection) -> ReplayError {
    match error.as_store_error() {
        Some(StoreError::RunNotFound) => ReplayError::RunNotFound,
        Some(
            StoreError::AccessDenied { .. }
            | StoreError::InvalidAuthorityBinding { .. }
            | StoreError::AdmissionAuthorityMismatch,
        ) => ReplayError::AuthorityMismatch,
        Some(StoreError::InvalidAuditPage { .. } | StoreError::InvalidTracePage { .. }) => {
            ReplayError::InvalidPage
        }
        _ => ReplayError::InvalidRecordedHistory,
    }
}

/// Closed replay error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReplayErrorKind {
    /// Supplied history or a canonical replay value is invalid.
    InvalidRecordedHistory,
    /// The sealed current candidate is unavailable.
    CandidateUnavailable,
    /// Capability-free candidate execution failed.
    CandidateExecutionFailed,
    /// Candidate comparison inputs or outputs failed integrity checks.
    ComparisonIntegrityFailed,
    /// The authorized tenant-scoped run has no committed journal.
    RunNotFound,
    /// A purpose authority has the wrong store, tenant, run, or grant.
    AuthorityMismatch,
    /// A fixed-head page request is invalid.
    InvalidPage,
    /// An export stream or its complete closure is invalid.
    InvalidExport,
    /// An explicit portable export stream I/O operation failed.
    ExportStreamIo,
    /// Export authority is absent for a required source run.
    SourceRunExportDenied,
}

impl ReplayErrorKind {
    /// Returns the stable public error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRecordedHistory => "MFM_REPLAY_RECORDED_HISTORY_INVALID",
            Self::CandidateUnavailable => "MFM_REPLAY_CANDIDATE_UNAVAILABLE",
            Self::CandidateExecutionFailed => "MFM_REPLAY_CANDIDATE_EXECUTION_FAILED",
            Self::ComparisonIntegrityFailed => "MFM_REPLAY_COMPARISON_INTEGRITY_FAILED",
            Self::RunNotFound => "MFM_REPLAY_RUN_NOT_FOUND",
            Self::AuthorityMismatch => "MFM_REPLAY_AUTHORITY_MISMATCH",
            Self::InvalidPage => "MFM_REPLAY_PAGE_INVALID",
            Self::InvalidExport => "MFM_REPLAY_EXPORT_INVALID",
            Self::ExportStreamIo => "MFM_REPLAY_EXPORT_STREAM_IO_FAILED",
            Self::SourceRunExportDenied => "MFM_REPLAY_SOURCE_EXPORT_DENIED",
        }
    }
}

#[cfg(test)]
mod tests {
    use mfm_store::StoreError;

    use super::{store_error, ReplayError};

    #[test]
    fn invalid_store_inspection_pages_remain_public_page_errors() {
        for error in [
            StoreError::InvalidAuditPage { field: "limit" },
            StoreError::InvalidTracePage { field: "limit" },
        ] {
            assert!(matches!(store_error(&error), ReplayError::InvalidPage));
        }
    }

    #[test]
    fn candidate_failures_have_closed_redaction_safe_categories() {
        let cases = [
            (
                ReplayError::InvalidRecordedHistory,
                "MFM_REPLAY_RECORDED_HISTORY_INVALID",
                "recorded run history is invalid",
            ),
            (
                ReplayError::CandidateUnavailable,
                "MFM_REPLAY_CANDIDATE_UNAVAILABLE",
                "the selected replay candidate is unavailable",
            ),
            (
                ReplayError::CandidateExecutionFailed,
                "MFM_REPLAY_CANDIDATE_EXECUTION_FAILED",
                "the selected replay candidate failed during capability-free execution",
            ),
            (
                ReplayError::ComparisonIntegrityFailed,
                "MFM_REPLAY_COMPARISON_INTEGRITY_FAILED",
                "candidate comparison failed integrity verification",
            ),
        ];

        for (error, code, message) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), message);
        }
    }
}
