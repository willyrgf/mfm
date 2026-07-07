use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use bs58;
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
        validate_bitcoin_address(&raw)?;
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
        Self {
            wallet_id,
            subject: WalletSubject::new(address, subject_kind)?,
            network_id,
            implementation,
            symbol_ids,
            metadata,
        }
        .validated()
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

    /// Validates this wallet config, normalizes it, and returns the validated value.
    pub fn validated(mut self) -> Result<Self, WalletConfigError> {
        self.normalize();
        Ok(self)
    }
}

/// Canonical wallet implementation selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

/// Wallet signer details selected by runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-signer-config",
    schema = "mfm.portfolio.wallet_signer_config"
)]
pub struct WalletSignerConfig {
    /// Canonical signer kind.
    pub signer_kind: String,
    /// Opaque runtime signer reference.
    pub signer_ref: String,
}

/// Runtime wallet capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-capabilities",
    schema = "mfm.portfolio.wallet_capabilities"
)]
pub struct WalletCapabilities {
    /// Whether the implementation can resolve an address.
    pub can_resolve_address: bool,
    /// Whether the implementation can sign payloads.
    pub can_sign: bool,
    /// Whether the implementation can submit transactions.
    pub can_submit: bool,
}

/// Resolved wallet after runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-wallet",
    schema = "mfm.portfolio.resolved_wallet"
)]
pub struct ResolvedWallet {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Canonical resolved address.
    pub address: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Canonical implementation kind string.
    pub implementation_kind: String,
    /// Runtime capabilities.
    pub capabilities: WalletCapabilities,
    /// Optional signer details.
    pub signer: Option<WalletSignerConfig>,
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

fn validate_bitcoin_address(raw: &str) -> Result<(), String> {
    if raw.trim() != raw {
        return Err("address must not contain surrounding whitespace".to_string());
    }
    if raw.len() < 14 || raw.len() > 90 {
        return Err("address length was outside the supported bitcoin envelope".to_string());
    }
    if !raw.is_ascii() {
        return Err("address must be ASCII".to_string());
    }

    if raw.starts_with("bc1") || raw.starts_with("tb1") || raw.starts_with("bcrt1") {
        if raw != raw.to_ascii_lowercase() {
            return Err("bech32 bitcoin addresses must already be lowercase".to_string());
        }
        let (hrp, _, _) = bech32::segwit::decode(raw)
            .map_err(|_| "bech32 bitcoin address failed segwit validation".to_string())?;
        if !hrp.is_valid_on_mainnet() && !hrp.is_valid_on_testnet() && !hrp.is_valid_on_regtest() {
            return Err("unsupported bitcoin bech32 human-readable prefix".to_string());
        }
        return Ok(());
    }

    let base58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let first = raw.chars().next().unwrap_or_default();
    if !matches!(first, '1' | '3' | '2' | 'm' | 'n') {
        return Err("unsupported bitcoin address prefix".to_string());
    }
    if !raw.chars().all(|ch| base58.contains(ch)) {
        return Err("base58 bitcoin address contained unsupported characters".to_string());
    }
    bs58::decode(raw)
        .with_check(None)
        .into_vec()
        .map_err(|_| "base58 bitcoin address checksum validation failed".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_bitcoin_address;
    use bech32::{hrp, segwit};

    #[test]
    fn validate_bitcoin_address_accepts_supported_formats() {
        let segwit_v0 = segwit::encode_v0(hrp::BC, &[0x11; 20]).expect("valid v0 bech32 address");
        let taproot = segwit::encode_v1(hrp::BC, &[0x22; 32]).expect("valid v1 bech32m address");

        for (case, address, expected_prefix) in [
            (
                "base58",
                "1BoatSLRHtKNngkdXEeobR76b53LETtpyT".to_owned(),
                "1",
            ),
            ("segwit v0", segwit_v0, "bc1q"),
            ("taproot", taproot, "bc1p"),
        ] {
            assert!(validate_bitcoin_address(&address).is_ok(), "{case}");
            assert!(address.starts_with(expected_prefix), "{case}");
        }
    }

    #[test]
    fn validate_bitcoin_address_rejects_checksum_mismatches() {
        let mut bad_bech32 =
            segwit::encode_v0(hrp::BC, &[0x33; 20]).expect("valid v0 bech32 address");
        let last = bad_bech32.pop().expect("non-empty address");
        bad_bech32.push(if last == 'q' { 'p' } else { 'q' });

        for (case, address) in [
            ("base58", "1BoatSLRHtKNngkdXEeobR76b53LETtpyY".to_owned()),
            ("bech32", bad_bech32),
        ] {
            assert!(validate_bitcoin_address(&address).is_err(), "{case}");
        }
    }
}
