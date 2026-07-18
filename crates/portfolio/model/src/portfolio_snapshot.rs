use super::*;
use std::str::FromStr;

use alloy_primitives::{B256, U256};
use serde::de;

/// Concrete execution anchor captured for one pinned network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "family", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "execution-anchor",
    schema = "mfm.portfolio.execution_anchor"
)]
pub enum ExecutionAnchor {
    /// EVM execution pinned to one block hash and number on one chain id.
    Evm {
        /// EVM chain id.
        chain_id: u64,
        /// Concrete pinned U256 block number encoded as canonical decimal.
        block_number: String,
        /// Concrete pinned block hash.
        block_hash: String,
    },
    /// Bitcoin execution pinned to one height and block hash.
    Bitcoin {
        /// Concrete pinned block height.
        height: u64,
        /// Concrete pinned block hash.
        block_hash: String,
    },
}

impl<'de> Deserialize<'de> for ExecutionAnchor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Evm {
                chain_id: u64,
                block_number: String,
                block_hash: String,
            },
            Bitcoin {
                height: u64,
                block_hash: String,
            },
        }

        match Wire::deserialize(deserializer)? {
            Wire::Evm {
                chain_id,
                block_number,
                block_hash,
            } => {
                if chain_id == 0 {
                    return Err(de::Error::custom("EVM execution anchor chain id was zero"));
                }
                let number = U256::from_str(&block_number)
                    .map_err(|_| de::Error::custom("EVM block number exceeded U256"))?;
                if number.to_string() != block_number {
                    return Err(de::Error::custom(
                        "EVM block number was not canonical decimal",
                    ));
                }
                let hash = B256::from_str(&block_hash)
                    .map_err(|_| de::Error::custom("EVM block hash was invalid"))?;
                if format!("{hash:#x}") != block_hash {
                    return Err(de::Error::custom("EVM block hash was not canonical"));
                }
                Ok(Self::Evm {
                    chain_id,
                    block_number,
                    block_hash,
                })
            }
            Wire::Bitcoin { height, block_hash } => Ok(Self::Bitcoin { height, block_hash }),
        }
    }
}

/// Concrete pinned network view captured in a snapshot/report artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "network-pin",
    schema = "mfm.portfolio.network_pin"
)]
pub struct NetworkPin {
    /// Stable network identifier.
    pub network_id: String,
    /// Concrete pinned execution anchor.
    pub anchor: ExecutionAnchor,
}

/// Canonical snapshot for one wallet inside a portfolio snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-snapshot",
    schema = "mfm.portfolio.wallet_snapshot"
)]
pub struct WalletSnapshot {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Canonical wallet address.
    pub address: String,
    /// Canonical wallet subject kind.
    pub subject_kind: WalletSubjectKind,
    /// Stable network identifier.
    pub network_id: String,
    /// Observations collected for the wallet.
    pub observations: Vec<Observation>,
}

impl WalletSnapshot {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        for observation in &mut self.observations {
            observation.normalize();
        }
        self.observations.sort_by(|left, right| {
            (left.wallet_id.as_str(), left.symbol_id.as_str())
                .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
        });
    }
}

/// Canonical portfolio snapshot artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue, PublicOutputs)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-snapshot",
    schema = "mfm.portfolio.snapshot"
)]
pub struct PortfolioSnapshot {
    /// Version of the public snapshot schema.
    pub schema_version: u64,
    /// Stable portfolio identifier.
    pub portfolio_id: String,
    /// One pin per referenced network.
    pub network_pins: Vec<NetworkPin>,
    /// Per-wallet observations.
    pub wallets: Vec<WalletSnapshot>,
    /// Symbol configs used to interpret the snapshot.
    pub symbol_configs: Vec<SymbolConfig>,
}

impl PortfolioSnapshot {
    /// The only supported public snapshot schema version.
    pub const SCHEMA_VERSION: u64 = 1;

    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.network_pins
            .sort_by(|left, right| left.network_id.cmp(&right.network_id));
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
}

/// Canonical report derived from the snapshot artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, PublicOutputs)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-report",
    schema = "mfm.portfolio.report"
)]
pub struct PortfolioReport {
    /// Version of the public report schema.
    pub schema_version: u64,
    /// Stable portfolio identifier.
    pub portfolio_id: String,
    /// One pin per referenced network.
    pub network_pins: Vec<NetworkPin>,
    /// Per-wallet quote summaries.
    pub wallet_summaries: Vec<WalletReport>,
    /// Portfolio-level quote totals.
    pub totals_by_quote: Vec<PortfolioQuoteTotal>,
}

impl PortfolioReport {
    /// The only supported public report schema version.
    pub const SCHEMA_VERSION: u64 = 1;

    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.network_pins
            .sort_by(|left, right| left.network_id.cmp(&right.network_id));
        for wallet in &mut self.wallet_summaries {
            wallet.normalize();
        }
        self.wallet_summaries
            .sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
        self.totals_by_quote.sort_by_key(|total| total.quote);
    }
}

/// Canonical per-wallet report summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-report",
    schema = "mfm.portfolio.wallet_report"
)]
pub struct WalletReport {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Totals by quote for the wallet.
    pub totals_by_quote: Vec<PortfolioQuoteTotal>,
}

impl WalletReport {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.totals_by_quote.sort_by_key(|total| total.quote);
    }
}

/// Canonical quote total used by wallet and portfolio reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-quote-total",
    schema = "mfm.portfolio.quote_total"
)]
pub struct PortfolioQuoteTotal {
    /// Quote unit.
    pub quote: QuoteCode,
    /// Direct total value in the quote unit.
    pub total_value_dec: String,
}
