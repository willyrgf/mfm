#![warn(missing_docs)]
//! Generic EVM JSON-RPC transport.
//!
//! This crate owns live EVM JSON-RPC communication and process-local source
//! routing. It implements the reusable EVM capability traits without depending
//! on workflow lifecycle crates, signer providers, artifact stores, or binaries.
//!
//! ```rust
//! use mfm_evm_capabilities::{EvmSourcePolicyId, EvmSourceRef};
//! use mfm_transports_evm::{EvmJsonRpcClient, EvmRuntimeSource, EvmSourceRegistry};
//!
//! let source = EvmRuntimeSource::new(
//!     EvmSourceRef::new("local")?,
//!     1,
//!     "http://127.0.0.1:8545",
//!     None,
//! )?;
//! let registry = EvmSourceRegistry::single_source(
//!     source,
//!     EvmSourcePolicyId::new("dev")?,
//! )?;
//! let _client = EvmJsonRpcClient::new(registry);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use alloy_primitives::B256;
use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBalanceReadResponse, EvmBlockReadProvider,
    EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector, EvmCallReadProvider,
    EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError, EvmCapabilityFuture,
    EvmChainIdentityProvider, EvmChainIdentityRequest, EvmChainIdentityResponse,
    EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse, EvmGasEstimateProvider,
    EvmGasEstimateRequest, EvmGasEstimateResponse, EvmLogEntry, EvmLogsReadProvider,
    EvmLogsReadRequest, EvmLogsReadResponse, EvmNonceReadProvider, EvmNonceReadRequest,
    EvmNonceReadResponse, EvmReceiptReadProvider, EvmReceiptReadRequest, EvmReceiptReadResponse,
    EvmSourcePolicyId, EvmSourceRef, EvmTransactionSubmitProvider, EvmTransactionSubmitRequest,
    EvmTransactionSubmitResponse, RedactedEvmSourceEvidence,
};
use mfm_evm_core::encoding::parse_u256_hex;
use mfm_evm_core::hex::{bytes_to_hex_prefixed, hex_to_bytes};
use mfm_evm_core::tx::{parse_u128_quantity, parse_u64_quantity};
use serde::Deserialize;
use serde_json::{json, Value};

/// Environment variable used by [`EvmJsonRpcClient::from_env`].
pub const MFM_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";

/// Result type for EVM transport setup.
pub type TransportResult<T> = std::result::Result<T, EvmTransportError>;

/// Runtime EVM JSON-RPC source.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmRuntimeSource {
    id: EvmSourceRef,
    expected_chain_id: u64,
    rpc_url: String,
    authorization: Option<String>,
}

impl EvmRuntimeSource {
    /// Creates a runtime EVM source.
    pub fn new(
        id: EvmSourceRef,
        expected_chain_id: u64,
        rpc_url: impl Into<String>,
        authorization: Option<String>,
    ) -> TransportResult<Self> {
        let rpc_url = rpc_url.into();
        if rpc_url.trim().is_empty() || reqwest::Url::parse(&rpc_url).is_err() {
            return Err(EvmTransportError::InvalidRegistry);
        }
        Ok(Self {
            id,
            expected_chain_id,
            rpc_url,
            authorization: authorization.filter(|value| !value.trim().is_empty()),
        })
    }

    /// Returns the source id.
    pub fn id(&self) -> &EvmSourceRef {
        &self.id
    }

    /// Returns the expected EVM chain id.
    pub const fn expected_chain_id(&self) -> u64 {
        self.expected_chain_id
    }
}

impl fmt::Debug for EvmRuntimeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmRuntimeSource")
            .field("id", &self.id)
            .field("expected_chain_id", &self.expected_chain_id)
            .field("rpc_url", &"<redacted>")
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Ordered EVM source fallback policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmSourcePolicy {
    id: EvmSourcePolicyId,
    ordered_sources: Vec<EvmSourceRef>,
}

impl EvmSourcePolicy {
    /// Creates an ordered source policy.
    pub fn new(id: EvmSourcePolicyId, ordered_sources: Vec<EvmSourceRef>) -> TransportResult<Self> {
        if ordered_sources.is_empty() {
            return Err(EvmTransportError::InvalidRegistry);
        }
        Ok(Self {
            id,
            ordered_sources,
        })
    }

    /// Returns the policy id.
    pub fn id(&self) -> &EvmSourcePolicyId {
        &self.id
    }

    /// Returns the ordered source ids.
    pub fn ordered_sources(&self) -> &[EvmSourceRef] {
        &self.ordered_sources
    }
}

/// Runtime EVM source registry.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmSourceRegistry {
    sources: BTreeMap<EvmSourceRef, EvmRuntimeSource>,
    policies: BTreeMap<EvmSourcePolicyId, EvmSourcePolicy>,
}

impl EvmSourceRegistry {
    /// Creates a registry from runtime sources and policies.
    pub fn new(
        sources: impl IntoIterator<Item = EvmRuntimeSource>,
        policies: impl IntoIterator<Item = EvmSourcePolicy>,
    ) -> TransportResult<Self> {
        let mut source_map = BTreeMap::new();
        for source in sources {
            if source_map.insert(source.id.clone(), source).is_some() {
                return Err(EvmTransportError::InvalidRegistry);
            }
        }
        let mut policy_map = BTreeMap::new();
        for policy in policies {
            for source_id in policy.ordered_sources() {
                if !source_map.contains_key(source_id) {
                    return Err(EvmTransportError::InvalidRegistry);
                }
            }
            if policy_map.insert(policy.id.clone(), policy).is_some() {
                return Err(EvmTransportError::InvalidRegistry);
            }
        }
        Ok(Self {
            sources: source_map,
            policies: policy_map,
        })
    }

    /// Creates a registry with one source and one policy.
    pub fn single_source(
        source: EvmRuntimeSource,
        policy_id: EvmSourcePolicyId,
    ) -> TransportResult<Self> {
        let source_id = source.id.clone();
        let policy = EvmSourcePolicy::new(policy_id, vec![source_id])?;
        Self::new([source], [policy])
    }

    /// Parses a runtime source registry JSON object.
    pub fn from_json_str(raw: &str) -> TransportResult<Self> {
        let config: RegistryConfig =
            serde_json::from_str(raw).map_err(|_| EvmTransportError::InvalidRegistry)?;
        let sources = config
            .sources
            .into_iter()
            .map(|source| {
                EvmRuntimeSource::new(
                    EvmSourceRef::new(source.id).map_err(|_| EvmTransportError::InvalidRegistry)?,
                    source.expected_chain_id,
                    source.rpc_url,
                    source.authorization,
                )
            })
            .collect::<TransportResult<Vec<_>>>()?;
        let policies = config
            .policies
            .into_iter()
            .map(|policy| {
                let source_ids = policy
                    .ordered_sources
                    .into_iter()
                    .map(|id| EvmSourceRef::new(id).map_err(|_| EvmTransportError::InvalidRegistry))
                    .collect::<TransportResult<Vec<_>>>()?;
                EvmSourcePolicy::new(
                    EvmSourcePolicyId::new(policy.id)
                        .map_err(|_| EvmTransportError::InvalidRegistry)?,
                    source_ids,
                )
            })
            .collect::<TransportResult<Vec<_>>>()?;
        Self::new(sources, policies)
    }

    fn candidates<'a>(
        &'a self,
        policy_id: &EvmSourcePolicyId,
        source_ref: &EvmSourceRef,
    ) -> TransportResult<Vec<&'a EvmRuntimeSource>> {
        let policy = self
            .policies
            .get(policy_id)
            .ok_or(EvmTransportError::PolicyUnavailable)?;
        let start = policy
            .ordered_sources
            .iter()
            .position(|candidate| candidate == source_ref)
            .ok_or(EvmTransportError::SourceNotAllowed)?;
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for source_id in policy.ordered_sources[start..]
            .iter()
            .chain(policy.ordered_sources[..start].iter())
        {
            if seen.insert(source_id.clone()) {
                let source = self
                    .sources
                    .get(source_id)
                    .ok_or(EvmTransportError::SourceUnavailable)?;
                candidates.push(source);
            }
        }
        Ok(candidates)
    }
}

impl fmt::Debug for EvmSourceRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmSourceRegistry")
            .field("sources", &self.sources)
            .field("policies", &self.policies)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct RegistryConfig {
    sources: Vec<SourceConfig>,
    policies: Vec<PolicyConfig>,
}

#[derive(Debug, Deserialize)]
struct SourceConfig {
    id: String,
    expected_chain_id: u64,
    rpc_url: String,
    authorization: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PolicyConfig {
    id: String,
    ordered_sources: Vec<String>,
}

/// Generic EVM JSON-RPC client.
#[derive(Clone)]
pub struct EvmJsonRpcClient {
    client: reqwest::Client,
    registry: EvmSourceRegistry,
}

impl EvmJsonRpcClient {
    /// Creates a client from a runtime source registry.
    pub fn new(registry: EvmSourceRegistry) -> Self {
        Self {
            client: reqwest::Client::builder()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            registry,
        }
    }

    /// Creates a client by parsing [`MFM_EVM_RPC_SOURCES_JSON`].
    pub fn from_env() -> TransportResult<Self> {
        let raw = std::env::var(MFM_EVM_RPC_SOURCES_JSON)
            .map_err(|_| EvmTransportError::InvalidRegistry)?;
        Ok(Self::new(EvmSourceRegistry::from_json_str(&raw)?))
    }

    async fn chain_identity_impl(
        &self,
        request: &EvmChainIdentityRequest,
    ) -> TransportResult<EvmChainIdentityResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let client_version = self
            .rpc_call(selected.source, "web3_clientVersion", json!([]))
            .await
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned));
        Ok(EvmChainIdentityResponse {
            evidence: selected.evidence,
            chain_id: selected.chain_id,
            client_version,
        })
    }

    async fn block_read_impl(
        &self,
        request: &EvmBlockReadRequest,
    ) -> TransportResult<EvmBlockReadResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let value = match &request.block {
            EvmBlockSelector::Hash(hash) => {
                self.rpc_call(
                    selected.source,
                    "eth_getBlockByHash",
                    json!([format!("{hash:?}"), false]),
                )
                .await?
            }
            selector => {
                self.rpc_call(
                    selected.source,
                    "eth_getBlockByNumber",
                    json!([block_selector_tag(selector), false]),
                )
                .await?
            }
        };
        let block_number = value
            .get("number")
            .and_then(Value::as_str)
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_u64)?;
        let block_hash = parse_b256_field(&value, "hash")?;
        Ok(EvmBlockReadResponse {
            evidence: selected.evidence,
            block_number,
            block_hash,
        })
    }

    async fn call_read_impl(
        &self,
        request: &EvmCallReadRequest,
    ) -> TransportResult<EvmCallReadResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let result = self
            .rpc_call(
                selected.source,
                "eth_call",
                json!([{
                    "to": format!("{:?}", request.to),
                    "data": bytes_to_hex_prefixed(&request.calldata),
                }, block_selector_tag(&request.block)]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmCallReadResponse {
            evidence: selected.evidence,
            return_data: hex_to_bytes(raw).map_err(|_| EvmTransportError::InvalidResponse)?,
        })
    }

    async fn balance_read_impl(
        &self,
        request: &EvmBalanceReadRequest,
    ) -> TransportResult<EvmBalanceReadResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let result = self
            .rpc_call(
                selected.source,
                "eth_getBalance",
                json!([
                    format!("{:?}", request.account),
                    block_selector_tag(&request.block)
                ]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmBalanceReadResponse {
            evidence: selected.evidence,
            balance_wei: parse_u256_hex(raw).map_err(|_| EvmTransportError::InvalidResponse)?,
        })
    }

    async fn logs_read_impl(
        &self,
        request: &EvmLogsReadRequest,
    ) -> TransportResult<EvmLogsReadResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let mut filter = serde_json::Map::new();
        filter.insert(
            "fromBlock".to_owned(),
            json!(block_selector_tag(&request.from_block)),
        );
        filter.insert(
            "toBlock".to_owned(),
            json!(block_selector_tag(&request.to_block)),
        );
        if let Some(address) = request.address {
            filter.insert("address".to_owned(), json!(format!("{address:?}")));
        }
        if !request.topics.is_empty() {
            filter.insert(
                "topics".to_owned(),
                json!(request
                    .topics
                    .iter()
                    .map(|topic| format!("{topic:?}"))
                    .collect::<Vec<_>>()),
            );
        }
        let result = self
            .rpc_call(
                selected.source,
                "eth_getLogs",
                json!([Value::Object(filter)]),
            )
            .await?;
        let values = result
            .as_array()
            .ok_or(EvmTransportError::InvalidResponse)?;
        let logs = values
            .iter()
            .map(parse_log_entry)
            .collect::<TransportResult<Vec<_>>>()?;
        Ok(EvmLogsReadResponse {
            evidence: selected.evidence,
            logs,
        })
    }

    async fn nonce_read_impl(
        &self,
        request: &EvmNonceReadRequest,
    ) -> TransportResult<EvmNonceReadResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let result = self
            .rpc_call(
                selected.source,
                "eth_getTransactionCount",
                json!([
                    format!("{:?}", request.account),
                    block_selector_tag(&request.block)
                ]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmNonceReadResponse {
            evidence: selected.evidence,
            nonce: parse_u64(raw)?,
        })
    }

    async fn fee_read_impl(
        &self,
        request: &EvmFeeReadRequest,
    ) -> TransportResult<EvmFeeReadResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let legacy_gas_price = self
            .rpc_call(selected.source, "eth_gasPrice", json!([]))
            .await?
            .as_str()
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_u128)?;
        let priority_fee_per_gas = match self
            .rpc_call(selected.source, "eth_maxPriorityFeePerGas", json!([]))
            .await
        {
            Ok(value) => Some(
                value
                    .as_str()
                    .ok_or(EvmTransportError::InvalidResponse)
                    .and_then(parse_u128)?,
            ),
            Err(EvmTransportError::RequestFailed) => None,
            Err(error) => return Err(error),
        };
        let latest_block = match self
            .rpc_call(
                selected.source,
                "eth_getBlockByNumber",
                json!(["latest", false]),
            )
            .await
        {
            Ok(value) => Some(value),
            Err(EvmTransportError::RequestFailed) => None,
            Err(error) => return Err(error),
        };
        let base_fee_per_gas = latest_block
            .as_ref()
            .and_then(|block| block.get("baseFeePerGas"))
            .and_then(Value::as_str)
            .map(parse_u128)
            .transpose()?;
        let max_fee_per_gas = base_fee_per_gas
            .zip(priority_fee_per_gas)
            .map(|(base_fee, priority_fee)| base_fee.saturating_mul(2) + priority_fee);
        Ok(EvmFeeReadResponse {
            evidence: selected.evidence,
            base_fee_per_gas,
            priority_fee_per_gas,
            max_fee_per_gas,
            legacy_gas_price: Some(legacy_gas_price),
        })
    }

    async fn gas_estimate_impl(
        &self,
        request: &EvmGasEstimateRequest,
    ) -> TransportResult<EvmGasEstimateResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let mut call = serde_json::Map::new();
        if let Some(from) = request.from {
            call.insert("from".to_owned(), json!(format!("{from:?}")));
        }
        if let Some(to) = request.to {
            call.insert("to".to_owned(), json!(format!("{to:?}")));
        }
        call.insert(
            "value".to_owned(),
            json!(format!("0x{:x}", request.value_wei)),
        );
        call.insert(
            "data".to_owned(),
            json!(bytes_to_hex_prefixed(&request.data)),
        );
        let result = self
            .rpc_call(
                selected.source,
                "eth_estimateGas",
                json!([Value::Object(call)]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmGasEstimateResponse {
            evidence: selected.evidence,
            gas_limit: parse_u64(raw)?,
        })
    }

    async fn submit_impl(
        &self,
        request: &EvmTransactionSubmitRequest,
    ) -> TransportResult<EvmTransactionSubmitResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let result = self
            .rpc_call(
                selected.source,
                "eth_sendRawTransaction",
                json!([bytes_to_hex_prefixed(request.signed_payload.bytes())]),
            )
            .await?;
        let hash = parse_b256_str(result.as_str().ok_or(EvmTransportError::InvalidResponse)?)?;
        if hash != request.signed_payload.transaction_hash() {
            return Err(EvmTransportError::InvalidResponse);
        }
        Ok(EvmTransactionSubmitResponse {
            evidence: selected.evidence,
            transaction_hash: hash,
        })
    }

    async fn receipt_read_impl(
        &self,
        request: &EvmReceiptReadRequest,
    ) -> TransportResult<EvmReceiptReadResponse> {
        let selected = self
            .verified_source(&request.policy_id, &request.source_ref)
            .await?;
        let result = self
            .rpc_call(
                selected.source,
                "eth_getTransactionReceipt",
                json!([format!("{:?}", request.transaction_hash)]),
            )
            .await?;
        if result.is_null() {
            return Err(EvmTransportError::ReceiptPending);
        }
        let transaction_hash = parse_b256_field(&result, "transactionHash")?;
        let block_number = result
            .get("blockNumber")
            .and_then(Value::as_str)
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_u64)?;
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .map(|raw| raw == "0x1")
            .unwrap_or(false);
        Ok(EvmReceiptReadResponse {
            evidence: selected.evidence,
            transaction_hash,
            block_number,
            status,
        })
    }

    async fn verified_source<'a>(
        &'a self,
        policy_id: &EvmSourcePolicyId,
        source_ref: &EvmSourceRef,
    ) -> TransportResult<VerifiedSource<'a>> {
        let mut last_failure = EvmTransportError::SourceUnavailable;
        for source in self.registry.candidates(policy_id, source_ref)? {
            match self.chain_id_for_source(source).await {
                Ok(chain_id) if chain_id == source.expected_chain_id => {
                    return Ok(VerifiedSource {
                        source,
                        chain_id,
                        evidence: RedactedEvmSourceEvidence {
                            source_ref: source.id.clone(),
                            policy_id: policy_id.clone(),
                            chain_id,
                        },
                    });
                }
                Ok(_) => return Err(EvmTransportError::ChainIdMismatch),
                Err(EvmTransportError::RequestFailed) => {
                    last_failure = EvmTransportError::RequestFailed;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_failure)
    }

    async fn chain_id_for_source(&self, source: &EvmRuntimeSource) -> TransportResult<u64> {
        let result = self.rpc_call(source, "eth_chainId", json!([])).await?;
        result
            .as_str()
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_u64)
    }

    async fn rpc_call(
        &self,
        source: &EvmRuntimeSource,
        method: &'static str,
        params: Value,
    ) -> TransportResult<Value> {
        let mut request = self.client.post(&source.rpc_url).json(&json!({
            "jsonrpc": "2.0",
            "id": 1u64,
            "method": method,
            "params": params,
        }));
        if let Some(authorization) = &source.authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        let response = request
            .send()
            .await
            .map_err(|_| EvmTransportError::RequestFailed)?;
        if !response.status().is_success() {
            return Err(EvmTransportError::RequestFailed);
        }
        let body = response
            .json::<Value>()
            .await
            .map_err(|_| EvmTransportError::InvalidResponse)?;
        if body.get("error").is_some() {
            return Err(EvmTransportError::RequestFailed);
        }
        body.get("result")
            .cloned()
            .ok_or(EvmTransportError::InvalidResponse)
    }
}

impl fmt::Debug for EvmJsonRpcClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmJsonRpcClient")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

struct VerifiedSource<'a> {
    source: &'a EvmRuntimeSource,
    chain_id: u64,
    evidence: RedactedEvmSourceEvidence,
}

macro_rules! impl_provider {
    ($trait:ident, $method:ident, $req:ty, $resp:ty, $inner:ident) => {
        impl $trait for EvmJsonRpcClient {
            fn $method<'a>(&'a self, request: &'a $req) -> EvmCapabilityFuture<'a, $resp> {
                Box::pin(async move {
                    self.$inner(request)
                        .await
                        .map_err(capability_error_from_transport)
                })
            }
        }
    };
}

impl_provider!(
    EvmChainIdentityProvider,
    chain_identity,
    EvmChainIdentityRequest,
    EvmChainIdentityResponse,
    chain_identity_impl
);
impl_provider!(
    EvmBlockReadProvider,
    read_block,
    EvmBlockReadRequest,
    EvmBlockReadResponse,
    block_read_impl
);
impl_provider!(
    EvmBalanceReadProvider,
    read_balance,
    EvmBalanceReadRequest,
    EvmBalanceReadResponse,
    balance_read_impl
);
impl_provider!(
    EvmCallReadProvider,
    read_call,
    EvmCallReadRequest,
    EvmCallReadResponse,
    call_read_impl
);
impl_provider!(
    EvmLogsReadProvider,
    read_logs,
    EvmLogsReadRequest,
    EvmLogsReadResponse,
    logs_read_impl
);
impl_provider!(
    EvmNonceReadProvider,
    read_nonce,
    EvmNonceReadRequest,
    EvmNonceReadResponse,
    nonce_read_impl
);
impl_provider!(
    EvmFeeReadProvider,
    read_fee,
    EvmFeeReadRequest,
    EvmFeeReadResponse,
    fee_read_impl
);
impl_provider!(
    EvmGasEstimateProvider,
    estimate_gas,
    EvmGasEstimateRequest,
    EvmGasEstimateResponse,
    gas_estimate_impl
);
impl_provider!(
    EvmTransactionSubmitProvider,
    submit_transaction,
    EvmTransactionSubmitRequest,
    EvmTransactionSubmitResponse,
    submit_impl
);
impl_provider!(
    EvmReceiptReadProvider,
    read_receipt,
    EvmReceiptReadRequest,
    EvmReceiptReadResponse,
    receipt_read_impl
);

fn capability_error_from_transport(error: EvmTransportError) -> EvmCapabilityError {
    match error {
        EvmTransportError::ReceiptPending => EvmCapabilityError::ReceiptPending,
        other => EvmCapabilityError::redacted_provider_failure(other),
    }
}

fn block_selector_tag(selector: &EvmBlockSelector) -> String {
    match selector {
        EvmBlockSelector::Latest => "latest".to_owned(),
        EvmBlockSelector::Number(number) => format!("0x{number:x}"),
        EvmBlockSelector::Hash(hash) => format!("{hash:?}"),
    }
}

fn parse_u64(raw: &str) -> TransportResult<u64> {
    parse_u64_quantity(raw, "evm_quantity").map_err(|_| EvmTransportError::InvalidResponse)
}

fn parse_u128(raw: &str) -> TransportResult<u128> {
    parse_u128_quantity(raw, "evm_quantity").map_err(|_| EvmTransportError::InvalidResponse)
}

fn parse_b256_field(value: &Value, field: &'static str) -> TransportResult<B256> {
    parse_b256_str(
        value
            .get(field)
            .and_then(Value::as_str)
            .ok_or(EvmTransportError::InvalidResponse)?,
    )
}

fn parse_b256_str(value: &str) -> TransportResult<B256> {
    value
        .parse::<B256>()
        .map_err(|_| EvmTransportError::InvalidResponse)
}

fn parse_log_entry(value: &Value) -> TransportResult<EvmLogEntry> {
    let address = value
        .get("address")
        .and_then(Value::as_str)
        .ok_or(EvmTransportError::InvalidResponse)?
        .parse()
        .map_err(|_| EvmTransportError::InvalidResponse)?;
    let topics = value
        .get("topics")
        .and_then(Value::as_array)
        .ok_or(EvmTransportError::InvalidResponse)?
        .iter()
        .map(|topic| parse_b256_str(topic.as_str().ok_or(EvmTransportError::InvalidResponse)?))
        .collect::<TransportResult<Vec<_>>>()?;
    let data = hex_to_bytes(
        value
            .get("data")
            .and_then(Value::as_str)
            .ok_or(EvmTransportError::InvalidResponse)?,
    )
    .map_err(|_| EvmTransportError::InvalidResponse)?;
    let block_number = value
        .get("blockNumber")
        .and_then(Value::as_str)
        .map(parse_u64)
        .transpose()?;
    let transaction_hash = value
        .get("transactionHash")
        .and_then(Value::as_str)
        .map(parse_b256_str)
        .transpose()?;
    let log_index = value
        .get("logIndex")
        .and_then(Value::as_str)
        .map(parse_u64)
        .transpose()?;
    Ok(EvmLogEntry {
        address,
        topics,
        data,
        block_number,
        transaction_hash,
        log_index,
    })
}

/// Redaction-safe EVM transport setup/runtime error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmTransportError {
    /// Runtime source registry was invalid.
    InvalidRegistry,
    /// Requested source policy was unavailable.
    PolicyUnavailable,
    /// Requested source was unavailable.
    SourceUnavailable,
    /// Requested source is not allowed by the policy.
    SourceNotAllowed,
    /// Source chain id did not match the expected chain.
    ChainIdMismatch,
    /// JSON-RPC request failed.
    RequestFailed,
    /// JSON-RPC response failed contract validation.
    InvalidResponse,
    /// Transaction receipt is not yet available.
    ReceiptPending,
}

impl fmt::Display for EvmTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegistry => f.write_str("EVM source registry was invalid"),
            Self::PolicyUnavailable => f.write_str("EVM source policy was unavailable"),
            Self::SourceUnavailable => f.write_str("EVM source was unavailable"),
            Self::SourceNotAllowed => f.write_str("EVM source was not allowed by policy"),
            Self::ChainIdMismatch => {
                f.write_str("EVM source chain id did not match expected chain")
            }
            Self::RequestFailed => f.write_str("EVM JSON-RPC request failed"),
            Self::InvalidResponse => f.write_str("EVM JSON-RPC response was invalid"),
            Self::ReceiptPending => f.write_str("EVM transaction receipt is pending"),
        }
    }
}

impl std::error::Error for EvmTransportError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, B256, U256};
    use mfm_evm_capabilities::{EvmTransactionSubmitRequest, SignedEvmPayload};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const HASH_HEX: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

    #[tokio::test]
    async fn selects_source_by_policy_and_records_redacted_evidence() {
        let server = TestRpcServer::spawn("0x1").await;
        let client = client_for(&server.url, "primary", "mainnet", 1);

        let response = client
            .chain_identity(&chain_request("primary", "mainnet"))
            .await
            .expect("chain identity");

        assert_eq!(response.chain_id, 1);
        assert_eq!(response.evidence.source_ref.as_str(), "primary");
        assert_eq!(response.evidence.policy_id.as_str(), "mainnet");
        assert!(!format!("{:?}", response.evidence).contains(&server.url));
    }

    #[tokio::test]
    async fn rejects_chain_id_mismatch_without_leaking_source_details() {
        let server = TestRpcServer::spawn("0x2").await;
        let client = client_for(&server.url, "primary", "mainnet", 1);

        let error = client
            .chain_identity(&chain_request("primary", "mainnet"))
            .await
            .expect_err("chain mismatch");

        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(&server.url));
        assert!(!rendered.contains("Bearer"));
    }

    #[tokio::test]
    async fn supports_core_evm_json_rpc_calls() {
        let server = TestRpcServer::spawn("0x1").await;
        let client = client_for(&server.url, "primary", "mainnet", 1);
        let source_ref = EvmSourceRef::new("primary").expect("source");
        let policy_id = EvmSourcePolicyId::new("mainnet").expect("policy");
        let address = address!("0x1111111111111111111111111111111111111111");
        let hash = HASH_HEX.parse::<B256>().expect("hash");

        let block = client
            .read_block(&EvmBlockReadRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                block: EvmBlockSelector::Latest,
            })
            .await
            .expect("block");
        assert_eq!(block.block_number, 42);

        let balance = client
            .read_balance(&EvmBalanceReadRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                account: address,
                block: EvmBlockSelector::Latest,
            })
            .await
            .expect("balance");
        assert_eq!(
            balance.balance_wei,
            U256::from(1_000_000_000_000_000_000u128)
        );

        let call = client
            .read_call(&EvmCallReadRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                to: address,
                calldata: vec![0xab, 0xcd],
                block: EvmBlockSelector::Latest,
            })
            .await
            .expect("call");
        assert_eq!(call.return_data, vec![0x12, 0x34]);

        let logs = client
            .read_logs(&EvmLogsReadRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                from_block: EvmBlockSelector::Latest,
                to_block: EvmBlockSelector::Latest,
                address: Some(address),
                topics: vec![hash],
            })
            .await
            .expect("logs");
        assert_eq!(logs.logs.len(), 1);

        let nonce = client
            .read_nonce(&EvmNonceReadRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                account: address,
                block: EvmBlockSelector::Latest,
            })
            .await
            .expect("nonce");
        assert_eq!(nonce.nonce, 7);

        let fee = client
            .read_fee(&EvmFeeReadRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
            })
            .await
            .expect("fee");
        assert_eq!(fee.legacy_gas_price, Some(16));
        assert_eq!(fee.priority_fee_per_gas, Some(2));
        assert_eq!(fee.base_fee_per_gas, Some(32));
        assert_eq!(fee.max_fee_per_gas, Some(66));

        let gas = client
            .estimate_gas(&EvmGasEstimateRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                from: Some(address),
                to: Some(address),
                value_wei: 0,
                data: vec![0xab],
            })
            .await
            .expect("gas");
        assert_eq!(gas.gas_limit, 21_000);

        let submit = client
            .submit_transaction(&EvmTransactionSubmitRequest {
                source_ref: source_ref.clone(),
                policy_id: policy_id.clone(),
                signed_payload: SignedEvmPayload::from_verified_bytes(vec![0x01], hash)
                    .expect("payload"),
            })
            .await
            .expect("submit");
        assert_eq!(submit.transaction_hash, hash);

        let receipt = client
            .read_receipt(&EvmReceiptReadRequest {
                source_ref,
                policy_id,
                transaction_hash: hash,
            })
            .await
            .expect("receipt");
        assert_eq!(receipt.block_number, 42);
        assert!(receipt.status);
    }

    #[tokio::test]
    async fn pending_receipt_is_typed_capability_error() {
        let server = TestRpcServer::spawn_pending_receipt("0x1").await;
        let client = client_for(&server.url, "primary", "mainnet", 1);
        let error = client
            .read_receipt(&EvmReceiptReadRequest {
                source_ref: EvmSourceRef::new("primary").expect("source"),
                policy_id: EvmSourcePolicyId::new("mainnet").expect("policy"),
                transaction_hash: HASH_HEX.parse::<B256>().expect("hash"),
            })
            .await
            .expect_err("pending receipt");

        assert_eq!(error, EvmCapabilityError::ReceiptPending);
    }

    #[tokio::test]
    async fn supports_legacy_fee_source_without_eip1559_methods() {
        let server = TestRpcServer::spawn_legacy_fee("0x1").await;
        let client = client_for(&server.url, "primary", "mainnet", 1);

        let fee = client
            .read_fee(&EvmFeeReadRequest {
                source_ref: EvmSourceRef::new("primary").expect("source"),
                policy_id: EvmSourcePolicyId::new("mainnet").expect("policy"),
            })
            .await
            .expect("legacy fee response");

        assert_eq!(fee.legacy_gas_price, Some(16));
        assert_eq!(fee.priority_fee_per_gas, None);
        assert_eq!(fee.base_fee_per_gas, None);
        assert_eq!(fee.max_fee_per_gas, None);
    }

    #[tokio::test]
    async fn parses_ordered_fallback_policy_and_redacts_runtime_sources() {
        let server = TestRpcServer::spawn("0x1").await;
        let raw = serde_json::json!({
            "sources": [
                {
                    "id": "primary",
                    "expected_chain_id": 1,
                    "rpc_url": server.url,
                    "authorization": "Bearer top-secret"
                }
            ],
            "policies": [
                {
                    "id": "mainnet",
                    "ordered_sources": ["primary"]
                }
            ]
        })
        .to_string();
        let registry = EvmSourceRegistry::from_json_str(&raw).expect("registry");
        let rendered = format!("{registry:?}");

        assert!(!rendered.contains(&server.url));
        assert!(!rendered.contains("top-secret"));
    }

    #[tokio::test]
    async fn ordered_policy_falls_back_after_request_failure() {
        let failing = TestRpcServer::spawn_failure().await;
        let healthy = TestRpcServer::spawn("0x1").await;
        let primary = EvmRuntimeSource::new(
            EvmSourceRef::new("primary").expect("source"),
            1,
            &failing.url,
            None,
        )
        .expect("primary source");
        let secondary = EvmRuntimeSource::new(
            EvmSourceRef::new("secondary").expect("source"),
            1,
            &healthy.url,
            None,
        )
        .expect("secondary source");
        let policy = EvmSourcePolicy::new(
            EvmSourcePolicyId::new("mainnet").expect("policy"),
            vec![
                EvmSourceRef::new("primary").expect("primary"),
                EvmSourceRef::new("secondary").expect("secondary"),
            ],
        )
        .expect("policy");
        let client = EvmJsonRpcClient::new(
            EvmSourceRegistry::new([primary, secondary], [policy]).expect("registry"),
        );

        let response = client
            .chain_identity(&chain_request("primary", "mainnet"))
            .await
            .expect("fallback response");

        assert_eq!(response.evidence.source_ref.as_str(), "secondary");
        assert_eq!(response.chain_id, 1);
    }

    fn client_for(
        url: &str,
        source_id: &str,
        policy_id: &str,
        expected_chain_id: u64,
    ) -> EvmJsonRpcClient {
        let source = EvmRuntimeSource::new(
            EvmSourceRef::new(source_id).expect("source"),
            expected_chain_id,
            url,
            Some("Bearer top-secret".to_owned()),
        )
        .expect("runtime source");
        let registry = EvmSourceRegistry::single_source(
            source,
            EvmSourcePolicyId::new(policy_id).expect("policy"),
        )
        .expect("registry");
        EvmJsonRpcClient::new(registry)
    }

    fn chain_request(source_id: &str, policy_id: &str) -> EvmChainIdentityRequest {
        EvmChainIdentityRequest {
            source_ref: EvmSourceRef::new(source_id).expect("source"),
            policy_id: EvmSourcePolicyId::new(policy_id).expect("policy"),
        }
    }

    struct TestRpcServer {
        url: String,
        _requests: Arc<Mutex<Vec<String>>>,
    }

    impl TestRpcServer {
        async fn spawn(chain_id: &'static str) -> Self {
            Self::spawn_with_mode(TestRpcMode::Ok { chain_id }).await
        }

        async fn spawn_legacy_fee(chain_id: &'static str) -> Self {
            Self::spawn_with_mode(TestRpcMode::LegacyFee { chain_id }).await
        }

        async fn spawn_pending_receipt(chain_id: &'static str) -> Self {
            Self::spawn_with_mode(TestRpcMode::PendingReceipt { chain_id }).await
        }

        async fn spawn_failure() -> Self {
            Self::spawn_with_mode(TestRpcMode::Failure).await
        }

        async fn spawn_with_mode(mode: TestRpcMode) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let addr = listener.local_addr().expect("addr");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&requests);
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let captured = Arc::clone(&captured);
                    tokio::spawn(async move {
                        let mut buffer = vec![0_u8; 8192];
                        let mut read = 0_usize;
                        loop {
                            let n = stream.read(&mut buffer[read..]).await.expect("read");
                            if n == 0 {
                                return;
                            }
                            read += n;
                            if request_complete(&buffer[..read]) {
                                break;
                            }
                        }
                        let body = request_body(&buffer[..read]);
                        let request: Value = serde_json::from_slice(body).expect("json request");
                        let method = request
                            .get("method")
                            .and_then(Value::as_str)
                            .expect("method")
                            .to_owned();
                        captured.lock().expect("requests").push(method.clone());
                        let response = match mode {
                            TestRpcMode::Ok { chain_id } => {
                                let result = rpc_result(chain_id, &method);
                                let body = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": 1,
                                    "result": result,
                                })
                                .to_string();
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                    body.len(),
                                    body
                                )
                            }
                            TestRpcMode::LegacyFee { chain_id } => {
                                let body = if method == "eth_maxPriorityFeePerGas" {
                                    serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": 1,
                                        "error": {
                                            "code": -32601,
                                            "message": "method not found",
                                        },
                                    })
                                } else {
                                    serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": 1,
                                        "result": legacy_fee_rpc_result(chain_id, &method),
                                    })
                                }
                                .to_string();
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                    body.len(),
                                    body
                                )
                            }
                            TestRpcMode::PendingReceipt { chain_id } => {
                                let result = if method == "eth_getTransactionReceipt" {
                                    serde_json::Value::Null
                                } else {
                                    rpc_result(chain_id, &method)
                                };
                                let body = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": 1,
                                    "result": result,
                                })
                                .to_string();
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                    body.len(),
                                    body
                                )
                            }
                            TestRpcMode::Failure => {
                                "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\n\r\n"
                                    .to_owned()
                            }
                        };
                        stream.write_all(response.as_bytes()).await.expect("write");
                    });
                }
            });
            Self {
                url: format!("http://{addr}"),
                _requests: requests,
            }
        }
    }

    #[derive(Clone, Copy)]
    enum TestRpcMode {
        Ok { chain_id: &'static str },
        LegacyFee { chain_id: &'static str },
        PendingReceipt { chain_id: &'static str },
        Failure,
    }

    fn request_complete(bytes: &[u8]) -> bool {
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_len = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        bytes.len() >= header_end + 4 + content_len
    }

    fn request_body(bytes: &[u8]) -> &[u8] {
        let header_end = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("headers");
        &bytes[header_end + 4..]
    }

    fn rpc_result(chain_id: &str, method: &str) -> Value {
        match method {
            "eth_chainId" => json!(chain_id),
            "web3_clientVersion" => json!("mfm-test-rpc"),
            "eth_getBlockByNumber" => json!({
                "number": "0x2a",
                "hash": HASH_HEX,
                "baseFeePerGas": "0x20",
            }),
            "eth_getBlockByHash" => json!({
                "number": "0x2a",
                "hash": HASH_HEX,
            }),
            "eth_getBalance" => json!("0xde0b6b3a7640000"),
            "eth_call" => json!("0x1234"),
            "eth_getLogs" => json!([{
                "address": "0x1111111111111111111111111111111111111111",
                "topics": [HASH_HEX],
                "data": "0x1234",
                "blockNumber": "0x2a",
                "transactionHash": HASH_HEX,
                "logIndex": "0x0",
            }]),
            "eth_getTransactionCount" => json!("0x7"),
            "eth_gasPrice" => json!("0x10"),
            "eth_maxPriorityFeePerGas" => json!("0x2"),
            "eth_estimateGas" => json!("0x5208"),
            "eth_sendRawTransaction" => json!(HASH_HEX),
            "eth_getTransactionReceipt" => json!({
                "transactionHash": HASH_HEX,
                "blockNumber": "0x2a",
                "status": "0x1",
            }),
            other => panic!("unexpected method {other}"),
        }
    }

    fn legacy_fee_rpc_result(chain_id: &str, method: &str) -> Value {
        match method {
            "eth_chainId" => json!(chain_id),
            "eth_gasPrice" => json!("0x10"),
            "eth_getBlockByNumber" => json!({
                "number": "0x2a",
                "hash": HASH_HEX,
            }),
            other => panic!("unexpected legacy fee method {other}"),
        }
    }
}
