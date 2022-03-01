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
pub struct Asset {
    name: String,
    network_id: String,
    address: String,
    exchange_id: String,
}

impl Asset {
    pub fn name(&self) -> &str {
        &self.name
    }

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

    pub async fn decimals(&self, client: web3::Web3<Http>) -> u8 {
        let contract = self.contract(client);
        let result = contract.query("decimals", (), None, Options::default(), None);
        let result_decimals: u8 = result.await.unwrap();
        result_decimals
    }

    pub async fn balance_of(&self, client: web3::Web3<Http>, account: H160) -> U256 {
        let contract = self.contract(client);
        let result = contract.query("balanceOf", (account,), None, Options::default(), None);
        let result_balance: U256 = result.await.unwrap();
        result_balance
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Assets(HashMap<String, Asset>);
impl Assets {
    pub fn hashmap(&self) -> &HashMap<String, Asset> {
        &self.0
    }

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

    pub fn router_address(&self) -> &str {
        self.router_address.as_str()
    }

    pub fn as_router_address(&self) -> Result<H160, FromHexError> {
        Address::from_str(self.router_address())
    }

    pub fn router_abi_path(&self) -> String {
        format!("./res/exchanges/{}/abi.json", self.name.as_str())
    }

    pub fn router_abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.router_abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn router_contract(&self, client: web3::Web3<Http>) -> Contract<Http> {
        let contract_address = self.as_router_address().unwrap();
        let json_abi = self.router_abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub async fn get_amounts_out(
        &self,
        client: web3::Web3<Http>,
        decimals: u8,
        assets_path: Vec<H160>,
    ) -> Vec<U256> {
        let contract = self.router_contract(client);
        let quantity = 1;
        let amount: U256 = (quantity * 10_i32.pow(decimals.into())).into();
        let result = contract.query(
            "getAmountsOut",
            (amount, assets_path),
            None,
            Options::default(),
            None,
        );
        let result_amounts_out: Vec<U256> = result.await.unwrap();
        result_amounts_out
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
