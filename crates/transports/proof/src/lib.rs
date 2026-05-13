#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Minimal proof transport used by proof-oriented examples and tests.
//!
//! This transport keeps the `proof` namespace group wired into Live IO without introducing
//! external dependencies.
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::live_io::LiveIoTransportFactory;
//! use mfm_transports_proof::ProofIoTransportFactory;
//!
//! let factory = ProofIoTransportFactory;
//! assert_eq!(factory.namespace_group(), "proof");
//! ```
#![warn(missing_docs)]

use async_trait::async_trait;

use mfm_collectors_proof::{
    ProofReadRequest, ProofReadResponse, ProofSideEffectRequest, ProofSideEffectResponse,
    NAMESPACE_PROOF_READ, NAMESPACE_PROOF_SIDE_EFFECT,
};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};

fn info(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode::must_new(code),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

/// Transport factory for the `proof` namespace group.
#[derive(Clone, Default)]
pub struct ProofIoTransportFactory;

impl LiveIoTransportFactory for ProofIoTransportFactory {
    fn namespace_group(&self) -> &str {
        "proof"
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(ProofIoTransport)
    }
}

struct ProofIoTransport;

#[async_trait]
impl LiveIoTransport for ProofIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            NAMESPACE_PROOF_READ => {
                let _: ProofReadRequest = parse_request(call.request)?;
                encode_response(ProofReadResponse { n: 1 })
            }
            NAMESPACE_PROOF_SIDE_EFFECT => {
                let req: ProofSideEffectRequest = parse_request(call.request)?;
                let prefix: String = req.idempotency_key.chars().take(8).collect();
                encode_response(ProofSideEffectResponse {
                    tx_hash: format!("0x{prefix}"),
                })
            }
            other => Err(IoError::Other(info(
                "unknown_namespace",
                ErrorCategory::Unknown,
                format!("unknown namespace: {other}"),
            ))),
        }
    }
}

fn parse_request<T: serde::de::DeserializeOwned>(request: serde_json::Value) -> Result<T, IoError> {
    serde_json::from_value(request).map_err(|_| {
        IoError::Other(info(
            "invalid_proof_request",
            ErrorCategory::ParsingInput,
            "invalid proof io request payload",
        ))
    })
}

fn encode_response<T: serde::Serialize>(value: T) -> Result<serde_json::Value, IoError> {
    serde_json::to_value(value).map_err(|_| {
        IoError::Other(info(
            "proof_response_serialize_failed",
            ErrorCategory::Unknown,
            "failed to serialize proof io response payload",
        ))
    })
}
