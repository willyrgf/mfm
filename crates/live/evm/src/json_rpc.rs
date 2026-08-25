//! Production JSON-RPC 2.0 provider for bounded EVM Reads and transactions.
//!
//! The provider owns exactly one endpoint URL and the bounded Read and transaction methods the EVM
//! domain contracts require. It holds no key, nonce authority, or automatic retry.

use std::future::Future;
use std::num::NonZeroU64;
use std::pin::Pin;
#[cfg(test)]
use std::str::FromStr;
use std::time::Duration;

use alloy_primitives::{hex, Address, U256};
use mfm_canonical::CanonicalBytes;
use mfm_evm::{
    AnchoredContractCallResult, EvmBalanceSource, EvmBlockAnchor, EvmChainInstance, EvmHash,
    EvmReadIntent, EvmReadSubject, EvmReadValue, EvmU256, MAX_EVM_CALL_RETURN_BYTES,
};
use mfm_evm_transaction_authority::ExactRawTransaction;
use mfm_ids::StableId;
use mfm_runtime::AdapterError;
use serde::Serialize;
use url::Url;

use crate::{
    EvmProvider, EvmProviderResponse, EvmTransactionProvider, EvmTransactionProviderFuture,
    ProviderReceipt, ProviderReceiptResult,
};

/// Largest admitted JSON-RPC response body.
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
/// Complete per-request deadline covering connect, send, and body.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest admitted private EVM adapter locator.
pub const MAX_EVM_ADAPTER_LOCATOR_BYTES: usize = 16 * 1024;
/// ERC-20 `decimals()` selector.
const DECIMALS_SELECTOR: &str = "313ce567";
/// ERC-20 `balanceOf(address)` selector.
const BALANCE_OF_SELECTOR: &str = "70a08231";
/// Maximum deployed bytecode admitted before an anchored call.
const MAX_EVM_CONTRACT_CODE_BYTES: usize = 24_576;

/// Reviewed redaction-safe provider construction failure.
///
/// The RPC URL is a credential-bearing handle, so no construction failure ever names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("evm provider transport could not be constructed")]
pub struct EvmProviderBuildError;

/// Private EVM connection locator.
///
/// This value deliberately implements neither `Debug`, `Display`, nor serialization.
pub struct EvmAdapterLocator {
    url: Url,
}

impl EvmAdapterLocator {
    /// Parses one bounded HTTP(S) locator.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, EvmProviderBuildError> {
        let value = value.as_ref();
        if value.is_empty()
            || value.len() > MAX_EVM_ADAPTER_LOCATOR_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(EvmProviderBuildError);
        }
        let url = Url::parse(value).map_err(|_| EvmProviderBuildError)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.cannot_be_a_base()
            || !url.has_host()
            || url.fragment().is_some()
        {
            return Err(EvmProviderBuildError);
        }
        Ok(Self { url })
    }
}

/// Bounded JSON-RPC EVM provider bound to one endpoint URL.
pub struct JsonRpcEvmProvider {
    url: reqwest::Url,
    http: reqwest::Client,
}

impl JsonRpcEvmProvider {
    /// Builds one provider from its endpoint locator.
    ///
    /// The URL stays inside this provider and appears in no intent, evidence, log, or error.
    pub fn connect(locator: &EvmAdapterLocator) -> Result<Self, EvmProviderBuildError> {
        Ok(Self {
            url: locator.url.clone(),
            http: http_client()?,
        })
    }

    #[cfg(test)]
    fn new_http_for_test(url: String) -> Result<Self, EvmProviderBuildError> {
        let url = reqwest::Url::parse(&url).map_err(|_| EvmProviderBuildError)?;
        Ok(Self {
            url,
            http: http_client()?,
        })
    }

    /// Performs one bounded exact-envelope JSON-RPC call.
    async fn call_outcome(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcOutcome, AdapterError> {
        let response = self
            .http
            .post(self.url.clone())
            .json(&JsonRpcRequest {
                jsonrpc: "2.0",
                id: 1,
                method,
                params,
            })
            .send()
            .await
            .map_err(|_| AdapterError::Unavailable)?;
        if !response.status().is_success() {
            return Err(AdapterError::Unavailable);
        }
        let body = bounded_body(response).await?;
        let envelope: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| AdapterError::Unavailable)?;
        let object = envelope.as_object().ok_or(AdapterError::Unavailable)?;
        if object.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0")
            || object.get("id").and_then(serde_json::Value::as_u64) != Some(1)
            || object
                .keys()
                .any(|key| !matches!(key.as_str(), "jsonrpc" | "id" | "result" | "error"))
        {
            return Err(AdapterError::Unavailable);
        }
        match (object.get("result"), object.get("error")) {
            (Some(result), None) => Ok(RpcOutcome::Result(result.clone())),
            (None, Some(error)) if valid_rpc_error(error) => Ok(RpcOutcome::Error),
            _ => Err(AdapterError::Unavailable),
        }
    }

    /// Preserves the existing Read policy where an RPC error object is a reviewed safe failure.
    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, AdapterError> {
        match self.call_outcome(method, params).await? {
            RpcOutcome::Result(result) => Ok(Some(result)),
            RpcOutcome::Error => Ok(None),
        }
    }

    /// Uses the transaction/anchored policy where every RPC error object is unavailable.
    async fn call_strict(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        match self.call_outcome(method, params).await? {
            RpcOutcome::Result(result) => Ok(result),
            RpcOutcome::Error => Err(AdapterError::Unavailable),
        }
    }

    /// Reads one block anchor by the supplied block tag.
    async fn anchor(&self, tag: serde_json::Value) -> Result<EvmProviderResponse, AdapterError> {
        let Some(block) = self
            .call("eth_getBlockByNumber", serde_json::json!([tag, false]))
            .await?
        else {
            return Ok(EvmProviderResponse::SafeFailure);
        };
        let number = block
            .get("number")
            .and_then(serde_json::Value::as_str)
            .and_then(quantity_to_decimal)
            .and_then(|number| EvmU256::new(number).ok())
            .ok_or(AdapterError::Unavailable)?;
        let hash = block
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .and_then(|hash| EvmHash::new(hash.to_owned()).ok())
            .ok_or(AdapterError::Unavailable)?;
        Ok(returned(EvmReadValue::Anchor(EvmBlockAnchor::new(
            number, hash,
        ))))
    }

    /// Performs one anchored `eth_call` and returns its result word.
    ///
    /// `Ok(None)` is a definite safe failure: a reviewed revert or a codeless address.
    async fn contract_call(
        &self,
        source: &EvmBalanceSource,
        anchor: &EvmBlockAnchor,
        data: String,
    ) -> Result<Option<AbiWord>, AdapterError> {
        let token = source.token().ok_or(AdapterError::Internal)?;
        let to = token.to_string();
        let tag = block_tag(anchor.number())?;
        let Some(result) = self
            .call(
                "eth_call",
                serde_json::json!([{ "to": to, "data": data }, tag]),
            )
            .await?
        else {
            return Ok(None);
        };
        let data = RpcData::parse(result.as_str().ok_or(AdapterError::Unavailable)?, 32)?;
        if data.0.is_empty() {
            return Ok(None);
        }
        AbiWord::try_from(data).map(Some)
    }

    /// Routes one checked Read subject to its exact RPC call.
    async fn observe(&self, subject: &EvmReadSubject) -> Result<EvmProviderResponse, AdapterError> {
        match subject {
            EvmReadSubject::ChainIdentity => {
                let Some(result) = self.call("eth_chainId", serde_json::json!([])).await? else {
                    return Ok(EvmProviderResponse::SafeFailure);
                };
                let chain_id = result
                    .as_str()
                    .and_then(quantity_to_u64)
                    .and_then(NonZeroU64::new)
                    .ok_or(AdapterError::Unavailable)?;
                Ok(returned(EvmReadValue::ChainId(chain_id)))
            }
            EvmReadSubject::InitialAnchor => self.anchor(serde_json::json!("latest")).await,
            EvmReadSubject::NativeBalance { source, anchor } => {
                let address = source.address().to_string();
                let tag = block_tag(anchor.number())?;
                let Some(result) = self
                    .call("eth_getBalance", serde_json::json!([address, tag]))
                    .await?
                else {
                    return Ok(EvmProviderResponse::SafeFailure);
                };
                let units = result
                    .as_str()
                    .and_then(quantity_to_decimal)
                    .ok_or(AdapterError::Unavailable)?;
                Ok(returned(EvmReadValue::RawUnits(
                    EvmU256::new(units).map_err(|_| AdapterError::Unavailable)?,
                )))
            }
            EvmReadSubject::TokenDecimals { source, anchor } => {
                let Some(word) = self
                    .contract_call(source, anchor, decimals_calldata())
                    .await?
                else {
                    return Ok(EvmProviderResponse::SafeFailure);
                };
                let decimals = word_to_u8(word).ok_or(AdapterError::Unavailable)?;
                Ok(returned(EvmReadValue::TokenDecimals(decimals)))
            }
            EvmReadSubject::TokenBalance { source, anchor } => {
                let holder = Address::from(*source.address().as_bytes());
                let Some(word) = self
                    .contract_call(source, anchor, balance_of_calldata(&holder))
                    .await?
                else {
                    return Ok(EvmProviderResponse::SafeFailure);
                };
                let units = decode_abi_u256(word).to_string();
                Ok(returned(EvmReadValue::RawUnits(
                    EvmU256::new(units).map_err(|_| AdapterError::Unavailable)?,
                )))
            }
            // Confirmation reads the committed number, never the moving head.
            EvmReadSubject::ConfirmAnchor { anchor, .. } => {
                let tag = block_tag(anchor.number())?;
                self.anchor(serde_json::json!(tag)).await
            }
            EvmReadSubject::AnchoredContractCall {
                anchor,
                calldata,
                target,
            } => self.anchored_contract_call(anchor, calldata, target).await,
        }
    }

    async fn anchored_contract_call(
        &self,
        authored_anchor: &EvmBlockAnchor,
        calldata: &CanonicalBytes,
        target: &mfm_evm::EvmAddress,
    ) -> Result<EvmProviderResponse, AdapterError> {
        let Some(observed_anchor) = self
            .strict_block_anchor(block_tag(authored_anchor.number())?)
            .await?
        else {
            return Ok(EvmProviderResponse::SafeFailure);
        };
        if &observed_anchor != authored_anchor {
            return Ok(EvmProviderResponse::IntegrityBlocked);
        }
        let tag = serde_json::json!({
            "blockHash": authored_anchor.hash().to_string(),
            "requireCanonical": true
        });
        let code = self
            .call_strict(
                "eth_getCode",
                serde_json::json!([target.to_string(), tag.clone()]),
            )
            .await?;
        let code = RpcData::parse(
            code.as_str().ok_or(AdapterError::Unavailable)?,
            MAX_EVM_CONTRACT_CODE_BYTES,
        )?;
        if code.0.is_empty() {
            return Ok(EvmProviderResponse::Rejected);
        }
        let result = self
            .call_strict(
                "eth_call",
                serde_json::json!([{
                    "to": target.to_string(),
                    "data": format!("0x{}", hex::encode(calldata.as_bytes())),
                }, tag]),
            )
            .await?;
        let return_bytes = RpcData::parse(
            result.as_str().ok_or(AdapterError::Unavailable)?,
            MAX_EVM_CALL_RETURN_BYTES,
        )?
        .0;
        let Some(confirmed_anchor) = self
            .strict_block_anchor(block_tag(authored_anchor.number())?)
            .await?
        else {
            return Ok(EvmProviderResponse::SafeFailure);
        };
        if &confirmed_anchor != authored_anchor {
            return Ok(EvmProviderResponse::IntegrityBlocked);
        }
        let result = AnchoredContractCallResult::new(confirmed_anchor, return_bytes)
            .map_err(|_| AdapterError::Unavailable)?;
        Ok(returned(EvmReadValue::AnchoredContractCall(result)))
    }

    async fn strict_block_anchor(
        &self,
        tag: String,
    ) -> Result<Option<EvmBlockAnchor>, AdapterError> {
        let block = self
            .call_strict("eth_getBlockByNumber", serde_json::json!([tag, false]))
            .await?;
        if block.is_null() {
            return Ok(None);
        }
        parse_block_anchor(&block).map(Some)
    }
}

fn http_client() -> Result<reqwest::Client, EvmProviderBuildError> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .retry(reqwest::retry::never())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| EvmProviderBuildError)
}

impl EvmProvider for JsonRpcEvmProvider {
    fn request<'a>(
        &'a self,
        operation: StableId,
        request_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<EvmProviderResponse, AdapterError>> + Send + 'a>> {
        Box::pin(async move {
            // The request bytes are the serialized intent: decode with the domain's own
            // checked deserializer so a subject change is a compile error, not wire drift.
            let intent: EvmReadIntent =
                serde_json::from_slice(&request_bytes).map_err(|_| AdapterError::Internal)?;
            if intent.operation_and_chain_id().0 != operation.as_str() {
                return Err(AdapterError::Internal);
            }
            self.observe(intent.subject()).await
        })
    }
}

impl EvmTransactionProvider for JsonRpcEvmProvider {
    fn chain_instance(&self) -> EvmTransactionProviderFuture<'_, EvmChainInstance> {
        Box::pin(async move {
            let chain_id = self
                .call_strict("eth_chainId", serde_json::json!([]))
                .await?
                .as_str()
                .and_then(quantity_to_u64)
                .and_then(NonZeroU64::new)
                .ok_or(AdapterError::Unavailable)?;
            let genesis = self
                .call_strict("eth_getBlockByNumber", serde_json::json!(["0x0", false]))
                .await?;
            if genesis.is_null() {
                return Err(AdapterError::Unavailable);
            }
            let anchor = parse_block_anchor(&genesis)?;
            if anchor.number().as_str() != "0" {
                return Err(AdapterError::Unavailable);
            }
            Ok(EvmChainInstance::new(chain_id, anchor.hash().clone()))
        })
    }

    fn pending_nonce<'a>(
        &'a self,
        sender: &'a mfm_evm::EvmAddress,
    ) -> EvmTransactionProviderFuture<'a, u64> {
        let sender = sender.to_string();
        Box::pin(async move {
            self.call_strict(
                "eth_getTransactionCount",
                serde_json::json!([sender, "pending"]),
            )
            .await?
            .as_str()
            .and_then(quantity_to_u64)
            .ok_or(AdapterError::Unavailable)
        })
    }

    fn receipt<'a>(
        &'a self,
        transaction_hash: &'a EvmHash,
    ) -> EvmTransactionProviderFuture<'a, Option<ProviderReceipt>> {
        let transaction_hash = transaction_hash.to_string();
        Box::pin(async move {
            let result = self
                .call_strict(
                    "eth_getTransactionReceipt",
                    serde_json::json!([transaction_hash]),
                )
                .await?;
            if result.is_null() {
                return Ok(None);
            }
            parse_receipt(&result).map(Some)
        })
    }

    fn canonical_block<'a>(
        &'a self,
        block_number: &'a EvmU256,
    ) -> EvmTransactionProviderFuture<'a, EvmBlockAnchor> {
        let tag = block_tag(block_number);
        Box::pin(async move {
            let tag = tag?;
            self.strict_block_anchor(tag)
                .await?
                .ok_or(AdapterError::Unavailable)
        })
    }

    fn submit_raw<'a>(
        &'a self,
        raw_transaction: &'a ExactRawTransaction,
    ) -> EvmTransactionProviderFuture<'a, EvmHash> {
        let encoded = format!("0x{}", hex::encode(raw_transaction.as_bytes()));
        Box::pin(async move {
            self.call_strict("eth_sendRawTransaction", serde_json::json!([encoded]))
                .await?
                .as_str()
                .and_then(|hash| EvmHash::new(hash.to_owned()).ok())
                .ok_or(AdapterError::Unavailable)
        })
    }
}

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u8,
    method: &'a str,
    params: serde_json::Value,
}

enum RpcOutcome {
    Result(serde_json::Value),
    Error,
}

/// Checked JSON-RPC QUANTITY ingress.
struct RpcQuantity(U256);

impl RpcQuantity {
    fn parse(value: &str) -> Result<Self, AdapterError> {
        let digits = value.strip_prefix("0x").ok_or(AdapterError::Unavailable)?;
        if digits.is_empty()
            || (digits.len() > 1 && digits.starts_with('0'))
            || digits.len() > 64
            || !is_lower_hex(digits)
        {
            return Err(AdapterError::Unavailable);
        }
        U256::from_str_radix(digits, 16)
            .map(Self)
            .map_err(|_| AdapterError::Unavailable)
    }

    fn decimal(&self) -> String {
        self.0.to_string()
    }

    fn to_u64(&self) -> Option<u64> {
        u64::try_from(self.0).ok()
    }
}

/// Checked JSON-RPC DATA ingress.
#[derive(Debug, PartialEq, Eq)]
struct RpcData(Vec<u8>);

impl RpcData {
    fn parse(value: &str, maximum: usize) -> Result<Self, AdapterError> {
        let digits = value.strip_prefix("0x").ok_or(AdapterError::Unavailable)?;
        if digits.len() % 2 != 0 || digits.len() / 2 > maximum || !is_lower_hex(digits) {
            return Err(AdapterError::Unavailable);
        }
        hex::decode(digits)
            .map(Self)
            .map_err(|_| AdapterError::Unavailable)
    }
}

/// One exact ABI word decoded from JSON-RPC DATA.
struct AbiWord([u8; 32]);

impl TryFrom<RpcData> for AbiWord {
    type Error = AdapterError;

    fn try_from(data: RpcData) -> Result<Self, Self::Error> {
        data.0
            .try_into()
            .map(Self)
            .map_err(|_| AdapterError::Unavailable)
    }
}

async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, AdapterError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(AdapterError::Unavailable);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AdapterError::Unavailable)?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(AdapterError::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

const fn returned(value: EvmReadValue) -> EvmProviderResponse {
    EvmProviderResponse::Read(value)
}

/// Parses a 20-byte address and re-renders it; addresses are never spliced as text.
#[cfg(test)]
fn checked_address(value: &str) -> Result<String, AdapterError> {
    Address::from_str(value)
        .map(|address| format!("0x{}", hex::encode(address)))
        .map_err(|_| AdapterError::Internal)
}

fn decimals_calldata() -> String {
    format!("0x{DECIMALS_SELECTOR}")
}

fn balance_of_calldata(holder: &Address) -> String {
    format!("0x{BALANCE_OF_SELECTOR}{}", hex::encode(holder.into_word()))
}

/// Renders one checked decimal block number as its `0x` quantity tag.
fn block_tag(number: &EvmU256) -> Result<String, AdapterError> {
    number
        .as_str()
        .parse::<U256>()
        .map(|number| format!("{number:#x}"))
        .map_err(|_| AdapterError::Internal)
}

fn is_lower_hex(digits: &str) -> bool {
    digits
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn quantity_to_u64(value: &str) -> Option<u64> {
    RpcQuantity::parse(value).ok()?.to_u64()
}

fn quantity_to_decimal(value: &str) -> Option<String> {
    RpcQuantity::parse(value).ok().map(|value| value.decimal())
}

fn decode_abi_u256(word: AbiWord) -> U256 {
    U256::from_be_bytes(word.0)
}

fn word_to_u8(word: AbiWord) -> Option<u8> {
    u8::try_from(decode_abi_u256(word)).ok()
}

fn parse_block_anchor(value: &serde_json::Value) -> Result<EvmBlockAnchor, AdapterError> {
    let number = value
        .get("number")
        .and_then(serde_json::Value::as_str)
        .and_then(quantity_to_decimal)
        .and_then(|number| EvmU256::new(number).ok())
        .ok_or(AdapterError::Unavailable)?;
    let hash = value
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .and_then(|hash| EvmHash::new(hash.to_owned()).ok())
        .ok_or(AdapterError::Unavailable)?;
    Ok(EvmBlockAnchor::new(number, hash))
}

fn parse_receipt(value: &serde_json::Value) -> Result<ProviderReceipt, AdapterError> {
    let transaction_hash = checked_hash_field(value, "transactionHash")?;
    let sender = checked_address_field(value, "from")?;
    let block_number = value
        .get("blockNumber")
        .and_then(serde_json::Value::as_str)
        .and_then(quantity_to_decimal)
        .and_then(|number| EvmU256::new(number).ok())
        .ok_or(AdapterError::Unavailable)?;
    let block_hash = checked_hash_field(value, "blockHash")?;
    let block_anchor = EvmBlockAnchor::new(block_number, block_hash);
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .and_then(quantity_to_u64)
        .filter(|status| *status <= 1)
        .ok_or(AdapterError::Unavailable)?;
    let target = nullable_address_field(value, "to")?;
    let contract_address = nullable_address_field(value, "contractAddress")?;
    let result = match (status, target, contract_address) {
        (1, None, Some(contract_address)) => {
            ProviderReceiptResult::SuccessCreate { contract_address }
        }
        (0, None, None) => ProviderReceiptResult::RevertedCreate,
        (1, Some(target), None) => ProviderReceiptResult::SuccessCall { target },
        (0, Some(target), None) => ProviderReceiptResult::RevertedCall { target },
        _ => return Err(AdapterError::Unavailable),
    };
    Ok(ProviderReceipt::new(
        transaction_hash,
        sender,
        result,
        block_anchor,
    ))
}

fn checked_hash_field(value: &serde_json::Value, field: &str) -> Result<EvmHash, AdapterError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| EvmHash::new(value.to_owned()).ok())
        .ok_or(AdapterError::Unavailable)
}

fn checked_address_field(
    value: &serde_json::Value,
    field: &str,
) -> Result<mfm_evm::EvmAddress, AdapterError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| mfm_evm::EvmAddress::new(value.to_owned()).ok())
        .ok_or(AdapterError::Unavailable)
}

fn nullable_address_field(
    value: &serde_json::Value,
    field: &str,
) -> Result<Option<mfm_evm::EvmAddress>, AdapterError> {
    let field = value.get(field).ok_or(AdapterError::Unavailable)?;
    if field.is_null() {
        return Ok(None);
    }
    field
        .as_str()
        .and_then(|value| mfm_evm::EvmAddress::new(value.to_owned()).ok())
        .map(Some)
        .ok_or(AdapterError::Unavailable)
}

fn valid_rpc_error(value: &serde_json::Value) -> bool {
    let Some(error) = value.as_object() else {
        return false;
    };
    error
        .get("code")
        .and_then(serde_json::Value::as_i64)
        .is_some()
        && error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .is_some()
        && error
            .keys()
            .all(|key| matches!(key.as_str(), "code" | "message" | "data"))
}

#[cfg(test)]
#[path = "json_rpc_tests.rs"]
mod tests;
