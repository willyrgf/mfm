//! Static DEX registry configuration.
//!
//! These types model the named exchange backends that runtime config selects from, including both
//! on-chain router/factory addresses and off-chain API endpoints when a DEX relies on them.

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Supported DEX families in static configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Uniswap V2-compatible router.
    UniswapV2,
    /// CowSwap settlement flow.
    CowSwap,
    /// Uniswap V3-compatible router.
    UniswapV3,
}

/// Static DEX definition keyed from [`Dexes`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dex {
    /// Human-readable DEX name.
    pub name: String,
    /// DEX family identifier.
    pub kind: Kind,
    /// Router contract address when the DEX uses one.
    pub router_address: Option<Address>,
    /// Factory contract address when the DEX exposes one.
    pub factory_address: Option<Address>,
    /// Logical network identifier where this DEX is deployed.
    pub network_id: String,
    /// Settlement contract address for auction-based DEXes.
    pub settlement_contract: Option<Address>,
    /// Optional HTTP API endpoint for off-chain routing.
    pub api_url: Option<String>,
}

/// Mapping of named DEX definitions.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dexes(HashMap<String, Dex>);
impl Dexes {
    /// Returns the underlying map of named DEX definitions.
    ///
    /// Use this when validating the full registry or when building derived lookup structures.
    pub fn hashmap(&self) -> &HashMap<String, Dex> {
        &self.0
    }

    /// Looks up a DEX definition by key.
    ///
    /// Keys are the logical DEX identifiers from the YAML registry, not the human-readable
    /// [`Dex::name`] field.
    pub fn get(&self, key: &str) -> Option<&Dex> {
        self.0.get(key)
    }
}
