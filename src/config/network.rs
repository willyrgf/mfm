use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use web3::{transports::Http, types::U256, Web3};

use super::{exchange::Exchange, Config};
use crate::asset::Asset;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Network {
    name: String,
    symbol: String,
    decimals: Option<u8>,
    chain_id: u32,
    rpc_url: String,
    blockexplorer_url: String,
    min_balance_coin: f64,
    wrapped_asset: String,
}

impl Network {
    pub fn rpc_url(&self) -> &str {
        self.rpc_url.as_str()
    }

    pub fn get_name(&self) -> &str {
        self.name.as_str()
    }

    pub fn get_symbol(&self) -> &str {
        self.symbol.as_str()
    }

    // TODO: try get this value from some request in the blockchain
    pub fn coin_decimals(&self) -> u8 {
        match self.decimals {
            Some(n) => n,
            _ => 18_u8,
        }
    }

    pub fn get_wrapped_asset(&self) -> Asset {
        Config::global()
            .assets
            .find_by_name_and_network(self.wrapped_asset.as_str(), self.name.as_str())
            .unwrap()
    }
    pub fn get_min_balance_coin(&self, decimals: u8) -> U256 {
        let qe = (self.min_balance_coin * 10_f64.powf(decimals.into())) as i64;
        U256::from(qe)
    }

    pub fn get_web3_client_http(&self) -> Web3<Http> {
        Web3::new(Http::new(self.rpc_url()).unwrap())
    }

    pub fn get_exchanges(&self) -> Vec<&Exchange> {
        Config::global()
            .exchanges
            .hashmap()
            .values()
            .filter(|e| e.network_id() == &self.name)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Networks(HashMap<String, Network>);
impl Networks {
    pub fn get(&self, key: &str) -> Option<&Network> {
        self.0.get(key)
    }
}
