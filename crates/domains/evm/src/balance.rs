//! Canonical values for structured EVM balance collection.

use std::collections::BTreeSet;
use std::str::FromStr;

use alloy_primitives::Address;
use mfm_program_derive::{MfmConfig, MfmValue};
use mfm_values::ConfigError;
use serde::{de, Deserialize, Serialize};

use crate::{EvmAnchoredSource, EvmNetworkBinding};

/// Maximum source demand admitted by one EVM balance collection.
pub const EVM_BALANCE_COLLECTION_SOURCE_LIMIT: usize = 1_024;
/// Maximum distinct ERC-20 metadata demand admitted by one collection.
pub const EVM_BALANCE_COLLECTION_TOKEN_LIMIT: usize = 1_024;
/// Maximum number of states in one maximal structured collection.
pub const EVM_BALANCE_COLLECTION_STATE_LIMIT: usize = 2_052;

/// Redaction-safe construction or aggregation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmBalanceCollectionError {
    /// One value violated the closed EVM balance contract.
    #[error("invalid EVM balance collection value: {0}")]
    Invalid(&'static str),
}

/// Asset identity for one EVM balance request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-asset",
    version = "1",
    schema = "mfm.evm.balance_asset"
)]
pub enum EvmBalanceAsset {
    /// Native network asset.
    Native,
    /// ERC-20 contract asset.
    Erc20 {
        /// Canonical non-zero contract address.
        contract_address: String,
    },
}

impl EvmBalanceAsset {
    /// Creates a checked ERC-20 asset.
    pub fn erc20(contract_address: Address) -> Result<Self, EvmBalanceCollectionError> {
        if contract_address.is_zero() {
            return Err(EvmBalanceCollectionError::Invalid(
                "zero ERC-20 contract address",
            ));
        }
        Ok(Self::Erc20 {
            contract_address: canonical_address(contract_address),
        })
    }

    /// Returns the token contract for an ERC-20 asset.
    pub fn contract_address(&self) -> Option<&str> {
        match self {
            Self::Native => None,
            Self::Erc20 { contract_address } => Some(contract_address),
        }
    }

    fn validate(&self) -> Result<(), EvmBalanceCollectionError> {
        if let Some(raw) = self.contract_address() {
            let address = parse_address(raw)?;
            if address.is_zero() {
                return Err(EvmBalanceCollectionError::Invalid(
                    "zero ERC-20 contract address",
                ));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmBalanceAsset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Native,
            Erc20 { contract_address: String },
        }

        match Wire::deserialize(deserializer)? {
            Wire::Native => Ok(Self::Native),
            Wire::Erc20 { contract_address } => {
                let address = parse_address(&contract_address).map_err(de::Error::custom)?;
                Self::erc20(address).map_err(de::Error::custom)
            }
        }
    }
}

/// One unique account/asset balance source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-source",
    version = "1",
    schema = "mfm.evm.balance_source"
)]
pub struct EvmBalanceSource {
    account: String,
    asset: EvmBalanceAsset,
}

impl EvmBalanceSource {
    /// Creates one checked source.
    pub fn new(
        account: Address,
        asset: EvmBalanceAsset,
    ) -> Result<Self, EvmBalanceCollectionError> {
        if account.is_zero() {
            return Err(EvmBalanceCollectionError::Invalid("zero account address"));
        }
        asset.validate()?;
        Ok(Self {
            account: canonical_address(account),
            asset,
        })
    }

    /// Returns the canonical account address.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the requested asset.
    pub const fn asset(&self) -> &EvmBalanceAsset {
        &self.asset
    }

    /// Parses the checked account address.
    pub fn account_address(&self) -> Result<Address, EvmBalanceCollectionError> {
        parse_nonzero_address(&self.account)
    }

    /// Parses the optional token contract.
    pub fn contract_address_value(&self) -> Result<Option<Address>, EvmBalanceCollectionError> {
        self.asset
            .contract_address()
            .map(parse_nonzero_address)
            .transpose()
    }

    fn validate(&self) -> Result<(), EvmBalanceCollectionError> {
        self.account_address()?;
        self.asset.validate()
    }
}

/// Certified EVM network demand and immutable routing generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.evm.config.balance_collection",
    version = "1",
    validate = "validate_evm_balance_collection_config"
)]
pub struct EvmBalanceCollectionConfig {
    binding: EvmNetworkBinding,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
}

impl EvmBalanceCollectionConfig {
    /// Creates checked, canonical source demand.
    pub fn new(
        binding: EvmNetworkBinding,
        native_decimals: u8,
        mut sources: Vec<EvmBalanceSource>,
    ) -> Result<Self, ConfigError> {
        sources.sort();
        let config = Self {
            binding,
            native_decimals,
            sources,
        };
        validate_evm_balance_collection_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the semantic network and immutable generation.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }

    /// Returns the configured native scale.
    pub const fn native_decimals(&self) -> u8 {
        self.native_decimals
    }

    /// Returns sorted, unique source demand.
    pub fn sources(&self) -> &[EvmBalanceSource] {
        &self.sources
    }

    /// Returns distinct token contracts in canonical order.
    pub fn token_contracts(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter_map(|source| source.asset.contract_address().map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn state_count(&self) -> usize {
        self.sources.len() + self.token_contracts().len() + 4
    }
}

/// Validates collection bounds and canonical demand order.
pub fn validate_evm_balance_collection_config(
    config: &EvmBalanceCollectionConfig,
) -> Result<(), String> {
    if config.sources.is_empty() || config.sources.len() > EVM_BALANCE_COLLECTION_SOURCE_LIMIT {
        return Err("EVM source demand must contain between 1 and 1024 entries".to_owned());
    }
    let mut previous = None;
    for source in &config.sources {
        source.validate().map_err(|error| error.to_string())?;
        if previous.is_some_and(|prior: &EvmBalanceSource| prior >= source) {
            return Err("EVM source demand must be strictly sorted and unique".to_owned());
        }
        previous = Some(source);
    }
    if config.token_contracts().len() > EVM_BALANCE_COLLECTION_TOKEN_LIMIT {
        return Err("EVM token demand exceeded 1024 distinct contracts".to_owned());
    }
    if config.state_count() > EVM_BALANCE_COLLECTION_STATE_LIMIT {
        return Err("EVM collection exceeded 2052 states".to_owned());
    }
    Ok(())
}

/// One final typed balance produced by pure aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "collected-balance",
    version = "1",
    schema = "mfm.evm.collected_balance"
)]
pub struct EvmCollectedBalance {
    source: EvmBalanceSource,
    raw_units: String,
    decimals: u8,
}

impl EvmCollectedBalance {
    pub(crate) fn new(source: EvmBalanceSource, raw_units: String, decimals: u8) -> Self {
        Self {
            source,
            raw_units,
            decimals,
        }
    }

    /// Returns the account/asset demand.
    pub const fn source(&self) -> &EvmBalanceSource {
        &self.source
    }

    /// Returns the canonical U256 decimal quantity.
    pub fn raw_units(&self) -> &str {
        &self.raw_units
    }

    /// Returns the native or token scale.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// Complete same-run EVM collection consumed by portfolio aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-collection",
    version = "1",
    schema = "mfm.evm.balance_collection"
)]
pub struct EvmBalanceCollection {
    source: EvmAnchoredSource,
    balances: Vec<EvmCollectedBalance>,
}

impl EvmBalanceCollection {
    pub(crate) fn new(source: EvmAnchoredSource, balances: Vec<EvmCollectedBalance>) -> Self {
        Self { source, balances }
    }

    /// Returns the exact checked source and common anchor.
    pub const fn source(&self) -> &EvmAnchoredSource {
        &self.source
    }

    /// Returns balances in certified config order.
    pub fn balances(&self) -> &[EvmCollectedBalance] {
        &self.balances
    }
}

fn canonical_address(value: Address) -> String {
    format!("{value:#x}")
}

fn parse_address(value: &str) -> Result<Address, EvmBalanceCollectionError> {
    let parsed =
        Address::from_str(value).map_err(|_| EvmBalanceCollectionError::Invalid("address"))?;
    if canonical_address(parsed) != value {
        return Err(EvmBalanceCollectionError::Invalid("address"));
    }
    Ok(parsed)
}

fn parse_nonzero_address(value: &str) -> Result<Address, EvmBalanceCollectionError> {
    let parsed = parse_address(value)?;
    if parsed.is_zero() {
        return Err(EvmBalanceCollectionError::Invalid("zero address"));
    }
    Ok(parsed)
}
