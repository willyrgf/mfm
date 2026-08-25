//! Production JSON-RPC 2.0 provider for bounded EVM Reads and transactions.
//!
//! The provider owns exactly one endpoint URL and the bounded Read and transaction methods the EVM
//! domain contracts require. It holds no key, nonce authority, or automatic retry.

use std::num::NonZeroU64;
#[cfg(test)]
use std::str::FromStr;
use std::time::Duration;

use alloy_primitives::{hex, Address, U256};
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallIntent, AnchoredContractCallResult,
    EvmAddress, EvmBalanceSource, EvmBlockAnchor, EvmChainInstance, EvmHash, EvmReadEvidence,
    EvmReadIntent, EvmReadSubject, EvmReadValue, EvmTokenDecimals, EvmU256,
    MAX_EVM_CALL_RETURN_BYTES,
};
use mfm_evm_transaction_authority::ExactRawTransaction;
use mfm_ids::ContentRef;
use mfm_runtime::AdapterError;
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    EvmReadProvider, EvmTransactionProvider, EvmTransactionProviderFuture, ProviderFuture,
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

    async fn rpc<P: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: &P,
    ) -> Result<RpcEnvelope<T>, AdapterError> {
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
        serde_json::from_slice(&body).map_err(|_| AdapterError::Unavailable)
    }

    async fn broad_anchor(&self, tag: &str) -> Result<EvmBlockAnchor, AdapterError> {
        self.rpc::<_, RpcBlock>("eth_getBlockByNumber", &(tag, false))
            .await?
            .into_result()?
            .into_anchor()
    }

    async fn token_contract_call(
        &self,
        source: &EvmBalanceSource,
        anchor: &EvmBlockAnchor,
        data: String,
    ) -> Result<Option<AbiWord>, AdapterError> {
        let token = source.token().ok_or(AdapterError::Internal)?;
        let tag = block_tag(anchor.number())?;
        let call = RpcCall { to: token, data };
        let data = self
            .rpc::<_, RpcDataText>("eth_call", &(call, tag))
            .await?
            .into_result()?
            .parse(32)?;
        if data.0.is_empty() {
            return Ok(None);
        }
        AbiWord::try_from(data).map(Some)
    }

    async fn observe_read(
        &self,
        intent_value_ref: &ContentRef,
        intent: &EvmReadIntent,
    ) -> Result<EvmReadEvidence, AdapterError> {
        let value = match intent.subject() {
            EvmReadSubject::ChainIdentity => {
                let chain_id = self
                    .rpc::<_, RpcQuantity>("eth_chainId", &[] as &[u8; 0])
                    .await?
                    .into_result()?
                    .to_u64()
                    .and_then(NonZeroU64::new)
                    .ok_or(AdapterError::Unavailable)?;
                EvmReadValue::ChainId(chain_id)
            }
            EvmReadSubject::InitialAnchor => {
                EvmReadValue::Anchor(self.broad_anchor("latest").await?)
            }
            EvmReadSubject::NativeBalance { source, anchor } => {
                let tag = block_tag(anchor.number())?;
                let units = self
                    .rpc::<_, RpcQuantity>("eth_getBalance", &(source.address(), tag))
                    .await?
                    .into_result()?
                    .decimal();
                EvmReadValue::RawUnits(EvmU256::new(units).map_err(|_| AdapterError::Unavailable)?)
            }
            EvmReadSubject::TokenDecimals { source, anchor } => {
                let Some(word) = self
                    .token_contract_call(source, anchor, decimals_calldata())
                    .await?
                else {
                    return Ok(EvmReadEvidence::safe_failure(intent_value_ref.clone()));
                };
                let decimals = word_to_u8(word).ok_or(AdapterError::Unavailable)?;
                EvmReadValue::TokenDecimals(
                    EvmTokenDecimals::new(decimals).map_err(|_| AdapterError::Unavailable)?,
                )
            }
            EvmReadSubject::TokenBalance { source, anchor } => {
                let holder = Address::from(*source.address().as_bytes());
                let Some(word) = self
                    .token_contract_call(source, anchor, balance_of_calldata(&holder))
                    .await?
                else {
                    return Ok(EvmReadEvidence::safe_failure(intent_value_ref.clone()));
                };
                EvmReadValue::RawUnits(
                    EvmU256::new(decode_abi_u256(word).to_string())
                        .map_err(|_| AdapterError::Unavailable)?,
                )
            }
            // Confirmation reads the committed number, never the moving head.
            EvmReadSubject::ConfirmAnchor { anchor, .. } => {
                let tag = block_tag(anchor.number())?;
                EvmReadValue::Anchor(self.broad_anchor(&tag).await?)
            }
        };
        Ok(EvmReadEvidence::returned(intent_value_ref.clone(), value))
    }

    async fn anchored_contract_call(
        &self,
        intent_value_ref: &ContentRef,
        intent: &AnchoredContractCallIntent,
    ) -> Result<AnchoredContractCallEvidence, AdapterError> {
        let authored_anchor = intent.anchor();
        let Some(observed_anchor) = self
            .strict_block_anchor(block_tag(authored_anchor.number())?)
            .await?
        else {
            return Ok(AnchoredContractCallEvidence::safe_failure(
                intent_value_ref.clone(),
            ));
        };
        if &observed_anchor != authored_anchor {
            return Ok(AnchoredContractCallEvidence::integrity_blocked(
                intent_value_ref.clone(),
            ));
        }

        let selector = RpcBlockSelector {
            block_hash: authored_anchor.hash(),
            require_canonical: true,
        };
        let code = self
            .rpc::<_, RpcDataText>("eth_getCode", &(intent.target(), &selector))
            .await?
            .into_result()?
            .parse(MAX_EVM_CONTRACT_CODE_BYTES)?;
        if code.0.is_empty() {
            return Ok(AnchoredContractCallEvidence::rejected(
                intent_value_ref.clone(),
            ));
        }

        let call = RpcCall {
            to: intent.target(),
            data: format!("0x{}", hex::encode(intent.calldata())),
        };
        let return_bytes = self
            .rpc::<_, RpcDataText>("eth_call", &(call, &selector))
            .await?
            .into_result()?
            .parse(MAX_EVM_CALL_RETURN_BYTES)?
            .0;

        let Some(confirmed_anchor) = self
            .strict_block_anchor(block_tag(authored_anchor.number())?)
            .await?
        else {
            return Ok(AnchoredContractCallEvidence::safe_failure(
                intent_value_ref.clone(),
            ));
        };
        if &confirmed_anchor != authored_anchor {
            return Ok(AnchoredContractCallEvidence::integrity_blocked(
                intent_value_ref.clone(),
            ));
        }
        let result = AnchoredContractCallResult::new(confirmed_anchor, return_bytes)
            .map_err(|_| AdapterError::Unavailable)?;
        Ok(AnchoredContractCallEvidence::returned(
            intent_value_ref.clone(),
            result,
        ))
    }

    async fn strict_block_anchor(
        &self,
        tag: String,
    ) -> Result<Option<EvmBlockAnchor>, AdapterError> {
        self.rpc::<_, Option<RpcBlock>>("eth_getBlockByNumber", &(tag, false))
            .await?
            .into_result()?
            .map(RpcBlock::into_anchor)
            .transpose()
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

impl EvmReadProvider for JsonRpcEvmProvider {
    fn observe<'a>(
        &'a self,
        intent_value_ref: &'a ContentRef,
        intent: &'a EvmReadIntent,
    ) -> ProviderFuture<'a, EvmReadEvidence> {
        Box::pin(async move { self.observe_read(intent_value_ref, intent).await })
    }

    fn observe_anchored_call<'a>(
        &'a self,
        intent_value_ref: &'a ContentRef,
        intent: &'a AnchoredContractCallIntent,
    ) -> ProviderFuture<'a, AnchoredContractCallEvidence> {
        Box::pin(async move { self.anchored_contract_call(intent_value_ref, intent).await })
    }
}

impl EvmTransactionProvider for JsonRpcEvmProvider {
    fn chain_instance(&self) -> EvmTransactionProviderFuture<'_, EvmChainInstance> {
        Box::pin(async move {
            let chain_id = self
                .rpc::<_, RpcQuantity>("eth_chainId", &[] as &[u8; 0])
                .await?
                .into_result()?
                .to_u64()
                .and_then(NonZeroU64::new)
                .ok_or(AdapterError::Unavailable)?;
            let genesis = self
                .rpc::<_, Option<RpcBlock>>("eth_getBlockByNumber", &("0x0", false))
                .await?
                .into_result()?
                .ok_or(AdapterError::Unavailable)?
                .into_anchor()?;
            if genesis.number().as_str() != "0" {
                return Err(AdapterError::Unavailable);
            }
            Ok(EvmChainInstance::new(chain_id, genesis.hash().clone()))
        })
    }

    fn pending_nonce<'a>(
        &'a self,
        sender: &'a EvmAddress,
    ) -> EvmTransactionProviderFuture<'a, u64> {
        Box::pin(async move {
            self.rpc::<_, RpcQuantity>("eth_getTransactionCount", &(sender, "pending"))
                .await?
                .into_result()?
                .to_u64()
                .ok_or(AdapterError::Unavailable)
        })
    }

    fn receipt<'a>(
        &'a self,
        transaction_hash: &'a EvmHash,
    ) -> EvmTransactionProviderFuture<'a, Option<ProviderReceipt>> {
        Box::pin(async move {
            self.rpc::<_, Option<RpcReceipt>>("eth_getTransactionReceipt", &(transaction_hash,))
                .await?
                .into_result()?
                .map(parse_receipt)
                .transpose()
        })
    }

    fn canonical_block<'a>(
        &'a self,
        block_number: &'a EvmU256,
    ) -> EvmTransactionProviderFuture<'a, EvmBlockAnchor> {
        Box::pin(async move {
            let tag = block_tag(block_number)?;
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
            self.rpc::<_, EvmHash>("eth_sendRawTransaction", &(encoded,))
                .await?
                .into_result()
        })
    }
}

#[derive(Serialize)]
struct JsonRpcRequest<'a, P: Serialize + ?Sized> {
    jsonrpc: &'static str,
    id: u8,
    method: &'static str,
    params: &'a P,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcSuccess<T> {
    jsonrpc: RpcVersion,
    id: RpcId,
    result: T,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcFailure {
    jsonrpc: RpcVersion,
    id: RpcId,
    error: RpcError,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RpcEnvelope<T> {
    Success(RpcSuccess<T>),
    Failure(RpcFailure),
}

impl<T> RpcEnvelope<T> {
    fn into_result(self) -> Result<T, AdapterError> {
        match self {
            Self::Success(RpcSuccess {
                jsonrpc,
                id,
                result,
            }) => {
                let _ = (jsonrpc, id);
                Ok(result)
            }
            Self::Failure(RpcFailure { jsonrpc, id, error }) => {
                let _ = (jsonrpc, id, error.code, error.message.len(), error.data);
                Err(AdapterError::Unavailable)
            }
        }
    }
}

#[derive(Clone, Copy)]
struct RpcVersion;

impl<'de> Deserialize<'de> for RpcVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        (value == "2.0")
            .then_some(Self)
            .ok_or_else(|| de::Error::custom("invalid JSON-RPC version"))
    }
}

#[derive(Clone, Copy)]
struct RpcId;

impl<'de> Deserialize<'de> for RpcId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        (value == 1)
            .then_some(Self)
            .ok_or_else(|| de::Error::custom("invalid JSON-RPC id"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct RpcCall<'a> {
    to: &'a EvmAddress,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RpcBlockSelector<'a> {
    block_hash: &'a EvmHash,
    require_canonical: bool,
}

#[derive(Deserialize)]
struct RpcBlock {
    number: RpcQuantity,
    hash: EvmHash,
}

impl RpcBlock {
    fn into_anchor(self) -> Result<EvmBlockAnchor, AdapterError> {
        Ok(EvmBlockAnchor::new(
            EvmU256::new(self.number.decimal()).map_err(|_| AdapterError::Unavailable)?,
            self.hash,
        ))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcReceipt {
    transaction_hash: EvmHash,
    #[serde(rename = "from")]
    sender: EvmAddress,
    #[serde(deserialize_with = "required_nullable")]
    to: Option<EvmAddress>,
    #[serde(deserialize_with = "required_nullable")]
    contract_address: Option<EvmAddress>,
    status: RpcQuantity,
    block_number: RpcQuantity,
    block_hash: EvmHash,
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
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

impl<'de> Deserialize<'de> for RpcQuantity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct RpcDataText(String);

impl RpcDataText {
    fn parse(self, maximum: usize) -> Result<RpcData, AdapterError> {
        RpcData::parse(&self.0, maximum)
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

#[cfg(test)]
fn quantity_to_u64(value: &str) -> Option<u64> {
    RpcQuantity::parse(value).ok()?.to_u64()
}

#[cfg(test)]
fn quantity_to_decimal(value: &str) -> Option<String> {
    RpcQuantity::parse(value).ok().map(|value| value.decimal())
}

fn decode_abi_u256(word: AbiWord) -> U256 {
    U256::from_be_bytes(word.0)
}

fn word_to_u8(word: AbiWord) -> Option<u8> {
    u8::try_from(decode_abi_u256(word)).ok()
}

fn parse_receipt(value: RpcReceipt) -> Result<ProviderReceipt, AdapterError> {
    let status = value.status.to_u64().filter(|status| *status <= 1);
    let block_anchor = EvmBlockAnchor::new(
        EvmU256::new(value.block_number.decimal()).map_err(|_| AdapterError::Unavailable)?,
        value.block_hash,
    );
    let result = match (status, value.to, value.contract_address) {
        (Some(1), None, Some(contract_address)) => {
            ProviderReceiptResult::SuccessCreate { contract_address }
        }
        (Some(0), None, None) => ProviderReceiptResult::RevertedCreate,
        (Some(1), Some(target), None) => ProviderReceiptResult::SuccessCall { target },
        (Some(0), Some(target), None) => ProviderReceiptResult::RevertedCall { target },
        _ => return Err(AdapterError::Unavailable),
    };
    Ok(ProviderReceipt::new(
        value.transaction_hash,
        value.sender,
        result,
        block_anchor,
    ))
}

#[cfg(test)]
#[path = "json_rpc_tests.rs"]
mod tests;
