use crate::config::asset::{Asset, Assets};

use std::str::FromStr;
use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use rustc_hex::FromHexError;
use serde::{Deserialize, Serialize};
use web3::{
    contract::{Contract, Options},
    transports::Http,
    types::{Address, H160, U256},
};

use super::network::{Network, Networks};
use super::wallet::Wallet;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchange {
    name: String,
    router_address: String,
    network_id: String,
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

    pub async fn wrapper_address(&self, client: web3::Web3<Http>) -> H160 {
        let router_contract = self.router_contract(client);

        let wrapped_addr = router_contract
            .query("WETH", (), None, Options::default(), None)
            .await
            .unwrap();

        wrapped_addr
    }

    pub async fn wrapped_asset<'a>(
        &self,
        assets: &'a Assets,
        client: web3::Web3<Http>,
    ) -> &'a Asset {
        let wrapped_address = self.wrapper_address(client).await.to_string();
        let wrapped_asset = assets.find_by_address(wrapped_address.as_str());
        wrapped_asset
    }

    pub async fn get_amounts_out(
        &self,
        client: web3::Web3<Http>,
        // decimals: u8,
        amount: U256,
        assets_path: Vec<H160>,
    ) -> Vec<U256> {
        let contract = self.router_contract(client);
        // let quantity = 1;
        // let amount: U256 = (quantity * 10_i32.pow(decimals.into())).into();
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

    fn get_valid_timestamp(&self, future_millis: u128) -> u128 {
        let start = SystemTime::now();
        let since_epoch = start.duration_since(UNIX_EPOCH).unwrap();
        let time_millis = since_epoch.as_millis().checked_add(future_millis).unwrap();

        time_millis
    }

    pub fn get_network<'a>(&self, networks: &'a Networks) -> &'a Network {
        let network = networks.get(self.network_id.as_str());
        network
    }

    pub async fn swap_tokens_for_tokens(
        &self,
        client: web3::Web3<Http>,
        wallet: &Wallet,
        gas_price: U256,
        amount_in: U256,
        amount_min_out: U256,
        asset_path: Vec<H160>,
    ) {
        let valid_timestamp = self.get_valid_timestamp(300000);
        let estimate_gas = self
            .router_contract(client.clone())
            .estimate_gas(
                "swapExactTokensForTokensSupportingFeeOnTransferTokens",
                (
                    amount_in,
                    amount_min_out,
                    asset_path,
                    wallet.address(),
                    U256::from_dec_str(&valid_timestamp.to_string()).unwrap(),
                ),
                wallet.address(),
                web3::contract::Options {
                    //value: Some(amount_in),
                    gas_price: Some(gas_price),
                    // gas: Some(500_000.into()),
                    // gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        println!("swap_tokens_for_tokens: {}", estimate_gas);
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchanges(HashMap<String, Exchange>);
impl Exchanges {
    pub fn get(&self, key: &str) -> &Exchange {
        self.0.get(key).unwrap()
    }
}
