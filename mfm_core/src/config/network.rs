use super::token::Token;
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
