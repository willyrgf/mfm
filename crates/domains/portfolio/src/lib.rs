#![warn(missing_docs)]
//! Secret-free sequential Portfolio State values.
//!
//! Portfolio owns one singular admission context and one cumulative continuation.  Each EVM
//! collection is expanded into an ordinary sequential child State; there is no runtime collection
//! loop, output map, parallel branch, or multi-result join.

use mfm_evm::{EvmBalanceCollectionResult, EvmBalanceRequest};
use mfm_ids::StableId;
use mfm_program_derive::{MfmConfig, MfmValue};
use serde::de;
use serde::{Deserialize, Serialize};

/// Stable Portfolio snapshot entry-point identity.
pub const PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID: &str = "mfm.portfolio/snapshot@1";
/// Stable Portfolio operation identity.
pub const PORTFOLIO_SNAPSHOT_OPERATION_ID: &str = "mfm.portfolio.snapshot@1";
/// Maximum declaration-ordered EVM collections.
pub const PORTFOLIO_COLLECTION_LIMIT: usize = 64;

/// Public quote identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
pub enum QuoteCode {
    /// US dollar quote.
    Usd,
    /// Euro quote.
    Eur,
}

/// One public Portfolio identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioId {
    /// Stable public spelling.
    pub value: String,
}

impl<'de> Deserialize<'de> for PortfolioId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            value: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        if !valid_public_text(&wire.value, 256) {
            return Err(de::Error::custom(PortfolioError::InvalidValue));
        }
        Ok(Self { value: wire.value })
    }
}

/// One singular domain-planned Portfolio admission value.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSnapshotInput {
    /// Stable Portfolio identity.
    pub portfolio_id: PortfolioId,
    /// One declaration-ordered EVM collection per child fragment.
    pub collections: Vec<EvmBalanceRequest>,
    /// Requested public quote.
    pub quote: QuoteCode,
}

impl<'de> Deserialize<'de> for PortfolioSnapshotInput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            portfolio_id: PortfolioId,
            collections: Vec<EvmBalanceRequest>,
            quote: QuoteCode,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.portfolio_id, wire.collections, wire.quote).map_err(de::Error::custom)
    }
}

impl PortfolioSnapshotInput {
    /// Creates one bounded singular admission value.
    pub fn new(
        portfolio_id: PortfolioId,
        collections: Vec<EvmBalanceRequest>,
        quote: QuoteCode,
    ) -> Result<Self, PortfolioError> {
        if !valid_public_text(&portfolio_id.value, 256)
            || collections.is_empty()
            || collections.len() > PORTFOLIO_COLLECTION_LIMIT
            || collections
                .iter()
                .any(|collection| collection.validate().is_err())
            || total_sources(&collections) > mfm_evm::EVM_BALANCE_SOURCE_LIMIT
        {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(Self {
            portfolio_id,
            collections,
            quote,
        })
    }

    /// Validates a decoded snapshot input and every declaration-ordered child request.
    pub fn validate(&self) -> Result<(), PortfolioError> {
        if !valid_public_text(&self.portfolio_id.value, 256)
            || self.collections.is_empty()
            || self.collections.len() > PORTFOLIO_COLLECTION_LIMIT
            || self
                .collections
                .iter()
                .any(|collection| collection.validate().is_err())
            || total_sources(&self.collections) > mfm_evm::EVM_BALANCE_SOURCE_LIMIT
        {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(())
    }
}

/// Complete cumulative Portfolio continuation passed between sequential States.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioContinuation {
    /// Singular admitted input retained for late consolidation.
    pub input: PortfolioSnapshotInput,
    /// Completed collections in declaration order.
    pub completed: Vec<EvmBalanceCollectionResult>,
    /// Next collection ordinal.
    pub next_collection: u16,
    /// Declaration ordinals that remain after the current completed prefix.
    pub remaining_collections: Vec<u16>,
}

impl<'de> Deserialize<'de> for PortfolioContinuation {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            input: PortfolioSnapshotInput,
            completed: Vec<EvmBalanceCollectionResult>,
            next_collection: u16,
            remaining_collections: Vec<u16>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let continuation = Self {
            input: wire.input,
            completed: wire.completed,
            next_collection: wire.next_collection,
            remaining_collections: wire.remaining_collections,
        };
        continuation
            .validate()
            .map(|_| continuation)
            .map_err(de::Error::custom)
    }
}

impl PortfolioContinuation {
    /// Creates one declaration-ordered continuation at a collection boundary.
    pub fn new(
        input: PortfolioSnapshotInput,
        completed: Vec<EvmBalanceCollectionResult>,
        next_collection: u16,
    ) -> Result<Self, PortfolioError> {
        let remaining_collections = (usize::from(next_collection)..input.collections.len())
            .map(|ordinal| u16::try_from(ordinal).map_err(|_| PortfolioError::InvalidValue))
            .collect::<Result<Vec<_>, _>>()?;
        let continuation = Self {
            input,
            completed,
            next_collection,
            remaining_collections,
        };
        continuation.validate()?;
        Ok(continuation)
    }

    /// Validates declaration order, completed result count, and remaining work.
    pub fn validate(&self) -> Result<(), PortfolioError> {
        self.input.validate()?;
        let next = usize::from(self.next_collection);
        if next > self.input.collections.len()
            || self.completed.len() != next
            || self
                .completed
                .iter()
                .any(|result| result.validate().is_err())
            || self.completed.iter().enumerate().any(|(ordinal, result)| {
                let request = &self.input.collections[ordinal];
                result.results.len() != request.sources.len()
                    || result
                        .results
                        .iter()
                        .zip(&request.sources)
                        .any(|(result, source)| result.source_id != source.source_id)
            })
        {
            return Err(PortfolioError::InvalidContinuation);
        }
        let expected: Vec<u16> = (next..self.input.collections.len())
            .map(|ordinal| u16::try_from(ordinal).map_err(|_| PortfolioError::InvalidContinuation))
            .collect::<Result<Vec<_>, _>>()?;
        if self.remaining_collections != expected {
            return Err(PortfolioError::InvalidContinuation);
        }
        Ok(())
    }
}

/// Final public Portfolio snapshot output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSnapshotOutput {
    /// Stable Portfolio identity.
    pub portfolio_id: PortfolioId,
    /// Quote selected at admission.
    pub quote: QuoteCode,
    /// Collection totals in declaration order.
    pub collection_totals: Vec<String>,
}

impl<'de> Deserialize<'de> for PortfolioSnapshotOutput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            portfolio_id: PortfolioId,
            quote: QuoteCode,
            collection_totals: Vec<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let output = Self {
            portfolio_id: wire.portfolio_id,
            quote: wire.quote,
            collection_totals: wire.collection_totals,
        };
        output.validate().map(|_| output).map_err(de::Error::custom)
    }
}

impl PortfolioSnapshotOutput {
    /// Validates the public output envelope and integer-scaled totals.
    pub fn validate(&self) -> Result<(), PortfolioError> {
        if !valid_public_text(&self.portfolio_id.value, 256)
            || self.collection_totals.is_empty()
            || self.collection_totals.len() > PORTFOLIO_COLLECTION_LIMIT
            || self
                .collection_totals
                .iter()
                .any(|total| total.is_empty() || total.len() > 80 || !is_decimal_integer(total))
        {
            return Err(PortfolioError::InvalidValue);
        }
        Ok(())
    }
}

/// Explicit fail-fast Portfolio failure route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PortfolioSnapshotFailure {
    /// The admitted configuration is invalid.
    InvalidInput,
    /// One child collection failed and later children are suppressed.
    CollectionFailed {
        /// Declaration-ordered collection ordinal.
        ordinal: u16,
        /// Stable redacted collection failure code.
        code: String,
    },
    /// Final arithmetic or binding validation failed.
    ConsolidationFailed,
}

/// Selector used by the fixed-tenant application to choose the admitted Portfolio target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSnapshotSelector {
    /// Target Portfolio identity.
    pub target: PortfolioId,
}

impl PortfolioSnapshotSelector {
    /// Returns the selected target identity.
    pub const fn target(&self) -> &PortfolioId {
        &self.target
    }
}

/// Secret-free routing binding used by domain planning and immutable adapter descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioEvmRoutingBinding {
    /// Public network id.
    pub network_id: u64,
    /// Public chain identity.
    pub chain_id: u64,
    /// Content identity of the physical route descriptor.
    pub route_ref: String,
}

/// Public routing manifest admitted with one Portfolio operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct PortfolioRoutingManifest {
    /// Ordered EVM route bindings.
    pub evm: Vec<PortfolioEvmRoutingBinding>,
}

/// Minimal persisted Portfolio configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(validate = "validate_portfolio_config")]
pub struct PortfolioConfig {
    /// Portfolio identity.
    pub portfolio_id: PortfolioId,
    /// Public quote list.
    pub quotes: Vec<QuoteCode>,
}

fn validate_portfolio_config(config: &PortfolioConfig) -> Result<(), PortfolioError> {
    if config.portfolio_id.value.is_empty() || config.quotes.is_empty() {
        return Err(PortfolioError::InvalidValue);
    }
    Ok(())
}

/// Redaction-safe Portfolio domain error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortfolioError {
    /// A bounded public value is invalid.
    #[error("Portfolio domain value is invalid")]
    InvalidValue,
    /// A declaration-order continuation is not valid.
    #[error("Portfolio continuation is invalid")]
    InvalidContinuation,
}

/// Returns a stable `StableId` for the Portfolio entry point.
pub fn entry_point_id() -> Result<StableId, PortfolioError> {
    StableId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).map_err(|_| PortfolioError::InvalidValue)
}

fn is_decimal_integer(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes().iter().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn valid_public_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
        && ![
            "password",
            "passphrase",
            "mnemonic",
            "private_key",
            "privatekey",
            "secret",
            "access_token",
            "api_key",
        ]
        .iter()
        .any(|marker| value.to_ascii_lowercase().contains(marker))
}

fn total_sources(collections: &[EvmBalanceRequest]) -> usize {
    collections
        .iter()
        .map(|collection| collection.sources.len())
        .try_fold(0usize, usize::checked_add)
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_evm::EvmBalanceSource;
    use mfm_values::ValidatedConfig;

    #[test]
    fn shared_config_validation_rejects_domain_invalid_values() {
        assert!(ValidatedConfig::new(PortfolioConfig {
            portfolio_id: PortfolioId {
                value: "portfolio-1".to_owned(),
            },
            quotes: Vec::new(),
        })
        .is_err());
    }

    fn request() -> EvmBalanceRequest {
        EvmBalanceRequest::new(
            vec![mfm_evm::EvmBalanceSource {
                source_id: "source-1".to_owned(),
                chain_id: 1,
                address: "0xabc".to_owned(),
                token: None,
            }],
            18,
        )
        .expect("request")
    }

    #[test]
    fn continuation_proves_remaining_declaration_suffix() {
        let input = PortfolioSnapshotInput::new(
            PortfolioId {
                value: "portfolio-1".to_owned(),
            },
            vec![request()],
            QuoteCode::Usd,
        )
        .expect("input");
        let mut continuation =
            PortfolioContinuation::new(input, Vec::new(), 0).expect("continuation");
        assert_eq!(continuation.remaining_collections, vec![0]);
        continuation.remaining_collections.clear();
        assert_eq!(
            continuation.validate(),
            Err(PortfolioError::InvalidContinuation)
        );
    }

    #[test]
    fn output_rejects_non_integer_totals() {
        let output = PortfolioSnapshotOutput {
            portfolio_id: PortfolioId {
                value: "portfolio-1".to_owned(),
            },
            quote: QuoteCode::Usd,
            collection_totals: vec!["1.5".to_owned()],
        };
        assert_eq!(output.validate(), Err(PortfolioError::InvalidValue));
    }

    #[test]
    fn portfolio_total_source_bound_is_exact() {
        let sources = |count: usize| {
            (0..count)
                .map(|ordinal| EvmBalanceSource {
                    source_id: format!("source-{ordinal}"),
                    chain_id: 1,
                    address: format!("0x{ordinal:x}"),
                    token: None,
                })
                .collect::<Vec<_>>()
        };
        let accepted = EvmBalanceRequest::new(sources(64), 18).expect("64 sources");
        assert!(PortfolioSnapshotInput::new(
            PortfolioId {
                value: "portfolio-1".to_owned(),
            },
            vec![accepted],
            QuoteCode::Usd,
        )
        .is_ok());

        let half = EvmBalanceRequest::new(sources(32), 18).expect("32 sources");
        let over = EvmBalanceRequest::new(sources(33), 18).expect("33 sources");
        assert!(PortfolioSnapshotInput::new(
            PortfolioId {
                value: "portfolio-1".to_owned(),
            },
            vec![half, over],
            QuoteCode::Usd,
        )
        .is_err());

        let rejected = EvmBalanceRequest::new(sources(65), 18);
        assert!(rejected.is_err());
    }

    #[test]
    fn admission_deserialization_reenters_nested_domain_validation() {
        let invalid = serde_json::json!({
            "portfolio_id": {"value": "portfolio-1"},
            "collections": [{
                "sources": [{
                    "source_id": "source-1",
                    "chain_id": 1,
                    "address": "0xABC",
                    "token": null
                }],
                "decimals": 18
            }],
            "quote": "usd"
        });
        assert!(serde_json::from_value::<PortfolioSnapshotInput>(invalid).is_err());
    }
}
