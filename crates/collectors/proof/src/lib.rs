#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Typed proof collectors.
//!
//! This crate defines typed adapters over the generic `IoCall` surface for proof flows.
//! It intentionally does NOT perform IO itself.
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

/// Canonical namespace group used for proof IO calls.
pub const NAMESPACE_PROOF: &str = "proof";
/// Namespace used for proof fact reads.
pub const NAMESPACE_PROOF_READ: &str = "proof.read";
/// Namespace used for proof side effects.
pub const NAMESPACE_PROOF_SIDE_EFFECT: &str = "proof.side_effect";

/// Typed request for `proof.read`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofReadRequest {}

/// Typed response for `proof.read`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofReadResponse {
    /// Static proof payload field used by the acceptance tests.
    pub n: u64,
}

/// Typed request for `proof.side_effect`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofSideEffectRequest {
    /// Idempotency key to bind the side effect.
    pub idempotency_key: String,
}

/// Typed response for `proof.side_effect`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofSideEffectResponse {
    /// Deterministic fake transaction hash used by proof flows.
    pub tx_hash: String,
}

/// Error returned when a fact key cannot be derived from a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactKeyDerivationError {
    /// The request could not be converted into JSON before hashing.
    Serialization(String),
    /// The request could not be canonically hashed for fact recording.
    NotCanonical(CanonicalJsonError),
}

impl std::fmt::Display for FactKeyDerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactKeyDerivationError::Serialization(err) => {
                write!(f, "request could not be serialized to json: {err}")
            }
            FactKeyDerivationError::NotCanonical(err) => write!(f, "request not canonical: {err}"),
        }
    }
}

impl std::error::Error for FactKeyDerivationError {}

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn io_other(code: &'static str, category: ErrorCategory, message: &'static str) -> IoError {
    IoError::Other(info(code, category, message))
}

fn request_to_value<Request: Serialize>(
    request: &Request,
) -> Result<serde_json::Value, FactKeyDerivationError> {
    serde_json::to_value(request)
        .map_err(|err| FactKeyDerivationError::Serialization(err.to_string()))
}

fn fact_key_for_request_value(
    state_id: &StateId,
    purpose: &str,
    request: &serde_json::Value,
) -> Result<FactKey, FactKeyDerivationError> {
    let req_id = artifact_id_for_json(request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:proof|state:{}|purpose:{purpose}|req:{}",
        state_id.as_str(),
        req_id.0
    )))
}

/// Derives a deterministic fact key for a proof request.
pub fn fact_key_for_request<Request: Serialize>(
    state_id: &StateId,
    purpose: &str,
    request: &Request,
) -> Result<FactKey, FactKeyDerivationError> {
    let request = request_to_value(request)?;
    fact_key_for_request_value(state_id, purpose, &request)
}

fn proof_io_call(namespace: &'static str, request: serde_json::Value, fact_key: FactKey) -> IoCall {
    IoCall {
        namespace: namespace.to_string(),
        request,
        fact_key: Some(fact_key),
    }
}

/// First-class proof client wrapper over `IoProvider`.
pub struct ProofIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> ProofIoClient<'a> {
    /// Creates a new client for the given state and IO provider.
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    async fn call<Response, Request>(
        &mut self,
        namespace: &'static str,
        purpose: &str,
        request: Request,
    ) -> Result<Response, IoError>
    where
        Response: for<'de> Deserialize<'de>,
        Request: Serialize,
    {
        let request = request_to_value(&request).map_err(|err| match err {
            FactKeyDerivationError::Serialization(_) => io_other(
                "proof_request_serialize_failed",
                ErrorCategory::ParsingInput,
                "proof request could not be serialized to json",
            ),
            FactKeyDerivationError::NotCanonical(_) => {
                unreachable!("request_to_value only returns serialization errors")
            }
        })?;
        let fact_key = fact_key_for_request_value(&self.state_id, purpose, &request).map_err(
            |err| match err {
                FactKeyDerivationError::Serialization(_) => unreachable!(
                    "fact_key_for_request_value only hashes already-serialized json values"
                ),
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => {
                    io_other(
                        "proof_request_not_canonical",
                        ErrorCategory::ParsingInput,
                        "proof request was not canonical-json-hashable (floats are forbidden)",
                    )
                }
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                    io_other(
                        "secrets_detected",
                        ErrorCategory::Unknown,
                        "proof request contained secrets",
                    )
                }
            },
        )?;

        let result = self
            .io
            .call(proof_io_call(namespace, request, fact_key))
            .await?;
        serde_json::from_value(result.response).map_err(|_| {
            io_other(
                "proof_response_invalid",
                ErrorCategory::Unknown,
                "proof response payload had an unexpected shape",
            )
        })
    }

    /// Reads the proof fact payload.
    pub async fn read(
        &mut self,
        purpose: &str,
        request: ProofReadRequest,
    ) -> Result<ProofReadResponse, IoError> {
        self.call(NAMESPACE_PROOF_READ, purpose, request).await
    }

    /// Applies the proof side effect.
    pub async fn apply_side_effect(
        &mut self,
        purpose: &str,
        request: ProofSideEffectRequest,
    ) -> Result<ProofSideEffectResponse, IoError> {
        self.call(NAMESPACE_PROOF_SIDE_EFFECT, purpose, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::ids::ArtifactId;
    use std::collections::BTreeMap;

    struct PanicIo;

    #[async_trait]
    impl IoProvider for PanicIo {
        async fn call(&mut self, _call: IoCall) -> Result<mfm_machine::io::IoResult, IoError> {
            panic!("proof client should fail before reaching the io provider")
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            Ok(ArtifactId("0".repeat(64)))
        }

        async fn get_recorded_fact(
            &mut self,
            _key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
            Ok(None)
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(0)
        }

        async fn random_bytes(&mut self, _n: usize) -> Result<Vec<u8>, IoError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn fact_key_for_request_reports_serialization_failure() {
        let mut request = BTreeMap::new();
        request.insert((1u8, 2u8), 3u8);

        let err = fact_key_for_request(
            &StateId::must_new("proof.tests.fact_key".to_string()),
            "proof.read",
            &request,
        )
        .expect_err("tuple-key map should not serialize to json objects");

        match err {
            FactKeyDerivationError::Serialization(message) => {
                assert!(
                    !message.is_empty(),
                    "serialization error should include context"
                );
            }
            other => panic!("unexpected fact-key derivation error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn proof_client_returns_typed_error_on_request_serialization_failure() {
        let mut io = PanicIo;
        let mut client =
            ProofIoClient::new(StateId::must_new("proof.tests.client".to_string()), &mut io);
        let mut request = BTreeMap::new();
        request.insert((1u8, 2u8), 3u8);

        let err = client
            .call::<ProofReadResponse, _>(NAMESPACE_PROOF_READ, "proof.read", request)
            .await
            .expect_err("bad json serialization should be surfaced as an io error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, "proof_request_serialize_failed"),
            other => panic!("unexpected io error: {other:?}"),
        }
    }
}
