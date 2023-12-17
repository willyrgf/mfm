use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Kind {
    EVM,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Network {
    pub name: String,
    pub kind: Kind,
    pub symbol: String,
    pub decimals: Option<u8>,
    pub chain_id: u32,
    pub node_url: String,
    pub node_url_failover: Option<String>,
    pub blockexplorer_url: Option<String>,
    pub min_balance_coin: f64,
    pub wrapped_asset: Option<Token>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Networks(HashMap<String, Network>);
impl Networks {
    pub fn get(&self, key: &str) -> Option<&Network> {
        self.0.get(key)
    }
    pub fn hashmap(&self) -> &HashMap<String, Network> {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dex {
    pub name: String,
    pub kind: Kind,
    pub router_address: String,
    pub factory_address: String,
    pub network_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dexes(HashMap<String, Dex>);
impl Dexes {
    pub fn hashmap(&self) -> &HashMap<String, Dex> {
        &self.0
    }
    pub fn get(&self, key: &str) -> Option<&Dex> {
        self.0.get(key)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetwork {
    pub(crate) name: String,
    pub(crate) network_id: String,
    pub(crate) address: String,
    pub(crate) slippage: f64,
    pub(crate) path_asset: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetworks(HashMap<String, TokenNetwork>);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Token {
    pub(crate) kind: String,
    pub(crate) networks: TokenNetworks,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Tokens(HashMap<String, Token>);
impl Tokens {
    pub fn hashmap(&self) -> &HashMap<String, Token> {
        &self.0
    }

    pub fn get(&self, key: &str) -> Option<&Token> {
        self.0.get(key)
    }
}

pub struct Config {
    pub networks: Networks,
    pub dexes: Dexes,
    pub tokens: Tokens,
}
