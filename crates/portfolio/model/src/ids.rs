use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

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
