use std::collections::HashMap;
use std::str::FromStr;

use rustc_hex::{FromHex, FromHexError};
use serde::{Deserialize, Serialize};
use web3::{
    transports::Http,
    contract::{Contract},
    types::{Address, H160},
};
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Asset {
    pub name: String,
    network_id: String,
    address: String,
    exchange_id: String,
}

impl Asset {
    pub fn address(&self) -> &str {
        self.address.as_str()
    }

    pub fn as_address(&self) -> Result<H160, FromHexError> {
        Address::from_str(self.address())
    }

    pub fn exchange_id(&self) -> &str {
        self.exchange_id.as_str()
    }

    pub fn abi_path(&self) -> String {
        format!(
            "./res/assets/{}/{}/{}/abi.json",
            self.network_id.as_str(),
            self.exchange_id.as_str(),
            self.name.as_str()
        )
    }

    pub fn abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn contract(&self, client: web3::Web3<Http>) -> Contract<Http> {
        let contract_address = self.as_address().unwrap();
        let json_abi = self.abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Assets(pub HashMap<String, Asset>);
impl Assets {
    pub fn get(&self, key: &str) -> &Asset {
        self.0.get(key).unwrap()
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Wallet {
    private_key: String,
}

impl Wallet {
    pub fn to_raw(&self) -> Vec<u8> {
        self.private_key.from_hex().unwrap()
    }
    pub fn private_key(&self) -> String {
        self.private_key.clone()
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
pub struct Network {
    name: String,
    symbol: String,
    chain_id: u32,
    rpc_url: String,
    blockexplorer_url: String,
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

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchange {
    name: String,
    router_address: String,
}

impl Exchange {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchanges(HashMap<String, Exchange>);
impl Exchanges {
    pub fn get(&self, key: &str) -> &Exchange {
        self.0.get(key).unwrap()
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub wallets: Wallets,
    pub assets: Assets,
    pub networks: Networks,
    pub exchanges: Exchanges,
}

impl Config {
    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();
        config
    }
}
