#![warn(missing_docs)]
//! Checked Bitcoin balance-collection capability contracts.
//!
//! This crate owns semantic source bindings, rust-bitcoin-backed address identity, the one
//! aggregate read request/response, and the route-aware session boundary. Protocol clients and
//! runtime registration live outside this crate.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use bitcoin::address::NetworkUnchecked;
use bitcoin::{Address, BlockHash, Network, ScriptBuf};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ProviderDiagnosticCode, ReadExternalRole,
    RedactedProviderDiagnostic,
};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm, LocalPublicId};

/// Stable implementation identity of the strict Bitcoin Core JSON-RPC session.
pub const BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID: &str =
    "mfm.bitcoin.jsonrpc.balance_collection.v1";

/// Maximum addresses admitted by one aggregate Bitcoin balance read.
pub const BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT: usize = 1_024;

/// Result type for Bitcoin capability contracts.
pub type Result<T> = std::result::Result<T, BitcoinCapabilityError>;

/// Boxed future returned by a Bitcoin balance session.
pub type BitcoinSessionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Aggregate Bitcoin balance-read authority.
pub struct BitcoinBalanceCollectionReadCapability;

impl CapabilitySpec for BitcoinBalanceCollectionReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.bitcoin",
            "balance_collection.read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.bitcoin.capability:balance_collection.read"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.bitcoin.balance_collection.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.balance_collection.read"
    }
}

/// Route-aware aggregate Bitcoin balance session.
pub trait BitcoinBalanceSession: Send + Sync {
    /// Returns the stable implementation identity installed in the runtime registry.
    fn implementation_id(&self) -> &'static str;

    /// Validates that the semantic binding can be served without live network IO.
    fn validate_binding(&self, binding: &BitcoinSourceBinding) -> Result<()>;

    /// Executes one complete aggregate balance read for the checked request.
    fn collect_balances<'a>(
        &'a self,
        request: &'a BitcoinBalanceCollectionRequest,
    ) -> BitcoinSessionFuture<'a, BitcoinBalanceCollectionResponse>;
}

macro_rules! checked_public_id {
    ($(#[$meta:meta])* $name:ident, $reason:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(LocalPublicId);

        impl $name {
            /// Creates a checked public identifier.
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                LocalPublicId::new(value)
                    .map(Self)
                    .map_err(|_| BitcoinCapabilityError::InvalidRequest {
                        reason: BitcoinInvalidRequest::$reason,
                    })
            }

            /// Returns the checked identifier string.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = BitcoinCapabilityError;

            fn from_str(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }
    };
}

checked_public_id!(
    /// Semantic Bitcoin network identifier.
    BitcoinNetworkId,
    InvalidNetworkId
);
checked_public_id!(
    /// Non-secret semantic Bitcoin source identity.
    BitcoinSourceIdentity,
    InvalidSourceIdentity
);

/// Checked Bitcoin Core chain tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BitcoinNetworkTag {
    /// Mainnet (`main`).
    Main,
    /// Testnet3 (`test`).
    Test,
    /// Testnet4 (`testnet4`).
    Testnet4,
    /// Signet (`signet`).
    Signet,
    /// Regression test network (`regtest`).
    Regtest,
}

impl BitcoinNetworkTag {
    /// Parses a supported Bitcoin Core chain tag.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        match value.as_ref() {
            "main" => Ok(Self::Main),
            "test" => Ok(Self::Test),
            "testnet4" => Ok(Self::Testnet4),
            "signet" => Ok(Self::Signet),
            "regtest" => Ok(Self::Regtest),
            _ => Err(BitcoinCapabilityError::InvalidRequest {
                reason: BitcoinInvalidRequest::InvalidBitcoinNetwork,
            }),
        }
    }

    /// Returns the Bitcoin Core chain tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Test => "test",
            Self::Testnet4 => "testnet4",
            Self::Signet => "signet",
            Self::Regtest => "regtest",
        }
    }

    /// Returns the rust-bitcoin network used for address checks.
    pub const fn network(self) -> Network {
        match self {
            Self::Main => Network::Bitcoin,
            Self::Test => Network::Testnet,
            Self::Testnet4 => Network::Testnet4,
            Self::Signet => Network::Signet,
            Self::Regtest => Network::Regtest,
        }
    }
}

impl fmt::Display for BitcoinNetworkTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BitcoinNetworkTag {
    type Err = BitcoinCapabilityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// Checked semantic source binding for one aggregate read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinSourceBinding {
    network_id: BitcoinNetworkId,
    bitcoin_network: BitcoinNetworkTag,
    semantic_source_identity: BitcoinSourceIdentity,
}

impl BitcoinSourceBinding {
    /// Creates a binding from checked components.
    pub const fn new(
        network_id: BitcoinNetworkId,
        bitcoin_network: BitcoinNetworkTag,
        semantic_source_identity: BitcoinSourceIdentity,
    ) -> Self {
        Self {
            network_id,
            bitcoin_network,
            semantic_source_identity,
        }
    }

    /// Returns the semantic network identifier.
    pub const fn network_id(&self) -> &BitcoinNetworkId {
        &self.network_id
    }

    /// Returns the required Bitcoin chain.
    pub const fn bitcoin_network(&self) -> BitcoinNetworkTag {
        self.bitcoin_network
    }

    /// Returns the semantic source identity.
    pub const fn semantic_source_identity(&self) -> &BitcoinSourceIdentity {
        &self.semantic_source_identity
    }
}

/// A canonical rust-bitcoin-checked address and its script identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinAddress {
    canonical: String,
    script_pubkey: ScriptBuf,
}

impl BitcoinAddress {
    /// Parses a canonical address without claiming a concrete test-family chain.
    pub fn parse_any(value: &str) -> Result<Self> {
        let unchecked = value.parse::<Address<NetworkUnchecked>>().map_err(|_| {
            BitcoinCapabilityError::InvalidRequest {
                reason: BitcoinInvalidRequest::InvalidAddress,
            }
        })?;
        let checked = unchecked.assume_checked();
        let canonical = checked.to_string();
        if canonical != value {
            return Err(BitcoinCapabilityError::InvalidRequest {
                reason: BitcoinInvalidRequest::NonCanonicalAddress,
            });
        }
        Ok(Self {
            canonical,
            script_pubkey: checked.script_pubkey(),
        })
    }

    /// Parses an address, checks network compatibility, and requires canonical rendering.
    pub fn parse(value: &str, network: BitcoinNetworkTag) -> Result<Self> {
        let unchecked = value.parse::<Address<NetworkUnchecked>>().map_err(|_| {
            BitcoinCapabilityError::InvalidRequest {
                reason: BitcoinInvalidRequest::InvalidAddress,
            }
        })?;
        let checked = unchecked.require_network(network.network()).map_err(|_| {
            BitcoinCapabilityError::InvalidRequest {
                reason: BitcoinInvalidRequest::AddressNetworkMismatch,
            }
        })?;
        let canonical = checked.to_string();
        if canonical != value {
            return Err(BitcoinCapabilityError::InvalidRequest {
                reason: BitcoinInvalidRequest::NonCanonicalAddress,
            });
        }
        Ok(Self {
            canonical,
            script_pubkey: checked.script_pubkey(),
        })
    }

    /// Returns the canonical rendered address.
    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    /// Returns the exact scriptPubKey bytes derived by rust-bitcoin.
    pub fn script_pubkey(&self) -> &ScriptBuf {
        &self.script_pubkey
    }

    /// Returns the fixed `scantxoutset` descriptor.
    pub fn scan_descriptor(&self) -> String {
        format!("addr({})", self.canonical)
    }

    /// Checks whether this canonical address encoding is compatible with a configured network.
    pub fn require_network(self, network: BitcoinNetworkTag) -> Result<Self> {
        Self::parse(&self.canonical, network)
    }
}

/// Checked request for one complete aggregate Bitcoin read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinBalanceCollectionRequest {
    binding: BitcoinSourceBinding,
    addresses: Vec<BitcoinAddress>,
}

impl BitcoinBalanceCollectionRequest {
    /// Creates a bounded request and enforces canonical address and script ordering/uniqueness.
    pub fn new(binding: BitcoinSourceBinding, addresses: Vec<String>) -> Result<Self> {
        if addresses.is_empty() || addresses.len() > BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT {
            return Err(BitcoinCapabilityError::InvalidRequest {
                reason: BitcoinInvalidRequest::AddressCount,
            });
        }
        let mut checked = Vec::with_capacity(addresses.len());
        let mut previous: Option<&str> = None;
        let mut scripts = std::collections::BTreeSet::new();
        for value in &addresses {
            if previous.is_some_and(|prior| prior.as_bytes() >= value.as_bytes()) {
                return Err(BitcoinCapabilityError::InvalidRequest {
                    reason: BitcoinInvalidRequest::AddressOrder,
                });
            }
            let address = BitcoinAddress::parse(value, binding.bitcoin_network())?;
            if !scripts.insert(address.script_pubkey().as_bytes().to_vec()) {
                return Err(BitcoinCapabilityError::InvalidRequest {
                    reason: BitcoinInvalidRequest::DuplicateScript,
                });
            }
            checked.push(address);
            previous = Some(value);
        }
        Ok(Self {
            binding,
            addresses: checked,
        })
    }

    /// Returns the exact semantic source binding.
    pub const fn binding(&self) -> &BitcoinSourceBinding {
        &self.binding
    }

    /// Returns the checked addresses in canonical UTF-8 order.
    pub fn addresses(&self) -> &[BitcoinAddress] {
        &self.addresses
    }
}

/// One address/satoshi observation returned by a checked session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinAddressBalance {
    address: String,
    balance_sats: u64,
}

impl BitcoinAddressBalance {
    /// Creates one checked balance observation.
    pub fn new(address: String, balance_sats: u64) -> Self {
        Self {
            address,
            balance_sats,
        }
    }

    /// Returns the canonical address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns the exact satoshi balance.
    pub const fn balance_sats(&self) -> u64 {
        self.balance_sats
    }
}

/// Checked result of one three-call aggregate Bitcoin read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinBalanceCollectionResponse {
    binding: BitcoinSourceBinding,
    implementation_id: &'static str,
    anchor_height: u64,
    anchor_hash: BlockHash,
    balances: Vec<BitcoinAddressBalance>,
    final_canonical_hash: BlockHash,
}

impl BitcoinBalanceCollectionResponse {
    /// Creates source-bound aggregate evidence from a checked protocol reduction.
    pub fn new(
        binding: BitcoinSourceBinding,
        implementation_id: &'static str,
        anchor_height: u64,
        anchor_hash: BlockHash,
        balances: Vec<BitcoinAddressBalance>,
        final_canonical_hash: BlockHash,
    ) -> Self {
        Self {
            binding,
            implementation_id,
            anchor_height,
            anchor_hash,
            balances,
            final_canonical_hash,
        }
    }

    /// Returns the checked source binding.
    pub const fn binding(&self) -> &BitcoinSourceBinding {
        &self.binding
    }

    /// Returns the session implementation identity.
    pub const fn implementation_id(&self) -> &'static str {
        self.implementation_id
    }

    /// Returns the shared scan height.
    pub const fn anchor_height(&self) -> u64 {
        self.anchor_height
    }

    /// Returns the shared scan block hash.
    pub const fn anchor_hash(&self) -> BlockHash {
        self.anchor_hash
    }

    /// Returns balances in exact request order.
    pub fn balances(&self) -> &[BitcoinAddressBalance] {
        &self.balances
    }

    /// Returns the final canonical hash observed at the scan height.
    pub const fn final_canonical_hash(&self) -> BlockHash {
        self.final_canonical_hash
    }
}

/// Closed invalid-request reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitcoinInvalidRequest {
    /// Semantic network identifier was invalid.
    InvalidNetworkId,
    /// Semantic source identity was invalid.
    InvalidSourceIdentity,
    /// Bitcoin Core chain tag was unsupported.
    InvalidBitcoinNetwork,
    /// Address count was outside the supported range.
    AddressCount,
    /// Address order or canonical uniqueness was invalid.
    AddressOrder,
    /// Address syntax was invalid.
    InvalidAddress,
    /// Address encoding was incompatible with the configured network.
    AddressNetworkMismatch,
    /// Address did not use rust-bitcoin's canonical rendering.
    NonCanonicalAddress,
    /// Two addresses resolved to the same scriptPubKey.
    DuplicateScript,
}

/// Redaction-safe Bitcoin capability failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BitcoinCapabilityError {
    /// Certified request material was invalid.
    #[error("Bitcoin balance collection request was invalid")]
    InvalidRequest {
        /// Closed reason available to trusted callers and tests.
        reason: BitcoinInvalidRequest,
    },
    /// Selected source did not match the request binding.
    #[error("Bitcoin balance collection source did not match")]
    SourceMismatch,
    /// Provider failed with a redacted diagnostic.
    #[error("Bitcoin balance collection provider failed: {diagnostic}")]
    Provider {
        /// Redaction-safe provider diagnostic.
        diagnostic: RedactedProviderDiagnostic,
        /// Whether a runtime retry may repeat the complete read.
        retryable: bool,
    },
}

impl BitcoinCapabilityError {
    /// Creates a provider failure without accepting raw provider text.
    pub fn provider(
        code: ProviderDiagnosticCode,
        operation: &'static str,
        retryable: bool,
    ) -> Self {
        let provider_family =
            LocalPublicId::new("bitcoin_core").expect("static Bitcoin provider family is valid");
        let operation = LocalPublicId::new(operation).expect("static Bitcoin operation is valid");
        Self::Provider {
            diagnostic: RedactedProviderDiagnostic::new(provider_family, code)
                .with_operation(operation),
            retryable,
        }
    }

    /// Returns whether this failure permits a complete runtime retry.
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Provider {
                retryable: true,
                ..
            }
        )
    }
}

#[cfg(test)]
mod tests;
