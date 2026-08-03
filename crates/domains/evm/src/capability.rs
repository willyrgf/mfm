//! Closed EVM protocol values used at audited capability boundaries.
//!
//! Every read request in this module denotes exactly one JSON-RPC method call.
//! The values contain an immutable routing-generation reference, but never an
//! endpoint, credential, provider message, response body, or local path.

use std::str::FromStr;

use alloy_eips::eip2930::AccessList;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use mfm_ids::{ContentDigest, ContentRef, LocalPublicId, SchemaId};
use mfm_program_derive::MfmValue;
use serde::{de, Deserialize, Serialize};

use crate::{model::EvmBlockAnchor, EvmChainInstanceBinding};

/// Maximum decoded bytes admitted for one EVM JSON-RPC result.
pub const EVM_READ_MAX_RESPONSE_BYTES: usize = 128 * 1024;
/// Exact operation id for one `eth_chainId` call.
pub const EVM_CHAIN_ID_OPERATION_ID: &str = "eth_chain_id";
/// Exact operation id for the initial latest-block call.
pub const EVM_LATEST_ANCHOR_OPERATION_ID: &str = "eth_get_block_by_number_latest";
/// Exact operation id for one ERC-20 decimals call.
pub const EVM_TOKEN_DECIMALS_OPERATION_ID: &str = "eth_call_erc20_decimals";
/// Exact operation id for one native balance call.
pub const EVM_NATIVE_BALANCE_OPERATION_ID: &str = "eth_get_balance";
/// Exact operation id for one ERC-20 balance call.
pub const EVM_TOKEN_BALANCE_OPERATION_ID: &str = "eth_call_erc20_balance_of";
/// Exact operation id for final number-to-hash confirmation.
pub const EVM_CONFIRM_ANCHOR_OPERATION_ID: &str = "eth_get_block_by_number_confirm";
/// Complete closed EVM read operation inventory.
pub const EVM_READ_OPERATION_IDS: [&str; 6] = [
    EVM_CHAIN_ID_OPERATION_ID,
    EVM_LATEST_ANCHOR_OPERATION_ID,
    EVM_TOKEN_DECIMALS_OPERATION_ID,
    EVM_NATIVE_BALANCE_OPERATION_ID,
    EVM_TOKEN_BALANCE_OPERATION_ID,
    EVM_CONFIRM_ANCHOR_OPERATION_ID,
];
/// EIP-2718 transaction type retained by the neutral signing protocol.
pub const EVM_EIP1559_TRANSACTION_TYPE: u8 = 2;

/// Result type for checked EVM protocol construction.
pub type Result<T> = std::result::Result<T, EvmProtocolError>;

/// Redaction-safe checked-protocol failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmProtocolError {
    /// A persisted identity or scalar was not canonical.
    #[error("invalid EVM protocol value: {0}")]
    InvalidValue(&'static str),
    /// Checked fee arithmetic overflowed.
    #[error("EVM fee arithmetic overflowed")]
    FeeOverflow,
    /// A transaction quantity cannot be represented by the EIP-1559 model.
    #[error("EVM transaction quantity is out of range: {0}")]
    QuantityOutOfRange(&'static str),
    /// A receipt and its log material disagree.
    #[error("EVM receipt material is internally inconsistent")]
    IncoherentReceipt,
}

/// Immutable non-secret reference to one locally qualified routing generation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "routing-generation-ref",
    version = "1",
    schema = "mfm.evm.routing_generation_ref"
)]
pub struct EvmRoutingGenerationRef {
    schema_id: String,
    content_digest: String,
}

impl EvmRoutingGenerationRef {
    /// Creates a checked generation reference from its exact content reference.
    pub fn from_content_ref(content_ref: ContentRef) -> Result<Self> {
        Ok(Self {
            schema_id: content_ref.schema_id().as_str().to_owned(),
            content_digest: content_ref.content_digest().as_str().to_owned(),
        })
    }

    /// Reconstructs the exact content reference.
    pub fn to_content_ref(&self) -> Result<ContentRef> {
        let schema_id = SchemaId::from_str(&self.schema_id)
            .map_err(|_| EvmProtocolError::InvalidValue("routing_generation_ref"))?;
        let content_digest = ContentDigest::from_str(&self.content_digest)
            .map_err(|_| EvmProtocolError::InvalidValue("routing_generation_ref"))?;
        ContentRef::new(schema_id, content_digest)
            .map_err(|_| EvmProtocolError::InvalidValue("routing_generation_ref"))
    }

    /// Returns the exact schema identity string.
    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    /// Returns the exact byte-digest identity string.
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }
}

impl<'de> Deserialize<'de> for EvmRoutingGenerationRef {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_id: String,
            content_digest: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let reference = Self {
            schema_id: wire.schema_id,
            content_digest: wire.content_digest,
        };
        reference.to_content_ref().map_err(de::Error::custom)?;
        Ok(reference)
    }
}

/// Checked semantic network and expected chain selected before live execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "network-binding",
    version = "1",
    schema = "mfm.evm.network_binding"
)]
pub struct EvmNetworkBinding {
    network_id: String,
    chain_instance: EvmChainInstanceBinding,
    routing_generation_ref: EvmRoutingGenerationRef,
}

impl EvmNetworkBinding {
    /// Creates one semantic binding to an exact qualified physical chain.
    pub fn new(
        network_id: impl Into<String>,
        chain_instance: EvmChainInstanceBinding,
        routing_generation_ref: EvmRoutingGenerationRef,
    ) -> Result<Self> {
        let network_id = network_id.into();
        LocalPublicId::new(&network_id)
            .map_err(|_| EvmProtocolError::InvalidValue("network_id"))?;
        chain_instance
            .validate()
            .map_err(|_| EvmProtocolError::InvalidValue("chain_instance"))?;
        routing_generation_ref
            .to_content_ref()
            .map_err(|_| EvmProtocolError::InvalidValue("routing_generation_ref"))?;
        Ok(Self {
            network_id,
            chain_instance,
            routing_generation_ref,
        })
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the required EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_instance.chain_id()
    }

    /// Returns the exact qualified physical-chain binding.
    pub const fn chain_instance(&self) -> &EvmChainInstanceBinding {
        &self.chain_instance
    }

    /// Returns the exact immutable routing generation.
    pub const fn routing_generation_ref(&self) -> &EvmRoutingGenerationRef {
        &self.routing_generation_ref
    }
}

impl<'de> Deserialize<'de> for EvmNetworkBinding {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network_id: String,
            chain_instance: EvmChainInstanceBinding,
            routing_generation_ref: EvmRoutingGenerationRef,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.network_id,
            wire.chain_instance,
            wire.routing_generation_ref,
        )
        .map_err(de::Error::custom)
    }
}

/// Reviewed non-secret source identity returned by the bootstrap operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "checked-source",
    version = "1",
    schema = "mfm.evm.checked_source"
)]
pub struct EvmCheckedSource {
    binding: EvmNetworkBinding,
    source_scope: String,
    implementation_id: String,
}

impl EvmCheckedSource {
    /// Creates source evidence after an exact-generation chain-id call.
    pub fn new(
        binding: EvmNetworkBinding,
        source_scope: impl Into<String>,
        implementation_id: impl Into<String>,
    ) -> Result<Self> {
        let source_scope = source_scope.into();
        let implementation_id = implementation_id.into();
        LocalPublicId::new(&source_scope)
            .map_err(|_| EvmProtocolError::InvalidValue("source_scope"))?;
        LocalPublicId::new(&implementation_id)
            .map_err(|_| EvmProtocolError::InvalidValue("implementation_id"))?;
        Ok(Self {
            binding,
            source_scope,
            implementation_id,
        })
    }

    /// Returns the semantic binding.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }

    /// Returns reviewed local source scope identity.
    pub fn source_scope(&self) -> &str {
        &self.source_scope
    }

    /// Returns reviewed implementation identity.
    pub fn implementation_id(&self) -> &str {
        &self.implementation_id
    }
}

impl<'de> Deserialize<'de> for EvmCheckedSource {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            binding: EvmNetworkBinding,
            source_scope: String,
            implementation_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.binding, wire.source_scope, wire.implementation_id)
            .map_err(de::Error::custom)
    }
}

/// One-operation request for `eth_chainId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "chain-identity-request",
    version = "1",
    schema = "mfm.evm.request.chain_identity"
)]
pub struct EvmChainIdentityRequest {
    binding: EvmNetworkBinding,
}

impl EvmChainIdentityRequest {
    /// Creates the exact-generation bootstrap request.
    pub fn new(binding: EvmNetworkBinding) -> Self {
        Self { binding }
    }

    /// Returns the expected binding and immutable route.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }
}

/// Returned result of one exact-generation `eth_chainId` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "chain-identity-response",
    version = "1",
    schema = "mfm.evm.response.chain_identity"
)]
pub struct EvmChainIdentityResponse {
    /// Provider-returned chain id.
    pub chain_id: u64,
    /// Reviewed local source scope resolved from the exact generation.
    pub source_scope: String,
    /// Reviewed implementation identity.
    pub implementation_id: String,
}

/// One-operation request for the latest block number/hash pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "latest-anchor-request",
    version = "1",
    schema = "mfm.evm.request.latest_anchor"
)]
pub struct EvmLatestAnchorRequest {
    source: EvmCheckedSource,
}

impl EvmLatestAnchorRequest {
    /// Creates an exact-source latest-anchor request.
    pub fn new(source: EvmCheckedSource) -> Self {
        Self { source }
    }

    /// Returns the checked source.
    pub const fn source(&self) -> &EvmCheckedSource {
        &self.source
    }
}

/// Returned result of one block-header operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "block-response",
    version = "1",
    schema = "mfm.evm.response.block"
)]
pub struct EvmBlockResponse {
    /// Exact number/hash pair returned by the provider.
    pub anchor: EvmBlockAnchor,
}

/// Checked source pinned to one initial block anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchored-source",
    version = "1",
    schema = "mfm.evm.anchored_source"
)]
pub struct EvmAnchoredSource {
    source: EvmCheckedSource,
    anchor: EvmBlockAnchor,
}

impl EvmAnchoredSource {
    /// Creates a source/anchor pair.
    pub fn new(source: EvmCheckedSource, anchor: EvmBlockAnchor) -> Result<Self> {
        anchor
            .validate()
            .map_err(|_| EvmProtocolError::InvalidValue("block_anchor"))?;
        Ok(Self { source, anchor })
    }

    /// Returns the checked source.
    pub const fn source(&self) -> &EvmCheckedSource {
        &self.source
    }

    /// Returns the initial block anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }
}

/// One-operation request for native balance at an exact EIP-1898 block hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "native-balance-request",
    version = "1",
    schema = "mfm.evm.request.native_balance"
)]
pub struct EvmNativeBalanceRequest {
    source: EvmAnchoredSource,
    account: String,
}

impl EvmNativeBalanceRequest {
    /// Creates a checked one-account request.
    pub fn new(source: EvmAnchoredSource, account: Address) -> Self {
        Self {
            source,
            account: format!("{account:#x}"),
        }
    }

    /// Returns the pinned source.
    pub const fn source(&self) -> &EvmAnchoredSource {
        &self.source
    }

    /// Returns the canonical account address.
    pub fn account(&self) -> &str {
        &self.account
    }
}

/// One-operation request for ERC-20 `decimals()` at an exact block hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "token-decimals-request",
    version = "1",
    schema = "mfm.evm.request.token_decimals"
)]
pub struct EvmTokenDecimalsRequest {
    source: EvmAnchoredSource,
    contract_address: String,
}

impl EvmTokenDecimalsRequest {
    /// Creates a checked one-contract metadata request.
    pub fn new(source: EvmAnchoredSource, contract_address: Address) -> Self {
        Self {
            source,
            contract_address: format!("{contract_address:#x}"),
        }
    }

    /// Returns the pinned source.
    pub const fn source(&self) -> &EvmAnchoredSource {
        &self.source
    }

    /// Returns the canonical token address.
    pub fn contract_address(&self) -> &str {
        &self.contract_address
    }
}

/// One-operation request for ERC-20 `balanceOf(address)` at an exact block hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "token-balance-request",
    version = "1",
    schema = "mfm.evm.request.token_balance"
)]
pub struct EvmTokenBalanceRequest {
    source: EvmAnchoredSource,
    account: String,
    contract_address: String,
}

impl EvmTokenBalanceRequest {
    /// Creates a checked one-account/contract request.
    pub fn new(source: EvmAnchoredSource, account: Address, contract_address: Address) -> Self {
        Self {
            source,
            account: format!("{account:#x}"),
            contract_address: format!("{contract_address:#x}"),
        }
    }

    /// Returns the pinned source.
    pub const fn source(&self) -> &EvmAnchoredSource {
        &self.source
    }

    /// Returns the canonical account address.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the canonical token address.
    pub fn contract_address(&self) -> &str {
        &self.contract_address
    }
}

/// Returned canonical U256 quantity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "quantity-response",
    version = "1",
    schema = "mfm.evm.response.quantity"
)]
pub struct EvmQuantityResponse {
    quantity_dec: String,
}

impl EvmQuantityResponse {
    /// Creates a canonical decimal response from an Alloy U256.
    pub fn new(quantity: U256) -> Self {
        Self {
            quantity_dec: quantity.to_string(),
        }
    }

    /// Returns the canonical decimal quantity.
    pub fn quantity_dec(&self) -> &str {
        &self.quantity_dec
    }

    /// Parses the response quantity.
    pub fn quantity(&self) -> Result<U256> {
        parse_u256_decimal(&self.quantity_dec)
    }
}

impl<'de> Deserialize<'de> for EvmQuantityResponse {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            quantity_dec: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let quantity = parse_u256_decimal(&wire.quantity_dec).map_err(de::Error::custom)?;
        Ok(Self::new(quantity))
    }
}

/// Returned ERC-20 decimal scale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "token-decimals-response",
    version = "1",
    schema = "mfm.evm.response.token_decimals"
)]
pub struct EvmTokenDecimalsResponse {
    /// ABI-decoded decimal scale.
    pub decimals: u8,
}

/// One-operation request that resolves the initial block number to its current hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "anchor-confirmation-request",
    version = "1",
    schema = "mfm.evm.request.anchor_confirmation"
)]
pub struct EvmAnchorConfirmationRequest {
    source: Option<EvmAnchoredSource>,
}

impl EvmAnchorConfirmationRequest {
    /// Creates a number-to-hash confirmation request.
    pub fn new(source: EvmAnchoredSource) -> Self {
        Self {
            source: Some(source),
        }
    }

    /// Creates the total request-author result for semantically invalid fan-in.
    ///
    /// Live binding rejects this value as `request_invalid` before provider IO.
    pub fn invalid_input() -> Self {
        Self { source: None }
    }

    /// Returns the initial source and number/hash pair when input was valid.
    pub const fn source(&self) -> Option<&EvmAnchoredSource> {
        self.source.as_ref()
    }
}

/// Closed reviewed response-shape diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "response-invalid-kind",
    version = "1",
    schema = "mfm.evm.response_invalid_kind"
)]
pub enum EvmResponseInvalidKind {
    /// JSON-RPC envelope or id was malformed.
    MalformedEnvelope,
    /// The mutually exclusive result field was absent.
    MissingResult,
    /// The typed result could not be decoded.
    InvalidResult,
    /// The bounded response limit was exceeded.
    TooLarge,
}

/// Reviewed coarse response-size class retained without raw byte counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "coarse-size-class",
    version = "1",
    schema = "mfm.evm.coarse_size_class"
)]
pub enum EvmCoarseSizeClass {
    /// No response bytes were present.
    Zero,
    /// At most 16 KiB were present.
    UpTo16Kib,
    /// More than 16 KiB and at most 1 MiB were present.
    UpTo1Mib,
    /// More than 1 MiB were present.
    Over1Mib,
}

impl EvmCoarseSizeClass {
    /// Classifies a byte count without retaining the exact count.
    pub const fn from_byte_length(bytes: usize) -> Self {
        match bytes {
            0 => Self::Zero,
            1..=16_384 => Self::UpTo16Kib,
            16_385..=1_048_576 => Self::UpTo1Mib,
            _ => Self::Over1Mib,
        }
    }
}

/// Closed redaction-safe diagnostic retained only for value-bearing safe failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "diagnostic", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "safe-diagnostic",
    version = "1",
    schema = "mfm.evm.safe_diagnostic"
)]
pub enum EvmSafeDiagnostic {
    /// Reviewed HTTP status only.
    HttpStatus {
        /// Numeric status; no body or header material.
        status: u16,
    },
    /// Reviewed JSON-RPC numeric code only.
    JsonRpcError {
        /// Numeric JSON-RPC error code; no provider message/data.
        code: i64,
    },
    /// Reviewed response-invalid category only.
    ResponseInvalid {
        /// Exact invalid-response category admitted by the classifier rule.
        kind: EvmResponseInvalidKind,
    },
}

/// Transient closed EVM transport failure projected into safe metadata and an optional diagnostic.
///
/// This transport value is never persisted. The adapter maps it totally to the generic
/// safe-failure envelope plus an optional [`EvmSafeDiagnostic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmSafeFailure {
    /// The exact immutable local generation was unavailable.
    RoutingGenerationUnavailable,
    /// The qualified generation could not construct a valid transport.
    ConfigurationInvalid,
    /// The authored request was not a valid operation request.
    RequestInvalid,
    /// Access was cancelled.
    AccessCancelled,
    /// Transport entry or outcome failed without a response.
    TransportFailed,
    /// Reviewed HTTP status only.
    HttpStatus {
        /// Numeric status; no body or header material.
        status: u16,
    },
    /// Reviewed JSON-RPC numeric code only.
    JsonRpcError {
        /// Numeric JSON-RPC error code; no provider message/data.
        json_rpc_code: i64,
    },
    /// Closed response-invalid diagnostic.
    ResponseInvalid {
        /// Reviewed invalid-response class.
        response_kind: EvmResponseInvalidKind,
        /// Reviewed response-size class.
        size_class: EvmCoarseSizeClass,
    },
    /// A provider result field was missing.
    ResponseMissingResult {
        /// Reviewed response-size class.
        size_class: EvmCoarseSizeClass,
    },
    /// A transport or decoded-result bound was exceeded.
    ResponseTooLarge {
        /// Reviewed response-size class.
        size_class: EvmCoarseSizeClass,
    },
    /// No more specific reviewed failure class applied.
    UnclassifiedFailure,
}

/// Semantic terminal read failure exposed by state settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "read-failure",
    version = "1",
    schema = "mfm.evm.read_failure"
)]
pub enum EvmReadFailure {
    /// The selected exact route could not return reviewed evidence.
    Unavailable,
    /// Returned chain identity did not match the certified semantic source.
    SourceMismatch,
    /// Final number-to-hash resolution did not preserve the initial anchor.
    AnchorChanged,
    /// A terminal HTTP or JSON-RPC numeric rejection was observed.
    DestinationRejected,
    /// Pure aggregation input violated the certified structured-program contract.
    InvalidAggregate,
}

/// Checked EIP-1559 fee inputs retained as neutral protocol primitives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmFeeInputs {
    /// Observed base fee per gas.
    pub base_fee_per_gas: U256,
    /// Suggested priority fee per gas.
    pub max_priority_fee_per_gas: U256,
    /// Checked `base_fee * 2 + priority_fee` maximum fee.
    pub max_fee_per_gas: U256,
}

impl EvmFeeInputs {
    /// Builds the retained fee formula with checked arithmetic.
    pub fn from_base_and_priority(
        base_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<Self> {
        let max_fee_per_gas = base_fee_per_gas
            .checked_mul(U256::from(2))
            .and_then(|value| value.checked_add(max_priority_fee_per_gas))
            .ok_or(EvmProtocolError::FeeOverflow)?;
        Ok(Self {
            base_fee_per_gas,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        })
    }
}

/// Exact pre-gas-limit EIP-1559 estimation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTransactionEstimate {
    chain_id: U256,
    nonce: U256,
    from: Address,
    to: TxKind,
    value: U256,
    input: Bytes,
    access_list: AccessList,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
}

impl EvmTransactionEstimate {
    /// Creates one checked type-2 gas-estimation request.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: U256,
        nonce: U256,
        from: Address,
        to: TxKind,
        value: U256,
        input: Bytes,
        access_list: AccessList,
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<Self> {
        if u64::try_from(chain_id)
            .ok()
            .filter(|value| *value != 0)
            .is_none()
        {
            return Err(EvmProtocolError::QuantityOutOfRange("chain_id"));
        }
        u64::try_from(nonce).map_err(|_| EvmProtocolError::QuantityOutOfRange("nonce"))?;
        let max_fee = u128::try_from(max_fee_per_gas)
            .map_err(|_| EvmProtocolError::QuantityOutOfRange("max_fee_per_gas"))?;
        let priority = u128::try_from(max_priority_fee_per_gas)
            .map_err(|_| EvmProtocolError::QuantityOutOfRange("max_priority_fee_per_gas"))?;
        if priority > max_fee {
            return Err(EvmProtocolError::InvalidValue("max_priority_fee_per_gas"));
        }
        Ok(Self {
            chain_id,
            nonce,
            from,
            to,
            value,
            input,
            access_list,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        })
    }

    /// Returns the EIP-2718 transaction type.
    pub const fn transaction_type(&self) -> u8 {
        EVM_EIP1559_TRANSACTION_TYPE
    }

    /// Returns the exact chain id.
    pub const fn chain_id(&self) -> U256 {
        self.chain_id
    }

    /// Returns the exact pending sender nonce.
    pub const fn nonce(&self) -> U256 {
        self.nonce
    }

    /// Returns the sender.
    pub const fn from(&self) -> Address {
        self.from
    }

    /// Returns call or creation destination kind.
    pub const fn to(&self) -> TxKind {
        self.to
    }

    /// Returns transferred value.
    pub const fn value(&self) -> U256 {
        self.value
    }

    /// Returns calldata or init code.
    pub const fn input(&self) -> &Bytes {
        &self.input
    }

    /// Returns the access list.
    pub const fn access_list(&self) -> &AccessList {
        &self.access_list
    }

    /// Returns the maximum total fee per gas.
    pub const fn max_fee_per_gas(&self) -> U256 {
        self.max_fee_per_gas
    }

    /// Returns the maximum priority fee per gas.
    pub const fn max_priority_fee_per_gas(&self) -> U256 {
        self.max_priority_fee_per_gas
    }
}

/// Optional block placement of an observed transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTransactionPlacement {
    /// Exact inclusion block identity.
    pub block: EvmBlockAnchor,
    /// Transaction index within the block.
    pub transaction_index: U256,
}

/// Public fields of an observed EIP-1559 transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmObservedTransaction {
    /// Transaction hash.
    pub transaction_hash: B256,
    /// Chain id.
    pub chain_id: U256,
    /// Sender nonce.
    pub nonce: U256,
    /// Sender.
    pub from: Address,
    /// Call or creation destination kind.
    pub to: TxKind,
    /// Transferred value.
    pub value: U256,
    /// Calldata or init code.
    pub input: Bytes,
    /// Gas limit.
    pub gas_limit: U256,
    /// Maximum fee per gas.
    pub max_fee_per_gas: U256,
    /// Maximum priority fee per gas.
    pub max_priority_fee_per_gas: U256,
    /// Access list.
    pub access_list: AccessList,
    /// Block placement, or none while pending.
    pub placement: Option<EvmTransactionPlacement>,
}

/// Strict receipt execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvmReceiptStatus {
    /// Execution succeeded.
    Success,
    /// Execution reverted after consuming nonce and gas.
    Reverted,
}

/// Complete log entry carried by a transaction receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmReceiptLog {
    /// Emitting address.
    pub address: Address,
    /// Indexed topics.
    pub topics: Vec<B256>,
    /// Unindexed log data.
    pub data: Bytes,
    /// Exact inclusion block identity.
    pub block: EvmBlockAnchor,
    /// Enclosing transaction hash.
    pub transaction_hash: B256,
    /// Transaction index within the block.
    pub transaction_index: U256,
    /// Log index within the block.
    pub log_index: U256,
    /// Whether the provider marked the log removed.
    pub removed: bool,
}

/// Strict transaction receipt observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmReceipt {
    /// Transaction hash.
    pub transaction_hash: B256,
    /// Transaction index within the block.
    pub transaction_index: U256,
    /// Exact inclusion block identity.
    pub block: EvmBlockAnchor,
    /// Sender.
    pub from: Address,
    /// Destination, or none for creation.
    pub to: Option<Address>,
    /// Created contract address, when reported.
    pub contract_address: Option<Address>,
    /// Execution status.
    pub status: EvmReceiptStatus,
    /// Gas used by this transaction.
    pub gas_used: U256,
    /// Cumulative gas used in the block.
    pub cumulative_gas_used: U256,
    /// Complete, internally coherent logs.
    pub logs: Vec<EvmReceiptLog>,
}

impl EvmReceipt {
    /// Validates execution status, log identity, and removal status.
    pub fn validate(&self) -> Result<()> {
        if matches!(self.status, EvmReceiptStatus::Reverted) && !self.logs.is_empty() {
            return Err(EvmProtocolError::IncoherentReceipt);
        }
        if self.logs.iter().any(|log| {
            log.removed
                || log.transaction_hash != self.transaction_hash
                || log.transaction_index != self.transaction_index
                || log.block != self.block
        }) {
            return Err(EvmProtocolError::IncoherentReceipt);
        }
        Ok(())
    }
}

fn parse_u256_decimal(raw: &str) -> Result<U256> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EvmProtocolError::InvalidValue("quantity_dec"));
    }
    let value = U256::from_str(raw).map_err(|_| EvmProtocolError::InvalidValue("quantity_dec"))?;
    if value.to_string() != raw {
        return Err(EvmProtocolError::InvalidValue("quantity_dec"));
    }
    Ok(value)
}
