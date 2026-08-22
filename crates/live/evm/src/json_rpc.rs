//! Production JSON-RPC 2.0 provider for the bounded EVM Read capabilities.
//!
//! The provider owns exactly one endpoint URL and the six frozen-wire RPC calls the EVM
//! domain's Read subjects require. It holds no key, nonce, broadcast path, or automatic retry.

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::time::Duration;

use alloy_primitives::{hex, Address, U256};
use mfm_evm::{EvmBalanceSource, EvmBlockAnchor, EvmReadIntent, EvmReadSubject, EvmReadValue};
use mfm_ids::StableId;
use mfm_runtime::ReadAdapterError;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{EvmProvider, EvmProviderResponse};

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

    /// Performs one JSON-RPC call.
    ///
    /// `Ok(Some(result))` is a returned result, `Ok(None)` is a reviewed JSON-RPC error object,
    /// and every transport, status, bound, or envelope failure is `Unavailable`.
    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, ReadAdapterError> {
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
            .map_err(|_| ReadAdapterError::Unavailable)?;
        if !response.status().is_success() {
            return Err(ReadAdapterError::Unavailable);
        }
        let body = bounded_body(response).await?;
        let envelope: JsonRpcResponse =
            serde_json::from_slice(&body).map_err(|_| ReadAdapterError::Unavailable)?;
        match (envelope.result, envelope.error) {
            (Some(result), None) => Ok(Some(result)),
            (None, Some(_)) => Ok(None),
            _ => Err(ReadAdapterError::Unavailable),
        }
    }

    /// Reads one block anchor by the supplied block tag.
    async fn anchor(
        &self,
        tag: serde_json::Value,
    ) -> Result<EvmProviderResponse, ReadAdapterError> {
        let Some(block) = self
            .call("eth_getBlockByNumber", serde_json::json!([tag, false]))
            .await?
        else {
            return Ok(EvmProviderResponse::SafeFailure);
        };
        let number = block
            .get("number")
            .and_then(serde_json::Value::as_str)
            .and_then(quantity_to_u64)
            .ok_or(ReadAdapterError::Unavailable)?;
        let hash = block
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .filter(|hash| is_block_hash(hash))
            .ok_or(ReadAdapterError::Unavailable)?;
        Ok(returned(EvmReadValue::Anchor {
            number: number.to_string(),
            hash: hash.to_owned(),
        }))
    }

    /// Performs one anchored `eth_call` and returns its result word.
    ///
    /// `Ok(None)` is a definite safe failure: a reviewed revert or a codeless address.
    async fn contract_call(
        &self,
        source: &EvmBalanceSource,
        anchor: &EvmBlockAnchor,
        data: String,
    ) -> Result<Option<String>, ReadAdapterError> {
        let token = source.token.as_deref().ok_or(ReadAdapterError::Internal)?;
        let to = checked_address(token)?;
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
        let word = result.as_str().ok_or(ReadAdapterError::Unavailable)?;
        if word == "0x" {
            return Ok(None);
        }
        Ok(Some(word.to_owned()))
    }

    /// Routes one checked Read subject to its exact RPC call.
    async fn observe(
        &self,
        subject: &EvmReadSubject,
    ) -> Result<EvmProviderResponse, ReadAdapterError> {
        match subject {
            EvmReadSubject::ChainIdentity => {
                let Some(result) = self.call("eth_chainId", serde_json::json!([])).await? else {
                    return Ok(EvmProviderResponse::SafeFailure);
                };
                let chain_id = result
                    .as_str()
                    .and_then(quantity_to_u64)
                    .ok_or(ReadAdapterError::Unavailable)?;
                Ok(returned(EvmReadValue::ChainId(chain_id)))
            }
            EvmReadSubject::InitialAnchor => self.anchor(serde_json::json!("latest")).await,
            EvmReadSubject::NativeBalance { source, anchor } => {
                let address = checked_address(&source.address)?;
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
                    .ok_or(ReadAdapterError::Unavailable)?;
                Ok(returned(EvmReadValue::RawUnits(units)))
            }
            EvmReadSubject::TokenDecimals { source, anchor } => {
                let Some(word) = self
                    .contract_call(source, anchor, decimals_calldata())
                    .await?
                else {
                    return Ok(EvmProviderResponse::SafeFailure);
                };
                let decimals = word_to_u8(&word).ok_or(ReadAdapterError::Unavailable)?;
                Ok(returned(EvmReadValue::TokenDecimals(decimals)))
            }
            EvmReadSubject::TokenBalance { source, anchor } => {
                let holder = Address::from_str(&source.address).map_err(|_| {
                    // A malformed address is a local domain-value defect, never a node error.
                    ReadAdapterError::Internal
                })?;
                let Some(word) = self
                    .contract_call(source, anchor, balance_of_calldata(&holder))
                    .await?
                else {
                    return Ok(EvmProviderResponse::SafeFailure);
                };
                let units = quantity_to_decimal(&word).ok_or(ReadAdapterError::Unavailable)?;
                Ok(returned(EvmReadValue::RawUnits(units)))
            }
            // Confirmation reads the committed number, never the moving head.
            EvmReadSubject::ConfirmAnchor { anchor, .. } => {
                let tag = block_tag(anchor.number())?;
                self.anchor(serde_json::json!(tag)).await
            }
        }
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
    ) -> Pin<Box<dyn Future<Output = Result<EvmProviderResponse, ReadAdapterError>> + Send + 'a>>
    {
        Box::pin(async move {
            // The request bytes are the serialized intent: decode with the domain's own
            // checked deserializer so a subject change is a compile error, not wire drift.
            let intent: EvmReadIntent =
                serde_json::from_slice(&request_bytes).map_err(|_| ReadAdapterError::Internal)?;
            if intent.operation_and_chain_id().0 != operation.as_str() {
                return Err(ReadAdapterError::Internal);
            }
            self.observe(intent.subject()).await
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

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, ReadAdapterError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ReadAdapterError::Unavailable);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ReadAdapterError::Unavailable)?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ReadAdapterError::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

const fn returned(value: EvmReadValue) -> EvmProviderResponse {
    EvmProviderResponse::Read(value)
}

/// Parses a 20-byte address and re-renders it; addresses are never spliced as text.
fn checked_address(value: &str) -> Result<String, ReadAdapterError> {
    Address::from_str(value)
        .map(|address| format!("0x{}", hex::encode(address)))
        .map_err(|_| ReadAdapterError::Internal)
}

fn decimals_calldata() -> String {
    format!("0x{DECIMALS_SELECTOR}")
}

fn balance_of_calldata(holder: &Address) -> String {
    format!("0x{BALANCE_OF_SELECTOR}{:0>64}", hex::encode(holder))
}

/// Renders one checked decimal block number as its `0x` quantity tag.
fn block_tag(number: &str) -> Result<String, ReadAdapterError> {
    number
        .parse::<u64>()
        .map(|number| format!("0x{number:x}"))
        .map_err(|_| ReadAdapterError::Internal)
}

fn hex_digits(value: &str) -> Option<&str> {
    let digits = value.strip_prefix("0x")?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(digits)
}

fn quantity_to_u64(value: &str) -> Option<u64> {
    let digits = hex_digits(value)?;
    (digits.len() <= 16).then_some(())?;
    u64::from_str_radix(digits, 16).ok()
}

fn quantity_to_decimal(value: &str) -> Option<String> {
    let digits = hex_digits(value)?;
    (digits.len() <= 64).then_some(())?;
    U256::from_str_radix(digits, 16)
        .ok()
        .map(|value| value.to_string())
}

fn word_to_u8(value: &str) -> Option<u8> {
    let digits = hex_digits(value)?;
    (digits.len() == 64).then_some(())?;
    let (leading, last) = digits.split_at(62);
    leading.bytes().all(|byte| byte == b'0').then_some(())?;
    u8::from_str_radix(last, 16).ok()
}

fn is_block_hash(value: &str) -> bool {
    hex_digits(value).is_some_and(|digits| {
        digits.len() == 64 && digits.bytes().all(|byte| !byte.is_ascii_uppercase())
    })
}

#[cfg(test)]
#[path = "json_rpc_tests.rs"]
mod tests;
