use std::collections::BTreeMap;
use std::fmt;

use mfm_ids::LocalPublicId;
use serde_json::Value;

/// Stable redaction-safe provider diagnostic.
///
/// The diagnostic intentionally accepts only checked public identifiers, booleans, and integers.
/// It has no field for raw provider text, URLs, request parameters, response bodies, headers, or
/// transport error displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedProviderDiagnostic {
    provider_family: LocalPublicId,
    code: ProviderDiagnosticCode,
    operation: Option<LocalPublicId>,
    fields: BTreeMap<LocalPublicId, ProviderDiagnosticValue>,
}

impl RedactedProviderDiagnostic {
    /// Creates a redaction-safe provider diagnostic.
    pub fn new(provider_family: LocalPublicId, code: ProviderDiagnosticCode) -> Self {
        Self {
            provider_family,
            code,
            operation: None,
            fields: BTreeMap::new(),
        }
    }

    /// Attaches the provider operation associated with the failure.
    pub fn with_operation(mut self, operation: LocalPublicId) -> Self {
        self.operation = Some(operation);
        self
    }

    /// Attaches a closed redaction-safe diagnostic field.
    pub fn with_field(mut self, key: LocalPublicId, value: ProviderDiagnosticValue) -> Self {
        self.fields.insert(key, value);
        self
    }

    /// Returns the provider family.
    pub const fn provider_family(&self) -> &LocalPublicId {
        &self.provider_family
    }

    /// Returns the diagnostic code.
    pub const fn code(&self) -> ProviderDiagnosticCode {
        self.code
    }

    /// Returns the provider operation, when classified.
    pub const fn operation(&self) -> Option<&LocalPublicId> {
        self.operation.as_ref()
    }

    /// Returns the closed diagnostic fields.
    pub const fn fields(&self) -> &BTreeMap<LocalPublicId, ProviderDiagnosticValue> {
        &self.fields
    }

    /// Returns the generic diagnostic kind used in retained runtime public details.
    pub const fn diagnostic_kind(&self) -> &'static str {
        diagnostic_kind_for(self.code)
    }

    /// Returns a stable machine-readable error code for public output envelopes.
    pub fn stable_error_code(&self) -> String {
        format!("{}_{}", self.provider_family.as_str(), self.code.as_str())
    }

    /// Returns a compact redaction-safe summary for public messages and logs.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.code.as_str().to_owned()];
        if let Some(operation) = &self.operation {
            parts.push(format!("operation={operation}"));
        }
        for (key, value) in &self.fields {
            parts.push(format!("{key}={value}"));
        }
        parts.join(" ")
    }

    /// Converts the diagnostic to a closed JSON object for runtime public diagnostic artifacts.
    pub fn to_public_details_json(&self) -> serde_json::Value {
        let mut fields = serde_json::Map::new();
        for (key, value) in &self.fields {
            fields.insert(key.to_string(), value.to_json());
        }
        serde_json::json!({
            "diagnostic_kind": self.diagnostic_kind(),
            "provider_family": self.provider_family.to_string(),
            "code": self.code.as_str(),
            "operation": self.operation.as_ref().map(ToString::to_string),
            "fields": fields,
        })
    }

    /// Parses the closed public-details representation of a provider diagnostic.
    pub fn from_public_details_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        const FIELDS: [&str; 5] = [
            "code",
            "diagnostic_kind",
            "fields",
            "operation",
            "provider_family",
        ];
        if object.len() != FIELDS.len() || FIELDS.iter().any(|field| !object.contains_key(*field)) {
            return None;
        }

        let provider_family = LocalPublicId::new(object.get("provider_family")?.as_str()?).ok()?;
        let code = ProviderDiagnosticCode::from_str(object.get("code")?.as_str()?)?;
        let diagnostic_kind = object.get("diagnostic_kind")?.as_str()?;
        if diagnostic_kind != diagnostic_kind_for(code) {
            return None;
        }
        let operation = match object.get("operation")? {
            Value::Null => None,
            Value::String(value) => Some(LocalPublicId::new(value).ok()?),
            _ => return None,
        };
        let fields = object.get("fields")?.as_object()?;
        let fields = fields
            .iter()
            .map(|(key, value)| {
                Some((
                    LocalPublicId::new(key).ok()?,
                    ProviderDiagnosticValue::from_json(value)?,
                ))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(Self {
            provider_family,
            code,
            operation,
            fields,
        })
    }
}

impl fmt::Display for RedactedProviderDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

/// Closed provider diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderDiagnosticCode {
    /// Provider runtime configuration was invalid.
    ProviderConfigurationInvalid,
    /// Semantic network route was unavailable.
    RouteUnavailable,
    /// No allowed provider source was available.
    SourceUnavailable,
    /// Selected source was not allowed by policy.
    SourceNotAllowed,
    /// Transport request failed before a protocol response was available.
    TransportFailed,
    /// Provider returned a non-success HTTP status.
    RpcHttpStatus,
    /// Provider returned a JSON-RPC error object.
    RpcJsonError,
    /// Provider response failed the typed response contract.
    ResponseInvalid,
    /// Provider response did not contain the required result value.
    ResponseMissingResult,
    /// Provider evidence did not match the provider binding.
    SourceMismatch,
    /// Requested operation is not supported by the provider binding.
    UnsupportedOperation,
    /// Provider operation finished without producing a usable result.
    OperationIncomplete,
}

impl ProviderDiagnosticCode {
    /// Returns the stable code string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderConfigurationInvalid => "provider_configuration_invalid",
            Self::RouteUnavailable => "route_unavailable",
            Self::SourceUnavailable => "source_unavailable",
            Self::SourceNotAllowed => "source_not_allowed",
            Self::TransportFailed => "transport_failed",
            Self::RpcHttpStatus => "rpc_http_status",
            Self::RpcJsonError => "rpc_json_error",
            Self::ResponseInvalid => "response_invalid",
            Self::ResponseMissingResult => "response_missing_result",
            Self::SourceMismatch => "source_mismatch",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::OperationIncomplete => "operation_incomplete",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "provider_configuration_invalid" => Self::ProviderConfigurationInvalid,
            "route_unavailable" => Self::RouteUnavailable,
            "source_unavailable" => Self::SourceUnavailable,
            "source_not_allowed" => Self::SourceNotAllowed,
            "transport_failed" => Self::TransportFailed,
            "rpc_http_status" => Self::RpcHttpStatus,
            "rpc_json_error" => Self::RpcJsonError,
            "response_invalid" => Self::ResponseInvalid,
            "response_missing_result" => Self::ResponseMissingResult,
            "source_mismatch" => Self::SourceMismatch,
            "unsupported_operation" => Self::UnsupportedOperation,
            "operation_incomplete" => Self::OperationIncomplete,
            _ => return None,
        })
    }
}

impl fmt::Display for ProviderDiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Closed redaction-safe provider diagnostic value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderDiagnosticValue {
    /// Checked public identifier.
    Id(LocalPublicId),
    /// Unsigned integer.
    U64(u64),
    /// Signed integer.
    I64(i64),
    /// Boolean.
    Bool(bool),
}

impl ProviderDiagnosticValue {
    fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::String(value) => Some(Self::Id(LocalPublicId::new(value).ok()?)),
            Value::Number(value) => value
                .as_u64()
                .map(Self::U64)
                .or_else(|| value.as_i64().map(Self::I64)),
            Value::Bool(value) => Some(Self::Bool(*value)),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Id(value) => serde_json::Value::String(value.to_string()),
            Self::U64(value) => serde_json::json!(value),
            Self::I64(value) => serde_json::json!(value),
            Self::Bool(value) => serde_json::json!(value),
        }
    }
}

const fn diagnostic_kind_for(code: ProviderDiagnosticCode) -> &'static str {
    match code {
        ProviderDiagnosticCode::SourceMismatch => "provider_source_mismatch",
        _ => "provider_failure",
    }
}

impl fmt::Display for ProviderDiagnosticValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(value) => write!(f, "{value}"),
            Self::U64(value) => write!(f, "{value}"),
            Self::I64(value) => write!(f, "{value}"),
            Self::Bool(value) => write!(f, "{value}"),
        }
    }
}

#[cfg(test)]
#[path = "provider_diagnostic_tests.rs"]
mod tests;
