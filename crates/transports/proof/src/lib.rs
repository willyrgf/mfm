use async_trait::async_trait;

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};

fn info(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

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
            "proof.read" => Ok(serde_json::json!({ "n": 1 })),
            "proof.side_effect" => {
                let k = call
                    .request
                    .get("idempotency_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("missing");
                let prefix: String = k.chars().take(8).collect();
                Ok(serde_json::json!({ "tx_hash": format!("0x{prefix}") }))
            }
            other => Err(IoError::Other(info(
                "unknown_namespace",
                ErrorCategory::Unknown,
                format!("unknown namespace: {other}"),
            ))),
        }
    }
}
