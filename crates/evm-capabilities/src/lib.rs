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
//!     EvmFeeReadCapability, EvmSourcePolicyId, EvmSourceRef,
//! };
//!
//! let source_ref = EvmSourceRef::new("ethereum-mainnet")?;
//! let policy_id = EvmSourcePolicyId::new("local")?;
//! assert_eq!(source_ref.as_str(), "ethereum-mainnet");
//! assert_eq!(policy_id.as_str(), "local");
//! assert_eq!(EvmFeeReadCapability::name(), "mfm.evm.fee.read");
//! # Ok::<(), mfm_evm_capabilities::EvmCapabilityError>(())
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use alloy_primitives::{Address, B256, U256};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ExternalMutationAuthorityRole, ReadExternalRole,
};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm};

/// Result type for EVM capability contracts.
pub type Result<T> = std::result::Result<T, EvmCapabilityError>;

/// Boxed future returned by EVM capability providers.
pub type EvmCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

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
    fn chain_identity<'a>(
        &'a self,
        request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse>;
}

/// Provider interface for EVM block reads.
pub trait EvmBlockReadProvider: Send + Sync {
    /// Reads an EVM block summary.
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse>;
}

/// Provider interface for EVM account balance reads.
pub trait EvmBalanceReadProvider: Send + Sync {
    /// Reads an account balance at the selected block.
    fn read_balance<'a>(
        &'a self,
        request: &'a EvmBalanceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBalanceReadResponse>;
}

/// Provider interface for EVM contract call reads.
pub trait EvmCallReadProvider: Send + Sync {
    /// Executes a read-only EVM call.
    fn read_call<'a>(
        &'a self,
        request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse>;
}

/// Provider interface for EVM log reads.
pub trait EvmLogsReadProvider: Send + Sync {
    /// Reads EVM logs matching a filter.
    fn read_logs<'a>(
        &'a self,
        request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse>;
}

/// Provider interface for EVM nonce reads.
pub trait EvmNonceReadProvider: Send + Sync {
    /// Reads the account transaction count.
    fn read_nonce<'a>(
        &'a self,
        request: &'a EvmNonceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse>;
}

/// Provider interface for EVM fee-market reads.
pub trait EvmFeeReadProvider: Send + Sync {
    /// Reads fee-market inputs.
    fn read_fee<'a>(
        &'a self,
        request: &'a EvmFeeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse>;
}

/// Provider interface for EVM gas estimates.
pub trait EvmGasEstimateProvider: Send + Sync {
    /// Estimates gas for an EVM transaction intent.
    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmGasEstimateRequest,
    ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse>;
}

/// Provider interface for EVM transaction submission.
pub trait EvmTransactionSubmitProvider: Send + Sync {
    /// Submits a signed EVM payload.
    fn submit_transaction<'a>(
        &'a self,
        request: &'a EvmTransactionSubmitRequest,
    ) -> EvmCapabilityFuture<'a, EvmTransactionSubmitResponse>;
}

/// Provider interface for EVM receipt reads.
pub trait EvmReceiptReadProvider: Send + Sync {
    /// Reads an EVM transaction receipt.
    fn read_receipt<'a>(
        &'a self,
        request: &'a EvmReceiptReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse>;
}

/// Process-local EVM source reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvmSourceRef(String);

impl EvmSourceRef {
    /// Creates a checked EVM source reference.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        validate_identifier("source_ref", value.as_ref())?;
        Ok(Self(value.as_ref().to_owned()))
    }

    /// Returns the checked source reference string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EvmSourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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
        value.0
    }
}

/// Process-local EVM source policy id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvmSourcePolicyId(String);

impl EvmSourcePolicyId {
    /// Creates a checked EVM source policy id.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        validate_identifier("source_policy_id", value.as_ref())?;
        Ok(Self(value.as_ref().to_owned()))
    }

    /// Returns the checked policy id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EvmSourcePolicyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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
        value.0
    }
}

/// Redacted EVM source evidence attached to provider responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedEvmSourceEvidence {
    /// Process-local source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id used by the provider.
    pub policy_id: EvmSourcePolicyId,
    /// EVM chain id observed by the provider.
    pub chain_id: u64,
}

/// EVM block selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmBlockSelector {
    /// Latest available block.
    Latest,
    /// Concrete block number.
    Number(u64),
    /// Concrete block hash.
    Hash(B256),
}

/// Request for chain identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmChainIdentityRequest {
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
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
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Block selector.
    pub block: EvmBlockSelector,
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
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Account address.
    pub account: Address,
    /// Block selector.
    pub block: EvmBlockSelector,
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
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Destination contract address.
    pub to: Address,
    /// ABI-encoded call data.
    pub calldata: Vec<u8>,
    /// Block selector.
    pub block: EvmBlockSelector,
}

/// Response for a read-only EVM call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCallReadResponse {
    /// Redacted source evidence.
    pub evidence: RedactedEvmSourceEvidence,
    /// Returned call data.
    pub return_data: Vec<u8>,
}

/// Request for EVM logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmLogsReadRequest {
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Start block selector.
    pub from_block: EvmBlockSelector,
    /// End block selector.
    pub to_block: EvmBlockSelector,
    /// Optional emitting contract address.
    pub address: Option<Address>,
    /// Topic filters.
    pub topics: Vec<B256>,
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
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Account address.
    pub account: Address,
    /// Block selector.
    pub block: EvmBlockSelector,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmFeeReadRequest {
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
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
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Sender address, when required by the provider.
    pub from: Option<Address>,
    /// Destination address, or none for contract creation.
    pub to: Option<Address>,
    /// Value in wei.
    pub value_wei: u128,
    /// Transaction input bytes.
    pub data: Vec<u8>,
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
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Transient signed payload.
    pub signed_payload: SignedEvmPayload,
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
    /// Source reference.
    pub source_ref: EvmSourceRef,
    /// Source policy id.
    pub policy_id: EvmSourcePolicyId,
    /// Transaction hash.
    pub transaction_hash: B256,
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

/// Closed invalid-request reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmInvalidRequest {
    /// Identifier was invalid.
    InvalidIdentifier,
    /// Signed payload was empty.
    EmptySignedPayload,
}

/// Closed provider failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmProviderFailure {
    /// Provider failed without exposing concrete source details.
    Failed,
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
    #[error("EVM capability provider failed")]
    Provider {
        /// Closed provider failure reason.
        reason: EvmProviderFailure,
    },
    /// A transaction receipt is not available yet.
    #[error("EVM transaction receipt is pending")]
    ReceiptPending,
}

impl EvmCapabilityError {
    /// Builds a redacted provider failure, discarding source details.
    pub fn redacted_provider_failure(_source: impl fmt::Display) -> Self {
        Self::Provider {
            reason: EvmProviderFailure::Failed,
        }
    }
}

fn validate_identifier(_field: &'static str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 {
        return Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::InvalidIdentifier,
        });
    }
    let first = value.as_bytes()[0];
    let last = value.as_bytes()[value.len() - 1];
    if !matches!(first, b'a'..=b'z' | b'0'..=b'9') || !matches!(last, b'a'..=b'z' | b'0'..=b'9') {
        return Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::InvalidIdentifier,
        });
    }
    for byte in value.bytes() {
        if !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-') {
            return Err(EvmCapabilityError::InvalidRequest {
                reason: EvmInvalidRequest::InvalidIdentifier,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_names_are_authority_names_not_workflow_names() {
        let names = capability_names();

        assert_eq!(
            names,
            [
                "mfm.evm.chain_identity.read",
                "mfm.evm.block.read",
                "mfm.evm.balance.read",
                "mfm.evm.call.read",
                "mfm.evm.logs.read",
                "mfm.evm.nonce.read",
                "mfm.evm.fee.read",
                "mfm.evm.gas_estimate.read",
                "mfm.evm.transaction.submit",
                "mfm.evm.receipt.read",
            ]
        );
        for name in names {
            assert!(!name.contains(concat!("d", "cv")));
            assert!(!name.contains("deploy"));
            assert!(!name.contains("configure"));
            assert!(!name.contains("validate"));
        }
    }

    #[test]
    fn fee_market_and_gas_estimate_capabilities_are_separate() {
        assert_ne!(
            EvmFeeReadCapability::name(),
            EvmGasEstimateCapability::name()
        );
        assert_ne!(
            EvmFeeReadCapability::kind().expect("fee kind"),
            EvmGasEstimateCapability::kind().expect("gas kind")
        );
    }

    #[test]
    fn concrete_source_details_are_absent_from_contracts() {
        let source = include_str!("lib.rs");
        let forbidden = [
            concat!("rpc", "_", "url"),
            concat!("end", "point"),
            concat!("author", "ization"),
            concat!("api", "_", "key"),
            concat!("bear", "er"),
            concat!("pass", "word"),
            concat!("private", "_", "key"),
            concat!("mn", "emonic"),
            concat!("key", "store", "_", "path"),
        ];

        for term in forbidden {
            assert!(
                !source.contains(term),
                "forbidden concrete source detail: {term}"
            );
        }
    }

    #[test]
    fn signed_payload_debug_redacts_bytes() {
        let payload = SignedEvmPayload::from_verified_bytes(vec![1, 2, 3], B256::from([4; 32]))
            .expect("payload");

        let rendered = format!("{payload:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("1, 2, 3"));
    }

    fn capability_names() -> [&'static str; 10] {
        [
            EvmChainIdentityCapability::name(),
            EvmBlockReadCapability::name(),
            EvmBalanceReadCapability::name(),
            EvmCallReadCapability::name(),
            EvmLogsReadCapability::name(),
            EvmNonceReadCapability::name(),
            EvmFeeReadCapability::name(),
            EvmGasEstimateCapability::name(),
            EvmTransactionSubmitCapability::name(),
            EvmReceiptReadCapability::name(),
        ]
    }
}
