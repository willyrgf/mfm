use std::collections::HashMap;
use std::str::FromStr;

use rustc_hex::FromHexError;
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

    pub fn build_path_for_coin(&self, coin_address: H160) -> Vec<H160> {
        vec![coin_address, self.as_address().unwrap()]
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

    pub fn find_by_address(&self, address: &str) -> &Asset {
        let asset = match self.0.iter().filter(|(_, a)| a.address() == address).last() {
            Some((_, a)) => a,
            None => panic!("find_by_address() address: {} doesnt exist", address),
        };

        asset
    }
}
