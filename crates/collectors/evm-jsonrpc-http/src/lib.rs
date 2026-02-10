//! EVM JSON-RPC over HTTP live transport (Milestone 2).
//!
//! This crate implements a `LiveIoTransportFactory` for the `namespace = "evm"` IO surface.
//! It is intended to be plugged into the runtime via `LiveIoTransportFactory` routing.
//!
//! Security notes:
//! - RPC URL and authorization headers are runtime configuration and MUST NOT be persisted.
//! - Errors MUST NOT include request payloads, response bodies, or authorization values.

use std::time::Duration;

use async_trait::async_trait;

use mfm_collectors_evm::JsonRpcCall;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};

const CODE_EVM_RPC_URL_MISSING: &str = "evm_rpc_url_missing";
const CODE_EVM_REQUEST_INVALID: &str = "evm_request_invalid";
const CODE_EVM_HTTP_REQUEST_FAILED: &str = "evm_http_request_failed";
const CODE_EVM_HTTP_STATUS: &str = "evm_http_status";
const CODE_EVM_HTTP_BODY_READ_FAILED: &str = "evm_http_body_read_failed";
const CODE_EVM_RESPONSE_INVALID_JSON: &str = "evm_response_invalid_json";
const CODE_EVM_JSONRPC_ERROR: &str = "evm_jsonrpc_error";
const CODE_EVM_JSONRPC_MISSING_RESULT: &str = "evm_jsonrpc_missing_result";
const CODE_EVM_JSONRPC_INVALID_RESPONSE: &str = "evm_jsonrpc_invalid_response";
const CODE_EVM_RATE_LIMITED: &str = "evm_rate_limited";

fn info(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: &'static str,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details: None,
    }
}

#[derive(Clone)]
pub struct EvmJsonRpcHttpConfig {
    pub rpc_url: Option<String>,
    pub authorization: Option<String>,
    pub timeout: Duration,
}

impl Default for EvmJsonRpcHttpConfig {
    fn default() -> Self {
        Self {
            rpc_url: None,
            authorization: None,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone)]
pub struct EvmJsonRpcHttpTransportFactory {
    cfg: EvmJsonRpcHttpConfig,
    client: reqwest::Client,
}

impl EvmJsonRpcHttpTransportFactory {
    pub fn new(cfg: EvmJsonRpcHttpConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .build()
            .expect("reqwest client must build");
        Self { cfg, client }
    }
}

impl LiveIoTransportFactory for EvmJsonRpcHttpTransportFactory {
    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(EvmJsonRpcHttpTransport {
            cfg: self.cfg.clone(),
            client: self.client.clone(),
            next_id: 1,
        })
    }
}

struct EvmJsonRpcHttpTransport {
    cfg: EvmJsonRpcHttpConfig,
    client: reqwest::Client,
    next_id: u64,
}

#[async_trait]
impl LiveIoTransport for EvmJsonRpcHttpTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let Some(url) = &self.cfg.rpc_url else {
            return Err(IoError::Transport(info(
                CODE_EVM_RPC_URL_MISSING,
                ErrorCategory::Rpc,
                false,
                "evm rpc url is not configured",
            )));
        };

        let req: JsonRpcCall = serde_json::from_value(call.request).map_err(|_| {
            IoError::Other(info(
                CODE_EVM_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                false,
                "invalid evm jsonrpc request",
            ))
        })?;

        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": req.method,
            "params": req.params,
        });

        let mut rb = self.client.post(url).json(&body);
        if let Some(auth) = &self.cfg.authorization {
            rb = rb.header(reqwest::header::AUTHORIZATION, auth);
        }

        let resp = rb.send().await.map_err(|_| {
            IoError::Transport(info(
                CODE_EVM_HTTP_REQUEST_FAILED,
                ErrorCategory::Rpc,
                true,
                "evm http request failed",
            ))
        })?;

        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(IoError::RateLimited(info(
                CODE_EVM_RATE_LIMITED,
                ErrorCategory::Rpc,
                true,
                "evm rpc rate limited",
            )));
        }
        if !status.is_success() {
            return Err(IoError::Transport(info(
                CODE_EVM_HTTP_STATUS,
                ErrorCategory::Rpc,
                true,
                "evm rpc returned non-success http status",
            )));
        }

        let bytes = resp.bytes().await.map_err(|_| {
            IoError::Transport(info(
                CODE_EVM_HTTP_BODY_READ_FAILED,
                ErrorCategory::Rpc,
                true,
                "failed to read evm rpc response body",
            ))
        })?;

        let v = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
            IoError::Other(info(
                CODE_EVM_RESPONSE_INVALID_JSON,
                ErrorCategory::ParsingInput,
                false,
                "evm rpc response was not valid json",
            ))
        })?;

        let obj = v.as_object().ok_or_else(|| {
            IoError::Other(info(
                CODE_EVM_JSONRPC_INVALID_RESPONSE,
                ErrorCategory::ParsingInput,
                false,
                "evm jsonrpc response was not an object",
            ))
        })?;

        if obj.contains_key("error") {
            return Err(IoError::Transport(info(
                CODE_EVM_JSONRPC_ERROR,
                ErrorCategory::Rpc,
                true,
                "evm jsonrpc returned an error",
            )));
        }

        let Some(result) = obj.get("result") else {
            return Err(IoError::Other(info(
                CODE_EVM_JSONRPC_MISSING_RESULT,
                ErrorCategory::ParsingInput,
                false,
                "evm jsonrpc response missing result",
            )));
        };

        Ok(result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::engine::Stores;
    use mfm_machine::errors::StorageError;
    use mfm_machine::events::EventEnvelope;
    use mfm_machine::ids::{ArtifactId, RunId, StateId};
    use mfm_machine::live_io::LiveIoEnv;
    use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};
    use std::sync::Arc;

    #[derive(Clone)]
    struct NoopEventStore;

    #[async_trait]
    impl EventStore for NoopEventStore {
        async fn head_seq(&self, _run_id: RunId) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn append(
            &self,
            _run_id: RunId,
            _expected_seq: u64,
            _events: Vec<EventEnvelope>,
        ) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn read_range(
            &self,
            _run_id: RunId,
            _from_seq: u64,
            _to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, StorageError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone)]
    struct NoopArtifactStore;

    #[async_trait]
    impl ArtifactStore for NoopArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            Ok(ArtifactId("0".repeat(64)))
        }

        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(false)
        }
    }

    fn env() -> LiveIoEnv {
        LiveIoEnv {
            stores: Stores {
                events: Arc::new(NoopEventStore),
                artifacts: Arc::new(NoopArtifactStore),
            },
            run_id: serde_json::from_str::<RunId>("\"00000000-0000-0000-0000-000000000000\"")
                .expect("valid RunId"),
            state_id: StateId("machine.main.s1".to_string()),
            attempt: 0,
        }
    }

    #[tokio::test]
    async fn transport_missing_rpc_url_is_stable_error() {
        let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
            rpc_url: None,
            authorization: None,
            timeout: Duration::from_secs(1),
        });
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "evm".to_string(),
                request: serde_json::json!({"method": "eth_chainId", "params": []}),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Transport(info) => assert_eq!(info.code.0, CODE_EVM_RPC_URL_MISSING),
            other => panic!("expected Transport, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn transport_rejects_invalid_request_shape() {
        let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
            rpc_url: Some("http://localhost/".to_string()),
            authorization: None,
            timeout: Duration::from_secs(1),
        });
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "evm".to_string(),
                request: serde_json::json!("not an object"),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EVM_REQUEST_INVALID),
            other => panic!("expected Other, got: {other:?}"),
        }
    }
}
