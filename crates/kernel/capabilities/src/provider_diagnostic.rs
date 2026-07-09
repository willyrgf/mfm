use std::collections::BTreeMap;
use std::fmt;

use mfm_ids::LocalPublicId;

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
        match self.code {
            ProviderDiagnosticCode::SourceMismatch => "provider_source_mismatch",
            _ => "provider_failure",
        }
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
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Id(value) => serde_json::Value::String(value.to_string()),
            Self::U64(value) => serde_json::json!(value),
            Self::I64(value) => serde_json::json!(value),
            Self::Bool(value) => serde_json::json!(value),
        }
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
mod tests {
    use super::*;

    fn id(value: &str) -> LocalPublicId {
        LocalPublicId::new(value).expect("test id")
    }

    #[test]
    fn diagnostic_renders_stable_code_summary_and_public_details() {
        let diagnostic =
            RedactedProviderDiagnostic::new(id("bitcoin"), ProviderDiagnosticCode::RpcHttpStatus)
                .with_operation(id("scantxoutset"))
                .with_field(id("http_status"), ProviderDiagnosticValue::U64(403));

        assert_eq!(diagnostic.stable_error_code(), "bitcoin_rpc_http_status");
        assert_eq!(
            diagnostic.summary(),
            "rpc_http_status operation=scantxoutset http_status=403"
        );
        assert_eq!(
            diagnostic.to_public_details_json(),
            serde_json::json!({
                "diagnostic_kind": "provider_failure",
                "provider_family": "bitcoin",
                "code": "rpc_http_status",
                "operation": "scantxoutset",
                "fields": {
                    "http_status": 403,
                },
            })
        );
    }

    #[test]
    fn source_mismatch_uses_source_mismatch_diagnostic_kind() {
        let diagnostic =
            RedactedProviderDiagnostic::new(id("evm"), ProviderDiagnosticCode::SourceMismatch);

        assert_eq!(
            diagnostic.to_public_details_json()["diagnostic_kind"],
            "provider_source_mismatch"
        );
    }

    #[test]
    fn diagnostic_shape_has_no_raw_string_escape_hatch() {
        let diagnostic =
            RedactedProviderDiagnostic::new(id("evm"), ProviderDiagnosticCode::TransportFailed)
                .with_operation(id("eth_chain_id"))
                .with_field(id("retryable"), ProviderDiagnosticValue::Bool(false));
        let rendered = format!("{diagnostic:?} {}", diagnostic.summary());

        assert!(!rendered.contains("http://"));
        assert!(!rendered.contains("authorization"));
        assert!(!rendered.contains("token="));
    }
}
