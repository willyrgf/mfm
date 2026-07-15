#![warn(missing_docs)]
//! Reusable EVM capability contracts.
//!
//! This crate defines state/adapter-facing EVM authority contracts. It owns
//! checked source references, redacted source evidence, request/response
//! contracts, and provider traits. Concrete JSON-RPC clients and runtime
//! routing live outside this crate.
//!
//! ```rust
//! use mfm_capabilities::CapabilitySpec;
//! use mfm_evm_capabilities::{
//!     EvmFeeReadCapability, EvmFeeReadRequest, EvmNetworkBinding, EvmNetworkId,
//! };
//!
//! let binding = EvmNetworkBinding::new(EvmNetworkId::new("ethereum-mainnet")?, 1)?;
//! let request = EvmFeeReadRequest::new();
//! assert_eq!(binding.network_id().as_str(), "ethereum-mainnet");
//! assert_eq!(EvmFeeReadCapability::name(), "mfm.evm.fee.read");
//! assert_eq!(request, EvmFeeReadRequest::new());
//! # Ok::<(), mfm_evm_capabilities::EvmCapabilityError>(())
//! ```

use std::fmt;
use std::future::Future;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::str::FromStr;

use alloy_primitives::{Address, B256, U256};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ExternalMutationAuthorityRole, ProviderDiagnosticCode,
    ProviderDiagnosticValue, ReadExternalRole, RedactedProviderDiagnostic,
};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm, LocalPublicId};

/// Result type for EVM capability contracts.
pub type Result<T> = std::result::Result<T, EvmCapabilityError>;

/// Boxed future returned by EVM capability providers.
pub type EvmCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Stable runtime implementation id for generic EVM JSON-RPC read capabilities.
///
/// Multiple adapters may consume the same generic read capability through the
/// same process-local JSON-RPC provider. Their runner registrations must use
/// this one id so a certified capability descriptor has one unambiguous live
/// implementation binding.
pub const EVM_JSONRPC_CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.evm.jsonrpc.runtime.v1";

macro_rules! evm_capability {
    ($(#[$meta:meta])* $ty:ident, $role:ty, $name:literal) => {
        $(#[$meta])*
        pub struct $ty;

        impl CapabilitySpec for $ty {
            type Role = $role;

            fn kind() -> mfm_capabilities::Result<CapabilityKind> {
                evm_capability_kind($name)
            }

            fn version() -> mfm_capabilities::Result<CapabilityVersion> {
                evm_capability_version($name)
            }

            fn name() -> &'static str {
                concat!("mfm.evm.", $name)
            }
        }
    };
}

evm_capability!(
    /// EVM chain identity read authority.
    EvmChainIdentityCapability,
    ReadExternalRole,
    "chain_identity.read"
);
evm_capability!(
    /// EVM block read authority.
    EvmBlockReadCapability,
    ReadExternalRole,
    "block.read"
);
evm_capability!(
    /// EVM account balance read authority.
    EvmBalanceReadCapability,
    ReadExternalRole,
    "balance.read"
);
evm_capability!(
    /// EVM contract call read authority.
    EvmCallReadCapability,
    ReadExternalRole,
    "call.read"
);
evm_capability!(
    /// EVM contract bytecode read authority.
    EvmCodeReadCapability,
    ReadExternalRole,
    "code.read"
);
evm_capability!(
    /// EVM log read authority.
    EvmLogsReadCapability,
    ReadExternalRole,
    "logs.read"
);
evm_capability!(
    /// EVM account nonce read authority.
    EvmNonceReadCapability,
    ReadExternalRole,
    "nonce.read"
);
evm_capability!(
    /// EVM fee market read authority.
    EvmFeeReadCapability,
    ReadExternalRole,
    "fee.read"
);
evm_capability!(
    /// EVM gas estimate authority.
    EvmGasEstimateCapability,
    ReadExternalRole,
    "gas_estimate.read"
);
evm_capability!(
    /// EVM transaction submit authority.
    EvmTransactionSubmitCapability,
    ExternalMutationAuthorityRole,
    "transaction.submit"
);
evm_capability!(
    /// EVM transaction receipt read authority.
    EvmReceiptReadCapability,
    ReadExternalRole,
    "receipt.read"
);
evm_capability!(
    /// EVM account nonce occupancy investigation authority.
    EvmNonceOccupancyReadCapability,
    ReadExternalRole,
    "nonce_occupancy.read"
);

fn evm_capability_kind(name: &'static str) -> mfm_capabilities::Result<CapabilityKind> {
    CapabilityKind::new(
        "mfm.evm",
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.evm.capability:{name}").as_bytes()),
    )
    .map_err(|error| CapabilityError::Identity(error.to_string()))
}

fn evm_capability_version(name: &'static str) -> mfm_capabilities::Result<CapabilityVersion> {
    CapabilityVersion::new(format!("mfm.evm.{name}.v1"))
        .map_err(|error| CapabilityError::Identity(error.to_string()))
}

/// Provider interface for EVM chain identity reads.
pub trait EvmChainIdentityProvider: Send + Sync {
    /// Reads chain identity from the selected EVM source.
    ///
    /// A successful response has already enforced the provider binding and chain
    /// identity. Returned source evidence matches the provider binding by construction.
    fn chain_identity<'a>(
        &'a self,
        request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse>;
}

/// Provider interface for EVM block reads.
pub trait EvmBlockReadProvider: Send + Sync {
    /// Reads an EVM block summary.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse>;
}

/// Provider interface for EVM account balance reads.
pub trait EvmBalanceReadProvider: Send + Sync {
    /// Reads an account balance at the selected block.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_balance<'a>(
        &'a self,
        request: &'a EvmBalanceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBalanceReadResponse>;
}

/// Provider interface for EVM contract call reads.
pub trait EvmCallReadProvider: Send + Sync {
    /// Executes a read-only EVM call.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_call<'a>(
        &'a self,
        request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse>;
}

/// Provider interface for EVM contract code reads.
pub trait EvmCodeReadProvider: Send + Sync {
    /// Reads deployed bytecode at an address and block.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_code<'a>(
        &'a self,
        request: &'a EvmCodeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCodeReadResponse>;
}

/// Provider interface for EVM log reads.
pub trait EvmLogsReadProvider: Send + Sync {
    /// Reads EVM logs matching a filter.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_logs<'a>(
        &'a self,
        request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse>;
}

/// Provider interface for EVM nonce reads.
pub trait EvmNonceReadProvider: Send + Sync {
    /// Reads the account transaction count.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_nonce<'a>(
        &'a self,
        request: &'a EvmNonceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse>;
}

/// Provider interface for EVM fee-market reads.
pub trait EvmFeeReadProvider: Send + Sync {
    /// Reads fee-market inputs.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_fee<'a>(
        &'a self,
        request: &'a EvmFeeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse>;
}

/// Provider interface for EVM gas estimates.
pub trait EvmGasEstimateProvider: Send + Sync {
    /// Estimates gas for an EVM transaction intent.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmGasEstimateRequest,
    ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse>;
}

/// Provider interface for EVM transaction submission.
pub trait EvmTransactionSubmitProvider: Send + Sync {
    /// Submits a signed EVM payload.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn submit_transaction<'a>(
        &'a self,
        request: &'a EvmTransactionSubmitRequest,
    ) -> EvmCapabilityFuture<'a, EvmTransactionSubmitResponse>;
}

/// Provider interface for EVM receipt reads.
pub trait EvmReceiptReadProvider: Send + Sync {
    /// Reads an EVM transaction receipt.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding, and the receipt transaction hash matches the request
    /// by construction.
    fn read_receipt<'a>(
        &'a self,
        request: &'a EvmReceiptReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse>;
}

/// Provider interface for EVM nonce occupancy investigations.
pub trait EvmNonceOccupancyReadProvider: Send + Sync {
    /// Reads explicit evidence for whether a concrete sender nonce is occupied by a non-anchor tx.
    ///
    /// A successful response has already enforced the provider binding. Returned source
    /// evidence matches the provider binding by construction.
    fn read_nonce_occupancy<'a>(
        &'a self,
        request: &'a EvmNonceOccupancyReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceOccupancyReadResponse>;
}

/// Process-local EVM source reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvmSourceRef(LocalPublicId);

impl EvmSourceRef {
    /// Creates a checked EVM source reference.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = LocalPublicId::new(value).map_err(invalid_identifier)?;
        Ok(Self(value))
    }

    /// Returns the checked source reference string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for EvmSourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EvmSourceRef {
    type Err = EvmCapabilityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for EvmSourceRef {
    type Error = EvmCapabilityError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<EvmSourceRef> for String {
    fn from(value: EvmSourceRef) -> Self {
        value.0.into_string()
    }
}

/// Semantic EVM network id from authored workflow config.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvmNetworkId(LocalPublicId);

impl EvmNetworkId {
    /// Creates a checked semantic EVM network id.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = LocalPublicId::new(value).map_err(invalid_identifier)?;
        Ok(Self(value))
    }

    /// Returns the checked network id string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for EvmNetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EvmNetworkId {
    type Err = EvmCapabilityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for EvmNetworkId {
    type Error = EvmCapabilityError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<EvmNetworkId> for String {
    fn from(value: EvmNetworkId) -> Self {
        value.0.into_string()
    }
}

/// Process-local EVM source policy id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvmSourcePolicyId(LocalPublicId);

impl EvmSourcePolicyId {
    /// Creates a checked EVM source policy id.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = LocalPublicId::new(value).map_err(invalid_identifier)?;
        Ok(Self(value))
    }

    /// Returns the checked policy id string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for EvmSourcePolicyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EvmSourcePolicyId {
    type Err = EvmCapabilityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for EvmSourcePolicyId {
    type Error = EvmCapabilityError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<EvmSourcePolicyId> for String {
    fn from(value: EvmSourcePolicyId) -> Self {
        value.0.into_string()
    }
}

/// Checked semantic EVM network binding owned by a bound provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmNetworkBinding {
    network_id: EvmNetworkId,
    expected_chain_id: NonZeroU64,
}

impl EvmNetworkBinding {
    /// Creates a checked semantic EVM network binding.
    pub fn new(network_id: EvmNetworkId, expected_chain_id: u64) -> Result<Self> {
        let expected_chain_id =
            NonZeroU64::new(expected_chain_id).ok_or(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::ZeroExpectedChainId,
            })?;
        Ok(Self {
            network_id,
            expected_chain_id,
        })
    }

    /// Returns the semantic network id.
    pub const fn network_id(&self) -> &EvmNetworkId {
        &self.network_id
    }

    /// Returns the expected EVM chain id.
    pub const fn expected_chain_id(&self) -> u64 {
        self.expected_chain_id.get()
    }
}

/// Redacted EVM source evidence attached to provider responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedEvmSourceEvidence {
    /// Semantic network id from the provider binding.
    pub network_id: EvmNetworkId,
    /// Expected EVM chain id from the provider binding.
    pub expected_chain_id: u64,
    /// Observed EVM chain id.
    pub observed_chain_id: u64,
    /// Process-local source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id used by the provider.
    pub policy_id: EvmSourcePolicyId,
}

impl RedactedEvmSourceEvidence {
    /// Builds redacted source evidence from a provider binding and selected runtime source.
    pub fn from_binding(
        binding: &EvmNetworkBinding,
        observed_chain_id: u64,
        source_ref: EvmSourceRef,
        policy_id: EvmSourcePolicyId,
    ) -> Result<Self> {
        let evidence = Self {
            network_id: binding.network_id().clone(),
            expected_chain_id: binding.expected_chain_id(),
            observed_chain_id,
            source_ref,
            policy_id,
        };
        if observed_chain_id == binding.expected_chain_id() {
            Ok(evidence)
        } else {
            Err(EvmCapabilityError::SourceMismatch {
                diagnostic: evidence.source_mismatch_diagnostic(),
            })
        }
    }

    /// Returns closed redacted source-mismatch diagnostic details.
    pub fn source_mismatch_diagnostic(&self) -> RedactedProviderDiagnostic {
        evm_diagnostic(ProviderDiagnosticCode::SourceMismatch)
            .with_field(
                evm_public_id("network_id"),
                ProviderDiagnosticValue::Id(evm_public_id(self.network_id.as_str())),
            )
            .with_field(
                evm_public_id("expected_chain_id"),
                ProviderDiagnosticValue::U64(self.expected_chain_id),
            )
            .with_field(
                evm_public_id("observed_chain_id"),
                ProviderDiagnosticValue::U64(self.observed_chain_id),
            )
            .with_field(
                evm_public_id("source_ref"),
                ProviderDiagnosticValue::Id(evm_public_id(self.source_ref.as_str())),
            )
            .with_field(
                evm_public_id("policy_id"),
                ProviderDiagnosticValue::Id(evm_public_id(self.policy_id.as_str())),
            )
    }
}

/// EVM block selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmBlockSelector {
    /// Latest available block.
    Latest,
    /// Pending block, including known pool transactions when supported.
    Pending,
    /// Concrete block number.
    Number(u64),
    /// Concrete block hash.
    Hash(B256),
}

/// Request for chain identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvmChainIdentityRequest;

impl EvmChainIdentityRequest {
    /// Creates an EVM chain identity request.
    pub const fn new() -> Self {
        Self
    }
}

impl Default for EvmChainIdentityRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Response for chain identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmChainIdentityResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// EVM chain id.
    pub chain_id: u64,
    /// Client version string, when returned.
    pub client_version: Option<String>,
}

/// Request for an EVM block summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBlockReadRequest {
    block: EvmBlockSelector,
}

impl EvmBlockReadRequest {
    /// Creates an EVM block read request.
    pub fn new(block: EvmBlockSelector) -> Self {
        Self { block }
    }

    /// Returns the requested block selector.
    pub const fn block(&self) -> &EvmBlockSelector {
        &self.block
    }
}

/// Response for an EVM block summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBlockReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Block number.
    pub block_number: u64,
    /// Block hash.
    pub block_hash: B256,
}

/// Request for an EVM account balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBalanceReadRequest {
    account: Address,
    block: EvmBlockSelector,
}

impl EvmBalanceReadRequest {
    /// Creates an EVM balance read request.
    pub const fn new(account: Address, block: EvmBlockSelector) -> Self {
        Self { account, block }
    }

    /// Returns the account address.
    pub const fn account(&self) -> Address {
        self.account
    }

    /// Returns the requested block selector.
    pub const fn block(&self) -> &EvmBlockSelector {
        &self.block
    }
}

/// Response for an EVM account balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBalanceReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Account balance in wei.
    pub balance_wei: U256,
}

/// Request for a read-only EVM call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCallReadRequest {
    to: Address,
    calldata: Vec<u8>,
    block: EvmBlockSelector,
}

impl EvmCallReadRequest {
    /// Creates an EVM call read request.
    pub fn new(to: Address, calldata: Vec<u8>, block: EvmBlockSelector) -> Self {
        Self {
            to,
            calldata,
            block,
        }
    }

    /// Returns the destination contract address.
    pub const fn to(&self) -> Address {
        self.to
    }

    /// Returns ABI-encoded call data.
    pub fn calldata(&self) -> &[u8] {
        &self.calldata
    }

    /// Returns the requested block selector.
    pub const fn block(&self) -> &EvmBlockSelector {
        &self.block
    }
}

/// Response for a read-only EVM call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCallReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Returned call data.
    pub return_data: Vec<u8>,
}

/// Request for deployed EVM bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCodeReadRequest {
    address: Address,
    block: EvmBlockSelector,
}

impl EvmCodeReadRequest {
    /// Creates an EVM code read request.
    pub const fn new(address: Address, block: EvmBlockSelector) -> Self {
        Self { address, block }
    }

    /// Returns the contract/account address to inspect.
    pub const fn address(&self) -> Address {
        self.address
    }

    /// Returns the requested block selector.
    pub const fn block(&self) -> &EvmBlockSelector {
        &self.block
    }
}

/// Response for deployed EVM bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCodeReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Deployed bytecode bytes. Empty bytes mean no code was observed.
    pub code: Vec<u8>,
    /// Keccak-256 hash of `code`.
    pub code_hash: B256,
}

/// Request for EVM logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmLogsReadRequest {
    from_block: EvmBlockSelector,
    to_block: EvmBlockSelector,
    address: Option<Address>,
    topics: Vec<B256>,
}

impl EvmLogsReadRequest {
    /// Creates an EVM logs read request.
    pub fn new(
        from_block: EvmBlockSelector,
        to_block: EvmBlockSelector,
        address: Option<Address>,
        topics: Vec<B256>,
    ) -> Self {
        Self {
            from_block,
            to_block,
            address,
            topics,
        }
    }

    /// Returns the start block selector.
    pub const fn from_block(&self) -> &EvmBlockSelector {
        &self.from_block
    }

    /// Returns the end block selector.
    pub const fn to_block(&self) -> &EvmBlockSelector {
        &self.to_block
    }

    /// Returns the optional emitting contract address.
    pub const fn address(&self) -> Option<Address> {
        self.address
    }

    /// Returns topic filters.
    pub fn topics(&self) -> &[B256] {
        &self.topics
    }
}

/// One EVM log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmLogEntry {
    /// Emitting contract address.
    pub address: Address,
    /// Log topics.
    pub topics: Vec<B256>,
    /// Log data bytes.
    pub data: Vec<u8>,
    /// Block number, when known.
    pub block_number: Option<u64>,
    /// Transaction hash, when known.
    pub transaction_hash: Option<B256>,
    /// Log index, when known.
    pub log_index: Option<u64>,
}

/// Response for EVM logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmLogsReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Matching log entries.
    pub logs: Vec<EvmLogEntry>,
}

/// Request for an account nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmNonceReadRequest {
    account: Address,
    block: EvmBlockSelector,
}

impl EvmNonceReadRequest {
    /// Creates an EVM nonce read request.
    pub const fn new(account: Address, block: EvmBlockSelector) -> Self {
        Self { account, block }
    }

    /// Returns the account address.
    pub const fn account(&self) -> Address {
        self.account
    }

    /// Returns the requested block selector.
    pub const fn block(&self) -> &EvmBlockSelector {
        &self.block
    }
}

/// Response for an account nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmNonceReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Account nonce.
    pub nonce: u64,
}

/// Request for EVM fee-market data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvmFeeReadRequest;

impl EvmFeeReadRequest {
    /// Creates an EVM fee read request.
    pub const fn new() -> Self {
        Self
    }
}

impl Default for EvmFeeReadRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Response for EVM fee-market data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmFeeReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Current base fee, when available.
    pub base_fee_per_gas: Option<u128>,
    /// Suggested priority fee, when available.
    pub priority_fee_per_gas: Option<u128>,
    /// Suggested EIP-1559 max fee, when available.
    pub max_fee_per_gas: Option<u128>,
    /// Legacy gas price, when available.
    pub legacy_gas_price: Option<u128>,
}

/// Request for an EVM gas estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmGasEstimateRequest {
    from: Option<Address>,
    to: Option<Address>,
    value_wei: u128,
    data: Vec<u8>,
}

impl EvmGasEstimateRequest {
    /// Creates an EVM gas estimate request.
    pub fn new(from: Option<Address>, to: Option<Address>, value_wei: u128, data: Vec<u8>) -> Self {
        Self {
            from,
            to,
            value_wei,
            data,
        }
    }

    /// Returns the sender address, when supplied.
    pub const fn from(&self) -> Option<Address> {
        self.from
    }

    /// Returns the destination address, or none for contract creation.
    pub const fn to(&self) -> Option<Address> {
        self.to
    }

    /// Returns the value in wei.
    pub const fn value_wei(&self) -> u128 {
        self.value_wei
    }

    /// Returns transaction input bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Response for an EVM gas estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmGasEstimateResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Estimated gas limit.
    pub gas_limit: u64,
}

/// Transient signed EVM payload for submission.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedEvmPayload {
    bytes: Vec<u8>,
    transaction_hash: B256,
}

impl SignedEvmPayload {
    /// Creates a transient signed payload from verified bytes and transaction hash.
    pub fn from_verified_bytes(bytes: Vec<u8>, transaction_hash: B256) -> Result<Self> {
        if bytes.is_empty() {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::EmptySignedPayload,
            });
        }
        Ok(Self {
            bytes,
            transaction_hash,
        })
    }

    /// Returns the signed payload bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the expected EVM transaction hash.
    pub const fn transaction_hash(&self) -> B256 {
        self.transaction_hash
    }
}

impl fmt::Debug for SignedEvmPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedEvmPayload")
            .field("bytes", &"<redacted>")
            .field("transaction_hash", &self.transaction_hash)
            .finish()
    }
}

/// Request for EVM transaction submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTransactionSubmitRequest {
    signed_payload: SignedEvmPayload,
}

impl EvmTransactionSubmitRequest {
    /// Creates an EVM transaction submission request.
    pub const fn new(signed_payload: SignedEvmPayload) -> Self {
        Self { signed_payload }
    }

    /// Returns the transient signed payload.
    pub const fn signed_payload(&self) -> &SignedEvmPayload {
        &self.signed_payload
    }
}

/// Response for EVM transaction submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTransactionSubmitResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Submitted transaction hash.
    pub transaction_hash: B256,
}

/// Request for an EVM transaction receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmReceiptReadRequest {
    transaction_hash: B256,
}

impl EvmReceiptReadRequest {
    /// Creates an EVM receipt read request.
    pub const fn new(transaction_hash: B256) -> Self {
        Self { transaction_hash }
    }

    /// Returns the transaction hash.
    pub const fn transaction_hash(&self) -> B256 {
        self.transaction_hash
    }
}

/// Response for an EVM transaction receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmReceiptReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Transaction hash.
    pub transaction_hash: B256,
    /// Block number that included the transaction.
    pub block_number: u64,
    /// Receipt status success flag.
    pub status: bool,
}

/// Request for account nonce occupancy investigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmNonceOccupancyReadRequest {
    account: Address,
    nonce: u64,
    excluded_transaction_hash: B256,
}

impl EvmNonceOccupancyReadRequest {
    /// Creates an EVM nonce occupancy read request.
    pub fn new(account: Address, nonce: u64, excluded_transaction_hash: B256) -> Self {
        Self {
            account,
            nonce,
            excluded_transaction_hash,
        }
    }

    /// Returns the sender account whose nonce is being investigated.
    pub const fn account(&self) -> Address {
        self.account
    }

    /// Returns the sender nonce being investigated.
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Returns the MFM recorded submission anchor that must not match an occupied transaction.
    pub const fn excluded_transaction_hash(&self) -> B256 {
        self.excluded_transaction_hash
    }
}

/// Response for account nonce occupancy investigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmNonceOccupancyReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Occupancy outcome.
    pub outcome: EvmNonceOccupancy,
}

/// Explicit nonce occupancy outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmNonceOccupancy {
    /// No defensible occupancy proof was available from this read.
    Unknown,
    /// A non-anchor transaction was observed occupying the sender nonce.
    Occupied {
        /// Occupying transaction hash.
        transaction_hash: B256,
        /// Block number when the occupying transaction is mined.
        block_number: Option<u64>,
    },
}

/// Closed invalid-request reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmInvalidRequest {
    /// Identifier was invalid.
    InvalidIdentifier,
    /// Expected chain id was zero.
    ZeroExpectedChainId,
    /// Signed payload was empty.
    EmptySignedPayload,
}

/// Redaction-safe EVM capability error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmCapabilityError {
    /// Request failed contract validation.
    #[error("EVM capability request was invalid")]
    InvalidRequest {
        /// Closed invalid-request reason.
        reason: EvmInvalidRequest,
    },
    /// Provider failed without exposing concrete source details.
    #[error("EVM capability provider failed: {diagnostic}")]
    Provider {
        /// Closed redacted provider diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
    /// Provider evidence did not match the provider binding.
    #[error("EVM source evidence did not match provider binding: {diagnostic}")]
    SourceMismatch {
        /// Closed redacted source-mismatch diagnostic.
        diagnostic: RedactedProviderDiagnostic,
    },
    /// A transaction receipt is not available yet.
    #[error("EVM transaction receipt is pending")]
    ReceiptPending,
}

impl EvmCapabilityError {
    /// Builds a provider failure from a closed redacted diagnostic.
    pub fn provider_failure(diagnostic: RedactedProviderDiagnostic) -> Self {
        Self::Provider { diagnostic }
    }

    /// Returns the closed redacted provider diagnostic carried by this error.
    pub const fn redacted_diagnostic(&self) -> Option<&RedactedProviderDiagnostic> {
        match self {
            Self::Provider { diagnostic } | Self::SourceMismatch { diagnostic } => Some(diagnostic),
            Self::InvalidRequest { .. } | Self::ReceiptPending => None,
        }
    }
}

/// Builds a closed redacted EVM provider diagnostic.
pub fn evm_diagnostic(code: ProviderDiagnosticCode) -> RedactedProviderDiagnostic {
    RedactedProviderDiagnostic::new(evm_public_id("evm"), code)
}

fn evm_public_id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("EVM diagnostic id must be checked public text")
}

fn invalid_identifier(_source: mfm_ids::CheckedStringError) -> EvmCapabilityError {
    EvmCapabilityError::InvalidRequest {
        reason: EvmInvalidRequest::InvalidIdentifier,
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
