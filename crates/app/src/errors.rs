use serde::{Deserialize, Deserializer, Serialize};

use mfm_ids::{ContentRef, OccurrenceId, RunId, StoreEpoch, StoreScopeId};
use mfm_journal::structured::JournalHead;
use mfm_spec::structured::StructuredComponentKind;

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

/// Reviewed public Runtime boundary attached to an attributed fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicRuntimeFaultPhase {
    /// Invoke a qualified Pure state callback.
    InvokePure,
    /// Author a typed Read or Effect request.
    AuthorRequest,
    /// Select, preflight, or enter a qualified physical binding.
    QualifyAccess,
    /// Settle one already committed normal observation.
    SettleObservation,
    /// Build or callback-free qualify one proposed history candidate.
    QualifyCandidate,
    /// Atomically append one qualified candidate.
    AppendCandidate,
    /// Load and callback-free verify existing history.
    LoadHistory,
}

/// Secret-free qualified authority named by a public Runtime fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublicRuntimeFaultSubject {
    /// Qualified process component without its private implementation identity.
    Process {
        /// Semantic component kind.
        component_kind: StructuredComponentKind,
        /// Exact public semantic contract.
        semantic_contract_ref: ContentRef,
    },
    /// Immutable structured-history writer identity.
    Store {
        /// Qualified store lineage.
        store_scope_id: StoreScopeId,
        /// Authoritative writer epoch.
        store_epoch: StoreEpoch,
    },
}

/// Reviewed, secret-free context for one public Runtime fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicRuntimeFaultAttribution {
    /// Exact Runtime boundary that detected the fault.
    pub phase: PublicRuntimeFaultPhase,
    /// Affected run.
    pub run_id: RunId,
    /// Last verified journal head, absent before a head could be verified.
    pub pre_fault_head: Option<JournalHead>,
    /// Current executable occurrence, when applicable.
    pub occurrence_id: Option<OccurrenceId>,
    /// Qualified semantic component or store authority.
    pub subject: PublicRuntimeFaultSubject,
}

/// Stable redaction-safe public error payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
#[serde(deny_unknown_fields)]
pub struct PublicError {
    /// Transport-only classification, omitted from the public JSON object.
    #[serde(skip, default)]
    class: ErrorClass,
    /// Stable machine-readable code.
    code: String,
    /// Reviewed public message.
    message: String,
    /// Reviewed secret-free Runtime attribution, when this error came from Runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_fault: Option<Box<PublicRuntimeFaultAttribution>>,
}

/// Maximum UTF-8 bytes in one public error code.
pub const MAX_PUBLIC_ERROR_CODE_BYTES: usize = 128;
/// Maximum UTF-8 bytes in one reviewed public error message.
pub const MAX_PUBLIC_ERROR_MESSAGE_BYTES: usize = 4_096;

const INVALID_PUBLIC_ERROR_CODE: &str = "PublicErrorContractViolation";
const INVALID_PUBLIC_ERROR_MESSAGE: &str = "A public error could not be rendered";

impl PublicError {
    /// Constructs one reviewed public error.
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        let code = code.into();
        let message = message.into();
        if !public_error_fields_are_valid(&code, &message) {
            return Self {
                class: ErrorClass::Internal,
                code: INVALID_PUBLIC_ERROR_CODE.to_owned(),
                message: INVALID_PUBLIC_ERROR_MESSAGE.to_owned(),
                runtime_fault: None,
            };
        }
        Self {
            class,
            code,
            message,
            runtime_fault: None,
        }
    }

    /// Attaches reviewed secret-free Runtime attribution.
    pub fn with_runtime_fault(mut self, attribution: PublicRuntimeFaultAttribution) -> Self {
        self.runtime_fault = Some(Box::new(attribution));
        self
    }

    /// Returns the transport-only classification.
    pub const fn class(&self) -> ErrorClass {
        self.class
    }

    /// Returns the stable machine-readable code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the reviewed public message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns reviewed Runtime attribution when this error came from Runtime.
    pub fn runtime_fault(&self) -> Option<&PublicRuntimeFaultAttribution> {
        self.runtime_fault.as_deref()
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

    pub(crate) fn replay_verification_failed() -> Self {
        Self::backend(
            ErrorClass::Internal,
            "ReplayVerificationFailed",
            "Recorded run evidence failed verification",
        )
    }

    /// Constructs a generic not-found error for unrelated keystore surfaces.
    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::NotFound, code, message)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicErrorWire {
    code: String,
    message: String,
    #[serde(default)]
    runtime_fault: Option<PublicRuntimeFaultAttribution>,
}

impl<'de> Deserialize<'de> for PublicError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PublicErrorWire::deserialize(deserializer)?;
        if !public_error_fields_are_valid(&wire.code, &wire.message) {
            return Err(serde::de::Error::custom(
                "public error code or message exceeds the reviewed wire bounds",
            ));
        }
        Ok(Self {
            class: ErrorClass::Internal,
            code: wire.code,
            message: wire.message,
            runtime_fault: wire.runtime_fault.map(Box::new),
        })
    }
}

fn public_error_fields_are_valid(code: &str, message: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_PUBLIC_ERROR_CODE_BYTES
        && !message.is_empty()
        && message.len() <= MAX_PUBLIC_ERROR_MESSAGE_BYTES
}

impl From<crate::AccessPolicyError> for PublicError {
    fn from(error: crate::AccessPolicyError) -> Self {
        match error {
            crate::AccessPolicyError::AuthenticationRequired => Self::authentication_required(),
            crate::AccessPolicyError::GrantDenied => Self::grant_denied(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_canonical::sha256_digest_bytes;
    use mfm_ids::{DigestAlgorithm, StoreEpoch};

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
    fn public_error_construction_and_decode_enforce_the_frozen_bounds() {
        let oversized_code = "c".repeat(MAX_PUBLIC_ERROR_CODE_BYTES + 1);
        let normalized = PublicError::bad_request(oversized_code, "private sentinel");
        assert_eq!(normalized.class(), ErrorClass::Internal);
        assert_eq!(normalized.code(), INVALID_PUBLIC_ERROR_CODE);
        assert_eq!(normalized.message(), INVALID_PUBLIC_ERROR_MESSAGE);
        assert!(!serde_json::to_string(&normalized)
            .expect("normalized public error JSON")
            .contains("private sentinel"));

        let oversized_message = "m".repeat(MAX_PUBLIC_ERROR_MESSAGE_BYTES + 1);
        serde_json::from_value::<PublicError>(serde_json::json!({
            "code": "OversizedMessage",
            "message": oversized_message,
        }))
        .expect_err("oversized public error message must be rejected");
    }

    /// The minimum wire form and one process attribution decode exactly.
    #[test]
    fn public_error_wire_forms_round_trip_their_typed_owner() {
        for wire in [MINIMUM_PUBLIC_ERROR_WIRE, PROCESS_FAULT_PUBLIC_ERROR_WIRE] {
            let error: PublicError =
                serde_json::from_str(wire).expect("decode public error wire form");
            assert_eq!(
                serde_json::to_value(&error).expect("public error JSON"),
                serde_json::from_str::<serde_json::Value>(wire).expect("public error wire JSON")
            );
        }
        let process: PublicError = serde_json::from_str(PROCESS_FAULT_PUBLIC_ERROR_WIRE)
            .expect("decode process attribution");
        assert!(matches!(
            process.runtime_fault().map(|fault| &fault.subject),
            Some(PublicRuntimeFaultSubject::Process { .. })
        ));
    }

    /// The smallest complete public error wire form.
    const MINIMUM_PUBLIC_ERROR_WIRE: &str =
        r#"{"code":"AuthenticationRequired","message":"Authentication is required"}"#;

    /// One complete public error carrying a process fault attribution.
    const PROCESS_FAULT_PUBLIC_ERROR_WIRE: &str = concat!(
        r#"{"code":"StructuredRuntimeCallbackFault","#,
        r#""message":"A qualified Runtime callback failed","#,
        r#""runtime_fault":{"occurrence_id":"occurrence:sha256-jcs-v1:"#,
        "2222222222222222222222222222222222222222222222222222222222222222",
        r#"","phase":"invoke_pure","pre_fault_head":{"commit_digest":"sha256-jcs-v1:"#,
        "0000000000000000000000000000000000000000000000000000000000000000",
        r#"","run_sequence":1},"run_id":"run:sha256-jcs-v1:"#,
        "0000000000000000000000000000000000000000000000000000000000000000",
        r#"","subject":{"component_kind":"state","kind":"process","#,
        r#""semantic_contract_ref":{"content_digest":"content:sha256-v1:"#,
        "1111111111111111111111111111111111111111111111111111111111111111",
        r#"","schema_id":"schema:mfm.test.fact:1:sha256-jcs-v1:"#,
        "0000000000000000000000000000000000000000000000000000000000000000",
        r#""}}}}"#,
    );

    #[test]
    fn runtime_fault_wire_is_exact_and_omits_private_implementation_identity() {
        let run_id = RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"public runtime fault run"),
        );
        let store_scope_id =
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "7".repeat(32)))
                .expect("store scope");
        let error = PublicError::backend(
            ErrorClass::ServiceUnavailable,
            "RunStoreUnavailable",
            "The authoritative run store is unavailable",
        )
        .with_runtime_fault(PublicRuntimeFaultAttribution {
            phase: PublicRuntimeFaultPhase::LoadHistory,
            run_id: run_id.clone(),
            pre_fault_head: None,
            occurrence_id: None,
            subject: PublicRuntimeFaultSubject::Store {
                store_scope_id: store_scope_id.clone(),
                store_epoch: StoreEpoch::new(7),
            },
        });
        let value = serde_json::to_value(&error).expect("public Runtime fault JSON");
        assert_eq!(
            value,
            serde_json::json!({
                "code": "RunStoreUnavailable",
                "message": "The authoritative run store is unavailable",
                "runtime_fault": {
                    "phase": "load_history",
                    "run_id": run_id,
                    "pre_fault_head": null,
                    "occurrence_id": null,
                    "subject": {
                        "kind": "store",
                        "store_scope_id": store_scope_id,
                        "store_epoch": "7",
                    },
                },
            })
        );
        let encoded = serde_json::to_string(&error).expect("public Runtime fault bytes");
        assert!(!encoded.contains("implementation"));
        assert!(!encoded.contains("diagnostic"));

        let mut hostile = value;
        hostile["runtime_fault"]["subject"]["implementation_contract_ref"] =
            serde_json::Value::String("private".to_owned());
        serde_json::from_value::<PublicError>(hostile)
            .expect_err("private implementation identity is not a public wire field");
    }
}
