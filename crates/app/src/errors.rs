use mfm_replay::v1::ReplayError;
use mfm_store::v1 as store;

use super::EntryPointOpError;

/// High-level error classes used by typed application-facing APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// The caller provided invalid input.
    BadRequest,
    /// The requested run, artifact, or output was not found.
    NotFound,
    /// The request conflicted with current persisted state.
    Conflict,
    /// Runtime, storage, replay, or artifact evidence was internally invalid.
    Internal,
}

/// Stable error payload returned by typed application helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AppError {
    /// High-level error classification for HTTP/CLI mapping.
    pub class: ErrorClass,
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable message safe to display to callers.
    pub message: String,
}

/// Message text that has been selected for public CLI/REST/app surfaces.
///
/// This type marks the boundary where lower-level diagnostics are either intentionally exposed as
/// caller input validation messages or replaced by fixed safe text. It does not authorize callers
/// to forward backend `Display` output from storage, runtime, transport, signer, or provider
/// errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicSafeMessage(String);

impl PublicSafeMessage {
    /// Creates public-safe message text from an already reviewed string.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// Creates fixed public-safe text for a lower-level backend failure.
    pub fn backend(message: &'static str) -> Self {
        Self(message.to_owned())
    }

    /// Consumes the reviewed message into owned text for existing public response structs.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<&'static str> for PublicSafeMessage {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PublicSafeMessage {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl AppError {
    /// Creates an application error from the supplied classification, code, and message.
    pub fn new(
        class: ErrorClass,
        code: impl Into<String>,
        message: impl Into<PublicSafeMessage>,
    ) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into().into_string(),
        }
    }

    /// Creates an application error for a lower-level failure without exposing backend details.
    pub fn backend(class: ErrorClass, code: impl Into<String>, message: &'static str) -> Self {
        Self::new(class, code, PublicSafeMessage::backend(message))
    }

    /// Returns a not-found error with an explicit code and message.
    pub fn not_found(code: impl Into<String>, message: impl Into<PublicSafeMessage>) -> Self {
        Self::new(ErrorClass::NotFound, code, message)
    }
}

impl From<mfm_runtime::RuntimeError> for AppError {
    fn from(error: mfm_runtime::RuntimeError) -> Self {
        match error {
            mfm_runtime::RuntimeError::Store(_) => Self::backend(
                ErrorClass::Conflict,
                "RunStoreRejected",
                "Run store rejected the requested operation",
            ),
            mfm_runtime::RuntimeError::RunnerBinding(_) => Self::backend(
                ErrorClass::BadRequest,
                "LaunchRunnerUnavailable",
                "A required typed runner is unavailable",
            ),
            mfm_runtime::RuntimeError::SpecHash(_)
            | mfm_runtime::RuntimeError::InvalidSpec(_)
            | mfm_runtime::RuntimeError::InvalidRunStream(_)
            | mfm_runtime::RuntimeError::Blocked(_)
            | mfm_runtime::RuntimeError::ExecutionClaim(_)
            | mfm_runtime::RuntimeError::InputMaterialization(_)
            | mfm_runtime::RuntimeError::InvalidRunnerOutput(_)
            | mfm_runtime::RuntimeError::InvalidRunnerOutputFailure { .. }
            | mfm_runtime::RuntimeError::RuntimeValidation(_)
            | mfm_runtime::RuntimeError::Identity(_)
            | mfm_runtime::RuntimeError::Canonical(_) => Self::backend(
                ErrorClass::Internal,
                "LaunchRuntimeError",
                "Typed runtime rejected the requested operation",
            ),
        }
    }
}

impl From<store::StoreError> for AppError {
    fn from(error: store::StoreError) -> Self {
        match error {
            store::StoreError::MissingArtifact { artifact_id } => Self::not_found(
                "ArtifactNotFound",
                format!("typed artifact {artifact_id} was not found"),
            ),
            store::StoreError::ArtifactReadFailed { .. } => Self::backend(
                ErrorClass::Internal,
                "ArtifactReadFailed",
                "Typed artifact bytes could not be loaded",
            ),
            store::StoreError::ArtifactEvidenceMismatch { .. } => Self::backend(
                ErrorClass::Internal,
                "ArtifactEvidenceMismatch",
                "Typed artifact evidence did not match the requested authority",
            ),
            _ => Self::backend(
                ErrorClass::Conflict,
                "RunStoreRejected",
                "Run store rejected the requested operation",
            ),
        }
    }
}

impl From<mfm_stream_store_postgres::PostgresStoreError> for AppError {
    fn from(error: mfm_stream_store_postgres::PostgresStoreError) -> Self {
        match error {
            mfm_stream_store_postgres::PostgresStoreError::Authority(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreAuthorityInvalid",
                "Run store authority could not be validated",
            ),
            mfm_stream_store_postgres::PostgresStoreError::Store(_) => Self::backend(
                ErrorClass::Conflict,
                "RunStoreRejected",
                "Run store rejected the requested operation",
            ),
            mfm_stream_store_postgres::PostgresStoreError::Database(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreUnavailable",
                "Run store is unavailable",
            ),
            mfm_stream_store_postgres::PostgresStoreError::Corruption(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreCorruption",
                "Run store returned invalid data",
            ),
        }
    }
}

impl From<ReplayError> for AppError {
    fn from(error: ReplayError) -> Self {
        Self::backend(
            ErrorClass::Internal,
            error.code(),
            "Replay verification failed",
        )
    }
}

impl From<mfm_spec::SpecError> for AppError {
    fn from(_error: mfm_spec::SpecError) -> Self {
        Self::backend(
            ErrorClass::Internal,
            "CertifiedSpecInvalid",
            "Certified typed spec is invalid",
        )
    }
}

impl From<mfm_certify::CertifyError> for AppError {
    fn from(_error: mfm_certify::CertifyError) -> Self {
        Self::backend(
            ErrorClass::BadRequest,
            "CertifiedSpecVerificationFailed",
            "Certified typed spec verification failed",
        )
    }
}

impl From<mfm_facts::FactError> for AppError {
    fn from(_error: mfm_facts::FactError) -> Self {
        Self::backend(
            ErrorClass::BadRequest,
            "FactQueryInvalid",
            "Fact query input is invalid",
        )
    }
}

impl From<EntryPointOpError> for AppError {
    fn from(error: EntryPointOpError) -> Self {
        Self::new(
            ErrorClass::BadRequest,
            error.code().to_owned(),
            error.message().to_owned(),
        )
    }
}
