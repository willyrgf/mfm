use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::num::NonZeroU64;

use mfm_btc_capabilities::{BitcoinAddress as CheckedBitcoinAddress, BitcoinNetworkTag};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{
    BitcoinSourceIdentityId, NetworkId, PortfolioId, PortfolioScalarError, SymbolId, WalletId,
};
use crate::metadata::PublicMetadata;
use crate::symbol::{
    validate_symbol_config, HoldingSourceConfig, Observation, QuoteCode, SymbolConfig,
    SymbolConfigError,
};
use crate::wallet::{WalletConfig, WalletSubject, WalletSubjectKind};

#[path = "portfolio_snapshot.rs"]
mod portfolio_snapshot;
pub use self::portfolio_snapshot::*;

/// Maximum networks admitted by one portfolio config.
pub const PORTFOLIO_NETWORK_LIMIT: usize = 64;
/// Maximum wallets admitted by one portfolio config.
pub const PORTFOLIO_WALLET_LIMIT: usize = 1_024;
/// Maximum symbol configs admitted by one portfolio config.
pub const PORTFOLIO_SYMBOL_LIMIT: usize = 2_048;
/// Maximum explicit wallet-to-symbol relations admitted by one portfolio config.
pub const PORTFOLIO_HOLDING_RELATION_LIMIT: usize = 4_096;
/// Maximum distinct holding sources admitted for one EVM network.
pub const EVM_NETWORK_HOLDING_SOURCE_LIMIT: usize = 1_024;

/// Supported network families on the canonical portfolio config surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "network-family-config",
    schema = "mfm.portfolio.network_family_config"
)]
pub enum NetworkFamilyConfig {
    /// Ethereum-compatible execution network.
    #[default]
    Evm,
    /// Bitcoin-family execution network.
    Bitcoin,
}

impl mfm_values::MfmDefault for NetworkFamilyConfig {}

/// Canonical top-level portfolio configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-config",
    schema = "mfm.portfolio.config",
    validate = "validate_portfolio_config_for_mfm"
)]
pub struct PortfolioConfig {
    /// Stable machine identifier for the portfolio.
    pub portfolio_id: PortfolioId,
    /// Quote units required for every configured symbol.
    pub quote_codes: Vec<QuoteCode>,
    /// Networks available to the portfolio.
    pub networks: Vec<NetworkConfig>,
    /// Wallets included in the portfolio.
    pub wallets: Vec<WalletConfig>,
    /// Symbol configs available to the portfolio.
    pub symbol_configs: Vec<SymbolConfig>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl PortfolioConfig {
    /// Creates a normalized and validated portfolio config.
    pub fn new(
        portfolio_id: String,
        quote_codes: Vec<QuoteCode>,
        networks: Vec<NetworkConfig>,
        wallets: Vec<WalletConfig>,
        symbol_configs: Vec<SymbolConfig>,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, PortfolioConfigError> {
        let portfolio_id = PortfolioId::new(portfolio_id)
            .map_err(|source| PortfolioConfigError::InvalidPortfolioId { source })?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            PortfolioConfigError::MetadataContainsSecret {
                key: source.key().to_owned(),
            }
        })?;
        Self {
            portfolio_id,
            quote_codes,
            networks,
            wallets,
            symbol_configs,
            metadata,
        }
        .validated()
    }

    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.quote_codes.sort();
        self.networks
            .sort_by(|left, right| left.network_id().cmp(right.network_id()));
        for wallet in &mut self.wallets {
            wallet.normalize();
        }
        self.wallets
            .sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
        for symbol in &mut self.symbol_configs {
            symbol.normalize();
        }
        self.symbol_configs
            .sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));
    }

    /// Returns a normalized clone of the portfolio config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Validates this config, normalizes it, and returns the validated value.
    pub fn validated(self) -> Result<Self, PortfolioConfigError> {
        Ok(ValidatedPortfolioConfig::new(self)?.into_config())
    }
}

/// Canonical network configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "family", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "network-config",
    schema = "mfm.portfolio.network_config"
)]
pub enum NetworkConfig {
    /// Ethereum-compatible execution network.
    Evm {
        /// Stable machine identifier for the network.
        network_id: NetworkId,
        /// Non-zero EVM chain id for the network.
        chain_id: NonZeroU64,
        /// Native asset scale for this semantic EVM network.
        native_decimals: u8,
        /// Canonical metadata surface.
        #[serde(default)]
        metadata: PublicMetadata,
    },
    /// Bitcoin-family execution network.
    Bitcoin {
        /// Stable machine identifier for the network.
        network_id: NetworkId,
        /// Expected Bitcoin Core network tag.
        bitcoin_network: String,
        /// Semantic Bitcoin source identity used to bind runtime routes and evidence.
        source_identity: BitcoinSourceIdentityId,
        /// Canonical metadata surface.
        #[serde(default)]
        metadata: PublicMetadata,
    },
}

impl NetworkConfig {
    /// Creates a validated network config without any implicit native-scale fallback.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        network_id: String,
        family: NetworkFamilyConfig,
        chain_id: Option<u64>,
        native_decimals: Option<u8>,
        bitcoin_network: Option<String>,
        source_identity: Option<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, PortfolioConfigError> {
        let network_id = NetworkId::new(network_id)
            .map_err(|source| PortfolioConfigError::InvalidNetworkId { source })?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            PortfolioConfigError::NetworkMetadataContainsSecret {
                network_id: network_id.to_string(),
                key: source.key().to_owned(),
            }
        })?;
        match family {
            NetworkFamilyConfig::Evm => {
                if source_identity.is_some() {
                    return Err(PortfolioConfigError::UnexpectedEvmSourceIdentity {
                        network_id: network_id.to_string(),
                    });
                }
                if bitcoin_network.is_some() {
                    return Err(PortfolioConfigError::UnexpectedEvmBitcoinNetwork {
                        network_id: network_id.to_string(),
                    });
                }
                let Some(chain_id) = chain_id else {
                    return Err(PortfolioConfigError::MissingEvmChainId {
                        network_id: network_id.to_string(),
                    });
                };
                let Some(chain_id) = NonZeroU64::new(chain_id) else {
                    return Err(PortfolioConfigError::InvalidEvmChainId {
                        network_id: network_id.to_string(),
                    });
                };
                let Some(native_decimals) = native_decimals else {
                    return Err(PortfolioConfigError::MissingEvmNativeDecimals {
                        network_id: network_id.to_string(),
                    });
                };
                Self::Evm {
                    network_id,
                    chain_id,
                    native_decimals,
                    metadata,
                }
                .validated()
            }
            NetworkFamilyConfig::Bitcoin => {
                if chain_id.is_some() {
                    return Err(PortfolioConfigError::UnexpectedBitcoinChainId {
                        network_id: network_id.to_string(),
                    });
                }
                if native_decimals.is_some() {
                    return Err(PortfolioConfigError::UnexpectedBitcoinNativeDecimals {
                        network_id: network_id.to_string(),
                    });
                }
                let Some(bitcoin_network) = bitcoin_network else {
                    return Err(PortfolioConfigError::MissingBitcoinNetwork {
                        network_id: network_id.to_string(),
                    });
                };
                validate_bitcoin_network(&bitcoin_network).map_err(|_| {
                    PortfolioConfigError::InvalidBitcoinNetwork {
                        network_id: network_id.to_string(),
                    }
                })?;
                let Some(source_identity) = source_identity else {
                    return Err(PortfolioConfigError::MissingBitcoinSourceIdentity {
                        network_id: network_id.to_string(),
                    });
                };
                let source_identity =
                    BitcoinSourceIdentityId::new(source_identity).map_err(|source| {
                        PortfolioConfigError::InvalidBitcoinSourceIdentity {
                            network_id: network_id.to_string(),
                            source,
                        }
                    })?;
                Self::Bitcoin {
                    network_id,
                    bitcoin_network,
                    source_identity,
                    metadata,
                }
                .validated()
            }
        }
    }

    /// Validates this network config and returns it unchanged.
    pub fn validated(self) -> Result<Self, PortfolioConfigError> {
        validate_network_config(&self)?;
        Ok(self)
    }

    /// Returns the stable machine identifier for this network.
    pub fn network_id(&self) -> &NetworkId {
        match self {
            Self::Evm { network_id, .. } | Self::Bitcoin { network_id, .. } => network_id,
        }
    }

    /// Returns the execution family for this network.
    pub const fn family(&self) -> NetworkFamilyConfig {
        match self {
            Self::Evm { .. } => NetworkFamilyConfig::Evm,
            Self::Bitcoin { .. } => NetworkFamilyConfig::Bitcoin,
        }
    }

    /// Returns the EVM chain id when this is an EVM network.
    pub const fn chain_id(&self) -> Option<NonZeroU64> {
        match self {
            Self::Evm { chain_id, .. } => Some(*chain_id),
            Self::Bitcoin { .. } => None,
        }
    }

    /// Returns the EVM chain id as a primitive integer when present.
    pub const fn chain_id_u64(&self) -> Option<u64> {
        match self {
            Self::Evm { chain_id, .. } => Some(chain_id.get()),
            Self::Bitcoin { .. } => None,
        }
    }

    /// Returns the EVM-native decimal scale when this is an EVM network.
    pub const fn native_decimals(&self) -> Option<u8> {
        match self {
            Self::Evm {
                native_decimals, ..
            } => Some(*native_decimals),
            Self::Bitcoin { .. } => None,
        }
    }

    /// Returns the Bitcoin Core network tag when this is a Bitcoin network.
    pub fn bitcoin_network(&self) -> Option<&str> {
        match self {
            Self::Evm { .. } => None,
            Self::Bitcoin {
                bitcoin_network, ..
            } => Some(bitcoin_network),
        }
    }

    /// Returns the Bitcoin source identity when this is a Bitcoin network.
    pub fn source_identity(&self) -> Option<&BitcoinSourceIdentityId> {
        match self {
            Self::Evm { .. } => None,
            Self::Bitcoin {
                source_identity, ..
            } => Some(source_identity),
        }
    }

    /// Returns the public metadata associated with this network.
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        match self {
            Self::Evm { metadata, .. } | Self::Bitcoin { metadata, .. } => metadata.as_map(),
        }
    }

    /// Returns the checked public metadata authority associated with this network.
    pub fn public_metadata(&self) -> &PublicMetadata {
        match self {
            Self::Evm { metadata, .. } | Self::Bitcoin { metadata, .. } => metadata,
        }
    }
}

/// Validated, normalized portfolio config authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedPortfolioConfig {
    config: PortfolioConfig,
    index: PortfolioConfigIndex,
}

impl ValidatedPortfolioConfig {
    /// Creates a validated portfolio config authority.
    pub fn new(config: PortfolioConfig) -> Result<Self, PortfolioConfigError> {
        validate_portfolio_config_inner(&config)?;
        let mut config = config;
        config.normalize();
        let index = PortfolioConfigIndex::new(&config);
        Ok(Self { config, index })
    }

    /// Returns the normalized portfolio config.
    pub const fn as_config(&self) -> &PortfolioConfig {
        &self.config
    }

    /// Consumes this authority into the normalized portfolio config.
    pub fn into_config(self) -> PortfolioConfig {
        self.config
    }

    /// Returns a configured quote code when it exists.
    pub fn quote(&self, quote: QuoteCode) -> Option<QuoteCode> {
        self.index.quotes.contains(&quote).then_some(quote)
    }

    /// Returns a network by stable id.
    pub fn network(&self, network_id: &str) -> Option<&NetworkConfig> {
        let network_id = NetworkId::new(network_id).ok()?;
        self.index
            .networks
            .get(&network_id)
            .map(|index| &self.config.networks[*index])
    }

    /// Returns a wallet by stable id.
    pub fn wallet(&self, wallet_id: &str) -> Option<&WalletConfig> {
        let wallet_id = WalletId::new(wallet_id).ok()?;
        self.index
            .wallets
            .get(&wallet_id)
            .map(|index| &self.config.wallets[*index])
    }

    /// Returns a symbol by stable id.
    pub fn symbol(&self, symbol_id: &str) -> Option<&SymbolConfig> {
        let symbol_id = SymbolId::new(symbol_id).ok()?;
        self.index
            .symbols
            .get(&symbol_id)
            .map(|index| &self.config.symbol_configs[*index])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PortfolioConfigIndex {
    quotes: BTreeSet<QuoteCode>,
    networks: BTreeMap<NetworkId, usize>,
    wallets: BTreeMap<WalletId, usize>,
    symbols: BTreeMap<SymbolId, usize>,
}

impl PortfolioConfigIndex {
    fn new(config: &PortfolioConfig) -> Self {
        let quotes = config.quote_codes.iter().copied().collect();
        let networks = config
            .networks
            .iter()
            .enumerate()
            .map(|(index, network)| (network.network_id().clone(), index))
            .collect();
        let wallets = config
            .wallets
            .iter()
            .enumerate()
            .map(|(index, wallet)| (wallet.wallet_id.clone(), index))
            .collect();
        let symbols = config
            .symbol_configs
            .iter()
            .enumerate()
            .map(|(index, symbol)| (symbol.symbol_id.clone(), index))
            .collect();
        Self {
            quotes,
            networks,
            wallets,
            symbols,
        }
    }
}

/// Validation errors for canonical portfolio configs.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortfolioConfigError {
    /// The JSON payload could not be decoded into the canonical type.
    #[error("portfolio config decode failed: {0}")]
    Decode(String),
    /// `portfolio_id` did not satisfy the portfolio identifier grammar.
    #[error("portfolio_id is invalid: {source}")]
    InvalidPortfolioId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// Portfolio metadata contained a secret-shaped key or value.
    #[error("portfolio metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// A top-level portfolio collection exceeded its hard admission bound.
    #[error("portfolio {collection} count {actual} exceeded limit {limit}")]
    CollectionLimitExceeded {
        /// Stable collection label.
        collection: &'static str,
        /// Hard admitted maximum.
        limit: usize,
        /// Supplied collection size.
        actual: usize,
    },
    /// One EVM network exceeded its hard holding-source bound.
    #[error("EVM network `{network_id}` holding source count {actual} exceeded limit {limit}")]
    EvmNetworkHoldingSourceLimitExceeded {
        /// Network whose explicit demand exceeded the bound.
        network_id: String,
        /// Hard admitted maximum.
        limit: usize,
        /// Supplied source count.
        actual: usize,
    },
    /// `network_id` did not satisfy the portfolio identifier grammar.
    #[error("network_id is invalid: {source}")]
    InvalidNetworkId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// EVM networks must not declare a Bitcoin source identity.
    #[error("network `{network_id}` with family `evm` must not declare source_identity")]
    UnexpectedEvmSourceIdentity {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks require a source identity.
    #[error("network `{network_id}` with family `bitcoin` must declare source_identity")]
    MissingBitcoinSourceIdentity {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// `source_identity` did not satisfy the local public id grammar.
    #[error("network `{network_id}` source_identity is invalid: {source}")]
    InvalidBitcoinSourceIdentity {
        /// Network id associated with the failure.
        network_id: String,
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// EVM networks require an explicit chain id.
    #[error("network `{network_id}` with family `evm` must declare chain_id")]
    MissingEvmChainId {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// EVM networks require a non-zero chain id.
    #[error("network `{network_id}` with family `evm` must declare non-zero chain_id")]
    InvalidEvmChainId {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// EVM networks require an explicit native-asset scale.
    #[error("network `{network_id}` with family `evm` must declare native_decimals")]
    MissingEvmNativeDecimals {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks must not declare an EVM chain id.
    #[error("network `{network_id}` with family `bitcoin` must not declare chain_id")]
    UnexpectedBitcoinChainId {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks must not declare an EVM-native scale.
    #[error("network `{network_id}` with family `bitcoin` must not declare native_decimals")]
    UnexpectedBitcoinNativeDecimals {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// EVM networks must not declare a Bitcoin network tag.
    #[error("network `{network_id}` with family `evm` must not declare bitcoin_network")]
    UnexpectedEvmBitcoinNetwork {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks require an explicit Bitcoin Core network tag.
    #[error("network `{network_id}` with family `bitcoin` must declare bitcoin_network")]
    MissingBitcoinNetwork {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks require a supported Bitcoin Core network tag.
    #[error("network `{network_id}` with family `bitcoin` has invalid bitcoin_network")]
    InvalidBitcoinNetwork {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Two networks shared the same network id.
    #[error("network_id `{network_id}` must be unique")]
    DuplicateNetworkId {
        /// Duplicate network id.
        network_id: String,
    },
    /// Network metadata contained a secret-shaped key or value.
    #[error("network `{network_id}` metadata key `{key}` contains secret-shaped content")]
    NetworkMetadataContainsSecret {
        /// Network associated with the failure.
        network_id: String,
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// Two wallets shared the same wallet id.
    #[error("wallet_id `{wallet_id}` must be unique")]
    DuplicateWalletId {
        /// Duplicate wallet id.
        wallet_id: String,
    },
    /// Two wallet declarations named the same semantic subject on one network.
    #[error("wallet `{wallet_id}` duplicates semantic subject of wallet `{existing_wallet_id}` on network `{network_id}")]
    DuplicateWalletSubject {
        /// Later wallet id that duplicated the subject.
        wallet_id: String,
        /// Earlier wallet id using the same subject.
        existing_wallet_id: String,
        /// Shared network id.
        network_id: String,
    },
    /// Wallet referenced an unknown network.
    #[error("wallet `{wallet_id}` referenced unknown network `{network_id}`")]
    UnknownWalletNetwork {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Unknown network id.
        network_id: String,
    },
    /// Wallet subject family did not match the referenced network family.
    #[error("wallet `{wallet_id}` subject_kind `{wallet_subject_kind:?}` did not match network `{network_id}` family `{network_family:?}")]
    WalletSubjectNetworkFamilyMismatch {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Network id associated with the failure.
        network_id: String,
        /// Wallet subject kind selected in config.
        wallet_subject_kind: WalletSubjectKind,
        /// Network family selected by the referenced network.
        network_family: NetworkFamilyConfig,
    },
    /// A Bitcoin wallet address encoding did not match its configured network family.
    #[error("wallet `{wallet_id}` Bitcoin address did not match network `{network_id}`")]
    BitcoinWalletAddressNetworkMismatch {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Configured Bitcoin network id.
        network_id: String,
    },
    /// Wallet referenced an unknown symbol.
    #[error("wallet `{wallet_id}` referenced unknown symbol `{symbol_id}")]
    UnknownWalletSymbol {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Unknown symbol id.
        symbol_id: String,
    },
    /// Wallet listed the same symbol more than once.
    #[error("wallet `{wallet_id}` listed symbol `{symbol_id}` more than once")]
    DuplicateWalletSymbol {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Repeated symbol id.
        symbol_id: String,
    },
    /// Wallet referenced a symbol configured for a different network.
    #[error("wallet `{wallet_id}` on network `{wallet_network_id}` referenced symbol `{symbol_id}` on network `{symbol_network_id}")]
    WalletSymbolNetworkMismatch {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Wallet network id.
        wallet_network_id: String,
        /// Symbol network id.
        symbol_network_id: String,
    },
    /// Two symbols shared the same symbol id.
    #[error("symbol_id `{symbol_id}` must be unique")]
    DuplicateSymbolId {
        /// Duplicate symbol id.
        symbol_id: String,
    },
    /// A symbol config failed local validation.
    #[error("symbol `{symbol_id}` is invalid: {source}")]
    InvalidSymbolConfig {
        /// Symbol id associated with the validation failure.
        symbol_id: String,
        /// Underlying symbol validation failure.
        source: Box<SymbolConfigError>,
    },
    /// A holding source was unsupported by its network family.
    #[error("symbol `{symbol_id}` used unsupported holding source for network `{network_id}` family `{network_family:?}")]
    UnsupportedHoldingSourceNetworkFamily {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Network id associated with the failure.
        network_id: String,
        /// Network family selected by the referenced network.
        network_family: NetworkFamilyConfig,
    },
    /// Symbol referenced an unknown network.
    #[error("symbol `{symbol_id}` referenced unknown network `{network_id}")]
    UnknownSymbolNetwork {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Unknown network id.
        network_id: String,
    },
    /// Quote codes must be unique at the portfolio level.
    #[error("portfolio quote `{quote}` must be unique")]
    DuplicateQuoteCode {
        /// Duplicate quote code.
        quote: QuoteCode,
    },
    /// Symbol did not provide a route for one requested quote.
    #[error("symbol `{symbol_id}` is missing valuation route for quote `{quote}")]
    MissingValuationQuote {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Missing quote code.
        quote: QuoteCode,
    },
    /// Symbol defined a quote route that was not requested by the portfolio.
    #[error("symbol `{symbol_id}` defined unexpected valuation route for quote `{quote}")]
    UnexpectedValuationQuote {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Unexpected quote code.
        quote: QuoteCode,
    },
    /// A valuation route referenced an unknown priced symbol.
    #[error("symbol `{symbol_id}` quote `{quote}` referenced unknown priced_symbol_id `{priced_symbol_id}")]
    UnknownPricedSymbol {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Quote associated with the invalid route.
        quote: QuoteCode,
        /// Unknown priced symbol id.
        priced_symbol_id: String,
    },
    /// The explicit wallet-to-symbol holding relation was empty.
    #[error("portfolio must declare at least one wallet-to-symbol holding")]
    EmptyHoldingDemand,
    /// Two logical wallet-to-symbol edges targeted the same balance source.
    #[error("holding `{wallet_id}/{symbol_id}` aliases source used by `{existing_wallet_id}/{existing_symbol_id}` on network `{network_id}")]
    AliasedHoldingSource {
        /// Later wallet id that aliases a source.
        wallet_id: String,
        /// Later symbol id that aliases a source.
        symbol_id: String,
        /// Earlier wallet id using the same physical source.
        existing_wallet_id: String,
        /// Earlier symbol id using the same physical source.
        existing_symbol_id: String,
        /// Shared network id.
        network_id: String,
    },
}

/// Decodes and validates a canonical portfolio config.
pub fn decode_portfolio_config(value: &Value) -> Result<PortfolioConfig, PortfolioConfigError> {
    let cfg: PortfolioConfig = serde_json::from_value(value.clone())
        .map_err(|err| PortfolioConfigError::Decode(err.to_string()))?;
    cfg.validated()
}

fn validate_network_config(network: &NetworkConfig) -> Result<(), PortfolioConfigError> {
    if let NetworkConfig::Bitcoin {
        network_id,
        bitcoin_network,
        ..
    } = network
    {
        validate_bitcoin_network(bitcoin_network).map_err(|_| {
            PortfolioConfigError::InvalidBitcoinNetwork {
                network_id: network_id.to_string(),
            }
        })?;
    }
    Ok(())
}

fn validate_bitcoin_network(value: &str) -> Result<(), ()> {
    BitcoinNetworkTag::new(value).map(|_| ()).map_err(|_| ())
}

fn validate_portfolio_config_inner(cfg: &PortfolioConfig) -> Result<(), PortfolioConfigError> {
    require_collection_limit("network", cfg.networks.len(), PORTFOLIO_NETWORK_LIMIT)?;
    require_collection_limit("wallet", cfg.wallets.len(), PORTFOLIO_WALLET_LIMIT)?;
    require_collection_limit("symbol", cfg.symbol_configs.len(), PORTFOLIO_SYMBOL_LIMIT)?;
    let holding_relation_count = cfg.wallets.iter().fold(0_usize, |count, wallet| {
        count.saturating_add(wallet.symbol_ids.len())
    });
    require_collection_limit(
        "holding relation",
        holding_relation_count,
        PORTFOLIO_HOLDING_RELATION_LIMIT,
    )?;

    let mut requested_quotes = BTreeSet::new();
    for quote in &cfg.quote_codes {
        if !requested_quotes.insert(*quote) {
            return Err(PortfolioConfigError::DuplicateQuoteCode { quote: *quote });
        }
    }

    let mut network_ids = HashSet::new();
    let mut networks_by_id = BTreeMap::new();
    for network in &cfg.networks {
        validate_network_config(network)?;
        if !network_ids.insert(network.network_id().clone()) {
            return Err(PortfolioConfigError::DuplicateNetworkId {
                network_id: network.network_id().to_string(),
            });
        }
        networks_by_id.insert(network.network_id().clone(), network);
    }

    let mut evm_holding_sources_by_network = BTreeMap::<NetworkId, usize>::new();
    for wallet in &cfg.wallets {
        let Some(NetworkConfig::Evm { .. }) = networks_by_id.get(&wallet.network_id).copied()
        else {
            continue;
        };
        let count = evm_holding_sources_by_network
            .entry(wallet.network_id.clone())
            .or_default();
        *count = count.saturating_add(wallet.symbol_ids.len());
        if *count > EVM_NETWORK_HOLDING_SOURCE_LIMIT {
            return Err(PortfolioConfigError::EvmNetworkHoldingSourceLimitExceeded {
                network_id: wallet.network_id.to_string(),
                limit: EVM_NETWORK_HOLDING_SOURCE_LIMIT,
                actual: *count,
            });
        }
    }

    let mut symbol_ids = HashSet::new();
    let mut symbols_by_id = BTreeMap::new();
    for symbol in &cfg.symbol_configs {
        validate_symbol_config(symbol).map_err(|source| {
            PortfolioConfigError::InvalidSymbolConfig {
                symbol_id: symbol.symbol_id.to_string(),
                source: Box::new(source),
            }
        })?;
        if !symbol_ids.insert(symbol.symbol_id.clone()) {
            return Err(PortfolioConfigError::DuplicateSymbolId {
                symbol_id: symbol.symbol_id.to_string(),
            });
        }
        symbols_by_id.insert(symbol.symbol_id.clone(), symbol);
    }

    for symbol in &cfg.symbol_configs {
        let Some(network) = networks_by_id.get(&symbol.network_id).copied() else {
            return Err(PortfolioConfigError::UnknownSymbolNetwork {
                symbol_id: symbol.symbol_id.to_string(),
                network_id: symbol.network_id.to_string(),
            });
        };
        validate_symbol_for_network_family(symbol, network)?;
        validate_symbol_quote_routes(symbol, &requested_quotes, &symbol_ids)?;
    }

    let mut wallet_ids = HashSet::new();
    let mut wallet_subjects = BTreeMap::new();
    let mut physical_sources = BTreeMap::new();
    let mut demand_count = 0_u64;

    for wallet in &cfg.wallets {
        if !wallet_ids.insert(wallet.wallet_id.clone()) {
            return Err(PortfolioConfigError::DuplicateWalletId {
                wallet_id: wallet.wallet_id.to_string(),
            });
        }
        let Some(network) = networks_by_id.get(&wallet.network_id).copied() else {
            return Err(PortfolioConfigError::UnknownWalletNetwork {
                wallet_id: wallet.wallet_id.to_string(),
                network_id: wallet.network_id.to_string(),
            });
        };
        let wallet_subject_kind = wallet.subject.kind();
        if !wallet_subject_matches_network_family(wallet_subject_kind, network.family()) {
            return Err(PortfolioConfigError::WalletSubjectNetworkFamilyMismatch {
                wallet_id: wallet.wallet_id.to_string(),
                network_id: wallet.network_id.to_string(),
                wallet_subject_kind,
                network_family: network.family(),
            });
        }
        validate_bitcoin_wallet_network(wallet, network)?;
        let subject_key = (
            wallet.network_id.clone(),
            wallet.subject.address_str().to_owned(),
        );
        if let Some(existing_wallet_id) =
            wallet_subjects.insert(subject_key, wallet.wallet_id.clone())
        {
            return Err(PortfolioConfigError::DuplicateWalletSubject {
                wallet_id: wallet.wallet_id.to_string(),
                existing_wallet_id: existing_wallet_id.to_string(),
                network_id: wallet.network_id.to_string(),
            });
        }

        let mut wallet_symbol_ids = HashSet::new();
        for symbol_id in &wallet.symbol_ids {
            if !wallet_symbol_ids.insert(symbol_id) {
                return Err(PortfolioConfigError::DuplicateWalletSymbol {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol_id.to_string(),
                });
            }
            let Some(symbol) = symbols_by_id.get(symbol_id).copied() else {
                return Err(PortfolioConfigError::UnknownWalletSymbol {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol_id.to_string(),
                });
            };
            if symbol.network_id != wallet.network_id {
                return Err(PortfolioConfigError::WalletSymbolNetworkMismatch {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol.symbol_id.to_string(),
                    wallet_network_id: wallet.network_id.to_string(),
                    symbol_network_id: symbol.network_id.to_string(),
                });
            }

            demand_count += 1;
            let source_key = physical_source_key(wallet, symbol, network)?;
            if let Some((existing_wallet_id, existing_symbol_id)) = physical_sources.insert(
                source_key,
                (wallet.wallet_id.clone(), symbol.symbol_id.clone()),
            ) {
                return Err(PortfolioConfigError::AliasedHoldingSource {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol.symbol_id.to_string(),
                    existing_wallet_id: existing_wallet_id.to_string(),
                    existing_symbol_id: existing_symbol_id.to_string(),
                    network_id: wallet.network_id.to_string(),
                });
            }
        }
    }

    if demand_count == 0 {
        return Err(PortfolioConfigError::EmptyHoldingDemand);
    }

    Ok(())
}

fn validate_bitcoin_wallet_network(
    wallet: &WalletConfig,
    network: &NetworkConfig,
) -> Result<(), PortfolioConfigError> {
    let (
        WalletSubject::BitcoinAddress { address },
        NetworkConfig::Bitcoin {
            bitcoin_network, ..
        },
    ) = (&wallet.subject, network)
    else {
        return Ok(());
    };
    let parsed = CheckedBitcoinAddress::parse_any(address.as_str()).map_err(|_| {
        PortfolioConfigError::BitcoinWalletAddressNetworkMismatch {
            wallet_id: wallet.wallet_id.to_string(),
            network_id: wallet.network_id.to_string(),
        }
    })?;
    let expected = BitcoinNetworkTag::new(bitcoin_network).map_err(|_| {
        PortfolioConfigError::InvalidBitcoinNetwork {
            network_id: wallet.network_id.to_string(),
        }
    })?;
    if parsed.require_network(expected).is_ok() {
        Ok(())
    } else {
        Err(PortfolioConfigError::BitcoinWalletAddressNetworkMismatch {
            wallet_id: wallet.wallet_id.to_string(),
            network_id: wallet.network_id.to_string(),
        })
    }
}

fn require_collection_limit(
    collection: &'static str,
    actual: usize,
    limit: usize,
) -> Result<(), PortfolioConfigError> {
    if actual > limit {
        return Err(PortfolioConfigError::CollectionLimitExceeded {
            collection,
            limit,
            actual,
        });
    }
    Ok(())
}

fn physical_source_key(
    wallet: &WalletConfig,
    symbol: &SymbolConfig,
    network: &NetworkConfig,
) -> Result<(NetworkId, String), PortfolioConfigError> {
    let address = wallet.subject.address_str();
    let source = match (&symbol.source, network.family()) {
        (HoldingSourceConfig::Native, NetworkFamilyConfig::Bitcoin) => {
            format!("bitcoin-native:{address}")
        }
        (HoldingSourceConfig::Native, NetworkFamilyConfig::Evm) => {
            format!("evm-native:{address}")
        }
        (HoldingSourceConfig::Erc20 { contract_address }, NetworkFamilyConfig::Evm) => {
            format!("evm-erc20:{}:{address}", contract_address.as_str())
        }
        (HoldingSourceConfig::Erc20 { .. }, NetworkFamilyConfig::Bitcoin) => {
            return Err(
                PortfolioConfigError::UnsupportedHoldingSourceNetworkFamily {
                    symbol_id: symbol.symbol_id.to_string(),
                    network_id: network.network_id().to_string(),
                    network_family: network.family(),
                },
            );
        }
    };
    Ok((network.network_id().clone(), source))
}

fn validate_portfolio_config_for_mfm(cfg: &PortfolioConfig) -> Result<(), String> {
    ValidatedPortfolioConfig::new(cfg.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn wallet_subject_matches_network_family(
    subject_kind: WalletSubjectKind,
    network_family: NetworkFamilyConfig,
) -> bool {
    matches!(
        (subject_kind, network_family),
        (WalletSubjectKind::EvmAddress, NetworkFamilyConfig::Evm)
            | (
                WalletSubjectKind::BitcoinAddress,
                NetworkFamilyConfig::Bitcoin
            )
    )
}

fn validate_symbol_for_network_family(
    symbol: &SymbolConfig,
    network: &NetworkConfig,
) -> Result<(), PortfolioConfigError> {
    let supported = matches!(
        (network.family(), &symbol.source),
        (NetworkFamilyConfig::Bitcoin, HoldingSourceConfig::Native)
            | (
                NetworkFamilyConfig::Evm,
                HoldingSourceConfig::Native | HoldingSourceConfig::Erc20 { .. }
            )
    );
    if !supported {
        return Err(
            PortfolioConfigError::UnsupportedHoldingSourceNetworkFamily {
                symbol_id: symbol.symbol_id.to_string(),
                network_id: network.network_id().to_string(),
                network_family: network.family(),
            },
        );
    }
    Ok(())
}

fn validate_symbol_quote_routes(
    symbol: &SymbolConfig,
    requested_quotes: &BTreeSet<QuoteCode>,
    symbol_ids: &HashSet<SymbolId>,
) -> Result<(), PortfolioConfigError> {
    let symbol_quotes: BTreeSet<_> = symbol
        .valuation
        .quotes
        .iter()
        .map(|quote| quote.quote)
        .collect();

    for quote in requested_quotes {
        if !symbol_quotes.contains(quote) {
            return Err(PortfolioConfigError::MissingValuationQuote {
                symbol_id: symbol.symbol_id.to_string(),
                quote: *quote,
            });
        }
    }
    for quote in &symbol_quotes {
        if !requested_quotes.contains(quote) {
            return Err(PortfolioConfigError::UnexpectedValuationQuote {
                symbol_id: symbol.symbol_id.to_string(),
                quote: *quote,
            });
        }
    }

    for quote in &symbol.valuation.quotes {
        if !symbol_ids.contains(&quote.priced_symbol_id) {
            return Err(PortfolioConfigError::UnknownPricedSymbol {
                symbol_id: symbol.symbol_id.to_string(),
                quote: quote.quote,
                priced_symbol_id: quote.priced_symbol_id.to_string(),
            });
        }
    }

    Ok(())
}
