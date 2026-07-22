//! Bounded, source-stable EVM JSON-RPC sessions.
//!
//! One transport owns the bounded HTTP pool and process-local concurrency
//! guards shared by every session it binds. Binding probes `eth_chainId` once
//! and returns a session fixed to that source for its entire lifetime. Code
//! and contract-call body limits are enforced while reading the response,
//! before JSON decoding. Routing and secret resolution stay in app assembly.
//!
//! ```no_run
//! use mfm_evm::EvmNetworkBinding;
//! use mfm_ids::LocalPublicId;
//! use mfm_evm_live::transport::{EvmJsonRpcTransport, EvmRpcEndpoint};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let binding = EvmNetworkBinding::new(LocalPublicId::new("reth-dev")?, 31337)?;
//! let transport = EvmJsonRpcTransport::new()?;
//! let _session = transport.bind(
//!     binding,
//!     LocalPublicId::new("local")?,
//!     EvmRpcEndpoint::new("http://127.0.0.1:8545")?,
//!     None,
//! ).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use alloy_eips::eip2930::{AccessList, AccessListItem};
use alloy_primitives::{keccak256, Address, Bytes, TxKind, B256, U256};
use mfm_capabilities::{
    ProviderDiagnosticCode, ProviderDiagnosticValue, RedactedProviderDiagnostic,
};
use mfm_evm::{
    evm_diagnostic, source_mismatch_error, EvmBlockAnchor, EvmBlockSelector, EvmCall,
    EvmCapabilityError, EvmCode, EvmFeeInputs, EvmInvalidRequest, EvmNetworkBinding,
    EvmObservedTransaction, EvmReadSession, EvmReadSessionSet, EvmReceipt, EvmReceiptLog,
    EvmReceiptStatus, EvmSessionEvidence, EvmSessionFuture, EvmTransactionEstimate,
    EvmTransactionPlacement, EvmTransactionSession, EvmTransactionSessionSet,
    EVM_CODE_MAX_RESPONSE_BYTES, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_ids::LocalPublicId;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_LENGTH};
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use tracing::debug;
use zeroize::Zeroizing;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_JSON_RPC_ENVELOPE_BYTES: usize = 16 * 1024;
const MAX_GLOBAL_IN_FLIGHT_EXCHANGES: usize = 64;
const MAX_IN_FLIGHT_EXCHANGES_PER_SOURCE: usize = 16;
const MAX_IDLE_CONNECTIONS_PER_HOST: usize = 16;
const JSON_RPC_ID: u64 = 1;

#[derive(Clone, Copy)]
enum RpcResponseBodyLimit {
    TransportMaximum,
    HexResult { maximum_decoded_bytes: usize },
}

impl RpcResponseBodyLimit {
    fn maximum_body_bytes(self) -> TransportResult<usize> {
        match self {
            Self::TransportMaximum => Ok(MAX_RESPONSE_BYTES),
            Self::HexResult {
                maximum_decoded_bytes,
            } => maximum_decoded_bytes
                .checked_mul(2)
                .and_then(|hex_bytes| hex_bytes.checked_add(2))
                .and_then(|result_bytes| result_bytes.checked_add(MAX_JSON_RPC_ENVELOPE_BYTES))
                .filter(|maximum| *maximum <= MAX_RESPONSE_BYTES)
                .ok_or(EvmTransportError::ResponseTooLarge),
        }
    }
}

/// Result type for EVM transport setup and exchange.
pub type TransportResult<T> = std::result::Result<T, EvmTransportError>;

/// Checked resolved endpoint for one EVM JSON-RPC source.
#[derive(Clone)]
pub struct EvmRpcEndpoint {
    url: reqwest::Url,
}

impl EvmRpcEndpoint {
    /// Admits one HTTP(S) endpoint without embedded credentials, query, or fragment material.
    pub fn new(endpoint: impl AsRef<str>) -> TransportResult<Self> {
        let url = reqwest::Url::parse(endpoint.as_ref())
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        Ok(Self { url })
    }
}

impl fmt::Debug for EvmRpcEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EvmRpcEndpoint(<redacted>)")
    }
}

/// Consumed resolved authorization header for one EVM JSON-RPC source.
pub struct EvmRpcAuthorization {
    value: Zeroizing<String>,
}

impl EvmRpcAuthorization {
    /// Admits one non-empty HTTP authorization header value.
    pub fn new(value: Zeroizing<String>) -> TransportResult<Self> {
        if value.is_empty() {
            return Err(EvmTransportError::InvalidConfiguration);
        }
        HeaderValue::from_str(value.as_str())
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        Ok(Self { value })
    }
}

struct BoundEndpoint {
    url: reqwest::Url,
    authorization: Option<EvmRpcAuthorization>,
    source_limit: Arc<Semaphore>,
}

/// Shared bounded HTTP runtime used to bind checked EVM JSON-RPC sessions.
///
/// Clones share the same connection pool, global exchange bound, and
/// `source_ref` exchange bounds. Redirects and implicit HTTP retries are
/// disabled, so one capability call owns exactly one HTTP exchange.
#[derive(Clone)]
pub struct EvmJsonRpcTransport {
    shared: Arc<SharedHttpRuntime>,
}

struct SharedHttpRuntime {
    client: reqwest::Client,
    global_limit: Arc<Semaphore>,
    source_limits: Mutex<BTreeMap<String, Weak<Semaphore>>>,
}

impl EvmJsonRpcTransport {
    /// Constructs the shared bounded HTTP runtime.
    pub fn new() -> TransportResult<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .pool_max_idle_per_host(MAX_IDLE_CONNECTIONS_PER_HOST)
            .build()
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        Ok(Self {
            shared: Arc::new(SharedHttpRuntime {
                client,
                global_limit: Arc::new(Semaphore::new(MAX_GLOBAL_IN_FLIGHT_EXCHANGES)),
                source_limits: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    /// Binds one source-stable session through this transport's shared pool.
    pub async fn bind(
        &self,
        binding: EvmNetworkBinding,
        source_ref: LocalPublicId,
        endpoint: EvmRpcEndpoint,
        authorization: Option<EvmRpcAuthorization>,
    ) -> TransportResult<EvmJsonRpcSession> {
        let source_limit = self.source_limit(&source_ref)?;
        let endpoint = BoundEndpoint {
            url: endpoint.url,
            authorization,
            source_limit,
        };
        let unbound = EvmJsonRpcSession {
            shared: Arc::clone(&self.shared),
            endpoint: Arc::new(endpoint),
            evidence: EvmSessionEvidence::new(
                &binding,
                source_ref.clone(),
                LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("static session implementation id"),
            ),
        };
        let observed_chain_id = unbound
            .rpc_call(
                "eth_chainId",
                json!([]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?
            .as_str()
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_quantity)?;
        if observed_chain_id != U256::from(binding.expected_chain_id()) {
            let EvmCapabilityError::SourceMismatch { diagnostic } =
                source_mismatch_error(&binding, observed_chain_id, &source_ref)
            else {
                return Err(EvmTransportError::InvalidResponse);
            };
            return Err(EvmTransportError::SourceMismatch { diagnostic });
        }
        Ok(unbound)
    }

    fn source_limit(&self, source_ref: &LocalPublicId) -> TransportResult<Arc<Semaphore>> {
        let mut limits = self
            .shared
            .source_limits
            .lock()
            .map_err(|_| EvmTransportError::InvalidConfiguration)?;
        if let Some(limit) = limits.get(source_ref.as_str()).and_then(Weak::upgrade) {
            return Ok(limit);
        }
        let limit = Arc::new(Semaphore::new(MAX_IN_FLIGHT_EXCHANGES_PER_SOURCE));
        limits.insert(source_ref.as_str().to_owned(), Arc::downgrade(&limit));
        Ok(limit)
    }
}

/// One checked JSON-RPC session bound to one source and chain.
#[derive(Clone)]
pub struct EvmJsonRpcSession {
    shared: Arc<SharedHttpRuntime>,
    endpoint: Arc<BoundEndpoint>,
    evidence: EvmSessionEvidence,
}

impl EvmJsonRpcSession {
    async fn block(&self, selector: &EvmBlockSelector) -> TransportResult<EvmBlockAnchor> {
        let value = match selector {
            EvmBlockSelector::ExactHash(hash) => {
                self.rpc_call(
                    "eth_getBlockByHash",
                    json!([format!("{hash:#x}"), false]),
                    RpcResponseBodyLimit::TransportMaximum,
                )
                .await?
            }
            _ => {
                self.rpc_call(
                    "eth_getBlockByNumber",
                    json!([selector_tag(selector)?, false]),
                    RpcResponseBodyLimit::TransportMaximum,
                )
                .await?
            }
        };
        let object = required_object(&value)?;
        let number = quantity_field(object, "number")?;
        let hash = hash_field(object, "hash")?;
        match selector {
            EvmBlockSelector::Number(expected) if *expected != number => {
                Err(EvmTransportError::InvalidResponse)
            }
            EvmBlockSelector::ExactHash(expected) if *expected != hash => {
                Err(EvmTransportError::InvalidResponse)
            }
            _ => Ok(EvmBlockAnchor::new(number, hash)),
        }
    }

    async fn balance(&self, account: Address, block: &EvmBlockSelector) -> TransportResult<U256> {
        let value = self
            .rpc_call(
                "eth_getBalance",
                json!([format!("{account:#x}"), selector_param(block)]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?;
        parse_quantity(value.as_str().ok_or(EvmTransportError::InvalidResponse)?)
    }

    async fn code(&self, address: Address, block: &EvmBlockSelector) -> TransportResult<EvmCode> {
        let value = self
            .rpc_call(
                "eth_getCode",
                json!([format!("{address:#x}"), selector_param(block)]),
                RpcResponseBodyLimit::HexResult {
                    maximum_decoded_bytes: EVM_CODE_MAX_RESPONSE_BYTES,
                },
            )
            .await?;
        let bytes = parse_bounded_bytes(
            value.as_str().ok_or(EvmTransportError::InvalidResponse)?,
            EVM_CODE_MAX_RESPONSE_BYTES,
        )?;
        Ok(EvmCode {
            hash: keccak256(&bytes),
            bytes,
        })
    }

    async fn call_contract(&self, request: &EvmCall) -> TransportResult<Bytes> {
        let value = self
            .rpc_call(
                "eth_call",
                json!([{
                    "from": format!("{:#x}", request.from()),
                    "to": format!("{:#x}", request.to()),
                    "value": encode_quantity(request.value()),
                    "data": encode_bytes(request.input()),
                    "gas": encode_quantity(request.gas_limit()),
                    "accessList": encode_access_list(request.access_list()),
                }, selector_param(request.block())]),
                RpcResponseBodyLimit::HexResult {
                    maximum_decoded_bytes: request.max_response_bytes(),
                },
            )
            .await?;
        parse_bounded_bytes(
            value.as_str().ok_or(EvmTransportError::InvalidResponse)?,
            request.max_response_bytes(),
        )
    }

    async fn pending_nonce_value(&self, account: Address) -> TransportResult<U256> {
        let value = self
            .rpc_call(
                "eth_getTransactionCount",
                json!([format!("{account:#x}"), "pending"]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?;
        parse_quantity(value.as_str().ok_or(EvmTransportError::InvalidResponse)?)
    }

    async fn fee_input_values(&self) -> TransportResult<EvmFeeInputs> {
        let priority = self
            .rpc_call(
                "eth_maxPriorityFeePerGas",
                json!([]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?
            .as_str()
            .ok_or(EvmTransportError::InvalidResponse)
            .and_then(parse_quantity)?;
        let latest = self
            .rpc_call(
                "eth_getBlockByNumber",
                json!(["latest", false]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?;
        let base = quantity_field(required_object(&latest)?, "baseFeePerGas")?;
        EvmFeeInputs::from_base_and_priority(base, priority)
            .map_err(|_| EvmTransportError::InvalidResponse)
    }

    async fn estimate(&self, request: &EvmTransactionEstimate) -> TransportResult<U256> {
        let mut call = Map::new();
        call.insert(
            "type".to_owned(),
            json!(encode_quantity(U256::from(request.transaction_type()))),
        );
        call.insert(
            "chainId".to_owned(),
            json!(encode_quantity(request.chain_id())),
        );
        call.insert("nonce".to_owned(), json!(encode_quantity(request.nonce())));
        call.insert("from".to_owned(), json!(format!("{:#x}", request.from())));
        if let TxKind::Call(to) = request.to() {
            call.insert("to".to_owned(), json!(format!("{to:#x}")));
        }
        call.insert("value".to_owned(), json!(encode_quantity(request.value())));
        call.insert("data".to_owned(), json!(encode_bytes(request.input())));
        call.insert(
            "accessList".to_owned(),
            encode_access_list(request.access_list()),
        );
        call.insert(
            "maxFeePerGas".to_owned(),
            json!(encode_quantity(request.max_fee_per_gas())),
        );
        call.insert(
            "maxPriorityFeePerGas".to_owned(),
            json!(encode_quantity(request.max_priority_fee_per_gas())),
        );
        let value = self
            .rpc_call(
                "eth_estimateGas",
                json!([Value::Object(call), "pending"]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?;
        parse_quantity(value.as_str().ok_or(EvmTransportError::InvalidResponse)?)
    }

    async fn submit(&self, signed_bytes: &[u8], expected_hash: B256) -> TransportResult<B256> {
        if signed_bytes.is_empty() || keccak256(signed_bytes) != expected_hash {
            return Err(EvmTransportError::InvalidRequest);
        }
        let value = self
            .rpc_call(
                "eth_sendRawTransaction",
                json!([encode_bytes(signed_bytes)]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?;
        parse_hash(value.as_str().ok_or(EvmTransportError::InvalidResponse)?)
    }

    async fn transaction(
        &self,
        transaction_hash: B256,
    ) -> TransportResult<Option<EvmObservedTransaction>> {
        let value = self
            .rpc_call(
                "eth_getTransactionByHash",
                json!([format!("{transaction_hash:#x}")]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        let object = required_object(&value)?;
        let observed_hash = hash_field(object, "hash")?;
        if observed_hash != transaction_hash || quantity_field(object, "type")? != U256::from(2) {
            return Err(EvmTransportError::InvalidResponse);
        }
        let placement = transaction_placement(object)?;
        Ok(Some(EvmObservedTransaction {
            transaction_hash: observed_hash,
            chain_id: quantity_field(object, "chainId")?,
            nonce: quantity_field(object, "nonce")?,
            from: address_field(object, "from")?,
            to: optional_address_field(object, "to")?.map_or(TxKind::Create, TxKind::Call),
            value: quantity_field(object, "value")?,
            input: bytes_field(object, "input")?,
            gas_limit: quantity_field(object, "gas")?,
            max_fee_per_gas: quantity_field(object, "maxFeePerGas")?,
            max_priority_fee_per_gas: quantity_field(object, "maxPriorityFeePerGas")?,
            access_list: access_list_field(object, "accessList")?,
            placement,
        }))
    }

    async fn receipt(&self, transaction_hash: B256) -> TransportResult<Option<EvmReceipt>> {
        let value = self
            .rpc_call(
                "eth_getTransactionReceipt",
                json!([format!("{transaction_hash:#x}")]),
                RpcResponseBodyLimit::TransportMaximum,
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        let object = required_object(&value)?;
        let observed_hash = hash_field(object, "transactionHash")?;
        if observed_hash != transaction_hash {
            return Err(EvmTransportError::InvalidResponse);
        }
        let status = match quantity_field(object, "status")? {
            value if value == U256::ZERO => EvmReceiptStatus::Reverted,
            value if value == U256::from(1) => EvmReceiptStatus::Success,
            _ => return Err(EvmTransportError::InvalidResponse),
        };
        let logs = object
            .get("logs")
            .and_then(Value::as_array)
            .ok_or(EvmTransportError::InvalidResponse)?
            .iter()
            .map(parse_receipt_log)
            .collect::<TransportResult<Vec<_>>>()?;
        let receipt = EvmReceipt {
            transaction_hash: observed_hash,
            transaction_index: quantity_field(object, "transactionIndex")?,
            block: EvmBlockAnchor::new(
                quantity_field(object, "blockNumber")?,
                hash_field(object, "blockHash")?,
            ),
            from: address_field(object, "from")?,
            to: optional_address_field(object, "to")?,
            contract_address: optional_address_field(object, "contractAddress")?,
            status,
            gas_used: quantity_field(object, "gasUsed")?,
            cumulative_gas_used: quantity_field(object, "cumulativeGasUsed")?,
            logs,
        };
        receipt
            .validate()
            .map_err(|_| EvmTransportError::InvalidResponse)?;
        Ok(Some(receipt))
    }

    async fn rpc_call(
        &self,
        method: &'static str,
        params: Value,
        response_limit: RpcResponseBodyLimit,
    ) -> TransportResult<Value> {
        let maximum_body_bytes = response_limit.maximum_body_bytes()?;
        let operation = operation_id(method);
        let _source_permit = Arc::clone(&self.endpoint.source_limit)
            .acquire_owned()
            .await
            .map_err(|_| EvmTransportError::TransportFailed {
                operation: operation.clone(),
            })?;
        let _global_permit = Arc::clone(&self.shared.global_limit)
            .acquire_owned()
            .await
            .map_err(|_| EvmTransportError::TransportFailed {
                operation: operation.clone(),
            })?;
        let mut request = self
            .shared
            .client
            .post(self.endpoint.url.clone())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": JSON_RPC_ID,
                "method": method,
                "params": params,
            }));
        if let Some(authorization) = &self.endpoint.authorization {
            let authorization = HeaderValue::from_str(authorization.value.as_str())
                .map_err(|_| EvmTransportError::InvalidConfiguration)?;
            request = request.header(AUTHORIZATION, authorization);
        }
        debug!(operation = %operation, "evm rpc request");
        let mut response =
            request
                .send()
                .await
                .map_err(|_| EvmTransportError::TransportFailed {
                    operation: operation.clone(),
                })?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(EvmTransportError::RpcHttpStatus { operation, status });
        }
        if response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            .is_some_and(|length| length > maximum_body_bytes)
        {
            return Err(EvmTransportError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|_| EvmTransportError::TransportFailed {
                    operation: operation.clone(),
                })?
        {
            let next_len = bytes
                .len()
                .checked_add(chunk.len())
                .ok_or(EvmTransportError::ResponseTooLarge)?;
            if next_len > maximum_body_bytes {
                return Err(EvmTransportError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value =
            serde_json::from_slice(&bytes).map_err(|_| EvmTransportError::InvalidResponse)?;
        validate_rpc_response(body, operation)
    }
}

impl EvmReadSession for EvmJsonRpcSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(
        &'a self,
        selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        Box::pin(async move { self.block(selector).await.map_err(capability_error) })
    }

    fn read_balance<'a>(
        &'a self,
        account: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        Box::pin(async move { self.balance(account, block).await.map_err(capability_error) })
    }

    fn read_code<'a>(
        &'a self,
        address: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        Box::pin(async move { self.code(address, block).await.map_err(capability_error) })
    }

    fn call<'a>(&'a self, request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        Box::pin(async move { self.call_contract(request).await.map_err(capability_error) })
    }
}

impl EvmTransactionSession for EvmJsonRpcSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn pending_nonce<'a>(&'a self, account: Address) -> EvmSessionFuture<'a, U256> {
        Box::pin(async move {
            self.pending_nonce_value(account)
                .await
                .map_err(capability_error)
        })
    }

    fn fee_inputs(&self) -> EvmSessionFuture<'_, EvmFeeInputs> {
        Box::pin(async move { self.fee_input_values().await.map_err(capability_error) })
    }

    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmTransactionEstimate,
    ) -> EvmSessionFuture<'a, U256> {
        Box::pin(async move { self.estimate(request).await.map_err(capability_error) })
    }

    fn submit_raw_transaction<'a>(
        &'a self,
        signed_bytes: &'a [u8],
        expected_hash: B256,
    ) -> EvmSessionFuture<'a, B256> {
        Box::pin(async move {
            self.submit(signed_bytes, expected_hash)
                .await
                .map_err(capability_error)
        })
    }

    fn transaction_by_hash(
        &self,
        transaction_hash: B256,
    ) -> EvmSessionFuture<'_, Option<EvmObservedTransaction>> {
        Box::pin(async move {
            self.transaction(transaction_hash)
                .await
                .map_err(capability_error)
        })
    }

    fn receipt_by_hash(&self, transaction_hash: B256) -> EvmSessionFuture<'_, Option<EvmReceipt>> {
        Box::pin(async move {
            self.receipt(transaction_hash)
                .await
                .map_err(capability_error)
        })
    }

    fn read_block<'a>(
        &'a self,
        selector: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmBlockAnchor> {
        Box::pin(async move { self.block(selector).await.map_err(capability_error) })
    }
}

impl EvmReadSessionSet for EvmJsonRpcSession {
    fn implementation_id(&self) -> &str {
        self.evidence.implementation_id()
    }

    fn validate_binding<'a>(&'a self, binding: &'a EvmNetworkBinding) -> EvmSessionFuture<'a, ()> {
        Box::pin(async move { self.validate_singleton_binding(binding) })
    }

    fn session<'a>(
        &'a self,
        binding: &'a EvmNetworkBinding,
    ) -> EvmSessionFuture<'a, Arc<dyn EvmReadSession>> {
        Box::pin(async move {
            self.validate_singleton_binding(binding)?;
            Ok(Arc::new(self.clone()) as Arc<dyn EvmReadSession>)
        })
    }
}

impl EvmTransactionSessionSet for EvmJsonRpcSession {
    fn implementation_id(&self) -> &str {
        self.evidence.implementation_id()
    }

    fn session<'a>(
        &'a self,
        binding: &'a EvmNetworkBinding,
    ) -> EvmSessionFuture<'a, Arc<dyn EvmTransactionSession>> {
        Box::pin(async move {
            self.validate_singleton_binding(binding)?;
            Ok(Arc::new(self.clone()) as Arc<dyn EvmTransactionSession>)
        })
    }
}

impl EvmJsonRpcSession {
    fn validate_singleton_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<()> {
        if self.evidence.matches_binding(binding)
            && self.evidence.implementation_id() == EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
        {
            Ok(())
        } else {
            Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::SessionAuthorityMismatch,
            })
        }
    }
}

impl fmt::Debug for EvmJsonRpcSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmJsonRpcSession")
            .field("endpoint", &"<redacted>")
            .field("evidence", &self.evidence)
            .finish()
    }
}

fn validate_rpc_response(mut body: Value, operation: LocalPublicId) -> TransportResult<Value> {
    let object = body
        .as_object_mut()
        .ok_or(EvmTransportError::InvalidResponse)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || object.get("id").and_then(Value::as_u64) != Some(JSON_RPC_ID)
    {
        return Err(EvmTransportError::InvalidResponse);
    }
    match (object.contains_key("result"), object.get("error")) {
        (true, None) => object
            .remove("result")
            .ok_or(EvmTransportError::InvalidResponse),
        (false, Some(error)) => {
            let code = error
                .as_object()
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64)
                .ok_or(EvmTransportError::InvalidResponse)?;
            Err(EvmTransportError::RpcJsonError { operation, code })
        }
        (false, None) => Err(EvmTransportError::ResponseMissingResult { operation }),
        (true, Some(_)) => Err(EvmTransportError::InvalidResponse),
    }
}

fn selector_tag(selector: &EvmBlockSelector) -> TransportResult<String> {
    match selector {
        EvmBlockSelector::Latest => Ok("latest".to_owned()),
        EvmBlockSelector::Number(number) => Ok(encode_quantity(*number)),
        EvmBlockSelector::ExactHash(_) => Err(EvmTransportError::InvalidRequest),
    }
}

fn selector_param(selector: &EvmBlockSelector) -> Value {
    match selector {
        EvmBlockSelector::Latest => json!("latest"),
        EvmBlockSelector::Number(number) => json!(encode_quantity(*number)),
        EvmBlockSelector::ExactHash(hash) => json!({
            "blockHash": format!("{hash:#x}"),
            "requireCanonical": true,
        }),
    }
}

fn encode_quantity(value: U256) -> String {
    format!("0x{value:x}")
}

fn parse_quantity(raw: &str) -> TransportResult<U256> {
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

fn encode_bytes(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn parse_bytes(raw: &str) -> TransportResult<Bytes> {
    let digits = raw
        .strip_prefix("0x")
        .ok_or(EvmTransportError::InvalidResponse)?;
    if !digits.len().is_multiple_of(2) || !digits.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(EvmTransportError::InvalidResponse);
    }
    hex::decode(digits)
        .map(Bytes::from)
        .map_err(|_| EvmTransportError::InvalidResponse)
}

fn parse_bounded_bytes(raw: &str, maximum: usize) -> TransportResult<Bytes> {
    let digits = raw
        .strip_prefix("0x")
        .ok_or(EvmTransportError::InvalidResponse)?;
    if !digits.len().is_multiple_of(2) {
        return Err(EvmTransportError::InvalidResponse);
    }
    if digits.len() / 2 > maximum {
        return Err(EvmTransportError::ResponseTooLarge);
    }
    parse_bytes(raw)
}

fn parse_hash(raw: &str) -> TransportResult<B256> {
    if raw.len() != 66 || !raw.starts_with("0x") {
        return Err(EvmTransportError::InvalidResponse);
    }
    raw.parse().map_err(|_| EvmTransportError::InvalidResponse)
}

fn parse_address(raw: &str) -> TransportResult<Address> {
    if raw.len() != 42 || !raw.starts_with("0x") {
        return Err(EvmTransportError::InvalidResponse);
    }
    raw.parse().map_err(|_| EvmTransportError::InvalidResponse)
}

fn required_object(value: &Value) -> TransportResult<&Map<String, Value>> {
    value.as_object().ok_or(EvmTransportError::InvalidResponse)
}

fn string_field<'a>(object: &'a Map<String, Value>, field: &str) -> TransportResult<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(EvmTransportError::InvalidResponse)
}

fn quantity_field(object: &Map<String, Value>, field: &str) -> TransportResult<U256> {
    parse_quantity(string_field(object, field)?)
}

fn hash_field(object: &Map<String, Value>, field: &str) -> TransportResult<B256> {
    parse_hash(string_field(object, field)?)
}

fn address_field(object: &Map<String, Value>, field: &str) -> TransportResult<Address> {
    parse_address(string_field(object, field)?)
}

fn bytes_field(object: &Map<String, Value>, field: &str) -> TransportResult<Bytes> {
    parse_bytes(string_field(object, field)?)
}

fn optional_address_field(
    object: &Map<String, Value>,
    field: &str,
) -> TransportResult<Option<Address>> {
    let value = object
        .get(field)
        .ok_or(EvmTransportError::InvalidResponse)?;
    if value.is_null() {
        Ok(None)
    } else {
        parse_address(value.as_str().ok_or(EvmTransportError::InvalidResponse)?).map(Some)
    }
}

fn encode_access_list(access_list: &AccessList) -> Value {
    Value::Array(
        access_list
            .iter()
            .map(|item| {
                json!({
                    "address": format!("{:#x}", item.address),
                    "storageKeys": item.storage_keys.iter().map(|key| format!("{key:#x}")).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn access_list_field(object: &Map<String, Value>, field: &str) -> TransportResult<AccessList> {
    let items = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or(EvmTransportError::InvalidResponse)?;
    items
        .iter()
        .map(|item| {
            let item = required_object(item)?;
            let storage_keys = item
                .get("storageKeys")
                .and_then(Value::as_array)
                .ok_or(EvmTransportError::InvalidResponse)?
                .iter()
                .map(|key| parse_hash(key.as_str().ok_or(EvmTransportError::InvalidResponse)?))
                .collect::<TransportResult<Vec<_>>>()?;
            Ok(AccessListItem {
                address: address_field(item, "address")?,
                storage_keys,
            })
        })
        .collect::<TransportResult<Vec<_>>>()
        .map(AccessList)
}

fn transaction_placement(
    object: &Map<String, Value>,
) -> TransportResult<Option<EvmTransactionPlacement>> {
    let block_number = object
        .get("blockNumber")
        .ok_or(EvmTransportError::InvalidResponse)?;
    let block_hash = object
        .get("blockHash")
        .ok_or(EvmTransportError::InvalidResponse)?;
    let transaction_index = object
        .get("transactionIndex")
        .ok_or(EvmTransportError::InvalidResponse)?;
    if block_number.is_null() && block_hash.is_null() && transaction_index.is_null() {
        return Ok(None);
    }
    if block_number.is_null() || block_hash.is_null() || transaction_index.is_null() {
        return Err(EvmTransportError::InvalidResponse);
    }
    Ok(Some(EvmTransactionPlacement {
        block: EvmBlockAnchor::new(
            parse_quantity(
                block_number
                    .as_str()
                    .ok_or(EvmTransportError::InvalidResponse)?,
            )?,
            parse_hash(
                block_hash
                    .as_str()
                    .ok_or(EvmTransportError::InvalidResponse)?,
            )?,
        ),
        transaction_index: parse_quantity(
            transaction_index
                .as_str()
                .ok_or(EvmTransportError::InvalidResponse)?,
        )?,
    }))
}

fn parse_receipt_log(value: &Value) -> TransportResult<EvmReceiptLog> {
    let object = required_object(value)?;
    let topics = object
        .get("topics")
        .and_then(Value::as_array)
        .ok_or(EvmTransportError::InvalidResponse)?
        .iter()
        .map(|topic| parse_hash(topic.as_str().ok_or(EvmTransportError::InvalidResponse)?))
        .collect::<TransportResult<Vec<_>>>()?;
    Ok(EvmReceiptLog {
        address: address_field(object, "address")?,
        topics,
        data: bytes_field(object, "data")?,
        block: EvmBlockAnchor::new(
            quantity_field(object, "blockNumber")?,
            hash_field(object, "blockHash")?,
        ),
        transaction_hash: hash_field(object, "transactionHash")?,
        transaction_index: quantity_field(object, "transactionIndex")?,
        log_index: quantity_field(object, "logIndex")?,
        removed: object
            .get("removed")
            .and_then(Value::as_bool)
            .ok_or(EvmTransportError::InvalidResponse)?,
    })
}

fn capability_error(error: EvmTransportError) -> EvmCapabilityError {
    match error {
        EvmTransportError::SourceMismatch { diagnostic } => {
            EvmCapabilityError::SourceMismatch { diagnostic }
        }
        other => EvmCapabilityError::provider_failure(other.into_provider_diagnostic()),
    }
}

fn operation_id(method: &'static str) -> LocalPublicId {
    let value = match method {
        "eth_chainId" => "eth_chain_id",
        "eth_getBlockByHash" => "eth_get_block_by_hash",
        "eth_getBlockByNumber" => "eth_get_block_by_number",
        "eth_getBalance" => "eth_get_balance",
        "eth_call" => "eth_call",
        "eth_getCode" => "eth_get_code",
        "eth_getTransactionCount" => "eth_get_transaction_count",
        "eth_maxPriorityFeePerGas" => "eth_max_priority_fee_per_gas",
        "eth_estimateGas" => "eth_estimate_gas",
        "eth_sendRawTransaction" => "eth_send_raw_transaction",
        "eth_getTransactionByHash" => "eth_get_transaction_by_hash",
        "eth_getTransactionReceipt" => "eth_get_transaction_receipt",
        _ => "evm_rpc",
    };
    diagnostic_id(value)
}

fn diagnostic_id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("static EVM diagnostic id")
}

/// Redaction-safe EVM transport failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmTransportError {
    /// Session configuration was invalid.
    #[error("EVM JSON-RPC session configuration was invalid")]
    InvalidConfiguration,
    /// A caller supplied invalid transient material.
    #[error("EVM JSON-RPC request material was invalid")]
    InvalidRequest,
    /// Bind-time chain identity did not match the semantic binding.
    #[error("EVM JSON-RPC source did not match semantic binding")]
    SourceMismatch {
        /// Closed mismatch diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
    /// HTTP exchange failed before a protocol response was available.
    #[error("EVM JSON-RPC request failed")]
    TransportFailed {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
    },
    /// Endpoint returned a non-success HTTP status.
    #[error("EVM JSON-RPC HTTP status {status}")]
    RpcHttpStatus {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
        /// HTTP status code.
        status: u16,
    },
    /// Endpoint returned a JSON-RPC error object.
    #[error("EVM JSON-RPC error {code}")]
    RpcJsonError {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
        /// JSON-RPC error code.
        code: i64,
    },
    /// Response violated the strict protocol contract.
    #[error("EVM JSON-RPC response was invalid")]
    InvalidResponse,
    /// Response omitted both result and error.
    #[error("EVM JSON-RPC response missing result")]
    ResponseMissingResult {
        /// Redaction-safe operation id.
        operation: LocalPublicId,
    },
    /// Response exceeded the configured HTTP-body or decoded method-result bound.
    #[error("EVM JSON-RPC response exceeded size bound")]
    ResponseTooLarge,
}

impl EvmTransportError {
    /// Converts into one closed redacted provider diagnostic.
    pub fn into_provider_diagnostic(self) -> RedactedProviderDiagnostic {
        match self {
            Self::InvalidConfiguration => {
                evm_diagnostic(ProviderDiagnosticCode::ProviderConfigurationInvalid)
            }
            Self::InvalidRequest | Self::InvalidResponse | Self::ResponseTooLarge => {
                evm_diagnostic(ProviderDiagnosticCode::ResponseInvalid)
            }
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
            Self::ResponseMissingResult { operation } => {
                evm_diagnostic(ProviderDiagnosticCode::ResponseMissingResult)
                    .with_operation(operation)
            }
        }
    }
}

#[cfg(test)]
mod tests;
