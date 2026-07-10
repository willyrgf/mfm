use std::fmt;

use mfm_capabilities::RedactedProviderDiagnostic;
use mfm_events::v1 as events;
use mfm_store::v1 as store;
use serde_json::Value;

/// Schema name for retained runtime attempt-failure diagnostics.
pub const REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_SCHEMA: &str =
    "mfm.runtime.redacted_attempt_failure_diagnostic";
/// Schema version for retained runtime attempt-failure diagnostics.
pub const REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_VERSION: &str = "2";

/// Validated public metadata supplied by a typed runner failure.
///
/// Runtime owns retryability and attaches it when it creates the persisted event. A runner can
/// select only the stable public code, category, and safe message represented here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFailure {
    code: events::ErrorCode,
    category: events::ErrorCategory,
    safe_message: String,
}

impl RuntimeFailure {
    /// Creates validated public failure metadata.
    pub fn new(
        code: events::ErrorCode,
        category: events::ErrorCategory,
        safe_message: impl Into<String>,
    ) -> crate::Result<Self> {
        let safe_message = safe_message.into();
        events::MfmErrorInfo::new(code.clone(), category, false, safe_message.clone()).map_err(
            |_| RuntimeError::InvalidRunnerOutput("invalid runner failure contract".into()),
        )?;
        Ok(Self {
            code,
            category,
            safe_message,
        })
    }

    /// Returns the stable public error code.
    pub const fn code(&self) -> &events::ErrorCode {
        &self.code
    }

    /// Returns the public error category.
    pub const fn category(&self) -> events::ErrorCategory {
        self.category
    }

    /// Returns the validated public-safe message.
    pub fn safe_message(&self) -> &str {
        &self.safe_message
    }

    pub(crate) fn public_error(&self, retryable: bool) -> crate::Result<events::MfmErrorInfo> {
        events::MfmErrorInfo::new(
            self.code.clone(),
            self.category,
            retryable,
            self.safe_message.clone(),
        )
        .map_err(|_| RuntimeError::InvalidRunnerOutput("invalid runner failure contract".into()))
    }
}

impl fmt::Display for RuntimeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

/// Versioned, discriminated diagnostic evidence retained with a runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDiagnostic {
    /// Closed provider diagnostic evidence.
    Provider(RedactedProviderDiagnostic),
}

impl RuntimeDiagnostic {
    /// Creates provider diagnostic evidence.
    pub fn provider(diagnostic: RedactedProviderDiagnostic) -> Self {
        Self::Provider(diagnostic)
    }

    /// Parses the versioned diagnostic envelope from retained evidence.
    pub fn from_json(value: &Value) -> crate::Result<Self> {
        let object = value.as_object().ok_or_else(invalid_diagnostic)?;
        if object.len() != 3
            || object
                .keys()
                .any(|key| !matches!(key.as_str(), "details" | "kind" | "version"))
        {
            return Err(invalid_diagnostic());
        }
        if object.get("kind").and_then(Value::as_str) != Some("provider")
            || object.get("version").and_then(Value::as_u64) != Some(1)
        {
            return Err(invalid_diagnostic());
        }
        let diagnostic = RedactedProviderDiagnostic::from_public_details_json(
            object.get("details").ok_or_else(invalid_diagnostic)?,
        )
        .ok_or_else(invalid_diagnostic)?;
        Ok(Self::Provider(diagnostic))
    }

    /// Parses the complete retained attempt-failure diagnostic artifact.
    pub fn from_attempt_artifact_json(value: &Value) -> crate::Result<Option<Self>> {
        let object = value.as_object().ok_or_else(invalid_diagnostic)?;
        const FIELDS: [&str; 7] = [
            "attempt_id",
            "diagnostic",
            "diagnostic_schema",
            "diagnostic_schema_version",
            "node_id",
            "run_id",
            "spec_hash",
        ];
        if object.len() != FIELDS.len() || FIELDS.iter().any(|field| !object.contains_key(*field)) {
            return Err(invalid_diagnostic());
        }
        if object.get("diagnostic_schema").and_then(Value::as_str)
            != Some(REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_SCHEMA)
            || object
                .get("diagnostic_schema_version")
                .and_then(Value::as_str)
                != Some(REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_VERSION)
        {
            return Err(invalid_diagnostic());
        }
        for field in ["attempt_id", "node_id", "run_id", "spec_hash"] {
            if !object.get(field).is_some_and(Value::is_string) {
                return Err(invalid_diagnostic());
            }
        }
        match object.get("diagnostic").ok_or_else(invalid_diagnostic)? {
            Value::Null => Ok(None),
            diagnostic => Self::from_json(diagnostic).map(Some),
        }
    }

    /// Returns the canonical public JSON details represented by this diagnostic.
    pub fn public_details_json(&self) -> Value {
        match self {
            Self::Provider(diagnostic) => diagnostic.to_public_details_json(),
        }
    }

    /// Serializes the discriminated diagnostic envelope for retained evidence.
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({
            "details": self.public_details_json(),
            "kind": "provider",
            "version": 1,
        })
    }
}

fn invalid_diagnostic() -> RuntimeError {
    RuntimeError::Canonical("invalid runtime diagnostic envelope".to_owned())
}

/// Runtime failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    /// Certified spec hash verification failed.
    #[error("certified spec hash error: {0}")]
    SpecHash(String),
    /// Certified spec structure is not executable by the runtime.
    #[error("invalid typed runtime spec: {0}")]
    InvalidSpec(String),
    /// The run stream does not match the certified spec or requested run id.
    #[error("invalid typed run stream: {0}")]
    InvalidRunStream(String),
    /// A runner binding is missing or inconsistent with certified descriptor evidence.
    #[error("typed runner binding error: {0}")]
    RunnerBinding(String),
    /// No node is runnable and the run is not complete.
    #[error("typed scheduler blocked: {0}")]
    Blocked(String),
    /// The caller does not hold the run execution claim required to drive.
    #[error("typed execution claim rejected: {0}")]
    ExecutionClaim(String),
    /// Input materialization failed.
    #[error("typed input materialization failed: {0}")]
    InputMaterialization(String),
    /// Runner output violated certified node or capability authority.
    #[error("invalid runner output: {0}")]
    InvalidRunnerOutput(String),
    /// Runner output failed with typed public metadata and optional diagnostic evidence.
    #[error("invalid runner output: {failure}")]
    InvalidRunnerOutputFailure {
        /// Validated public failure metadata.
        failure: RuntimeFailure,
        /// Closed diagnostic evidence, when the runner produced it.
        diagnostic: Option<Box<RuntimeDiagnostic>>,
    },
    /// Runtime validation failed inside a valid started attempt.
    #[error("runtime validation failed: {0}")]
    RuntimeValidation(String),
    /// Store contract rejected a typed commit.
    #[error("typed store error: {0}")]
    Store(String),
    /// Identity construction failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// Canonical JSON construction failed.
    #[error("canonical JSON error: {0}")]
    Canonical(String),
}

impl From<store::StoreError> for RuntimeError {
    fn from(error: store::StoreError) -> Self {
        Self::Store(error.to_string())
    }
}

impl From<mfm_ids::IdentityError> for RuntimeError {
    fn from(error: mfm_ids::IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

impl From<mfm_spec::SpecError> for RuntimeError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::SpecHash(error.to_string())
    }
}

impl From<mfm_events::EventError> for RuntimeError {
    fn from(error: mfm_events::EventError) -> Self {
        Self::Identity(error.to_string())
    }
}

pub(crate) fn async_store_error(error: impl fmt::Display) -> RuntimeError {
    RuntimeError::Store(error.to_string())
}
