//! Production JSON-RPC 2.0 provider for bounded EVM Reads and transactions.
//!
//! The provider owns exactly one endpoint URL and the bounded Read and transaction methods the EVM
//! domain contracts require. It holds no key, nonce authority, or automatic retry.

use std::num::NonZeroU64;
use std::time::Duration;

use crate::error::{invariant, AdapterFailure};
use alloy_primitives::{hex, Address, U256};
use mfm_capabilities::AdapterError;
use mfm_evm::custody::ExactRawTransaction;
use mfm_evm::ObservedSize;
use mfm_evm::{
    EvmOperationalError, EvmOperationalKind, EvmRpcMethod, ProviderFailure, ProviderFailureKind,
    RpcField, RpcRejection, RpcStage,
};
use reqwest::StatusCode;
mod capture;
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallIntent, AnchoredContractCallResult,
    EvmAddress, EvmBalanceTarget, EvmBlockAnchor, EvmChainInstance, EvmHash, EvmReadEvidence,
    EvmReadIntent, EvmReadSubject, EvmReadValue, EvmTokenDecimals, EvmU256,
    MAX_EVM_CALL_RETURN_BYTES,
};
use mfm_ids::ContentRef;
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    EvmReadProvider, EvmTransactionProvider, ProviderFuture, ProviderReceipt, ProviderReceiptResult,
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmProviderBuildError {
    /// A local locator check refused the handle without retaining it.
    #[error("evm provider transport could not be constructed")]
    Locator {
        /// Checked locator invariant.
        check: EvmLocatorCheck,
    },
    /// The URL parser rejected the handle; its closed error contains no input.
    #[error("evm provider transport could not be constructed")]
    Parse {
        /// Original URL-parser category.
        source: url::ParseError,
    },
    /// Client construction failed before a request was made.
    #[error("evm provider transport could not be constructed")]
    Client {
        /// Reviewed construction source chain.
        diagnostics: mfm_values::DiagnosticEvidence,
    },
}

/// Local locator invariant, excluding locator text and credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmLocatorCheck {
    /// Empty input.
    Empty,
    /// Input exceeded the bounded locator length.
    Size {
        /// Observed UTF-8 bytes.
        observed: u64,
        /// Inclusive permitted bytes.
        limit: u64,
    },
    /// Control character in input.
    ControlCharacter,
    /// URL is not HTTP(S).
    Scheme,
    /// URL cannot act as an HTTP base.
    Base,
    /// URL has no host.
    Host,
    /// Fragment is not permitted.
    Fragment,
}

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
        let check = if value.is_empty() {
            Some(EvmLocatorCheck::Empty)
        } else if value.len() > MAX_EVM_ADAPTER_LOCATOR_BYTES {
            Some(EvmLocatorCheck::Size {
                observed: value.len() as u64,
                limit: MAX_EVM_ADAPTER_LOCATOR_BYTES as u64,
            })
        } else if value.chars().any(char::is_control) {
            Some(EvmLocatorCheck::ControlCharacter)
        } else {
            None
        };
        if let Some(check) = check {
            return Err(EvmProviderBuildError::Locator { check });
        }
        let url = Url::parse(value).map_err(|source| EvmProviderBuildError::Parse { source })?;
        let check = if !matches!(url.scheme(), "http" | "https") {
            Some(EvmLocatorCheck::Scheme)
        } else if url.cannot_be_a_base() {
            Some(EvmLocatorCheck::Base)
        } else if !url.has_host() {
            Some(EvmLocatorCheck::Host)
        } else if url.fragment().is_some() {
            Some(EvmLocatorCheck::Fragment)
        } else {
            None
        };
        if let Some(check) = check {
            return Err(EvmProviderBuildError::Locator { check });
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

    /// Funds a development sender through the node's unlocked account, without automatic retry.
    ///
    /// This development-only mutation has no Program or transaction-custody authority. A lost
    /// acknowledgement does not establish nonacceptance. Both calls retain the existing 16 KiB
    /// funding response bound and the provider's complete request deadline.
    pub async fn fund_development_sender(
        &self,
        sender: &EvmAddress,
        value: &EvmU256,
        maximum_fee: u128,
        priority_fee: u128,
    ) -> Result<EvmHash, AdapterError<EvmOperationalError>> {
        let account = self
            .rpc::<_, Vec<EvmAddress>>(EvmRpcMethod::Accounts, &[] as &[u8; 0])
            .await?
            .checked(RpcField::FundingAccount, |accounts| {
                accounts.into_iter().next().ok_or(RpcRejection::Missing)
            })?;
        self.rpc::<_, EvmHash>(
            EvmRpcMethod::SendTransaction,
            &[RpcFundingCall {
                from: &account,
                to: sender,
                value: block_tag(value)?,
                gas: "0x5208",
                max_fee_per_gas: format!("{maximum_fee:#x}"),
                max_priority_fee_per_gas: format!("{priority_fee:#x}"),
            }],
        )
        .await
        .map(RpcObservation::into_value)
    }

    async fn rpc<P: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: EvmRpcMethod,
        params: &P,
    ) -> Result<RpcObservation<T>, AdapterError<EvmOperationalError>> {
        let response = self
            .http
            .post(self.url.clone())
            .json(&JsonRpcRequest {
                jsonrpc: "2.0",
                id: 1,
                method: method.as_str(),
                params,
            })
            .send()
            .await
            .map_err(|error| client_failure(method, RpcStage::Send, None, &error))?;
        let status = response.status();
        if !response.status().is_success() {
            return Err(provider_failure(
                method,
                RpcStage::Status,
                ProviderFailureKind::HttpStatus,
                Some(status),
                None,
                None,
            ));
        }
        let body = bounded_body(method, status, response).await?;
        let envelope: RpcEnvelope<'_> = serde_json::from_slice(&body)
            .map_err(|error| client_failure(method, RpcStage::Envelope, Some(status), &error))?;
        let _ = (envelope.jsonrpc, envelope.id);
        match (envelope.result, envelope.error) {
            (Some(result), None) => {
                let value = serde_json::from_str(result.get()).map_err(|error| {
                    client_failure(method, RpcStage::Result, Some(status), &error)
                })?;
                Ok(RpcObservation {
                    method,
                    status,
                    value,
                })
            }
            (None, Some(error)) => {
                let error: RpcError<'_> = serde_json::from_str(error.get()).map_err(|error| {
                    client_failure(method, RpcStage::Envelope, Some(status), &error)
                })?;
                Err(provider_failure(
                    method,
                    RpcStage::Envelope,
                    ProviderFailureKind::RpcError,
                    Some(status),
                    None,
                    Some(&error),
                ))
            }
            _ => Err(rejected(
                method,
                status,
                RpcField::Result,
                RpcRejection::Encoding,
            )),
        }
    }

    async fn chain_id(&self) -> Result<NonZeroU64, AdapterError<EvmOperationalError>> {
        self.rpc::<_, String>(EvmRpcMethod::ChainId, &[] as &[u8; 0])
            .await?
            .checked(RpcField::ChainId, |value| {
                let quantity = RpcQuantity::parse(&value)?;
                let value = quantity.bounded_u64(1, u64::MAX)?;
                NonZeroU64::new(value).ok_or(RpcRejection::Range {
                    minimum: 1,
                    maximum: u64::MAX,
                    observed: EvmU256::from_u64(value),
                })
            })
    }

    async fn broad_anchor(
        &self,
        tag: &str,
    ) -> Result<EvmBlockAnchor, AdapterError<EvmOperationalError>> {
        self.rpc::<_, Option<RpcBlock>>(EvmRpcMethod::GetBlockByNumber, &(tag, false))
            .await?
            .checked(RpcField::Block, |block| {
                block.ok_or(RpcRejection::Missing)?.into_anchor()
            })
    }

    async fn token_contract_call<U>(
        &self,
        source: &EvmBalanceTarget,
        anchor: &EvmBlockAnchor,
        data: String,
        field: RpcField,
        check: impl FnOnce(AbiWord) -> Result<U, RpcRejection>,
    ) -> Result<Option<U>, AdapterError<EvmOperationalError>> {
        let token = source.token().ok_or_else(|| {
            invariant(AdapterFailure::MissingToken {
                balance_target: source.clone(),
            })
        })?;
        let tag = block_tag(&anchor.number)?;
        let call = RpcCall { to: token, data };
        self.rpc::<_, RpcDataText>(EvmRpcMethod::Call, &(call, tag))
            .await?
            .checked(field, |text| {
                let data = text.parse(32)?;
                if data.0.is_empty() {
                    return Ok(None);
                }
                check(AbiWord::try_from(data)?).map(Some)
            })
    }

    async fn observe_read(
        &self,
        intent_value_ref: &ContentRef,
        intent: &EvmReadIntent,
    ) -> Result<EvmReadEvidence, AdapterError<EvmOperationalError>> {
        let target = intent.target();
        let value = match intent.subject() {
            EvmReadSubject::ChainIdentity => {
                let chain_id = self.chain_id().await?;
                EvmReadValue::ChainId(chain_id)
            }
            EvmReadSubject::InitialAnchor => {
                EvmReadValue::Anchor(self.broad_anchor("latest").await?)
            }
            EvmReadSubject::NativeBalance { anchor } => {
                let tag = block_tag(&anchor.number)?;
                EvmReadValue::RawUnits(
                    self.rpc::<_, String>(EvmRpcMethod::GetBalance, &(target.account(), tag))
                        .await?
                        .checked(RpcField::Result, |value| {
                            EvmU256::new(RpcQuantity::parse(&value)?.decimal())
                                .map_err(|cause| RpcRejection::Domain { cause })
                        })?,
                )
            }
            EvmReadSubject::TokenDecimals { anchor } => {
                let Some(decimals) = self
                    .token_contract_call(
                        target,
                        anchor,
                        decimals_calldata(),
                        RpcField::Decimals,
                        |word| {
                            let quantity = RpcQuantity(decode_abi_u256(word));
                            let value = quantity.bounded_u64(0, 30)?;
                            EvmTokenDecimals::new(value as u8)
                                .map_err(|cause| RpcRejection::Domain { cause })
                        },
                    )
                    .await?
                else {
                    return Ok(EvmReadEvidence::safe_failure(intent_value_ref.clone()));
                };
                EvmReadValue::TokenDecimals(decimals)
            }
            EvmReadSubject::TokenBalance { anchor } => {
                let holder = Address::from(*target.account().as_bytes());
                let Some(units) = self
                    .token_contract_call(
                        target,
                        anchor,
                        balance_of_calldata(&holder),
                        RpcField::AbiWord,
                        |word| {
                            EvmU256::new(decode_abi_u256(word).to_string())
                                .map_err(|cause| RpcRejection::Domain { cause })
                        },
                    )
                    .await?
                else {
                    return Ok(EvmReadEvidence::safe_failure(intent_value_ref.clone()));
                };
                EvmReadValue::RawUnits(units)
            }
            // Confirmation reads the committed number, never the moving head.
            EvmReadSubject::ConfirmAnchor { anchor, .. } => {
                let tag = block_tag(&anchor.number)?;
                EvmReadValue::Anchor(self.broad_anchor(&tag).await?)
            }
        };
        Ok(EvmReadEvidence::returned(intent_value_ref.clone(), value))
    }

    async fn anchored_contract_call(
        &self,
        intent_value_ref: &ContentRef,
        intent: &AnchoredContractCallIntent,
    ) -> Result<AnchoredContractCallEvidence, AdapterError<EvmOperationalError>> {
        let authored_anchor = intent.anchor();
        let Some(observed_anchor) = self
            .block_anchor(&block_tag(&authored_anchor.number)?)
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
            block_hash: (&authored_anchor.hash),
            require_canonical: true,
        };
        let code = self
            .rpc::<_, RpcDataText>(EvmRpcMethod::GetCode, &(intent.target(), &selector))
            .await?
            .checked(RpcField::Data, |text| {
                text.parse(MAX_EVM_CONTRACT_CODE_BYTES)
            })?;
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
            .rpc::<_, RpcDataText>(EvmRpcMethod::Call, &(call, &selector))
            .await?
            .checked(RpcField::Data, |text| text.parse(MAX_EVM_CALL_RETURN_BYTES))?
            .0;

        let Some(confirmed_anchor) = self
            .block_anchor(&block_tag(&authored_anchor.number)?)
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
        let return_len = return_bytes.len();
        let result = AnchoredContractCallResult::new(confirmed_anchor.clone(), return_bytes)
            .map_err(|source| {
                invariant(AdapterFailure::AnchoredResult {
                    anchor: confirmed_anchor,
                    return_bytes: return_len,
                    source,
                })
            })?;
        Ok(AnchoredContractCallEvidence::returned(
            intent_value_ref.clone(),
            result,
        ))
    }

    async fn block_anchor(
        &self,
        tag: &str,
    ) -> Result<Option<EvmBlockAnchor>, AdapterError<EvmOperationalError>> {
        self.rpc::<_, Option<RpcBlock>>(EvmRpcMethod::GetBlockByNumber, &(tag, false))
            .await?
            .checked(RpcField::Block, |block| {
                block.map(RpcBlock::into_anchor).transpose()
            })
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
        .map_err(|error| EvmProviderBuildError::Client {
            diagnostics: capture::capture(None, Some(&error)),
        })
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
    fn chain_instance(&self) -> ProviderFuture<'_, EvmChainInstance> {
        Box::pin(async move {
            let chain_id = self.chain_id().await?;
            let genesis = self
                .rpc::<_, Option<RpcBlock>>(EvmRpcMethod::GetBlockByNumber, &("0x0", false))
                .await?
                .checked(RpcField::GenesisNumber, |block| {
                    let block = block.ok_or(RpcRejection::Missing)?;
                    RpcQuantity::parse(&block.number)?.bounded_u64(0, 0)?;
                    block.into_anchor()
                })?;
            Ok(EvmChainInstance {
                chain_id,
                expected_genesis_hash: genesis.hash.clone(),
            })
        })
    }

    fn pending_nonce<'a>(&'a self, sender: &'a EvmAddress) -> ProviderFuture<'a, u64> {
        Box::pin(async move {
            self.rpc::<_, String>(EvmRpcMethod::GetTransactionCount, &(sender, "pending"))
                .await?
                .checked(RpcField::Nonce, |value| {
                    RpcQuantity::parse(&value)?.bounded_u64(0, u64::MAX)
                })
        })
    }

    fn receipt<'a>(
        &'a self,
        transaction_hash: &'a EvmHash,
    ) -> ProviderFuture<'a, Option<ProviderReceipt>> {
        Box::pin(async move {
            self.rpc::<_, Option<RpcReceipt>>(
                EvmRpcMethod::GetTransactionReceipt,
                &(transaction_hash,),
            )
            .await?
            .checked(RpcField::ReceiptOutcome, |value| {
                value.map(parse_receipt).transpose()
            })
        })
    }

    fn transaction_known<'a>(&'a self, transaction_hash: &'a EvmHash) -> ProviderFuture<'a, bool> {
        #[derive(serde::Deserialize)]
        struct TransactionIdentity {
            hash: EvmHash,
        }
        Box::pin(async move {
            self.rpc::<_, Option<TransactionIdentity>>(
                EvmRpcMethod::GetTransactionByHash,
                &(transaction_hash,),
            )
            .await?
            .checked(RpcField::Result, |value| match value {
                None => Ok(false),
                Some(value) if &value.hash == transaction_hash => Ok(true),
                Some(value) => Err(RpcRejection::HashMismatch {
                    expected: transaction_hash.clone(),
                    observed: value.hash,
                }),
            })
        })
    }

    fn canonical_block<'a>(
        &'a self,
        block_number: &'a EvmU256,
    ) -> ProviderFuture<'a, EvmBlockAnchor> {
        Box::pin(async move {
            let tag = block_tag(block_number)?;
            self.broad_anchor(&tag).await
        })
    }

    fn submit_raw<'a>(
        &'a self,
        raw_transaction: &'a ExactRawTransaction,
    ) -> ProviderFuture<'a, EvmHash> {
        let encoded = format!("0x{}", hex::encode(raw_transaction.as_bytes()));
        Box::pin(async move {
            self.rpc::<_, EvmHash>(EvmRpcMethod::SendRawTransaction, &(encoded,))
                .await
                .map(RpcObservation::into_value)
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
struct RpcEnvelope<'a> {
    jsonrpc: RpcVersion,
    id: RpcId,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    result: Option<&'a serde_json::value::RawValue>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    error: Option<&'a serde_json::value::RawValue>,
}

fn present_raw<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<&'de serde_json::value::RawValue>, D::Error> {
    <&serde_json::value::RawValue>::deserialize(deserializer).map(Some)
}

struct RpcObservation<T> {
    method: EvmRpcMethod,
    status: StatusCode,
    value: T,
}
impl<T> RpcObservation<T> {
    fn checked<U>(
        self,
        field: RpcField,
        check: impl FnOnce(T) -> Result<U, RpcRejection>,
    ) -> Result<U, AdapterError<EvmOperationalError>> {
        check(self.value).map_err(|cause| rejected(self.method, self.status, field, cause))
    }
    fn into_value(self) -> T {
        self.value
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
struct RpcError<'a> {
    code: i64,
    message: String,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    data: Option<&'a serde_json::value::RawValue>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RpcFundingCall<'a> {
    from: &'a EvmAddress,
    to: &'a EvmAddress,
    value: String,
    gas: &'static str,
    max_fee_per_gas: String,
    max_priority_fee_per_gas: String,
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
    number: String,
    hash: EvmHash,
}

impl RpcBlock {
    fn into_anchor(self) -> Result<EvmBlockAnchor, RpcRejection> {
        Ok(EvmBlockAnchor {
            number: EvmU256::new(RpcQuantity::parse(&self.number)?.decimal())
                .map_err(|cause| RpcRejection::Domain { cause })?,
            hash: self.hash,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcReceipt {
    transaction_hash: EvmHash,
    #[serde(rename = "from")]
    sender: EvmAddress,
    #[serde(deserialize_with = "required_field")]
    to: Option<EvmAddress>,
    #[serde(deserialize_with = "required_field")]
    contract_address: Option<EvmAddress>,
    status: String,
    block_number: String,
    block_hash: EvmHash,
}

fn required_field<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer)
}

/// Checked JSON-RPC QUANTITY ingress.
struct RpcQuantity(U256);

impl RpcQuantity {
    fn parse(value: &str) -> Result<Self, RpcRejection> {
        let digits = value.strip_prefix("0x").ok_or(RpcRejection::Encoding)?;
        if digits.is_empty()
            || (digits.len() > 1 && digits.starts_with('0'))
            || digits.len() > 64
            || !is_lower_hex(digits)
        {
            return Err(RpcRejection::Encoding);
        }
        let mut bytes = [0_u8; 32];
        for (index, digit) in digits.bytes().rev().enumerate() {
            bytes[31 - index / 2] |= hex_digit(digit) << (4 * (index % 2));
        }
        Ok(Self(U256::from_be_bytes(bytes)))
    }

    fn bounded_u64(&self, minimum: u64, maximum: u64) -> Result<u64, RpcRejection> {
        match self
            .to_u64()
            .filter(|value| (minimum..=maximum).contains(value))
        {
            Some(value) => Ok(value),
            None => Err(RpcRejection::Range {
                minimum,
                maximum,
                observed: EvmU256::new(self.decimal())
                    .map_err(|cause| RpcRejection::Domain { cause })?,
            }),
        }
    }

    fn decimal(&self) -> String {
        self.0.to_string()
    }

    fn to_u64(&self) -> Option<u64> {
        u64::try_from(self.0).ok()
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct RpcDataText(String);

impl RpcDataText {
    fn parse(self, maximum: usize) -> Result<RpcData, RpcRejection> {
        RpcData::parse(&self.0, maximum)
    }
}

/// Checked JSON-RPC DATA ingress.
#[derive(Debug, PartialEq, Eq)]
struct RpcData(Vec<u8>);

impl RpcData {
    fn parse(value: &str, maximum: usize) -> Result<Self, RpcRejection> {
        let digits = value.strip_prefix("0x").ok_or(RpcRejection::Encoding)?;
        if digits.len() / 2 > maximum {
            return Err(RpcRejection::Size {
                limit: maximum as u64,
                observed: ObservedSize::Exact {
                    value: (digits.len() / 2) as u64,
                },
            });
        }
        if digits.len() % 2 != 0 || !is_lower_hex(digits) {
            return Err(RpcRejection::Encoding);
        }
        Ok(Self(
            digits
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| (hex_digit(pair[0]) << 4) | hex_digit(pair[1]))
                .collect(),
        ))
    }
}

/// One exact ABI word decoded from JSON-RPC DATA.
struct AbiWord([u8; 32]);

impl TryFrom<RpcData> for AbiWord {
    type Error = RpcRejection;

    fn try_from(data: RpcData) -> Result<Self, Self::Error> {
        let observed = data.0.len() as u64;
        data.0.try_into().map(Self).map_err(|_| RpcRejection::Size {
            limit: 32,
            observed: ObservedSize::Exact { value: observed },
        })
    }
}

fn provider_failure(
    method: EvmRpcMethod,
    stage: RpcStage,
    failure: ProviderFailureKind,
    status: Option<StatusCode>,
    source: Option<&(dyn std::error::Error + 'static)>,
    rpc_error: Option<&RpcError<'_>>,
) -> AdapterError<EvmOperationalError> {
    let rate_limited = status.is_some_and(|status| status == StatusCode::TOO_MANY_REQUESTS);
    let timeout = source
        .and_then(|source| source.downcast_ref::<reqwest::Error>())
        .is_some_and(reqwest::Error::is_timeout);
    let response = status.map(|status| {
        let mut response = serde_json::json!({
            "status": status.as_u16(),
            "rpc_code": rpc_error.map(|error| error.code),
        });
        if let Some(error) = rpc_error {
            response["message"] = error.message.as_str().into();
            response["data_json"] = serde_json::json!(error.data.map(|data| data.get()));
        }
        response
    });
    let diagnostics = capture::capture(response, source);
    let source = ProviderFailure {
        method,
        stage,
        failure,
        diagnostics,
    };
    let kind = if timeout {
        EvmOperationalKind::Timeout
    } else if rate_limited {
        EvmOperationalKind::RateLimited
    } else {
        EvmOperationalKind::Unavailable
    };
    AdapterError::Operational(EvmOperationalError::new(kind, source))
}

fn client_failure(
    method: EvmRpcMethod,
    stage: RpcStage,
    status: Option<StatusCode>,
    error: &(dyn std::error::Error + 'static),
) -> AdapterError<EvmOperationalError> {
    provider_failure(
        method,
        stage,
        ProviderFailureKind::Client,
        status,
        Some(error),
        None,
    )
}

fn rejected(
    method: EvmRpcMethod,
    status: StatusCode,
    field: RpcField,
    cause: RpcRejection,
) -> AdapterError<EvmOperationalError> {
    provider_failure(
        method,
        RpcStage::Result,
        ProviderFailureKind::Rejected { field, cause },
        Some(status),
        None,
        None,
    )
}

async fn bounded_body(
    method: EvmRpcMethod,
    status: StatusCode,
    mut response: reqwest::Response,
) -> Result<Vec<u8>, AdapterError<EvmOperationalError>> {
    let maximum = match method {
        EvmRpcMethod::Accounts | EvmRpcMethod::SendTransaction => 16 * 1024,
        _ => MAX_RESPONSE_BYTES,
    };
    if let Some(length) = response
        .content_length()
        .filter(|length| *length > maximum as u64)
    {
        return Err(provider_failure(
            method,
            RpcStage::Body,
            ProviderFailureKind::Rejected {
                field: RpcField::Data,
                cause: RpcRejection::Size {
                    limit: maximum as u64,
                    observed: ObservedSize::Exact { value: length },
                },
            },
            Some(status),
            None,
            None,
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| client_failure(method, RpcStage::Body, Some(status), &error))?
    {
        let observed = body.len() as u64 + chunk.len() as u64;
        if observed > maximum as u64 {
            return Err(provider_failure(
                method,
                RpcStage::Body,
                ProviderFailureKind::Rejected {
                    field: RpcField::Data,
                    cause: RpcRejection::Size {
                        limit: maximum as u64,
                        observed: ObservedSize::AtLeast { value: observed },
                    },
                },
                Some(status),
                None,
                None,
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn decimals_calldata() -> String {
    format!("0x{DECIMALS_SELECTOR}")
}

fn balance_of_calldata(holder: &Address) -> String {
    format!("0x{BALANCE_OF_SELECTOR}{}", hex::encode(holder.into_word()))
}

/// Renders one checked decimal block number as its `0x` quantity tag.
fn block_tag(number: &EvmU256) -> Result<String, AdapterError<EvmOperationalError>> {
    number
        .as_str()
        .parse::<U256>()
        .map(|number| format!("{number:#x}"))
        .map_err(|source| invariant(AdapterFailure::BlockTag { source }))
}

// Called only after checking the closed lowercase hexadecimal alphabet.
fn hex_digit(byte: u8) -> u8 {
    if byte.is_ascii_digit() {
        byte - b'0'
    } else {
        byte - b'a' + 10
    }
}

fn is_lower_hex(digits: &str) -> bool {
    digits
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_abi_u256(word: AbiWord) -> U256 {
    U256::from_be_bytes(word.0)
}

fn parse_receipt(value: RpcReceipt) -> Result<ProviderReceipt, RpcRejection> {
    let quantity = RpcQuantity::parse(&value.status)?;
    let status = quantity.to_u64().filter(|status| *status <= 1);
    let block_anchor = EvmBlockAnchor {
        number: EvmU256::new(RpcQuantity::parse(&value.block_number)?.decimal())
            .map_err(|cause| RpcRejection::Domain { cause })?,
        hash: value.block_hash,
    };
    let result = match (status, value.to, value.contract_address) {
        (Some(1), None, Some(contract_address)) => {
            ProviderReceiptResult::SuccessCreate { contract_address }
        }
        (Some(0), None, None) => ProviderReceiptResult::RevertedCreate,
        (Some(1), Some(target), None) => ProviderReceiptResult::SuccessCall { target },
        (Some(0), Some(target), None) => ProviderReceiptResult::RevertedCall { target },
        (_, to, contract_address) => {
            return Err(RpcRejection::ReceiptOutcome {
                status: EvmU256::new(quantity.decimal())
                    .map_err(|cause| RpcRejection::Domain { cause })?,
                to,
                contract_address,
            })
        }
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
pub(crate) mod tests;
