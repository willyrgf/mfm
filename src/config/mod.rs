use std::collections::HashMap;

use rustc_hex::FromHex;
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Wallet {
    private_key: String,
}

impl Wallet {
    pub fn to_raw(&self) -> Vec<u8> {
        self.private_key.from_hex().unwrap()
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Wallets(HashMap<String, Wallet>);

impl Wallets {
    pub fn get(&self, key: &str) -> &Wallet {
        self.0.get(key).unwrap()
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Asset {
    network_id: String,
    base: String,
    quote: String,
    pair_address: String,
    exchange_id: String,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Network {
    name: String,
    symbol: String,
    chain_id: u32,
    rpc_url: String,
    blockexplorer_url: String,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchange {
    name: String,
    router_address: String,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub wallets: Wallets,
    pub assets: HashMap<String, Asset>,
    pub networks: HashMap<String, Network>,
    pub exchanges: HashMap<String, Exchange>,
}

impl Config {
    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();
        config
    }
}
