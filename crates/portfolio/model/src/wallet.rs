use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use mfm_btc_capabilities::BitcoinAddress as CheckedBitcoinAddress;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::ids::{
    ExternalSignerId, KeystoreEntryId, NetworkId, NormalizedEvmAddress, PortfolioScalarError,
    SymbolId, WalletId,
};
use crate::metadata::PublicMetadata;

/// Canonical subject kind selected for one wallet declaration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-subject-kind",
    schema = "mfm.portfolio.wallet_subject_kind"
)]
pub enum WalletSubjectKind {
    /// Wallet resolves to an EVM address subject.
    #[default]
    EvmAddress,
    /// Wallet resolves to a Bitcoin address subject.
    BitcoinAddress,
}

impl mfm_values::MfmDefault for WalletSubjectKind {}

/// Canonical wallet subject selected by one wallet declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-subject",
    schema = "mfm.portfolio.wallet_subject"
)]
pub enum WalletSubject {
    /// EVM address subject.
    EvmAddress {
        /// Canonical normalized EVM address.
        address: NormalizedEvmAddress,
    },
    /// Bitcoin address subject.
    BitcoinAddress {
        /// Canonical Bitcoin address.
        address: BitcoinAddress,
    },
}

impl WalletSubject {
    /// Creates a checked wallet subject from an address and subject kind.
    pub fn new(
        address: impl Into<String>,
        subject_kind: WalletSubjectKind,
    ) -> Result<Self, WalletConfigError> {
        let address = address.into();
        match subject_kind {
            WalletSubjectKind::EvmAddress => {
                let address = NormalizedEvmAddress::new(address.clone(), "wallet_address")
                    .map_err(|_| WalletConfigError::InvalidAddress {
                        address,
                        subject_kind,
                    })?;
                Ok(Self::EvmAddress { address })
            }
            WalletSubjectKind::BitcoinAddress => {
                let address = BitcoinAddress::new(address.clone()).map_err(|_| {
                    WalletConfigError::InvalidAddress {
                        address,
                        subject_kind,
                    }
                })?;
                Ok(Self::BitcoinAddress { address })
            }
        }
    }

    /// Returns the subject kind.
    pub const fn kind(&self) -> WalletSubjectKind {
        match self {
            Self::EvmAddress { .. } => WalletSubjectKind::EvmAddress,
            Self::BitcoinAddress { .. } => WalletSubjectKind::BitcoinAddress,
        }
    }

    /// Returns the canonical address string.
    pub fn address_str(&self) -> &str {
        match self {
            Self::EvmAddress { address } => address.as_str(),
            Self::BitcoinAddress { address } => address.as_str(),
        }
    }

    /// Returns the EVM address when this subject is EVM-backed.
    pub fn evm_address(&self) -> Option<&NormalizedEvmAddress> {
        match self {
            Self::EvmAddress { address } => Some(address),
            Self::BitcoinAddress { .. } => None,
        }
    }
}

/// Checked Bitcoin address authority.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "bitcoin-address",
    schema = "mfm.portfolio.address.bitcoin",
    transparent_string
)]
pub struct BitcoinAddress {
    raw: String,
}

impl BitcoinAddress {
    /// Creates a checked Bitcoin address.
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let raw = value.into();
        CheckedBitcoinAddress::new(&raw).map_err(|_| "Bitcoin address was invalid".to_owned())?;
        Ok(Self { raw })
    }

    /// Returns the canonical address string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this authority into its address string.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl AsRef<str> for BitcoinAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for BitcoinAddress {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for BitcoinAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BitcoinAddress {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for BitcoinAddress {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for BitcoinAddress {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<BitcoinAddress> for String {
    fn from(value: BitcoinAddress) -> Self {
        value.raw
    }
}

/// Canonical wallet config referenced by portfolio configs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-config",
    schema = "mfm.portfolio.wallet_config"
)]
pub struct WalletConfig {
    /// Stable machine identifier for the wallet.
    pub wallet_id: WalletId,
    /// Canonical wallet subject.
    pub subject: WalletSubject,
    /// Stable network identifier.
    pub network_id: NetworkId,
    /// Wallet implementation selection.
    pub implementation: WalletImplementationConfig,
    /// Symbols to read for this wallet.
    pub symbol_ids: Vec<SymbolId>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl WalletConfig {
    /// Creates a normalized and validated wallet config.
    pub fn new(
        wallet_id: String,
        address: String,
        subject_kind: WalletSubjectKind,
        network_id: String,
        implementation: WalletImplementationConfig,
        symbol_ids: Vec<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, WalletConfigError> {
        let wallet_id = WalletId::new(wallet_id)
            .map_err(|source| WalletConfigError::InvalidWalletId { source })?;
        let network_id = NetworkId::new(network_id)
            .map_err(|source| WalletConfigError::InvalidNetworkId { source })?;
        let symbol_ids = symbol_ids
            .into_iter()
            .map(|symbol_id| {
                SymbolId::new(symbol_id.clone())
                    .map_err(|source| WalletConfigError::InvalidSymbolIdRef { symbol_id, source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            WalletConfigError::MetadataContainsSecret {
                key: source.key().to_owned(),
            }
        })?;
        Ok(Self {
            wallet_id,
            subject: WalletSubject::new(address, subject_kind)?,
            network_id,
            implementation,
            symbol_ids,
            metadata,
        }
        .normalized())
    }

    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.symbol_ids.sort();
    }

    /// Returns a normalized clone of the wallet config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }
}

/// Canonical wallet implementation selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-implementation-config",
    schema = "mfm.portfolio.wallet_implementation_config"
)]
pub enum WalletImplementationConfig {
    /// Read-only address supplied directly by config.
    AddressOnly {},
    /// Address/signer backed by a keystore entry.
    KeystoreEntry {
        /// Stable keystore entry identifier.
        entry_id: KeystoreEntryId,
    },
    /// Account managed by the node.
    NodeManagedAccount {
        /// Node account index.
        account_index: u64,
    },
    /// External signer resolved by id.
    ExternalSigner {
        /// Stable signer identifier.
        signer_id: ExternalSignerId,
    },
}

/// Validation errors for canonical wallet configs.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WalletConfigError {
    /// The JSON payload could not be decoded into the canonical type.
    #[error("wallet config decode failed: {0}")]
    Decode(String),
    /// `wallet_id` did not satisfy the portfolio identifier grammar.
    #[error("wallet_id is invalid: {source}")]
    InvalidWalletId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `address` was invalid for the selected subject kind.
    #[error("address must be valid for subject_kind `{subject_kind:?}`: {address}")]
    InvalidAddress {
        /// Invalid wallet address input.
        address: String,
        /// Subject kind that rejected the address.
        subject_kind: WalletSubjectKind,
    },
    /// `network_id` did not satisfy the portfolio identifier grammar.
    #[error("network_id is invalid: {source}")]
    InvalidNetworkId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// One of the symbol refs did not satisfy the portfolio identifier grammar.
    #[error("wallet symbol_id ref `{symbol_id}` is invalid: {source}")]
    InvalidSymbolIdRef {
        /// Rejected symbol id.
        symbol_id: String,
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// Wallet metadata contained a secret-shaped key or value.
    #[error("metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
    },
}

#[cfg(test)]
#[path = "wallet_tests.rs"]
mod tests;
