use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Wallet {
    import_private_key: String,
}
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct General {
    wallets: HashMap<String, Wallet>,
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
    general: General,
    assets: HashMap<String, Asset>,
    networks: HashMap<String, Network>,
    exchanges: HashMap<String, Exchange>,
}

impl Config {
    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();
        config
    }
}
