#![warn(missing_docs)]
//! Typed `rpc.control` adapters for managed EVM RPC calls.
//!
//! This crate defines the caller-facing request/response shapes emitted through
//! `namespace = "rpc.control"`. Canonical runtime states should depend on this
//! typed client instead of assembling raw `IoCall` payloads or selecting raw
//! transport URLs directly.
//!
//! The current slice keeps the public caller contract small:
//! - managed EVM JSON-RPC calls with explicit network context
//! - explicit control-plane source preparation for setup/probe/rank preflight
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_rpc_control::JsonRpcCall;
//!
//! let call = JsonRpcCall::for_network(
//!     "ethereum-mainnet",
//!     "eth_chainId",
//!     serde_json::json!([]),
//! );
//!
//! assert_eq!(call.network_id.as_deref(), Some("ethereum-mainnet"));
//! assert_eq!(call.control_scope, "shared");
//! ```

use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider, IoResult};

/// Canonical namespace used for managed RPC control-plane `IoCall`s.
pub const NAMESPACE_RPC_CONTROL: &str = "rpc.control";
/// Default shared control-plane scope used by production callers unless they opt into an isolated scope.
pub const DEFAULT_CONTROL_SCOPE: &str = "shared";

fn default_control_scope() -> String {
    DEFAULT_CONTROL_SCOPE.to_string()
}

/// Managed EVM JSON-RPC request shape emitted through `rpc.control`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcCall {
    /// Stable control-plane scope used to isolate managed source state.
    #[serde(default = "default_control_scope")]
    pub control_scope: String,
    /// Required stable network identifier.
    ///
    /// Callers must set this before deriving fact keys or dispatching the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_id: Option<String>,
    /// JSON-RPC method name such as `eth_call`.
    pub method: String,
    /// JSON-RPC params array or object payload.
    pub params: serde_json::Value,
}

impl JsonRpcCall {
    /// Creates a new managed EVM JSON-RPC request.
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            control_scope: default_control_scope(),
            network_id: None,
            method: method.into(),
            params,
        }
    }

    /// Creates a new managed EVM JSON-RPC request for `network_id` in the default shared scope.
    pub fn for_network(
        network_id: impl Into<String>,
        method: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        Self::for_scope_and_network(DEFAULT_CONTROL_SCOPE, network_id, method, params)
    }

    /// Creates a new managed EVM JSON-RPC request for `network_id` in `control_scope`.
    pub fn for_scope_and_network(
        control_scope: impl Into<String>,
        network_id: impl Into<String>,
        method: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        Self::new(method, params)
            .with_control_scope(control_scope)
            .with_network_id(network_id)
    }

    /// Returns a copy of the request scoped to `control_scope`.
    pub fn with_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        self.control_scope = control_scope.into();
        self
    }

    /// Returns a copy of the request scoped to `network_id`.
    pub fn with_network_id(mut self, network_id: impl Into<String>) -> Self {
        self.network_id = Some(network_id.into());
        self
    }
}

/// Request emitted through `rpc.control`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RpcControlRequest {
    /// Managed EVM JSON-RPC call.
    EvmCall {
        /// Embedded managed JSON-RPC call.
        #[serde(flatten)]
        call: JsonRpcCall,
    },
    /// Idempotent source setup/probe/rank preflight for one network.
    PrepareSources {
        /// Stable control-plane scope used to isolate managed source state.
        #[serde(default = "default_control_scope")]
        control_scope: String,
        /// Stable network identifier whose configured sources should be synced,
        /// probed, and ranked.
        network_id: String,
    },
    /// Bitcoin chain anchor request (block height + hash).
    BitcoinAnchor {
        /// Stable control-plane scope.
        #[serde(default = "default_control_scope")]
        control_scope: String,
        /// Stable Bitcoin network identifier (e.g. `"bitcoin-mainnet"`).
        network_id: String,
    },
    /// Bitcoin UTXO balance scan for a single address.
    BitcoinScanUtxos {
        /// Stable control-plane scope.
        #[serde(default = "default_control_scope")]
        control_scope: String,
        /// Stable Bitcoin network identifier.
        network_id: String,
        /// Bitcoin address to scan.
        address: String,
        /// Block height at which to anchor the scan.
        height: u64,
        /// Block hash at which to anchor the scan.
        block_hash: String,
    },
}

/// Summary returned for one prepared source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedSourceSummary {
    /// Stable source identifier.
    pub source_id: String,
    /// Whether the source is currently considered healthy by the control plane.
    pub healthy: bool,
    /// Current `eth_getProof` capability bit when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_get_proof: Option<bool>,
    /// Cooldown deadline in milliseconds since the Unix epoch when the source is
    /// cooling down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_ms: Option<u64>,
    /// Stable diagnostic code from the most recent failure when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
}

/// Response returned after preparing one network's control-plane source pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareSourcesResponse {
    /// Stable control-plane scope.
    #[serde(default = "default_control_scope")]
    pub control_scope: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Stable pool kind used by the current control-plane slice.
    pub pool_kind: String,
    /// Declared sources available for the network after setup sync.
    pub available_source_ids: Vec<String>,
    /// Durable ranked order produced by the control plane.
    pub ranked_source_ids: Vec<String>,
    /// Per-source health summary used to explain ranking decisions.
    #[serde(default)]
    pub sources: Vec<PreparedSourceSummary>,
}

/// Error returned when a fact key cannot be derived from a request payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactKeyDerivationError {
    /// The request could not be canonically hashed for fact recording.
    NotCanonical(CanonicalJsonError),
}

impl std::fmt::Display for FactKeyDerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

fn missing_network_id_error() -> IoError {
    io_other(
        "rpc_control_network_required",
        ErrorCategory::ParsingInput,
        "rpc.control managed requests require a non-empty network_id",
    )
}

fn request_json(request: &RpcControlRequest) -> serde_json::Value {
    serde_json::to_value(request).expect("rpc.control request must serialize")
}

/// Derives a deterministic fact key for a `rpc.control` request.
pub fn fact_key_for_request(
    state_id: &StateId,
    request: &RpcControlRequest,
) -> Result<FactKey, FactKeyDerivationError> {
    let req_id = artifact_id_for_json(&request_json(request))
        .map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:rpc.control|state:{}|req:{}",
        state_id.as_str(),
        req_id.as_str()
    )))
}

/// Wraps a typed `rpc.control` request in the generic `IoCall` envelope.
pub fn rpc_control_io_call(request: RpcControlRequest, fact_key: FactKey) -> IoCall {
    IoCall {
        namespace: NAMESPACE_RPC_CONTROL.to_string(),
        request: request_json(&request),
        fact_key: Some(fact_key),
    }
}

/// Error returned while parsing hex-encoded numeric responses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseHexError {
    /// The string did not start with `0x`.
    MissingPrefix,
    /// The string was `0x` with no digits.
    Empty,
    /// The value was not valid hexadecimal.
    Invalid,
    /// The value could not fit in `u64`.
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

/// Parses a `0x`-prefixed quantity string into `u64`.
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

/// Parses a JSON value that should contain a `0x`-prefixed quantity string.
pub fn parse_u64_hex_value(v: &serde_json::Value) -> Result<u64, ParseHexError> {
    let Some(s) = v.as_str() else {
        return Err(ParseHexError::Invalid);
    };
    parse_u64_hex(s)
}

/// First-class `rpc.control` client wrapper over `IoProvider`.
///
/// Canonical runtime states should use this client for managed EVM RPC behavior.
pub struct EvmIoClient<'a> {
    state_id: StateId,
    default_control_scope: String,
    io: &'a mut dyn IoProvider,
}

impl<'a> EvmIoClient<'a> {
    /// Creates a new client for the given state and IO provider.
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self {
            state_id,
            default_control_scope: default_control_scope(),
            io,
        }
    }

    /// Overrides the default control scope stamped into requests before hashing.
    pub fn with_default_control_scope(mut self, control_scope: impl Into<String>) -> Self {
        let control_scope = control_scope.into();
        self.default_control_scope = if control_scope.trim().is_empty() {
            default_control_scope()
        } else {
            control_scope
        };
        self
    }

    /// Returns the state identifier used when deriving fact keys.
    pub fn state_id(&self) -> &StateId {
        &self.state_id
    }

    /// Returns the underlying IO provider.
    pub fn io_mut(&mut self) -> &mut dyn IoProvider {
        self.io
    }

    fn stamp_control_scope(&self, mut call: JsonRpcCall) -> JsonRpcCall {
        if call.control_scope.trim().is_empty() {
            call.control_scope = self.default_control_scope.clone();
        }
        call
    }

    fn validate_call(&self, call: &JsonRpcCall) -> Result<(), IoError> {
        match call.network_id.as_deref() {
            Some(network_id) if !network_id.trim().is_empty() => Ok(()),
            _ => Err(missing_network_id_error()),
        }
    }

    /// Executes a managed JSON-RPC call through the generic IO provider with an
    /// explicit fact key.
    pub async fn call_with_fact_key(
        &mut self,
        call: JsonRpcCall,
        fact_key: FactKey,
    ) -> Result<IoResult, IoError> {
        let call = self.stamp_control_scope(call);
        self.validate_call(&call)?;
        self.io
            .call(rpc_control_io_call(
                RpcControlRequest::EvmCall { call },
                fact_key,
            ))
            .await
    }

    /// Executes a managed JSON-RPC call through the generic IO provider.
    pub async fn call(&mut self, call: JsonRpcCall) -> Result<IoResult, IoError> {
        let call = self.stamp_control_scope(call);
        self.validate_call(&call)?;
        let request = RpcControlRequest::EvmCall { call: call.clone() };
        let key = fact_key_for_request(&self.state_id, &request).map_err(|err| match err {
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => io_other(
                "rpc_control_request_not_canonical",
                ErrorCategory::ParsingInput,
                "rpc.control request was not canonical-json-hashable (floats are forbidden)",
            ),
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                io_other(
                    "secrets_detected",
                    ErrorCategory::Unknown,
                    "rpc.control request contained secrets (policy forbids persisting secrets)",
                )
            }
        })?;
        self.call_with_fact_key(call, key).await
    }

    /// Executes idempotent source setup/probe/rank preflight for one network.
    pub async fn prepare_sources(
        &mut self,
        network_id: impl Into<String>,
    ) -> Result<PrepareSourcesResponse, IoError> {
        self.prepare_sources_in_scope(self.default_control_scope.clone(), network_id)
            .await
    }

    /// Executes idempotent source setup/probe/rank preflight for one network within `control_scope`.
    pub async fn prepare_sources_in_scope(
        &mut self,
        control_scope: impl Into<String>,
        network_id: impl Into<String>,
    ) -> Result<PrepareSourcesResponse, IoError> {
        let control_scope = control_scope.into();
        let network_id = network_id.into();
        if network_id.trim().is_empty() {
            return Err(missing_network_id_error());
        }
        let request = RpcControlRequest::PrepareSources {
            control_scope: if control_scope.trim().is_empty() {
                self.default_control_scope.clone()
            } else {
                control_scope
            },
            network_id,
        };
        let key = fact_key_for_request(&self.state_id, &request).map_err(|err| match err {
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => io_other(
                "rpc_control_request_not_canonical",
                ErrorCategory::ParsingInput,
                "rpc.control request was not canonical-json-hashable (floats are forbidden)",
            ),
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                io_other(
                    "secrets_detected",
                    ErrorCategory::Unknown,
                    "rpc.control request contained secrets (policy forbids persisting secrets)",
                )
            }
        })?;
        let result = self.io.call(rpc_control_io_call(request, key)).await?;
        serde_json::from_value(result.response).map_err(|_| {
            io_other(
                "rpc_control_response_invalid",
                ErrorCategory::ParsingInput,
                "rpc.control prepare_sources response was invalid",
            )
        })
    }

    /// Fetches the remote chain ID for `network_id` within `control_scope` and parses it as `u64`.
    pub async fn chain_id_u64(
        &mut self,
        network_id: &str,
        control_scope: &str,
    ) -> Result<u64, IoError> {
        let res = self
            .call(JsonRpcCall::for_scope_and_network(
                control_scope,
                network_id,
                "eth_chainId",
                serde_json::json!([]),
            ))
            .await?;
        parse_u64_hex_value(&res.response).map_err(|_| {
            io_other(
                "evm_response_invalid",
                ErrorCategory::ParsingInput,
                "evm response was not a hex u64",
            )
        })
    }

    /// Fetches the latest block number for `network_id` within `control_scope` and parses it as `u64`.
    pub async fn block_number_u64(
        &mut self,
        network_id: &str,
        control_scope: &str,
    ) -> Result<u64, IoError> {
        let res = self
            .call(JsonRpcCall::for_scope_and_network(
                control_scope,
                network_id,
                "eth_blockNumber",
                serde_json::json!([]),
            ))
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
    fn fact_key_is_stable_for_same_request() {
        let sid = StateId::must_new("m.main.chain_id".to_string());
        let request = RpcControlRequest::EvmCall {
            call: JsonRpcCall::for_network(
                "ethereum-mainnet",
                "eth_chainId",
                serde_json::json!([]),
            ),
        };

        let k1 = fact_key_for_request(&sid, &request).expect("key");
        let k2 = fact_key_for_request(&sid, &request).expect("key");
        assert_eq!(k1, k2);
    }

    #[test]
    fn fact_key_differs_across_control_scope() {
        let sid = StateId::must_new("m.main.chain_id".to_string());
        let left = RpcControlRequest::EvmCall {
            call: JsonRpcCall::for_scope_and_network(
                "shared",
                "ethereum-mainnet",
                "eth_chainId",
                serde_json::json!([]),
            ),
        };
        let right = RpcControlRequest::EvmCall {
            call: JsonRpcCall::for_scope_and_network(
                "isolated",
                "ethereum-mainnet",
                "eth_chainId",
                serde_json::json!([]),
            ),
        };

        let left = fact_key_for_request(&sid, &left).expect("left key");
        let right = fact_key_for_request(&sid, &right).expect("right key");
        assert_ne!(left, right);
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

    #[test]
    fn call_serializes_network_without_route() {
        let request = serde_json::to_value(RpcControlRequest::EvmCall {
            call: JsonRpcCall::for_network(
                "reth-local",
                "eth_sendRawTransaction",
                serde_json::json!(["0x01"]),
            ),
        })
        .expect("request json");

        assert_eq!(
            request,
            serde_json::json!({
                "kind": "evm_call",
                "control_scope": "shared",
                "network_id": "reth-local",
                "method": "eth_sendRawTransaction",
                "params": ["0x01"],
            })
        );
    }

    #[test]
    fn prepare_sources_request_serializes() {
        let request = serde_json::to_value(RpcControlRequest::PrepareSources {
            control_scope: "shared".to_string(),
            network_id: "ethereum-mainnet".to_string(),
        })
        .expect("request json");

        assert_eq!(
            request,
            serde_json::json!({
                "kind": "prepare_sources",
                "control_scope": "shared",
                "network_id": "ethereum-mainnet",
            })
        );
    }

    #[test]
    fn bitcoin_anchor_request_serializes() {
        let request = serde_json::to_value(RpcControlRequest::BitcoinAnchor {
            control_scope: "shared".to_string(),
            network_id: "bitcoin-mainnet".to_string(),
        })
        .expect("request json");

        assert_eq!(
            request,
            serde_json::json!({
                "kind": "bitcoin_anchor",
                "control_scope": "shared",
                "network_id": "bitcoin-mainnet",
            })
        );
    }

    #[test]
    fn bitcoin_scan_utxos_request_serializes() {
        let request = serde_json::to_value(RpcControlRequest::BitcoinScanUtxos {
            control_scope: "shared".to_string(),
            network_id: "bitcoin-mainnet".to_string(),
            address: "1BoatSLRHtKNngkdXEeobR76b53LETtpyT".to_string(),
            height: 840000,
            block_hash: "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5"
                .to_string(),
        })
        .expect("request json");

        assert_eq!(
            request,
            serde_json::json!({
                "kind": "bitcoin_scan_utxos",
                "control_scope": "shared",
                "network_id": "bitcoin-mainnet",
                "address": "1BoatSLRHtKNngkdXEeobR76b53LETtpyT",
                "height": 840000,
                "block_hash": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            })
        );
    }

    #[test]
    fn bitcoin_anchor_request_deserializes_from_adapter_shape() {
        let json = serde_json::json!({
            "kind": "bitcoin_anchor",
            "control_scope": "shared",
            "network_id": "bitcoin-mainnet"
        });
        let request: RpcControlRequest = serde_json::from_value(json).expect("deserialize");
        assert_eq!(
            request,
            RpcControlRequest::BitcoinAnchor {
                control_scope: "shared".to_string(),
                network_id: "bitcoin-mainnet".to_string(),
            }
        );
    }

    #[derive(Default)]
    struct FixedIo {
        calls: Vec<IoCall>,
    }

    #[async_trait]
    impl IoProvider for FixedIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call.clone());
            let response = match call.request.get("kind").and_then(serde_json::Value::as_str) {
                Some("prepare_sources") => serde_json::json!({
                    "control_scope": "shared",
                    "network_id": "ethereum-mainnet",
                    "pool_kind": "default",
                    "available_source_ids": ["reth_local"],
                    "ranked_source_ids": ["reth_local"],
                    "sources": [{
                        "source_id": "reth_local",
                        "healthy": true,
                    }],
                }),
                _ => serde_json::json!("0x1"),
            };
            Ok(IoResult {
                response,
                recorded_payload_id: Some(ArtifactId::must_new("1".repeat(64))),
            })
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            Ok(ArtifactId::must_new("2".repeat(64)))
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

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0_u8; n])
        }
    }

    #[tokio::test]
    async fn client_routes_calls_through_rpc_control_namespace() {
        let mut io = FixedIo::default();
        let mut client =
            EvmIoClient::new(StateId::must_new("m.main.chain_id".to_string()), &mut io);

        let result = client
            .chain_id_u64("ethereum-mainnet", DEFAULT_CONTROL_SCOPE)
            .await
            .expect("chain id");
        assert_eq!(result, 1);
        assert_eq!(io.calls.len(), 1);
        assert_eq!(io.calls[0].namespace, NAMESPACE_RPC_CONTROL);
        assert_eq!(
            io.calls[0].request,
            serde_json::json!({
                "kind": "evm_call",
                "control_scope": "shared",
                "network_id": "ethereum-mainnet",
                "method": "eth_chainId",
                "params": [],
            })
        );
    }

    #[tokio::test]
    async fn client_decodes_prepare_sources_response() {
        let mut io = FixedIo::default();
        let mut client = EvmIoClient::new(StateId::must_new("m.main.prepare".to_string()), &mut io);

        let result = client
            .prepare_sources("ethereum-mainnet")
            .await
            .expect("prepare");
        assert_eq!(result.control_scope, "shared");
        assert_eq!(result.network_id, "ethereum-mainnet");
        assert_eq!(result.ranked_source_ids, vec!["reth_local".to_string()]);
    }

    #[tokio::test]
    async fn client_stamps_default_control_scope_before_hashing() {
        let mut io = FixedIo::default();
        let mut client =
            EvmIoClient::new(StateId::must_new("m.main.chain_id".to_string()), &mut io)
                .with_default_control_scope("workspace-a");

        let result = client
            .call(
                JsonRpcCall::new("eth_chainId", serde_json::json!([]))
                    .with_control_scope("")
                    .with_network_id("ethereum-mainnet"),
            )
            .await
            .expect("call");
        assert_eq!(result.response, serde_json::json!("0x1"));
        assert_eq!(
            io.calls[0].request,
            serde_json::json!({
                "kind": "evm_call",
                "control_scope": "workspace-a",
                "network_id": "ethereum-mainnet",
                "method": "eth_chainId",
                "params": [],
            })
        );
    }

    #[tokio::test]
    async fn client_rejects_missing_network_id_before_hashing() {
        let mut io = FixedIo::default();
        let mut client =
            EvmIoClient::new(StateId::must_new("m.main.chain_id".to_string()), &mut io);

        let err = client
            .call(JsonRpcCall::new("eth_chainId", serde_json::json!([])))
            .await
            .expect_err("missing network should fail");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, "rpc_control_network_required"),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(io.calls.is_empty());
    }
}
