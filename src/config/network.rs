use std::collections::HashMap;
use std::str::FromStr;

use rustc_hex::{FromHex, FromHexError};
use serde::{Deserialize, Serialize};
use web3::{
    contract::{Contract, Options},
    transports::Http,
    types::{Address, H160, U256},
};

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Network {
    name: String,
    symbol: String,
    chain_id: u32,
    rpc_url: String,
    blockexplorer_url: String,
    min_balance_coin: f64
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
