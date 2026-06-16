use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use mfm_evm_core::encoding::normalize_address;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain_key::{validate_author_key, StableDomainKeyError};

/// Error returned when constructing portfolio scalar authorities.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PortfolioScalarError {
    /// A scalar identifier did not satisfy the portfolio author-key grammar.
    #[error("{kind} `{value}` did not satisfy portfolio author-key grammar: {source}")]
    InvalidAuthorKey {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Rejected scalar value.
        value: String,
        /// Underlying author-key grammar error.
        source: StableDomainKeyError,
    },
    /// A scalar was not a normalized EVM address.
    #[error("{kind} `{value}` must be a normalized EVM address")]
    InvalidEvmAddress {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Rejected scalar value.
        value: String,
    },
    /// A scalar was not a non-negative decimal string.
    #[error("{kind} `{value}` must be a non-negative decimal string")]
    InvalidDecimalString {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Rejected scalar value.
        value: String,
    },
}

macro_rules! portfolio_id_type {
    ($ty:ident, $kind:literal, $semantic:literal, $schema:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue,
        )]
        #[serde(try_from = "String", into = "String")]
        #[mfm(namespace = "mfm.portfolio", name = $semantic, schema = $schema, transparent_string)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            /// Creates a checked portfolio scalar authority.
            pub fn new(value: impl Into<String>) -> Result<Self, PortfolioScalarError> {
                let raw = value.into();
                validate_author_key(&raw).map_err(|source| {
                    PortfolioScalarError::InvalidAuthorKey {
                        kind: $kind,
                        value: raw.clone(),
                        source,
                    }
                })?;
                Ok(Self { raw })
            }

            /// Returns the canonical string representation.
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            /// Consumes this authority into its canonical string representation.
            pub fn into_string(self) -> String {
                self.raw
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $ty {
            type Err = PortfolioScalarError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = PortfolioScalarError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $ty {
            type Error = PortfolioScalarError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.raw
            }
        }

        impl PartialEq<&str> for $ty {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }

        impl PartialEq<$ty> for &str {
            fn eq(&self, other: &$ty) -> bool {
                *self == other.as_str()
            }
        }
    };
}

portfolio_id_type!(
    PortfolioId,
    "portfolio_id",
    "portfolio-id",
    "mfm.portfolio.id.portfolio",
    "Stable typed portfolio identifier."
);

portfolio_id_type!(
    NetworkId,
    "network_id",
    "network-id",
    "mfm.portfolio.id.network",
    "Stable typed portfolio network identifier."
);

portfolio_id_type!(
    WalletId,
    "wallet_id",
    "wallet-id",
    "mfm.portfolio.id.wallet",
    "Stable typed portfolio wallet identifier."
);

portfolio_id_type!(
    SymbolId,
    "symbol_id",
    "symbol-id",
    "mfm.portfolio.id.symbol",
    "Stable typed portfolio symbol identifier."
);

portfolio_id_type!(
    ValuationSourceId,
    "source_id",
    "valuation-source-id",
    "mfm.portfolio.id.valuation_source",
    "Stable typed portfolio valuation-source identifier."
);

portfolio_id_type!(
    ProtocolId,
    "protocol",
    "protocol-id",
    "mfm.portfolio.id.protocol",
    "Stable typed portfolio protocol identifier."
);

portfolio_id_type!(
    ProtocolReaderId,
    "protocol_reader",
    "protocol-reader-id",
    "mfm.portfolio.id.protocol_reader",
    "Stable typed portfolio protocol reader identifier."
);

portfolio_id_type!(
    OracleKindId,
    "oracle_kind",
    "oracle-kind-id",
    "mfm.portfolio.id.oracle_kind",
    "Stable typed portfolio oracle kind identifier."
);

portfolio_id_type!(
    AaveMarketId,
    "aave_market_id",
    "aave-market-id",
    "mfm.portfolio.id.aave_market",
    "Stable typed Aave market identifier."
);

portfolio_id_type!(
    AaveReserveId,
    "aave_reserve_id",
    "aave-reserve-id",
    "mfm.portfolio.id.aave_reserve",
    "Stable typed Aave reserve identifier."
);

portfolio_id_type!(
    ControlScopeId,
    "control_scope",
    "control-scope-id",
    "mfm.portfolio.id.control_scope",
    "Stable typed portfolio control-plane scope identifier."
);

portfolio_id_type!(
    KeystoreEntryId,
    "entry_id",
    "keystore-entry-id",
    "mfm.portfolio.id.keystore_entry",
    "Stable typed portfolio keystore entry identifier."
);

portfolio_id_type!(
    ExternalSignerId,
    "signer_id",
    "external-signer-id",
    "mfm.portfolio.id.external_signer",
    "Stable typed portfolio external signer identifier."
);

/// Normalized lowercase EVM address authority.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "normalized-evm-address",
    schema = "mfm.portfolio.address.evm.normalized",
    transparent_string
)]
pub struct NormalizedEvmAddress {
    raw: String,
}

impl NormalizedEvmAddress {
    /// Creates a checked normalized EVM address.
    pub fn new(value: impl Into<String>, kind: &'static str) -> Result<Self, PortfolioScalarError> {
        let raw = value.into();
        let normalized =
            normalize_address(&raw).map_err(|_| PortfolioScalarError::InvalidEvmAddress {
                kind,
                value: raw.clone(),
            })?;
        if normalized != raw {
            return Err(PortfolioScalarError::InvalidEvmAddress { kind, value: raw });
        }
        Ok(Self { raw })
    }

    /// Creates a checked normalized EVM address for serde-decoded fields.
    pub fn parse(value: impl Into<String>) -> Result<Self, PortfolioScalarError> {
        Self::new(value, "evm_address")
    }

    /// Returns the canonical normalized address string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this authority into its canonical normalized address string.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl AsRef<str> for NormalizedEvmAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for NormalizedEvmAddress {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for NormalizedEvmAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for NormalizedEvmAddress {
    type Err = PortfolioScalarError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for NormalizedEvmAddress {
    type Error = PortfolioScalarError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for NormalizedEvmAddress {
    type Error = PortfolioScalarError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<NormalizedEvmAddress> for String {
    fn from(value: NormalizedEvmAddress) -> Self {
        value.raw
    }
}

/// Non-negative decimal string authority for portfolio unit-price config.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "unit-price-decimal",
    schema = "mfm.portfolio.decimal.unit_price",
    transparent_string
)]
pub struct UnitPriceDecimal {
    raw: String,
}

impl UnitPriceDecimal {
    /// Creates a checked non-negative decimal string.
    pub fn new(value: impl Into<String>) -> Result<Self, PortfolioScalarError> {
        let raw = value.into();
        if !is_non_negative_decimal_string(&raw) {
            return Err(PortfolioScalarError::InvalidDecimalString {
                kind: "unit_price_dec",
                value: raw,
            });
        }
        Ok(Self { raw })
    }

    /// Returns the canonical string representation.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this authority into its string representation.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl AsRef<str> for UnitPriceDecimal {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for UnitPriceDecimal {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for UnitPriceDecimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UnitPriceDecimal {
    type Err = PortfolioScalarError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for UnitPriceDecimal {
    type Error = PortfolioScalarError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for UnitPriceDecimal {
    type Error = PortfolioScalarError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<UnitPriceDecimal> for String {
    fn from(value: UnitPriceDecimal) -> Self {
        value.raw
    }
}

fn is_non_negative_decimal_string(raw: &str) -> bool {
    if raw.is_empty() || raw.trim() != raw || raw.starts_with(['+', '-']) {
        return false;
    }
    let mut parts = raw.split('.');
    let Some(integer) = parts.next() else {
        return false;
    };
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|ch| ch.is_ascii_digit())
    {
        return false;
    }
    match fraction {
        Some(fraction) => !fraction.is_empty() && fraction.bytes().all(|ch| ch.is_ascii_digit()),
        None => true,
    }
}
