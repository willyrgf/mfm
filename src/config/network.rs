use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Network {
    name: String,
    symbol: String,
    chain_id: u32,
    rpc_url: String,
    blockexplorer_url: String,
    min_balance_coin: f64,
}

impl Network {
    pub fn rpc_url(&self) -> &str {
        self.rpc_url.as_str()
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Networks(HashMap<String, Network>);
impl Networks {
    pub fn get(&self, key: &str) -> &Network {
        self.0.get(key).unwrap()
    }
}
