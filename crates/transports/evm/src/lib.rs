#![warn(missing_docs)]
//! Generic EVM JSON-RPC transport.
//!
//! This crate owns live EVM JSON-RPC communication and process-local source
//! routing. It implements the reusable EVM capability traits without depending
//! on workflow lifecycle crates, signer providers, artifact stores, or binaries.
//!
//! ```rust
//! use mfm_evm_capabilities::{EvmNetworkId, EvmSourcePolicyId, EvmSourceRef};
//! use mfm_transports_evm::{
//!     EvmJsonRpcClient, EvmRoute, EvmRouteRegistry, EvmRuntimeSource, EvmSourceRegistry,
//! };
//!
//! let source = EvmRuntimeSource::new(
//!     EvmSourceRef::new("local")?,
//!     "http://127.0.0.1:8545",
//!     None,
//! )?;
//! let registry = EvmSourceRegistry::single_source(
//!     source,
//!     EvmSourcePolicyId::new("dev")?,
//! )?;
//! let routes = EvmRouteRegistry::new([EvmRoute::new(
//!     EvmNetworkId::new("reth-dev")?,
//!     EvmSourceRef::new("local")?,
//!     EvmSourcePolicyId::new("dev")?,
//! )])?;
//! let _client = EvmJsonRpcClient::new(registry, routes);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use alloy_primitives::B256;
use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBalanceReadResponse, EvmBlockReadProvider,
    EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector, EvmCallReadProvider,
    EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError, EvmCapabilityFuture,
    EvmChainGuard, EvmChainIdentityProvider, EvmChainIdentityRequest, EvmChainIdentityResponse,
    EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse, EvmGasEstimateProvider,
    EvmGasEstimateRequest, EvmGasEstimateResponse, EvmLogEntry, EvmLogsReadProvider,
    EvmLogsReadRequest, EvmLogsReadResponse, EvmNetworkId, EvmNonceOccupancy,
    EvmNonceOccupancyReadProvider, EvmNonceOccupancyReadRequest, EvmNonceOccupancyReadResponse,
    EvmNonceReadProvider, EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadProvider,
    EvmReceiptReadRequest, EvmReceiptReadResponse, EvmSourcePolicyId, EvmSourceRef,
    EvmTransactionSubmitProvider, EvmTransactionSubmitRequest, EvmTransactionSubmitResponse,
    RedactedEvmSourceEvidence,
};
use mfm_evm_core::encoding::parse_u256_hex;
use mfm_evm_core::hex::{bytes_to_hex_prefixed, hex_to_bytes};
use mfm_evm_core::tx::{parse_u128_quantity, parse_u64_quantity};
use serde_json::{json, Value};

/// Result type for EVM transport setup.
pub type TransportResult<T> = std::result::Result<T, EvmTransportError>;

/// Runtime EVM JSON-RPC source.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmRuntimeSource {
    id: EvmSourceRef,
    rpc_url: String,
    authorization: Option<String>,
}

impl EvmRuntimeSource {
    /// Creates a runtime EVM source.
    pub fn new(
        id: EvmSourceRef,
        rpc_url: impl Into<String>,
        authorization: Option<String>,
    ) -> TransportResult<Self> {
        let rpc_url = rpc_url.into();
        if rpc_url.trim().is_empty() || reqwest::Url::parse(&rpc_url).is_err() {
            return Err(EvmTransportError::InvalidRegistry);
        }
        Ok(Self {
            id,
            rpc_url,
            authorization: authorization.filter(|value| !value.trim().is_empty()),
        })
    }

    /// Returns the source id.
    pub fn id(&self) -> &EvmSourceRef {
        &self.id
    }
}

impl fmt::Debug for EvmRuntimeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmRuntimeSource")
            .field("id", &self.id)
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

/// Runtime route from a semantic EVM network id to a local source policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRoute {
    network_id: EvmNetworkId,
    source_ref: EvmSourceRef,
    policy_id: EvmSourcePolicyId,
}

impl EvmRoute {
    /// Creates a runtime route.
    pub fn new(
        network_id: EvmNetworkId,
        source_ref: EvmSourceRef,
        policy_id: EvmSourcePolicyId,
    ) -> Self {
        Self {
            network_id,
            source_ref,
            policy_id,
        }
    }

    /// Returns the semantic network id.
    pub const fn network_id(&self) -> &EvmNetworkId {
        &self.network_id
    }

    /// Returns the preferred source reference.
    pub const fn source_ref(&self) -> &EvmSourceRef {
        &self.source_ref
    }

    /// Returns the source policy id.
    pub const fn policy_id(&self) -> &EvmSourcePolicyId {
        &self.policy_id
    }
}

/// Runtime EVM route registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRouteRegistry {
    routes: BTreeMap<EvmNetworkId, EvmRoute>,
}

impl EvmRouteRegistry {
    /// Creates a runtime route registry, rejecting duplicate semantic network ids.
    pub fn new(routes: impl IntoIterator<Item = EvmRoute>) -> TransportResult<Self> {
        let mut by_network = BTreeMap::new();
        for route in routes {
            if by_network
                .insert(route.network_id().clone(), route)
                .is_some()
            {
                return Err(EvmTransportError::InvalidRegistry);
            }
        }
        Ok(Self { routes: by_network })
    }

    /// Returns the route for a semantic network id.
    pub fn route(&self, network_id: &EvmNetworkId) -> TransportResult<&EvmRoute> {
        self.routes
            .get(network_id)
            .ok_or(EvmTransportError::RouteUnavailable)
    }
}

/// Generic EVM JSON-RPC client.
#[derive(Clone)]
pub struct EvmJsonRpcClient {
    client: reqwest::Client,
    registry: EvmSourceRegistry,
    routes: EvmRouteRegistry,
}

impl EvmJsonRpcClient {
    /// Creates a client from runtime source and route registries.
    pub fn new(registry: EvmSourceRegistry, routes: EvmRouteRegistry) -> Self {
        Self {
            client: reqwest::Client::builder()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            registry,
            routes,
        }
    }

    /// Validates that a guard can resolve to a source policy without network I/O.
    pub fn validate_guard(&self, guard: &EvmChainGuard) -> TransportResult<()> {
        let route = self.routes.route(guard.network_id())?;
        self.registry
            .candidates(route.policy_id(), route.source_ref())
            .map(|_| ())
    }

    async fn chain_identity_impl(
        &self,
        request: &EvmChainIdentityRequest,
    ) -> TransportResult<EvmChainIdentityResponse> {
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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
        let selected = self.verified_source(&request.guard).await?;
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

    async fn nonce_occupancy_read_impl(
        &self,
        request: &EvmNonceOccupancyReadRequest,
    ) -> TransportResult<EvmNonceOccupancyReadResponse> {
        let selected = self.verified_source(&request.guard).await?;
        let account = format!("{:?}", request.account).to_ascii_lowercase();
        for block_tag in ["latest", "pending"] {
            let result = self
                .rpc_call(
                    selected.source,
                    "eth_getBlockByNumber",
                    json!([block_tag, true]),
                )
                .await?;
            let Some(transactions) = result.get("transactions").and_then(Value::as_array) else {
                continue;
            };
            for transaction in transactions {
                let Some(from) = transaction.get("from").and_then(Value::as_str) else {
                    continue;
                };
                if from.to_ascii_lowercase() != account {
                    continue;
                }
                let Some(raw_nonce) = transaction.get("nonce").and_then(Value::as_str) else {
                    continue;
                };
                if parse_u64(raw_nonce)? != request.nonce {
                    continue;
                }
                let hash = parse_b256_field(transaction, "hash")?;
                if hash == request.excluded_transaction_hash {
                    return Ok(EvmNonceOccupancyReadResponse {
                        evidence: selected.evidence,
                        outcome: EvmNonceOccupancy::Unknown,
                    });
                }
                let block_number = transaction
                    .get("blockNumber")
                    .and_then(Value::as_str)
                    .map(parse_u64)
                    .transpose()?;
                return Ok(EvmNonceOccupancyReadResponse {
                    evidence: selected.evidence,
                    outcome: EvmNonceOccupancy::Occupied {
                        transaction_hash: hash,
                        block_number,
                    },
                });
            }
        }
        Ok(EvmNonceOccupancyReadResponse {
            evidence: selected.evidence,
            outcome: EvmNonceOccupancy::Unknown,
        })
    }

    async fn verified_source<'a>(
        &'a self,
        guard: &EvmChainGuard,
    ) -> TransportResult<VerifiedSource<'a>> {
        let route = self.routes.route(guard.network_id())?;
        let mut last_failure = EvmTransportError::SourceUnavailable;
        for source in self
            .registry
            .candidates(route.policy_id(), route.source_ref())?
        {
            match self.chain_id_for_source(source).await {
                Ok(chain_id) if chain_id == guard.expected_chain_id() => {
                    return Ok(VerifiedSource {
                        source,
                        chain_id,
                        evidence: RedactedEvmSourceEvidence {
                            network_id: guard.network_id().clone(),
                            expected_chain_id: guard.expected_chain_id(),
                            observed_chain_id: chain_id,
                            source_ref: source.id.clone(),
                            policy_id: route.policy_id().clone(),
                        },
                    });
                }
                Ok(chain_id) => {
                    return Err(EvmTransportError::ChainIdMismatch {
                        evidence: RedactedEvmSourceEvidence {
                            network_id: guard.network_id().clone(),
                            expected_chain_id: guard.expected_chain_id(),
                            observed_chain_id: chain_id,
                            source_ref: source.id.clone(),
                            policy_id: route.policy_id().clone(),
                        },
                    });
                }
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
            .field("routes", &self.routes)
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
impl_provider!(
    EvmNonceOccupancyReadProvider,
    read_nonce_occupancy,
    EvmNonceOccupancyReadRequest,
    EvmNonceOccupancyReadResponse,
    nonce_occupancy_read_impl
);

fn capability_error_from_transport(error: EvmTransportError) -> EvmCapabilityError {
    match error {
        EvmTransportError::ReceiptPending => EvmCapabilityError::ReceiptPending,
        EvmTransportError::ChainIdMismatch { evidence } => {
            EvmCapabilityError::ChainMismatch { evidence }
        }
        other => EvmCapabilityError::redacted_provider_failure(other),
    }
}

fn block_selector_tag(selector: &EvmBlockSelector) -> String {
    match selector {
        EvmBlockSelector::Latest => "latest".to_owned(),
        EvmBlockSelector::Pending => "pending".to_owned(),
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmTransportError {
    /// Runtime source registry was invalid.
    #[error("EVM source registry was invalid")]
    InvalidRegistry,
    /// Requested source policy was unavailable.
    #[error("EVM source policy was unavailable")]
    PolicyUnavailable,
    /// Requested semantic network route was unavailable.
    #[error("EVM route was unavailable")]
    RouteUnavailable,
    /// Requested source was unavailable.
    #[error("EVM source was unavailable")]
    SourceUnavailable,
    /// Requested source is not allowed by the policy.
    #[error("EVM source was not allowed by policy")]
    SourceNotAllowed,
    /// Source chain id did not match the expected chain.
    #[error("EVM source chain id did not match expected chain")]
    ChainIdMismatch {
        /// Closed redacted mismatch evidence.
        evidence: RedactedEvmSourceEvidence,
    },
    /// JSON-RPC request failed.
    #[error("EVM JSON-RPC request failed")]
    RequestFailed,
    /// JSON-RPC response failed contract validation.
    #[error("EVM JSON-RPC response was invalid")]
    InvalidResponse,
    /// Transaction receipt is not yet available.
    #[error("EVM transaction receipt is pending")]
    ReceiptPending,
}

#[cfg(test)]
mod tests;
