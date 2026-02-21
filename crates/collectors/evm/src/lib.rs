//! EVM collectors.
//!
//! This crate defines typed adapters over the generic `IoCall` surface for EVM JSON-RPC reads.
//! It intentionally does NOT perform IO itself.

use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider, IoResult};

pub const NAMESPACE_EVM: &str = "evm";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcCall {
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcCall {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactKeyDerivationError {
    NotCanonical(CanonicalJsonError),
}

impl std::fmt::Display for FactKeyDerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactKeyDerivationError::NotCanonical(e) => write!(f, "request not canonical: {e}"),
        }
    }
}

impl std::error::Error for FactKeyDerivationError {}

pub fn fact_key_for_jsonrpc_call(
    state_id: &StateId,
    call: &JsonRpcCall,
) -> Result<FactKey, FactKeyDerivationError> {
    let request =
        serde_json::to_value(call).expect("JsonRpcCall must be serializable to serde_json::Value");
    let req_id = artifact_id_for_json(&request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:evm|state:{}|req:{}",
        state_id.as_str(),
        req_id.0
    )))
}

pub fn evm_io_call(call: JsonRpcCall, fact_key: FactKey) -> IoCall {
    let request =
        serde_json::to_value(call).expect("JsonRpcCall must be serializable to serde_json::Value");
    IoCall {
        namespace: NAMESPACE_EVM.to_string(),
        request,
        fact_key: Some(fact_key),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseHexError {
    MissingPrefix,
    Empty,
    Invalid,
    Overflow,
}

impl std::fmt::Display for ParseHexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ParseHexError::MissingPrefix => "missing 0x prefix",
            ParseHexError::Empty => "empty hex string",
            ParseHexError::Invalid => "invalid hex",
            ParseHexError::Overflow => "hex value overflowed u64",
        };
        write!(f, "{s}")
    }
}

impl std::error::Error for ParseHexError {}

pub fn parse_u64_hex(s: &str) -> Result<u64, ParseHexError> {
    let Some(rest) = s.strip_prefix("0x") else {
        return Err(ParseHexError::MissingPrefix);
    };
    if rest.is_empty() {
        return Err(ParseHexError::Empty);
    }
    if rest.len() > 16 {
        return Err(ParseHexError::Overflow);
    }
    u64::from_str_radix(rest, 16).map_err(|_| ParseHexError::Invalid)
}

pub fn parse_u64_hex_value(v: &serde_json::Value) -> Result<u64, ParseHexError> {
    let Some(s) = v.as_str() else {
        return Err(ParseHexError::Invalid);
    };
    parse_u64_hex(s)
}

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

/// First-class EVM client wrapper over `IoProvider`.
///
/// States should use this instead of hand-rolling JSON-RPC requests.
pub struct EvmIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> EvmIoClient<'a> {
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    pub fn state_id(&self) -> &StateId {
        &self.state_id
    }

    pub fn io_mut(&mut self) -> &mut dyn IoProvider {
        self.io
    }

    pub async fn call(&mut self, call: JsonRpcCall) -> Result<IoResult, IoError> {
        let key = fact_key_for_jsonrpc_call(&self.state_id, &call).map_err(|e| match e {
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => io_other(
                "evm_request_not_canonical",
                ErrorCategory::ParsingInput,
                "evm request was not canonical-json-hashable (floats are forbidden)",
            ),
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                io_other(
                    "secrets_detected",
                    ErrorCategory::Unknown,
                    "evm request contained secrets (policy forbids persisting secrets)",
                )
            }
        })?;

        self.io.call(evm_io_call(call, key)).await
    }

    pub async fn chain_id_u64(&mut self) -> Result<u64, IoError> {
        let res = self
            .call(JsonRpcCall::new("eth_chainId", serde_json::json!([])))
            .await?;
        parse_u64_hex_value(&res.response).map_err(|_| {
            io_other(
                "evm_response_invalid",
                ErrorCategory::ParsingInput,
                "evm response was not a hex u64",
            )
        })
    }

    pub async fn block_number_u64(&mut self) -> Result<u64, IoError> {
        let res = self
            .call(JsonRpcCall::new("eth_blockNumber", serde_json::json!([])))
            .await?;
        parse_u64_hex_value(&res.response).map_err(|_| {
            io_other(
                "evm_response_invalid",
                ErrorCategory::ParsingInput,
                "evm response was not a hex u64",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mfm_machine::ids::{ArtifactId, FactKey};

    #[test]
    fn fact_key_is_stable_for_same_call() {
        let sid = StateId::must_new("m.main.chain_id".to_string());
        let call = JsonRpcCall::new("eth_chainId", serde_json::json!([]));

        let k1 = fact_key_for_jsonrpc_call(&sid, &call).expect("key");
        let k2 = fact_key_for_jsonrpc_call(&sid, &call).expect("key");
        assert_eq!(k1, k2);
    }

    #[test]
    fn parse_u64_hex_basic() {
        assert_eq!(parse_u64_hex("0x0").unwrap(), 0);
        assert_eq!(parse_u64_hex("0x1").unwrap(), 1);
        assert_eq!(parse_u64_hex("0x7b").unwrap(), 123);
    }

    #[test]
    fn parse_u64_hex_rejects_bad_inputs() {
        assert_eq!(
            parse_u64_hex("1").unwrap_err(),
            ParseHexError::MissingPrefix
        );
        assert_eq!(parse_u64_hex("0x").unwrap_err(), ParseHexError::Empty);
        assert_eq!(parse_u64_hex("0xzz").unwrap_err(), ParseHexError::Invalid);
        assert_eq!(
            parse_u64_hex("0x0123456789abcdef0").unwrap_err(),
            ParseHexError::Overflow
        );
    }

    struct FixedIo {
        response: serde_json::Value,
    }

    #[async_trait]
    impl IoProvider for FixedIo {
        async fn call(&mut self, _call: IoCall) -> Result<IoResult, IoError> {
            Ok(IoResult {
                response: self.response.clone(),
                recorded_payload_id: None,
            })
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

    #[tokio::test]
    async fn evm_client_parses_chain_id() {
        let mut io = FixedIo {
            response: serde_json::json!("0x1"),
        };
        let mut client =
            EvmIoClient::new(StateId::must_new("m.main.chain_id".to_string()), &mut io);
        let id = client.chain_id_u64().await.expect("chain id");
        assert_eq!(id, 1);
    }
}
