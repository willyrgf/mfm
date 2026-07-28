//! Runtime-independent Bitcoin Core read qualification models.
//!
//! This module deliberately contains no MFM state, capability registration, aggregate reader, or
//! replay contract. It retains only checked source/address values and the bounded values returned
//! by the three independently meaningful Bitcoin Core operations used by qualification tests.

use std::fmt;
use std::str::FromStr;

use bitcoin::address::NetworkUnchecked;
use bitcoin::{Address, BlockHash, Network, ScriptBuf};
use mfm_ids::{ContentRef, LocalPublicId};

/// Maximum addresses admitted by one independently audited `scantxoutset "start"` request.
pub const BITCOIN_SCAN_ADDRESS_LIMIT: usize = 1_024;

macro_rules! checked_public_id {
    ($(#[$meta:meta])* $name:ident, $reason:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(LocalPublicId);

        impl $name {
            /// Creates a checked public identifier.
            pub fn new(value: impl AsRef<str>) -> Result<Self, BitcoinModelError> {
                LocalPublicId::new(value)
                    .map(Self)
                    .map_err(|_| BitcoinModelError::InvalidRequest {
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
            type Err = BitcoinModelError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
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
    pub fn new(value: impl AsRef<str>) -> Result<Self, BitcoinModelError> {
        match value.as_ref() {
            "main" => Ok(Self::Main),
            "test" => Ok(Self::Test),
            "testnet4" => Ok(Self::Testnet4),
            "signet" => Ok(Self::Signet),
            "regtest" => Ok(Self::Regtest),
            _ => Err(BitcoinModelError::InvalidRequest {
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
    type Err = BitcoinModelError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Checked semantic Bitcoin source binding.
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

/// Immutable non-secret routing generation selected before admission.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitcoinRoutingGenerationRef(ContentRef);

impl BitcoinRoutingGenerationRef {
    /// Wraps a reviewed exact routing-generation content reference.
    pub const fn from_reviewed(content_ref: ContentRef) -> Self {
        Self(content_ref)
    }

    /// Returns the exact routing-generation content reference.
    pub const fn as_content_ref(&self) -> &ContentRef {
        &self.0
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
    pub fn parse_any(value: &str) -> Result<Self, BitcoinModelError> {
        let unchecked = value.parse::<Address<NetworkUnchecked>>().map_err(|_| {
            BitcoinModelError::InvalidRequest {
                reason: BitcoinInvalidRequest::InvalidAddress,
            }
        })?;
        let checked = unchecked.assume_checked();
        let canonical = checked.to_string();
        if canonical != value {
            return Err(BitcoinModelError::InvalidRequest {
                reason: BitcoinInvalidRequest::NonCanonicalAddress,
            });
        }
        Ok(Self {
            canonical,
            script_pubkey: checked.script_pubkey(),
        })
    }

    /// Parses an address, checks network compatibility, and requires canonical rendering.
    pub fn parse(value: &str, network: BitcoinNetworkTag) -> Result<Self, BitcoinModelError> {
        let unchecked = value.parse::<Address<NetworkUnchecked>>().map_err(|_| {
            BitcoinModelError::InvalidRequest {
                reason: BitcoinInvalidRequest::InvalidAddress,
            }
        })?;
        let checked = unchecked.require_network(network.network()).map_err(|_| {
            BitcoinModelError::InvalidRequest {
                reason: BitcoinInvalidRequest::AddressNetworkMismatch,
            }
        })?;
        let canonical = checked.to_string();
        if canonical != value {
            return Err(BitcoinModelError::InvalidRequest {
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
    pub fn require_network(self, network: BitcoinNetworkTag) -> Result<Self, BitcoinModelError> {
        Self::parse(&self.canonical, network)
    }
}

/// Checked input for one indivisible `scantxoutset "start"` protocol operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinScanRequest {
    addresses: Vec<BitcoinAddress>,
}

impl BitcoinScanRequest {
    /// Creates a bounded request with canonical address and script ordering/uniqueness.
    pub fn new(
        network: BitcoinNetworkTag,
        addresses: Vec<String>,
    ) -> Result<Self, BitcoinModelError> {
        if addresses.is_empty() || addresses.len() > BITCOIN_SCAN_ADDRESS_LIMIT {
            return Err(BitcoinModelError::InvalidRequest {
                reason: BitcoinInvalidRequest::AddressCount,
            });
        }
        let mut checked = Vec::with_capacity(addresses.len());
        let mut previous: Option<&str> = None;
        let mut scripts = std::collections::BTreeSet::new();
        for value in &addresses {
            if previous.is_some_and(|prior| prior.as_bytes() >= value.as_bytes()) {
                return Err(BitcoinModelError::InvalidRequest {
                    reason: BitcoinInvalidRequest::AddressOrder,
                });
            }
            let address = BitcoinAddress::parse(value, network)?;
            if !scripts.insert(address.script_pubkey().as_bytes().to_vec()) {
                return Err(BitcoinModelError::InvalidRequest {
                    reason: BitcoinInvalidRequest::DuplicateScript,
                });
            }
            checked.push(address);
            previous = Some(value);
        }
        Ok(Self { addresses: checked })
    }

    /// Returns the checked addresses in canonical UTF-8 order.
    pub fn addresses(&self) -> &[BitcoinAddress] {
        &self.addresses
    }
}

/// Bounded result of one `getblockchaininfo` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinBlockchainInfo {
    chain: String,
    initial_block_download: bool,
}

impl BitcoinBlockchainInfo {
    /// Creates one strictly decoded blockchain-info result.
    pub fn new(chain: String, initial_block_download: bool) -> Self {
        Self {
            chain,
            initial_block_download,
        }
    }

    /// Returns the Bitcoin Core chain tag.
    pub fn chain(&self) -> &str {
        &self.chain
    }

    /// Returns whether Bitcoin Core reports initial block download.
    pub const fn initial_block_download(&self) -> bool {
        self.initial_block_download
    }
}

/// One address/satoshi observation from a completed scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinScannedBalance {
    address: String,
    balance_sats: u64,
}

impl BitcoinScannedBalance {
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

/// Strictly decoded result of one `scantxoutset "start"` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinScanResult {
    success: bool,
    height: u64,
    anchor_hash: BlockHash,
    balances: Vec<BitcoinScannedBalance>,
}

impl BitcoinScanResult {
    /// Creates one bounded scan result.
    pub const fn new(
        success: bool,
        height: u64,
        anchor_hash: BlockHash,
        balances: Vec<BitcoinScannedBalance>,
    ) -> Self {
        Self {
            success,
            height,
            anchor_hash,
            balances,
        }
    }

    /// Returns whether the provider reported completed scan semantics.
    pub const fn success(&self) -> bool {
        self.success
    }

    /// Returns the scan height.
    pub const fn height(&self) -> u64 {
        self.height
    }

    /// Returns the scan anchor hash.
    pub const fn anchor_hash(&self) -> BlockHash {
        self.anchor_hash
    }

    /// Returns balances in exact request order.
    pub fn balances(&self) -> &[BitcoinScannedBalance] {
        &self.balances
    }
}

/// Closed invalid-request reason for retained Bitcoin model primitives.
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

/// Redaction-safe error for retained Bitcoin model primitives.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BitcoinModelError {
    /// Checked request material was invalid.
    #[error("Bitcoin qualification model request was invalid")]
    InvalidRequest {
        /// Closed reason available to trusted callers and tests.
        reason: BitcoinInvalidRequest,
    },
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
