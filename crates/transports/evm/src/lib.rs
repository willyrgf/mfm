#![warn(missing_docs)]
//! Generic EVM JSON-RPC transport.
//!
//! This crate owns live EVM JSON-RPC communication and process-local source routing. Raw clients
//! resolve runtime routes, while bound network providers implement reusable EVM capability traits.
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

use alloy_primitives::{keccak256, B256, U256};
use mfm_capabilities::{
    ProviderDiagnosticCode, ProviderDiagnosticValue, RedactedProviderDiagnostic,
};
use mfm_evm_capabilities::{
    evm_diagnostic, EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBalanceReadResponse,
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector,
    EvmCallReadProvider, EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError,
    EvmCapabilityFuture, EvmChainIdentityProvider, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCodeReadProvider, EvmCodeReadRequest, EvmCodeReadResponse,
    EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse, EvmGasEstimateProvider,
    EvmGasEstimateRequest, EvmGasEstimateResponse, EvmNetworkBinding, EvmNetworkId,
    EvmNonceReadProvider, EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadProvider,
    EvmReceiptReadRequest, EvmReceiptReadResponse, EvmSourcePolicyId, EvmSourceRef,
    EvmTransactionSubmitProvider, EvmTransactionSubmitRequest, EvmTransactionSubmitResponse,
    RedactedEvmSourceEvidence,
};
use mfm_ids::LocalPublicId;
use serde_json::{json, Value};
use tracing::debug;

/// Result type for EVM transport setup.
pub type TransportResult<T> = std::result::Result<T, EvmTransportError>;

mod registry;
pub use self::registry::{
    EvmRoute, EvmRouteRegistry, EvmRuntimeSource, EvmSourcePolicy, EvmSourceRegistry,
};

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

    fn route_candidates<'a>(
        &'a self,
        network_id: &EvmNetworkId,
    ) -> TransportResult<(&'a EvmRoute, Vec<&'a EvmRuntimeSource>)> {
        let route = self.routes.route(network_id)?;
        let candidates = self
            .registry
            .candidates(route.policy_id(), route.source_ref())?;
        Ok((route, candidates))
    }

    /// Validates that a network binding can resolve to configured runtime sources without I/O.
    pub fn validate_network_binding(&self, binding: &EvmNetworkBinding) -> TransportResult<()> {
        self.route_candidates(binding.network_id()).map(|_| ())
    }

    /// Binds a checked semantic network binding to a live capability provider.
    pub fn bind_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> TransportResult<EvmJsonRpcNetworkProvider> {
        self.validate_network_binding(&binding)?;
        Ok(EvmJsonRpcNetworkProvider {
            client: self.clone(),
            binding,
        })
    }

    async fn chain_identity_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmChainIdentityRequest,
    ) -> TransportResult<EvmChainIdentityResponse> {
        let _request = request;
        let client_version = self
            .verified_rpc_call(selected, "web3_clientVersion", json!([]))
            .await
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned));
        Ok(EvmChainIdentityResponse {
            evidence: selected.evidence.clone(),
            chain_id: selected.chain_id,
            client_version,
        })
    }

    async fn block_read_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmBlockReadRequest,
    ) -> TransportResult<EvmBlockReadResponse> {
        let value = match request.block() {
            EvmBlockSelector::Hash(hash) => {
                self.verified_rpc_call(
                    selected,
                    "eth_getBlockByHash",
                    json!([format!("{hash:?}"), false]),
                )
                .await?
            }
            selector => {
                self.verified_rpc_call(
                    selected,
                    "eth_getBlockByNumber",
                    json!([block_selector_tag(selector)?, false]),
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
        validate_block_identity(request.block(), block_number, block_hash)?;
        Ok(EvmBlockReadResponse {
            evidence: selected.evidence.clone(),
            block_number,
            block_hash,
        })
    }

    async fn call_read_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmCallReadRequest,
    ) -> TransportResult<EvmCallReadResponse> {
        let result = self
            .verified_rpc_call(
                selected,
                "eth_call",
                json!([{
                    "to": format!("{:?}", request.to()),
                    "data": encode_hex_bytes(request.calldata()),
                }, block_selector_param(request.block())]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmCallReadResponse {
            evidence: selected.evidence.clone(),
            return_data: decode_hex_bytes(raw)?,
        })
    }

    async fn code_read_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmCodeReadRequest,
    ) -> TransportResult<EvmCodeReadResponse> {
        let result = self
            .verified_rpc_call(
                selected,
                "eth_getCode",
                json!([
                    format!("{:?}", request.address()),
                    block_selector_param(request.block())
                ]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        let code = decode_hex_bytes(raw)?;
        let code_hash = keccak256(&code);
        Ok(EvmCodeReadResponse {
            evidence: selected.evidence.clone(),
            code,
            code_hash,
        })
    }

    async fn balance_read_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmBalanceReadRequest,
    ) -> TransportResult<EvmBalanceReadResponse> {
        let result = self
            .verified_rpc_call(
                selected,
                "eth_getBalance",
                json!([
                    format!("{:?}", request.account()),
                    block_selector_param(request.block())
                ]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmBalanceReadResponse {
            evidence: selected.evidence.clone(),
            balance_wei: parse_u256(raw)?,
        })
    }

    async fn nonce_read_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmNonceReadRequest,
    ) -> TransportResult<EvmNonceReadResponse> {
        let result = self
            .verified_rpc_call(
                selected,
                "eth_getTransactionCount",
                json!([
                    format!("{:?}", request.account()),
                    block_selector_param(request.block())
                ]),
            )
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmNonceReadResponse {
            evidence: selected.evidence.clone(),
            nonce: parse_u64(raw)?,
        })
    }

    async fn fee_read_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmFeeReadRequest,
    ) -> TransportResult<EvmFeeReadResponse> {
        let _request = request;
        let legacy_gas_price = self
            .verified_rpc_call(selected, "eth_gasPrice", json!([]))
            .await?
            .as_str()
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_u128)?;
        let priority_fee_per_gas = match self
            .verified_rpc_call(selected, "eth_maxPriorityFeePerGas", json!([]))
            .await
        {
            Ok(value) => Some(
                value
                    .as_str()
                    .ok_or(EvmTransportError::InvalidResponse)
                    .and_then(parse_u128)?,
            ),
            Err(error) if error.can_try_next_source() => None,
            Err(error) => return Err(error),
        };
        let latest_block = match self
            .verified_rpc_call(selected, "eth_getBlockByNumber", json!(["latest", false]))
            .await
        {
            Ok(value) => Some(value),
            Err(error) if error.can_try_next_source() => None,
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
            evidence: selected.evidence.clone(),
            base_fee_per_gas,
            priority_fee_per_gas,
            max_fee_per_gas,
            legacy_gas_price: Some(legacy_gas_price),
        })
    }

    async fn gas_estimate_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmGasEstimateRequest,
    ) -> TransportResult<EvmGasEstimateResponse> {
        let mut call = serde_json::Map::new();
        if let Some(from) = request.from() {
            call.insert("from".to_owned(), json!(format!("{from:?}")));
        }
        if let Some(to) = request.to() {
            call.insert("to".to_owned(), json!(format!("{to:?}")));
        }
        call.insert(
            "value".to_owned(),
            json!(format!("0x{:x}", request.value_wei())),
        );
        call.insert("data".to_owned(), json!(encode_hex_bytes(request.data())));
        let result = self
            .verified_rpc_call(selected, "eth_estimateGas", json!([Value::Object(call)]))
            .await?;
        let raw = result.as_str().ok_or(EvmTransportError::InvalidResponse)?;
        Ok(EvmGasEstimateResponse {
            evidence: selected.evidence.clone(),
            gas_limit: parse_u64(raw)?,
        })
    }

    async fn submit_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmTransactionSubmitRequest,
    ) -> TransportResult<EvmTransactionSubmitResponse> {
        let result = self
            .verified_rpc_call(
                selected,
                "eth_sendRawTransaction",
                json!([encode_hex_bytes(request.signed_payload().bytes())]),
            )
            .await?;
        let hash = parse_b256_str(result.as_str().ok_or(EvmTransportError::InvalidResponse)?)?;
        if hash != request.signed_payload().transaction_hash() {
            return Err(EvmTransportError::InvalidResponse);
        }
        Ok(EvmTransactionSubmitResponse {
            evidence: selected.evidence.clone(),
            transaction_hash: hash,
        })
    }

    async fn receipt_read_impl(
        &self,
        selected: &VerifiedEvmCall<'_>,
        request: &EvmReceiptReadRequest,
    ) -> TransportResult<EvmReceiptReadResponse> {
        let result = self
            .verified_rpc_call(
                selected,
                "eth_getTransactionReceipt",
                json!([format!("{:?}", request.transaction_hash())]),
            )
            .await?;
        if result.is_null() {
            return Err(EvmTransportError::ReceiptPending);
        }
        let transaction_hash = parse_b256_field(&result, "transactionHash")?;
        if transaction_hash != request.transaction_hash() {
            return Err(EvmTransportError::InvalidResponse);
        }
        let block_number = result
            .get("blockNumber")
            .and_then(Value::as_str)
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_u64)?;
        let block_hash = parse_b256_field(&result, "blockHash")?;
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .map(|raw| raw == "0x1")
            .unwrap_or(false);
        Ok(EvmReceiptReadResponse {
            evidence: selected.evidence.clone(),
            transaction_hash,
            block_number,
            block_hash,
            status,
        })
    }

    async fn verified_source<'a>(
        &'a self,
        binding: &EvmNetworkBinding,
    ) -> TransportResult<VerifiedEvmCall<'a>> {
        let (route, candidates) = self.route_candidates(binding.network_id())?;
        let mut last_failure = EvmTransportError::SourceUnavailable;
        for source in candidates {
            match self.probe_chain_id(source).await {
                Ok(chain_id) if chain_id == binding.expected_chain_id() => {
                    return Ok(VerifiedEvmCall {
                        source,
                        chain_id,
                        evidence: RedactedEvmSourceEvidence::from_binding(
                            binding,
                            chain_id,
                            source.id.clone(),
                            route.policy_id().clone(),
                        )
                        .map_err(source_evidence_error_into_transport)?,
                    });
                }
                Ok(chain_id) => {
                    let error = RedactedEvmSourceEvidence::from_binding(
                        binding,
                        chain_id,
                        source.id.clone(),
                        route.policy_id().clone(),
                    )
                    .expect_err("mismatched chain id must fail closed");
                    let EvmCapabilityError::SourceMismatch { diagnostic } = error else {
                        return Err(EvmTransportError::InvalidResponse);
                    };
                    return Err(EvmTransportError::SourceMismatch { diagnostic });
                }
                Err(error) if error.can_try_next_source() => {
                    last_failure = error;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_failure)
    }

    async fn probe_chain_id(&self, source: &EvmRuntimeSource) -> TransportResult<u64> {
        json_rpc_pipeline::probe_chain_id(self, source).await
    }

    async fn verified_rpc_call(
        &self,
        verified: &VerifiedEvmCall<'_>,
        method: &'static str,
        params: Value,
    ) -> TransportResult<Value> {
        json_rpc_pipeline::verified_call(self, verified, method, params).await
    }
}

mod json_rpc_pipeline {
    use super::*;

    struct RawJsonRpcExchange<'a> {
        source: &'a EvmRuntimeSource,
        operation: LocalPublicId,
        method: &'static str,
        params: Value,
    }

    impl<'a> RawJsonRpcExchange<'a> {
        fn chain_id_probe(source: &'a EvmRuntimeSource) -> Self {
            Self {
                source,
                operation: operation_id("eth_chainId"),
                method: "eth_chainId",
                params: json!([]),
            }
        }

        fn verified_operation(
            verified: &VerifiedEvmCall<'a>,
            method: &'static str,
            params: Value,
        ) -> Self {
            Self {
                source: verified.source,
                operation: operation_id(method),
                method,
                params,
            }
        }
    }

    pub(super) async fn probe_chain_id(
        client: &EvmJsonRpcClient,
        source: &EvmRuntimeSource,
    ) -> TransportResult<u64> {
        let result = send_json_rpc(client, RawJsonRpcExchange::chain_id_probe(source)).await?;
        result
            .as_str()
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_u64)
    }

    pub(super) async fn verified_call<'a>(
        client: &EvmJsonRpcClient,
        verified: &VerifiedEvmCall<'a>,
        method: &'static str,
        params: Value,
    ) -> TransportResult<Value> {
        send_json_rpc(
            client,
            RawJsonRpcExchange::verified_operation(verified, method, params),
        )
        .await
    }

    async fn send_json_rpc(
        client: &EvmJsonRpcClient,
        exchange: RawJsonRpcExchange<'_>,
    ) -> TransportResult<Value> {
        let operation = exchange.operation;
        let mut request = client.client.post(&exchange.source.rpc_url).json(&json!({
            "jsonrpc": "2.0",
            "id": 1u64,
            "method": exchange.method,
            "params": exchange.params,
        }));
        if let Some(authorization) = &exchange.source.authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        debug!(operation = %operation, "evm rpc request");
        let response = request
            .send()
            .await
            .map_err(|_| EvmTransportError::TransportFailed {
                operation: operation.clone(),
            })?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(EvmTransportError::RpcHttpStatus { operation, status });
        }
        let body = response
            .json::<Value>()
            .await
            .map_err(|_| EvmTransportError::InvalidResponse)?;
        if let Some(error) = body.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .ok_or(EvmTransportError::InvalidResponse)?;
            return Err(EvmTransportError::RpcJsonError { operation, code });
        }
        body.get("result")
            .cloned()
            .ok_or(EvmTransportError::ResponseMissingResult { operation })
    }
}

impl EvmTransportError {
    fn can_try_next_source(&self) -> bool {
        matches!(
            self,
            Self::TransportFailed { .. } | Self::RpcHttpStatus { .. } | Self::RpcJsonError { .. }
        )
    }

    /// Converts the transport error into a closed redaction-safe provider diagnostic.
    pub fn into_provider_diagnostic(self) -> RedactedProviderDiagnostic {
        match self {
            Self::InvalidRegistry => {
                evm_diagnostic(ProviderDiagnosticCode::ProviderConfigurationInvalid)
            }
            Self::PolicyUnavailable | Self::RouteUnavailable => {
                evm_diagnostic(ProviderDiagnosticCode::RouteUnavailable)
            }
            Self::SourceUnavailable => evm_diagnostic(ProviderDiagnosticCode::SourceUnavailable),
            Self::SourceNotAllowed => evm_diagnostic(ProviderDiagnosticCode::SourceNotAllowed),
            Self::SourceMismatch { diagnostic } => diagnostic,
            Self::TransportFailed { operation } => {
                evm_diagnostic(ProviderDiagnosticCode::TransportFailed).with_operation(operation)
            }
            Self::RpcHttpStatus { operation, status } => {
                evm_diagnostic(ProviderDiagnosticCode::RpcHttpStatus)
                    .with_operation(operation)
                    .with_field(
                        diagnostic_id("http_status"),
                        ProviderDiagnosticValue::U64(u64::from(status)),
                    )
            }
            Self::RpcJsonError { operation, code } => {
                evm_diagnostic(ProviderDiagnosticCode::RpcJsonError)
                    .with_operation(operation)
                    .with_field(
                        diagnostic_id("rpc_code"),
                        ProviderDiagnosticValue::I64(code),
                    )
            }
            Self::InvalidResponse => evm_diagnostic(ProviderDiagnosticCode::ResponseInvalid),
            Self::ResponseMissingResult { operation } => {
                evm_diagnostic(ProviderDiagnosticCode::ResponseMissingResult)
                    .with_operation(operation)
            }
            Self::ReceiptPending => evm_diagnostic(ProviderDiagnosticCode::OperationIncomplete)
                .with_operation(diagnostic_id("eth_get_transaction_receipt")),
        }
    }
}

fn operation_id(method: &'static str) -> LocalPublicId {
    match method {
        "eth_chainId" => diagnostic_id("eth_chain_id"),
        "web3_clientVersion" => diagnostic_id("web3_client_version"),
        "eth_getBlockByHash" => diagnostic_id("eth_get_block_by_hash"),
        "eth_getBlockByNumber" => diagnostic_id("eth_get_block_by_number"),
        "eth_call" => diagnostic_id("eth_call"),
        "eth_getCode" => diagnostic_id("eth_get_code"),
        "eth_getBalance" => diagnostic_id("eth_get_balance"),
        "eth_getTransactionCount" => diagnostic_id("eth_get_transaction_count"),
        "eth_gasPrice" => diagnostic_id("eth_gas_price"),
        "eth_maxPriorityFeePerGas" => diagnostic_id("eth_max_priority_fee_per_gas"),
        "eth_estimateGas" => diagnostic_id("eth_estimate_gas"),
        "eth_sendRawTransaction" => diagnostic_id("eth_send_raw_transaction"),
        "eth_getTransactionReceipt" => diagnostic_id("eth_get_transaction_receipt"),
        "eth_getTransactionByHash" => diagnostic_id("eth_get_transaction_by_hash"),
        _ => diagnostic_id("evm_rpc"),
    }
}

fn diagnostic_id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("EVM diagnostic label must be checked public text")
}

impl fmt::Debug for EvmJsonRpcClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmJsonRpcClient")
            .field("registry", &self.registry)
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}

/// EVM JSON-RPC capability provider bound to a checked semantic network binding.
#[derive(Clone)]
pub struct EvmJsonRpcNetworkProvider {
    client: EvmJsonRpcClient,
    binding: EvmNetworkBinding,
}

impl fmt::Debug for EvmJsonRpcNetworkProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmJsonRpcNetworkProvider")
            .field("client", &self.client)
            .field("binding", &self.binding)
            .finish()
    }
}

struct VerifiedEvmCall<'a> {
    source: &'a EvmRuntimeSource,
    chain_id: u64,
    evidence: RedactedEvmSourceEvidence,
}

macro_rules! impl_network_provider {
    ($trait:ident, $method:ident, $req:ty, $resp:ty, $inner:ident) => {
        impl $trait for EvmJsonRpcNetworkProvider {
            fn $method<'a>(&'a self, request: &'a $req) -> EvmCapabilityFuture<'a, $resp> {
                Box::pin(async move {
                    let selected = self
                        .client
                        .verified_source(&self.binding)
                        .await
                        .map_err(capability_error_from_transport)?;
                    self.client
                        .$inner(&selected, request)
                        .await
                        .map_err(capability_error_from_transport)
                })
            }
        }
    };
}

impl_network_provider!(
    EvmChainIdentityProvider,
    chain_identity,
    EvmChainIdentityRequest,
    EvmChainIdentityResponse,
    chain_identity_impl
);
impl_network_provider!(
    EvmBlockReadProvider,
    read_block,
    EvmBlockReadRequest,
    EvmBlockReadResponse,
    block_read_impl
);
impl_network_provider!(
    EvmBalanceReadProvider,
    read_balance,
    EvmBalanceReadRequest,
    EvmBalanceReadResponse,
    balance_read_impl
);
impl_network_provider!(
    EvmCallReadProvider,
    read_call,
    EvmCallReadRequest,
    EvmCallReadResponse,
    call_read_impl
);
impl_network_provider!(
    EvmCodeReadProvider,
    read_code,
    EvmCodeReadRequest,
    EvmCodeReadResponse,
    code_read_impl
);
impl_network_provider!(
    EvmNonceReadProvider,
    read_nonce,
    EvmNonceReadRequest,
    EvmNonceReadResponse,
    nonce_read_impl
);
impl_network_provider!(
    EvmFeeReadProvider,
    read_fee,
    EvmFeeReadRequest,
    EvmFeeReadResponse,
    fee_read_impl
);
impl_network_provider!(
    EvmGasEstimateProvider,
    estimate_gas,
    EvmGasEstimateRequest,
    EvmGasEstimateResponse,
    gas_estimate_impl
);
impl_network_provider!(
    EvmTransactionSubmitProvider,
    submit_transaction,
    EvmTransactionSubmitRequest,
    EvmTransactionSubmitResponse,
    submit_impl
);
impl_network_provider!(
    EvmReceiptReadProvider,
    read_receipt,
    EvmReceiptReadRequest,
    EvmReceiptReadResponse,
    receipt_read_impl
);

fn capability_error_from_transport(error: EvmTransportError) -> EvmCapabilityError {
    match error {
        EvmTransportError::ReceiptPending => EvmCapabilityError::ReceiptPending,
        EvmTransportError::SourceMismatch { diagnostic } => {
            EvmCapabilityError::SourceMismatch { diagnostic }
        }
        other => EvmCapabilityError::provider_failure(other.into_provider_diagnostic()),
    }
}

fn source_evidence_error_into_transport(error: EvmCapabilityError) -> EvmTransportError {
    match error {
        EvmCapabilityError::SourceMismatch { diagnostic } => {
            EvmTransportError::SourceMismatch { diagnostic }
        }
        _ => EvmTransportError::InvalidResponse,
    }
}

fn block_selector_tag(selector: &EvmBlockSelector) -> TransportResult<String> {
    match selector {
        EvmBlockSelector::Latest => Ok("latest".to_owned()),
        EvmBlockSelector::Pending => Ok("pending".to_owned()),
        EvmBlockSelector::Number(number) => Ok(format!("0x{number:x}")),
        EvmBlockSelector::Hash(_) => Err(EvmTransportError::InvalidResponse),
    }
}

fn block_selector_param(selector: &EvmBlockSelector) -> Value {
    match selector {
        EvmBlockSelector::Latest => json!("latest"),
        EvmBlockSelector::Pending => json!("pending"),
        EvmBlockSelector::Number(number) => json!(format!("0x{number:x}")),
        EvmBlockSelector::Hash(hash) => json!({
            "blockHash": format!("{hash:?}"),
            "requireCanonical": true,
        }),
    }
}

fn parse_u64(raw: &str) -> TransportResult<u64> {
    parse_u256(raw)?
        .try_into()
        .map_err(|_| EvmTransportError::InvalidResponse)
}

fn parse_u128(raw: &str) -> TransportResult<u128> {
    parse_u256(raw)?
        .try_into()
        .map_err(|_| EvmTransportError::InvalidResponse)
}

fn parse_u256(raw: &str) -> TransportResult<U256> {
    let digits = raw
        .strip_prefix("0x")
        .ok_or(EvmTransportError::InvalidResponse)?;
    if digits.is_empty()
        || digits.len() > 64
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.as_bytes().iter().all(u8::is_ascii_hexdigit)
    {
        return Err(EvmTransportError::InvalidResponse);
    }
    U256::from_str_radix(digits, 16).map_err(|_| EvmTransportError::InvalidResponse)
}

fn encode_hex_bytes(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn decode_hex_bytes(raw: &str) -> TransportResult<Vec<u8>> {
    let digits = raw
        .strip_prefix("0x")
        .ok_or(EvmTransportError::InvalidResponse)?;
    if !digits.len().is_multiple_of(2) || !digits.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(EvmTransportError::InvalidResponse);
    }
    hex::decode(digits).map_err(|_| EvmTransportError::InvalidResponse)
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

fn validate_block_identity(
    selector: &EvmBlockSelector,
    block_number: u64,
    block_hash: B256,
) -> TransportResult<()> {
    match selector {
        EvmBlockSelector::Number(expected) if *expected != block_number => {
            Err(EvmTransportError::InvalidResponse)
        }
        EvmBlockSelector::Hash(expected) if *expected != block_hash => {
            Err(EvmTransportError::InvalidResponse)
        }
        _ => Ok(()),
    }
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
    /// Source evidence did not match the expected chain.
    #[error("EVM source evidence did not match expected chain")]
    SourceMismatch {
        /// Closed redacted source-mismatch diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
    /// JSON-RPC transport request failed before a protocol response was available.
    #[error("EVM JSON-RPC request failed")]
    TransportFailed {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
    },
    /// JSON-RPC endpoint returned a non-success HTTP status.
    #[error("EVM JSON-RPC HTTP status {status}")]
    RpcHttpStatus {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
        /// HTTP status code.
        status: u16,
    },
    /// JSON-RPC endpoint returned an error object.
    #[error("EVM JSON-RPC error {code}")]
    RpcJsonError {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
        /// JSON-RPC error code.
        code: i64,
    },
    /// JSON-RPC response failed contract validation.
    #[error("EVM JSON-RPC response was invalid")]
    InvalidResponse,
    /// JSON-RPC response did not contain a result.
    #[error("EVM JSON-RPC response missing result")]
    ResponseMissingResult {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
    },
    /// Transaction receipt is not yet available.
    #[error("EVM transaction receipt is pending")]
    ReceiptPending,
}

#[cfg(test)]
mod tests;
