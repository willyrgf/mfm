//! Canonical persisted EVM values shared by portfolio and capability surfaces.

use std::str::FromStr;

use alloy_primitives::{B256, U256};
use mfm_program_derive::MfmValue;
use serde::{de, Deserialize, Serialize};

/// Error returned when persisted EVM block-anchor material is not canonical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmBlockAnchorError {
    /// The block number is not a canonical U256 decimal string.
    #[error("EVM block number was not canonical U256 decimal")]
    InvalidNumber,
    /// The block hash is not canonical fixed-width lower-case hexadecimal.
    #[error("EVM block hash was not canonical fixed-width hexadecimal")]
    InvalidHash,
}

/// Canonical persisted EVM block number and hash pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "block_anchor",
    version = "1",
    schema = "mfm.evm.block_anchor"
)]
pub struct EvmBlockAnchor {
    number: String,
    hash: String,
}

impl EvmBlockAnchor {
    /// Creates one concrete block anchor without narrowing its U256 number.
    pub fn new(number: U256, hash: B256) -> Self {
        Self {
            number: number.to_string(),
            hash: format!("{hash:#x}"),
        }
    }

    /// Returns the canonical decimal block number.
    pub fn number(&self) -> &str {
        &self.number
    }

    /// Returns the canonical lower-case block hash.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Parses the checked block number as an Alloy U256.
    pub fn number_quantity(&self) -> Result<U256, EvmBlockAnchorError> {
        parse_anchor_number(&self.number)
    }

    /// Parses the checked block hash as an Alloy B256.
    pub fn hash_value(&self) -> Result<B256, EvmBlockAnchorError> {
        parse_anchor_hash(&self.hash)
    }

    /// Revalidates both canonical fields.
    pub fn validate(&self) -> Result<(), EvmBlockAnchorError> {
        self.number_quantity()?;
        self.hash_value()?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmBlockAnchor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            number: String,
            hash: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let number = parse_anchor_number(&wire.number).map_err(de::Error::custom)?;
        let hash = parse_anchor_hash(&wire.hash).map_err(de::Error::custom)?;
        Ok(Self::new(number, hash))
    }
}

fn parse_anchor_number(value: &str) -> Result<U256, EvmBlockAnchorError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EvmBlockAnchorError::InvalidNumber);
    }
    let number = U256::from_str(value).map_err(|_| EvmBlockAnchorError::InvalidNumber)?;
    if number.to_string() != value {
        return Err(EvmBlockAnchorError::InvalidNumber);
    }
    Ok(number)
}

fn parse_anchor_hash(value: &str) -> Result<B256, EvmBlockAnchorError> {
    let hash = B256::from_str(value).map_err(|_| EvmBlockAnchorError::InvalidHash)?;
    if format!("{hash:#x}") != value {
        return Err(EvmBlockAnchorError::InvalidHash);
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_anchor_round_trips_only_canonical_values() {
        let number = U256::MAX;
        let hash = B256::from([0xab; 32]);
        let anchor = EvmBlockAnchor::new(number, hash);
        assert_eq!(anchor.number_quantity().expect("number"), number);
        assert_eq!(anchor.hash_value().expect("hash"), hash);

        for invalid in [
            serde_json::json!({"number": "01", "hash": format!("{hash:#x}")}),
            serde_json::json!({"number": "1", "hash": "0xabc"}),
        ] {
            assert!(serde_json::from_value::<EvmBlockAnchor>(invalid).is_err());
        }
    }
}
