use serde::{Deserialize, Serialize};

/// High-level error classes used by application-facing APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorClass {
    /// The caller provided invalid input.
    BadRequest,
    /// The request has no valid credential.
    Unauthorized,
    /// The valid credential lacks the exact required grant.
    Forbidden,
    /// The exact tenant-scoped resource does not exist.
    NotFound,
    /// The request conflicts with committed state.
    Conflict,
    /// Internal verification or storage failed.
    #[default]
    Internal,
    /// A required deployment capability is unavailable.
    ServiceUnavailable,
}

/// Stable redaction-safe public error payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PublicError {
    /// Transport-only classification, omitted from the public JSON object.
    #[serde(skip, default)]
    pub class: ErrorClass,
    /// Stable machine-readable code.
    pub code: String,
    /// Reviewed public message.
    pub message: String,
}

impl PublicError {
    /// Constructs one reviewed public error.
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into(),
        }
    }

    /// Constructs a caller-input error.
    pub fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::BadRequest, code, message)
    }

    /// Constructs a fixed internal error.
    pub fn internal(code: impl Into<String>, message: &'static str) -> Self {
        Self::new(ErrorClass::Internal, code, message)
    }

    /// Constructs a redacted lower-boundary error.
    pub fn backend(class: ErrorClass, code: impl Into<String>, message: &'static str) -> Self {
        Self::new(class, code, message)
    }

    /// Returns the one authentication failure contract.
    pub fn authentication_required() -> Self {
        Self::new(
            ErrorClass::Unauthorized,
            "AuthenticationRequired",
            "Authentication is required",
        )
    }

    /// Returns the one exact-grant denial contract.
    pub fn grant_denied() -> Self {
        Self::new(
            ErrorClass::Forbidden,
            "GrantDenied",
            "The credential does not grant this operation",
        )
    }

    /// Returns the tenant-indistinguishable run-not-found contract.
    pub fn run_not_found() -> Self {
        Self::new(
            ErrorClass::NotFound,
            "RunNotFound",
            "The requested run was not found",
        )
    }

    /// Returns the dependency-export denial contract.
    pub fn source_run_export_denied() -> Self {
        Self::new(
            ErrorClass::Forbidden,
            "SourceRunExportDenied",
            "A required source run does not grant export access",
        )
    }

    /// Returns the fixed caller-supplied replay-artifact validation error.
    pub fn replay_artifact_invalid() -> Self {
        Self::new(
            ErrorClass::BadRequest,
            "ReplayArtifactInvalid",
            "The replay artifact is invalid.",
        )
    }

    /// Returns the fixed replay-artifact size-limit error.
    pub fn replay_artifact_too_large() -> Self {
        Self::new(
            ErrorClass::BadRequest,
            "ReplayArtifactTooLarge",
            "The replay artifact exceeds the allowed size.",
        )
    }

    pub(crate) fn replay_verification_failed() -> Self {
        Self::backend(
            ErrorClass::Internal,
            "ReplayVerificationFailed",
            "Recorded run evidence failed verification",
        )
    }

    fn runtime_catalog_unavailable() -> Self {
        Self::backend(
            ErrorClass::ServiceUnavailable,
            "RuntimeCatalogUnavailable",
            "The exact admitted runtime catalog is unavailable",
        )
    }

    /// Constructs a generic not-found error for unrelated keystore surfaces.
    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::NotFound, code, message)
    }
}

impl From<crate::AccessPolicyError> for PublicError {
    fn from(error: crate::AccessPolicyError) -> Self {
        match error {
            crate::AccessPolicyError::AuthenticationRequired => Self::authentication_required(),
            crate::AccessPolicyError::GrantDenied => Self::grant_denied(),
        }
    }
}

impl From<mfm_storage_postgres::PostgresStoreError> for PublicError {
    fn from(error: mfm_storage_postgres::PostgresStoreError) -> Self {
        use mfm_storage_postgres::PostgresStoreError;

        match error {
            PostgresStoreError::SchemaAuthorityMismatch
            | PostgresStoreError::MigrationChecksumMismatch => Self::backend(
                ErrorClass::Internal,
                "IncompatibleStoreSchema",
                "Run store schema is incompatible with this MFM build",
            ),
            PostgresStoreError::Connection
            | PostgresStoreError::WriterRequired
            | PostgresStoreError::WriterFenceRejected
            | PostgresStoreError::Database(_) => Self::backend(
                ErrorClass::ServiceUnavailable,
                "RunStoreUnavailable",
                "The authoritative run store is unavailable",
            ),
            PostgresStoreError::OutcomeUnknown => Self::backend(
                ErrorClass::ServiceUnavailable,
                "RunStoreOutcomeUnknown",
                "The run store commit outcome is unknown",
            ),
            PostgresStoreError::Store(error) => (*error).into(),
            PostgresStoreError::Corruption(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreCorruption",
                "Run store returned invalid data",
            ),
        }
    }
}

impl From<mfm_store::v2::StoreError> for PublicError {
    fn from(error: mfm_store::v2::StoreError) -> Self {
        use mfm_store::v2::StoreError;

        match error {
            StoreError::RunNotFound | StoreError::AppendRunNotFound { .. } => Self::run_not_found(),
            StoreError::AdmissionConflict => Self::backend(
                ErrorClass::Conflict,
                "AdmissionConflict",
                "The invocation identity is already bound to different root material",
            ),
            StoreError::HeadMismatch { .. } => Self::backend(
                ErrorClass::Conflict,
                "RunAdvanced",
                "The run advanced before this action could commit",
            ),
            StoreError::AppendRequestConflict => Self::backend(
                ErrorClass::Conflict,
                "AppendRequestConflict",
                "The append request identity conflicts with committed material",
            ),
            StoreError::RunClosed => Self::backend(
                ErrorClass::Conflict,
                "RunClosed",
                "The run is already closed",
            ),
            StoreError::AccessDenied { .. }
            | StoreError::InvalidAuthorityBinding { .. }
            | StoreError::AdmissionAuthorityMismatch
            | StoreError::JournalContract
            | StoreError::InvalidPreparedAppend { .. }
            | StoreError::EmptyJournal
            | StoreError::PersistedMismatch { .. }
            | StoreError::SequenceOverflow
            | StoreError::FactOrderOverflow { .. }
            | StoreError::InvalidClosure
            | StoreError::DuplicateLogicalRecord
            | StoreError::AuthorizationNotEligible
            | StoreError::UnknownAuthorization
            | StoreError::ObservationAlreadyCommitted
            | StoreError::InvalidAuditTail
            | StoreError::InvalidAuditPage { .. }
            | StoreError::InvalidTracePage { .. }
            | StoreError::FrozenReadIntentConflict
            | StoreError::ObservationNotConsumable
            | StoreError::TransitionFoldMismatch { .. }
            | StoreError::InvalidObjectAuthority { .. }
            | StoreError::MissingObjectAuthority { .. }
            | StoreError::ObjectContentMismatch { .. }
            | StoreError::ObjectAuthorityConflict { .. }
            | StoreError::ObjectNotReachable
            | StoreError::InvalidFactCoordinate
            | StoreError::CorruptFactHistory
            | StoreError::FactSelectionLimitExceeded
            | StoreError::FactScanBindingMismatch
            | StoreError::SourceScopeMismatch
            | StoreError::InvalidSourceClosure
            | StoreError::DigestMismatch { .. }
            | StoreError::MemoryLockPoisoned
            | StoreError::MemoryFailureSelectorAlreadyArmed
            | StoreError::InjectedFailure { .. } => Self::backend(
                ErrorClass::Internal,
                "RunAuthorityInvalid",
                "Run authority verification failed",
            ),
        }
    }
}

impl From<mfm_runtime::RuntimeError> for PublicError {
    fn from(error: mfm_runtime::RuntimeError) -> Self {
        use mfm_runtime::RuntimeError;

        match error {
            RuntimeError::Store(error) => error.into(),
            RuntimeError::StoreBackendUnavailable => Self::backend(
                ErrorClass::ServiceUnavailable,
                "RunStoreUnavailable",
                "The authoritative run store is unavailable",
            ),
            RuntimeError::OutcomeUnknown => Self::backend(
                ErrorClass::ServiceUnavailable,
                "RunStoreOutcomeUnknown",
                "The run store commit outcome is unknown",
            ),
            RuntimeError::CatalogSelection => Self::runtime_catalog_unavailable(),
            RuntimeError::Journal(_)
            | RuntimeError::Identity(_)
            | RuntimeError::Canonical(_)
            | RuntimeError::Program(_)
            | RuntimeError::CandidateCertification(_)
            | RuntimeError::Fact(_)
            | RuntimeError::Spec(_)
            | RuntimeError::Executor(_)
            | RuntimeError::InvalidCallbackResult
            | RuntimeError::AuthorityMismatch
            | RuntimeError::EffectIdentityMismatch
            | RuntimeError::ObservationConflict => Self::backend(
                ErrorClass::Internal,
                "RunExecutionInvalid",
                "The run action failed integrity verification",
            ),
        }
    }
}

impl From<mfm_replay::v2::ReplayError> for PublicError {
    fn from(error: mfm_replay::v2::ReplayError) -> Self {
        use mfm_replay::v2::ReplayErrorKind;

        match error.kind() {
            ReplayErrorKind::RunNotFound => Self::run_not_found(),
            ReplayErrorKind::InvalidPage => Self::bad_request(
                "PageRequestInvalid",
                "The page request or cursor is invalid",
            ),
            ReplayErrorKind::SourceRunExportDenied => Self::source_run_export_denied(),
            ReplayErrorKind::CandidateUnavailable => Self::runtime_catalog_unavailable(),
            ReplayErrorKind::ExportStreamIo => Self::internal(
                "ExportStreamIoFailed",
                "The export stream could not be processed",
            ),
            ReplayErrorKind::InvalidRecordedHistory
            | ReplayErrorKind::CandidateExecutionFailed
            | ReplayErrorKind::ComparisonIntegrityFailed
            | ReplayErrorKind::AuthorityMismatch
            | ReplayErrorKind::InvalidExport => Self::replay_verification_failed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_errors_have_no_diagnostic_escape_hatch() {
        let value = serde_json::to_value(PublicError::authentication_required())
            .expect("serialize public error");
        assert_eq!(
            value,
            serde_json::json!({
                "code": "AuthenticationRequired",
                "message": "Authentication is required",
            })
        );
    }

    #[test]
    fn replay_run_not_found_uses_the_tenant_indistinguishable_contract() {
        let error: PublicError = mfm_replay::v2::ReplayError::RunNotFound.into();
        assert_eq!(error, PublicError::run_not_found());
    }

    #[test]
    fn replay_dependency_denial_uses_the_export_specific_contract() {
        let error: PublicError = mfm_replay::v2::ReplayError::SourceRunExportDenied.into();
        assert_eq!(error, PublicError::source_run_export_denied());
    }

    #[test]
    fn replay_integrity_failures_share_one_redacted_contract() {
        for replay_error in [
            mfm_replay::v2::ReplayError::InvalidRecordedHistory,
            mfm_replay::v2::ReplayError::CandidateExecutionFailed,
            mfm_replay::v2::ReplayError::ComparisonIntegrityFailed,
            mfm_replay::v2::ReplayError::InvalidExport,
        ] {
            let error: PublicError = replay_error.into();
            assert_eq!(error, PublicError::replay_verification_failed());
        }
    }

    #[test]
    fn unavailable_candidate_uses_the_existing_runtime_catalog_contract() {
        let error: PublicError = mfm_replay::v2::ReplayError::CandidateUnavailable.into();
        assert_eq!(error, PublicError::runtime_catalog_unavailable());
        assert_eq!(error.class, ErrorClass::ServiceUnavailable);
        assert_eq!(error.code, "RuntimeCatalogUnavailable");
        assert_eq!(
            error.message,
            "The exact admitted runtime catalog is unavailable"
        );
    }

    #[test]
    fn caller_replay_artifact_errors_have_the_frozen_wire_text() {
        let invalid = PublicError::replay_artifact_invalid();
        assert_eq!(invalid.class, ErrorClass::BadRequest);
        assert_eq!(invalid.code, "ReplayArtifactInvalid");
        assert_eq!(invalid.message, "The replay artifact is invalid.");

        let too_large = PublicError::replay_artifact_too_large();
        assert_eq!(too_large.class, ErrorClass::BadRequest);
        assert_eq!(too_large.code, "ReplayArtifactTooLarge");
        assert_eq!(
            too_large.message,
            "The replay artifact exceeds the allowed size."
        );
    }
}
