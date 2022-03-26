use crate::asset::Asset;
// use crate::config::asset::{Asset, Assets};

use std::path::Path;
use std::str::FromStr;
use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use rustc_hex::FromHexError;
use serde::{Deserialize, Serialize};
use web3::Web3;
use web3::{
    contract::{Contract, Options},
    ethabi::Token,
    transports::Http,
    types::{Address, H160, U256},
};

use super::network::Network;
use super::wallet::Wallet;
use super::Config;

pub mod swap_eth_for_tokens;
pub mod swap_tokens_for_tokens;

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

//TODO: validate the fields in the new mod initialization
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchange {
    name: String,
    router_address: String,
    factory_address: String,
    network_id: String,
}

impl Exchange {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn network_id(&self) -> &str {
        self.network_id.as_str()
    }

    pub fn router_address(&self) -> &str {
        self.router_address.as_str()
    }

    pub fn factory_address(&self) -> &str {
        self.router_address.as_str()
    }

    pub fn as_router_address(&self) -> Result<H160, FromHexError> {
        Address::from_str(self.factory_address())
    }

    pub fn as_factory_address(&self) -> H160 {
        Address::from_str(self.factory_address()).unwrap()
    }

    pub fn router_abi_path(&self) -> String {
        let path = format!("./res/exchanges/{}/abi.json", self.name.as_str());
        // TODO: move it to const static
        let fallback_path = "./res/exchanges/uniswap_v2_router_abi.json".to_string();
        if Path::new(&path).exists() {
            return path;
        }
        fallback_path
    }

    pub fn factory_abi_path(&self) -> String {
        let path = format!("./res/exchanges/{}/factory_abi.json", self.name.as_str());
        // TODO: move it to const static
        let fallback_path = "./res/exchanges/uniswap_v2_factory_abi.json".to_string();
        if Path::new(&path).exists() {
            return path;
        }
        fallback_path
    }

    pub fn router_abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.router_abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn factory_abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.factory_abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn router_contract(&self) -> Contract<Http> {
        let client = self.get_web3_client_http();
        let contract_address = self.as_router_address().unwrap();
        let json_abi = self.router_abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub fn factory_contract(&self) -> Contract<Http> {
        let client = self.get_web3_client_http();
        let contract_address = self.as_factory_address();
        let json_abi = self.factory_abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub async fn wrapper_address(&self) -> H160 {
        let router_contract = self.router_contract();

        let wrapped_addr = router_contract
            .query("WETH", (), None, Options::default(), None)
            .await
            .unwrap();

        wrapped_addr
    }

    pub async fn get_factory_pair(
        &self,
        input_asset: &Asset,
        output_asset: &Asset,
    ) -> Option<Address> {
        let contract = self.factory_contract();

        let result = contract.query(
            "getPair",
            (
                input_asset.as_address().unwrap(),
                output_asset.as_address().unwrap(),
            ),
            None,
            Options::default(),
            None,
        );

        let address = match result.await {
            Ok(a) => Some(a),
            _ => None,
        };

        address
    }

    pub async fn build_route_for(&self, input_asset: &Asset, output_asset: &Asset) -> Vec<H160> {
        // Example to transform this result into tokens
        // Vec<Token> = paths
        //         //     .into_iter()
        //         //     .map(|p| Token::Address(p))
        //         //     .collect::<Vec<_>>();
        let mut v = vec![];
        let network = self.get_network();
        let wrapped_asset = network.get_wrapped_asset();
        let wrapped_is_output = wrapped_asset.address() == output_asset.address();
        let wrapped_is_input = wrapped_asset.address() == input_asset.address();
        let has_direct_route = match self.get_factory_pair(input_asset, output_asset).await {
            Some(a) => (a.to_string().as_str() != ZERO_ADDRESS),
            _ => false,
        };

        v.push(input_asset.as_address().unwrap());
        if !has_direct_route && !wrapped_is_output && !wrapped_is_input {
            v.push(wrapped_asset.as_address().unwrap());
        }
        v.push(output_asset.as_address().unwrap());

        v
    }

    pub async fn get_amounts_out(
        &self,
        // decimals: u8,
        amount: U256,
        assets_path: Vec<H160>,
    ) -> Vec<U256> {
        let zero = U256::from(0_i32);

        //TODO: check if the amount is sufficient
        if amount == zero {
            return vec![zero];
        }

        let contract = self.router_contract();
        // let quantity = 1;
        // let amount: U256 = (quantity * 10_i32.pow(decimals.into())).into();
        let result = contract.query(
            "getAmountsOut",
            (amount, assets_path),
            None,
            Options::default(),
            None,
        );
        let result_amounts_out: Vec<U256> = match result.await {
            Ok(a) => a,
            Err(e) => {
                log::error!(
                    "get_amounts_out(): result err: {:?}, return zeroed value",
                    e
                );
                vec![zero]
            }
        };
        result_amounts_out
    }

    fn get_valid_timestamp(&self, future_millis: u128) -> u128 {
        let start = SystemTime::now();
        let since_epoch = start.duration_since(UNIX_EPOCH).unwrap();
        since_epoch.as_millis().checked_add(future_millis).unwrap()
    }

    pub fn get_network<'a>(&self) -> &'a Network {
        Config::global()
            .networks
            .get(self.network_id.as_str())
            .unwrap()
    }

    pub fn get_web3_client_http(&self) -> Web3<Http> {
        self.get_network().get_web3_client_http()
    }

    // pub async fn swap_eth_for_tokens(
    //     &self,
    //     from_wallet: &Wallet,
    //     amount_in: U256,
    //     amount_min_out: U256,
    //     asset: &Asset,
    // ) {
    //     let asset_path = vec![asset.as_address().unwrap()]
    //         .clone()
    //         .into_iter()
    //         .map(|p| Token::Address(p))
    //         .collect::<Vec<_>>();

    //     swap_eth_for_tokens::swap(
    //         &self,
    //         from_wallet,
    //         amount_in,
    //         amount_min_out,
    //         Token::Array(asset_path),
    //     )
    //     .await
    // }

    pub async fn swap_tokens_for_tokens(
        &self,
        from_wallet: &Wallet,
        amount_in: U256,
        amount_min_out: U256,
        asset_path: Token,
    ) {
        swap_tokens_for_tokens::swap(self, from_wallet, amount_in, amount_min_out, asset_path).await
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchanges(HashMap<String, Exchange>);
impl Exchanges {
    pub fn get(&self, key: &str) -> &Exchange {
        match self.0.get(key) {
            Some(e) => e,
            None => {
                log::error!("get(): key {} doesnt exist", key);
                panic!();
            }
        }
    }
}
