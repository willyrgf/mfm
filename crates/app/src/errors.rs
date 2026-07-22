use mfm_capabilities::RedactedProviderDiagnostic;
use mfm_replay::v1::ReplayError;
use mfm_store::v1 as store;
use serde::{Deserialize, Serialize};

/// High-level error classes used by typed application-facing APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorClass {
    /// The caller provided invalid input.
    BadRequest,
    /// The requested run, artifact, or output was not found.
    NotFound,
    /// The request conflicted with current persisted state.
    Conflict,
    /// Runtime, storage, replay, or artifact evidence was internally invalid.
    #[default]
    Internal,
    /// A required live service or runtime capability is unavailable.
    ServiceUnavailable,
}

/// Stable error payload returned by typed application helpers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PublicError {
    /// High-level error classification for HTTP/CLI mapping.
    #[serde(skip, default)]
    pub class: ErrorClass,
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable message safe to display to callers.
    pub message: String,
    /// Closed redaction-safe provider diagnostics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<RedactedProviderDiagnostic>,
}

impl PublicError {
    /// Creates an application error from the supplied classification, code, and message.
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }

    /// Attaches deterministically ordered, deduplicated provider diagnostics.
    pub fn with_diagnostics(mut self, mut diagnostics: Vec<RedactedProviderDiagnostic>) -> Self {
        diagnostics.sort();
        diagnostics.dedup();
        self.diagnostics = diagnostics;
        self
    }

    /// Creates a caller-input error.
    pub fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::BadRequest, code, message)
    }

    /// Creates an internal error with fixed reviewed public text.
    pub fn internal(code: impl Into<String>, message: &'static str) -> Self {
        Self::new(ErrorClass::Internal, code, message)
    }

    /// Creates an application error for a lower-level failure without exposing backend details.
    pub fn backend(class: ErrorClass, code: impl Into<String>, message: &'static str) -> Self {
        Self::new(class, code, message)
    }

    /// Returns a not-found error with an explicit code and message.
    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::NotFound, code, message)
    }
}

impl From<mfm_runtime::RuntimeError> for PublicError {
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
            mfm_runtime::RuntimeError::Failure(failure) => {
                let class = match failure.code().as_str() {
                    "RuntimeConfigRequired" | "RuntimeConfigInvalid" => {
                        ErrorClass::ServiceUnavailable
                    }
                    _ => ErrorClass::Internal,
                };
                Self::new(class, failure.code().as_str(), failure.safe_message())
                    .with_diagnostics(failure.diagnostics().to_vec())
            }
            mfm_runtime::RuntimeError::SpecHash(_)
            | mfm_runtime::RuntimeError::InvalidSpec(_)
            | mfm_runtime::RuntimeError::InvalidRunStream(_)
            | mfm_runtime::RuntimeError::Blocked(_)
            | mfm_runtime::RuntimeError::ExecutionClaim(_)
            | mfm_runtime::RuntimeError::InputMaterialization(_)
            | mfm_runtime::RuntimeError::InvalidRunnerOutput(_)
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

pub(crate) fn runtime_error_with_launch_context(
    error: mfm_runtime::RuntimeError,
    entry_point: &mfm_events::v1::EntryPointLaunchEvidence,
) -> PublicError {
    let mut public = PublicError::from(error);
    if public.code != "RuntimeConfigRequired" {
        return public;
    }

    let targets = entry_point
        .configured_targets
        .iter()
        .map(|configured| configured.target.as_str())
        .collect::<Vec<_>>();
    let target_summary = joined_public_names(&targets);
    if target_summary.is_empty() {
        return public;
    }

    let requires_evm = public
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.provider_family().as_str() == "evm");
    let requires_bitcoin = public
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.provider_family().as_str() == "bitcoin");
    let family_summary = match (requires_evm, requires_bitcoin) {
        (true, true) => "EVM and Bitcoin",
        (true, false) => "EVM",
        (false, true) => "Bitcoin",
        (false, false) => return public,
    };
    public.message = format!("{target_summary} requires {family_summary} runtime routes");
    public
}

fn joined_public_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [name] => (*name).to_owned(),
        [first, second] => format!("{first} and {second}"),
        _ => {
            let (last, rest) = names.split_last().expect("non-empty names were checked");
            format!("{}, and {last}", rest.join(", "))
        }
    }
}

impl From<store::StoreError> for PublicError {
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

impl From<mfm_storage_postgres::PostgresStoreError> for PublicError {
    fn from(error: mfm_storage_postgres::PostgresStoreError) -> Self {
        match error {
            mfm_storage_postgres::PostgresStoreError::Authority(
                mfm_storage_postgres::PostgresStoreAuthorityError::StoreAuthorityMismatch
                | mfm_storage_postgres::PostgresStoreAuthorityError::MigrationChecksumMismatch,
            ) => Self::backend(
                ErrorClass::Internal,
                "IncompatibleStoreSchema",
                "Run store schema is incompatible with this MFM build",
            ),
            mfm_storage_postgres::PostgresStoreError::Authority(
                mfm_storage_postgres::PostgresStoreAuthorityError::Connection,
            ) => Self::backend(
                ErrorClass::Internal,
                "RunStoreUnavailable",
                "Run store is unavailable",
            ),
            mfm_storage_postgres::PostgresStoreError::Store(_) => Self::backend(
                ErrorClass::Conflict,
                "RunStoreRejected",
                "Run store rejected the requested operation",
            ),
            mfm_storage_postgres::PostgresStoreError::Database(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreUnavailable",
                "Run store is unavailable",
            ),
            mfm_storage_postgres::PostgresStoreError::Corruption(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreCorruption",
                "Run store returned invalid data",
            ),
        }
    }
}

impl From<ReplayError> for PublicError {
    fn from(error: ReplayError) -> Self {
        Self::backend(
            ErrorClass::Internal,
            error.code(),
            "Replay verification failed",
        )
    }
}

impl From<mfm_spec::SpecError> for PublicError {
    fn from(_error: mfm_spec::SpecError) -> Self {
        Self::backend(
            ErrorClass::Internal,
            "CertifiedSpecInvalid",
            "Certified typed spec is invalid",
        )
    }
}

impl From<mfm_certify::CertifyError> for PublicError {
    fn from(_error: mfm_certify::CertifyError) -> Self {
        Self::backend(
            ErrorClass::BadRequest,
            "CertifiedSpecVerificationFailed",
            "Certified typed spec verification failed",
        )
    }
}

impl From<mfm_facts::FactError> for PublicError {
    fn from(_error: mfm_facts::FactError) -> Self {
        Self::backend(
            ErrorClass::BadRequest,
            "FactQueryInvalid",
            "Fact query input is invalid",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_capabilities::{ProviderDiagnosticCode, ProviderDiagnosticValue};
    use mfm_ids::LocalPublicId;

    fn id(value: &str) -> LocalPublicId {
        LocalPublicId::new(value).expect("checked test id")
    }

    #[test]
    fn public_error_json_omits_class_and_empty_diagnostics() {
        let error = PublicError::bad_request("InvalidInput", "input is invalid");
        assert_eq!(
            serde_json::to_value(error).expect("serialize public error"),
            serde_json::json!({
                "code": "InvalidInput",
                "message": "input is invalid",
            })
        );
    }

    #[test]
    fn public_error_json_uses_the_canonical_provider_diagnostic() {
        let diagnostic = RedactedProviderDiagnostic::new(
            id("evm"),
            ProviderDiagnosticCode::ProviderConfigurationMissing,
        )
        .with_field(
            id("network_id"),
            ProviderDiagnosticValue::Id(id("test-evm")),
        )
        .with_field(id("expected_chain_id"), ProviderDiagnosticValue::U64(1));
        let error = PublicError::new(
            ErrorClass::ServiceUnavailable,
            "RuntimeConfigRequired",
            "test/primary requires EVM runtime routes",
        )
        .with_diagnostics(vec![diagnostic]);
        let value = serde_json::to_value(error).expect("serialize public error");

        assert_eq!(value["diagnostics"][0]["provider_family"], "evm");
        assert_eq!(
            value["diagnostics"][0]["code"],
            "provider_configuration_missing"
        );
        assert!(value.get("class").is_none());
    }

    #[test]
    fn postgres_authority_failures_share_one_redacted_public_contract() {
        use mfm_storage_postgres::{PostgresStoreAuthorityError, PostgresStoreError};

        for internal in [
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
            PostgresStoreAuthorityError::MigrationChecksumMismatch,
        ] {
            let error = PublicError::from(PostgresStoreError::Authority(internal));
            assert_eq!(error.code, "IncompatibleStoreSchema");
            assert_eq!(
                error.message,
                "Run store schema is incompatible with this MFM build"
            );
            let rendered = format!("{error:?}\n{error}");
            assert!(!rendered.contains("authority"));
            assert!(!rendered.contains("migration"));
            assert!(!rendered.contains("checksum"));
        }
    }

    #[test]
    fn launch_context_summarizes_mixed_families_in_presentation_order() {
        let diagnostic = |family| {
            RedactedProviderDiagnostic::new(
                id(family),
                ProviderDiagnosticCode::ProviderConfigurationMissing,
            )
        };
        let failure = mfm_runtime::RuntimeFailure::new(
            mfm_events::v1::ErrorCode::new("RuntimeConfigRequired").expect("code"),
            mfm_events::v1::ErrorCategory::Capability,
            "runtime configuration is required",
            vec![diagnostic("bitcoin"), diagnostic("evm")],
        )
        .expect("failure");
        let entry_point = mfm_events::v1::EntryPointLaunchEvidence::new(
            "mfm.portfolio/snapshot@2",
            vec![mfm_events::v1::ConfiguredTargetEvidence::new(
                mfm_ids::StableAuthorKey::new("test/primary").expect("target"),
                mfm_ids::SchemaId::new(
                    "mfm.test.schema",
                    "1",
                    mfm_ids::DigestAlgorithm::Sha256JcsV1,
                    mfm_canonical::sha256_digest_bytes(b"mfm.test.schema"),
                )
                .expect("schema"),
                mfm_ids::ContentDigest::from_digest(
                    mfm_ids::DigestAlgorithm::Sha256JcsV1,
                    mfm_canonical::sha256_digest_bytes(b"test target"),
                ),
            )],
        )
        .expect("entry point");

        let error = runtime_error_with_launch_context(
            mfm_runtime::RuntimeError::Failure(failure),
            &entry_point,
        );

        assert_eq!(
            error.message,
            "test/primary requires EVM and Bitcoin runtime routes"
        );
    }
}
