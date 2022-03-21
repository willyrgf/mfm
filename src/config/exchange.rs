use crate::config::asset::{Asset, Assets};
use crate::shared;

use std::path::Path;
use std::str::FromStr;
use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use rustc_hex::FromHexError;
use serde::{Deserialize, Serialize};
use web3::{
    contract::{Contract, Options},
    ethabi::Token,
    transports::Http,
    types::{Address, Bytes, TransactionParameters, H160, U256},
};

use super::network::{Network, Networks};
use super::wallet::Wallet;
use super::Config;

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
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
        let fallback_path = format!("./res/exchanges/uniswap_v2_router_abi.json");
        if Path::new(&path).exists() {
            return path;
        }
        fallback_path
    }

    pub fn factory_abi_path(&self) -> String {
        let path = format!("./res/exchanges/{}/factory_abi.json", self.name.as_str());
        let fallback_path = format!("./res/exchanges/uniswap_v2_factory_abi.json");
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

    pub fn router_contract(&self, client: web3::Web3<Http>) -> Contract<Http> {
        let contract_address = self.as_router_address().unwrap();
        let json_abi = self.router_abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub fn factory_contract(&self, client: web3::Web3<Http>) -> Contract<Http> {
        let contract_address = self.as_factory_address();
        let json_abi = self.factory_abi_json_string();
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
        let wrapped_address = self.wrapper_address(client).await;
        log::debug!("wrapped_asset(): {:?}", wrapped_address);
        let wrapped_asset = assets.find_by_address(wrapped_address.to_string().as_str());
        wrapped_asset
    }

    pub async fn get_factory_pair(
        &self,
        client: web3::Web3<Http>,
        input_asset: &Asset,
        output_asset: &Asset,
    ) -> Option<Address> {
        let contract = self.factory_contract(client.clone());

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

    pub async fn build_route_for(
        &self,
        config: &Config,
        client: web3::Web3<Http>,
        input_asset: &Asset,
        output_asset: &Asset,
    ) -> Vec<H160> {
        // Example to transform this result into tokens
        // Vec<Token> = paths
        //         //     .into_iter()
        //         //     .map(|p| Token::Address(p))
        //         //     .collect::<Vec<_>>();
        let mut v = vec![];
        let network = self.get_network(&config.networks);
        let wrapped_asset = network.get_wrapped_asset(&config.assets);
        let wrapped_is_output = wrapped_asset.address() == output_asset.address();
        let has_direct_route = match self
            .get_factory_pair(client.clone(), input_asset, output_asset)
            .await
        {
            Some(a) => (a.to_string().as_str() != ZERO_ADDRESS),
            _ => false,
        };

        v.push(input_asset.as_address().unwrap());
        if !has_direct_route && !wrapped_is_output {
            v.push(wrapped_asset.as_address().unwrap());
        }
        v.push(output_asset.as_address().unwrap());

        v
    }

    pub async fn get_amounts_out(
        &self,
        client: web3::Web3<Http>,
        // decimals: u8,
        amount: U256,
        assets_path: Vec<H160>,
    ) -> Vec<U256> {
        let zero = U256::from(0);

        //TODO: check if the amount is sufficient
        if amount == zero {
            return vec![zero];
        }

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

    pub fn get_network<'a>(&self, networks: &'a Networks) -> &'a Network {
        let network = networks.get(self.network_id.as_str());
        network
    }

    pub async fn swap_tokens_for_tokens(
        &self,
        client: web3::Web3<Http>,
        from_wallet: &Wallet,
        gas_price: U256,
        amount_in: U256,
        amount_min_out: U256,
        asset_path: Token,
    ) {
        let valid_timestamp = self.get_valid_timestamp(30000000);
        let estimate_gas = self
            .router_contract(client.clone())
            .estimate_gas(
                "swapExactTokensForTokensSupportingFeeOnTransferTokens",
                // "swapExactTokensForTokens",
                (
                    amount_in,
                    amount_min_out,
                    asset_path.clone(),
                    from_wallet.address(),
                    U256::from_dec_str(&valid_timestamp.to_string()).unwrap(),
                ),
                from_wallet.address(),
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

        log::debug!("swap_tokens_for_tokens estimate_gas: {}", estimate_gas);

        let func_data = self
            .router_contract(client.clone())
            .abi()
            .function("swapExactTokensForTokensSupportingFeeOnTransferTokens")
            // .function("swapExactTokensForTokens")
            .unwrap()
            .encode_input(&[
                Token::Uint(amount_in),
                Token::Uint(amount_min_out),
                asset_path,
                Token::Address(from_wallet.address()),
                Token::Uint(U256::from_dec_str(&valid_timestamp.to_string()).unwrap()),
            ])
            .unwrap();
        log::debug!("swap_tokens_for_tokens(): func_data: {:?}", func_data);

        let nonce = from_wallet.nonce(client.clone()).await;
        log::debug!("swap_tokens_for_tokens(): nonce: {:?}", nonce);

        let estimate_with_margin =
            (estimate_gas * (U256::from(10000_i32) + U256::from(1000_i32))) / U256::from(10000_i32);
        let transaction_obj = TransactionParameters {
            nonce: Some(nonce),
            to: Some(self.as_router_address().unwrap()),
            value: U256::from(0_i32),
            gas_price: Some(gas_price),
            gas: estimate_with_margin,
            data: Bytes(func_data),
            ..Default::default()
        };
        log::debug!(
            "swap_tokens_for_tokens(): transaction_obj: {:?}",
            transaction_obj
        );

        shared::blockchain_utils::sign_send_and_wait_txn(
            client.clone(),
            transaction_obj,
            from_wallet,
        )
        .await;
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
