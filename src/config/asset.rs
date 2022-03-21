use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use rustc_hex::FromHexError;
use serde::{Deserialize, Serialize};
use web3::ethabi::Token;
use web3::Web3;
use web3::{
    contract::{Contract, Options},
    transports::Http,
    types::{Address, Bytes, H160, U256},
};

use crate::shared;

use super::exchange::Exchange;
use super::network::{Network, Networks};
use super::wallet::Wallet;
use super::withdraw_wallet::WithdrawWallet;
use super::Config;
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Asset {
    name: String,
    network_id: String,
    address: String,
    exchange_id: String,
    slippage: f64,
}

impl Asset {
    pub fn slippage_u256(&self, asset_decimals: u8) -> U256 {
        //TODO: review u128
        let qe = ((self.slippage / 100.0) * 10_f64.powf(asset_decimals.into())) as u128;
        U256::from(qe)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn address(&self) -> &str {
        self.address.as_str()
    }

    pub fn as_address(&self) -> Result<H160, FromHexError> {
        Address::from_str(self.address())
    }

    pub fn network_id(&self) -> &str {
        self.network_id.as_str()
    }

    pub fn exchange_id(&self) -> &str {
        self.exchange_id.as_str()
    }

    pub fn get_exchange<'a>(&self, config: &'a Config) -> &'a Exchange {
        config.exchanges.get(self.exchange_id())
    }

    pub fn abi_path(&self) -> String {
        let path = format!(
            "./res/assets/{}/{}/{}/abi.json",
            self.network_id.as_str(),
            self.exchange_id.as_str(),
            self.name.as_str()
        );
        let fallback_path = format!("./res/assets/erc20_abi.json");
        if Path::new(&path).exists() {
            return path;
        }
        fallback_path
    }

    pub fn abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn get_network<'a>(&self, networks: &'a Networks) -> &'a Network {
        networks.get(self.network_id.as_str())
    }

    pub fn get_web3_client_http(&self, config: &Config) -> Web3<Http> {
        self.get_network(&config.networks).get_web3_client_http()
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

    pub async fn balance_of_quoted_in(
        &self,
        client: web3::Web3<Http>,
        config: &Config,
        wallet: &Wallet,
        quoted: &Asset,
    ) -> U256 {
        let account = wallet.address();
        let exchange = self.get_exchange(config);
        let base_balance = self.balance_of(client.clone(), account).await;

        if self.name() == quoted.name() {
            return base_balance;
        }

        let assets_path = exchange
            .build_route_for(config, client.clone(), &self, quoted)
            .await;

        exchange
            .get_amounts_out(client.clone(), base_balance, assets_path)
            .await
            .last()
            .unwrap()
            .into()
    }

    pub async fn allowance(&self, client: web3::Web3<Http>, owner: H160, spender: H160) -> U256 {
        let result: U256 = self
            .contract(client.clone())
            .query(
                "allowance",
                (owner, spender),
                None,
                Options::default(),
                None,
            )
            .await
            .unwrap();
        result
    }

    pub async fn approve_spender(
        &self,
        client: web3::Web3<Http>,
        gas_price: U256,
        from_wallet: &Wallet,
        spender: H160,
        amount: U256,
    ) {
        let estimate_gas = shared::blockchain_utils::estimate_gas(
            &self.contract(client.clone()),
            from_wallet,
            "approve",
            (spender, amount),
            web3::contract::Options {
                gas_price: Some(gas_price),
                ..Default::default()
            },
        )
        .await;

        log::debug!("approve_spender called estimate_gas: {:?}", estimate_gas);

        let func_data = shared::blockchain_utils::generate_func_data(
            &self.contract(client.clone()),
            "approve",
            &[Token::Address(spender), Token::Uint(amount)],
        );
        log::debug!("approve_spender(): func_data: {:?}", func_data);

        let nonce = from_wallet.nonce(client.clone()).await;
        log::debug!("approve_spender(): nonce: {:?}", nonce);

        let transaction_obj = shared::blockchain_utils::build_transaction_params(
            nonce,
            self.as_address().unwrap(),
            U256::from(0_i32),
            gas_price,
            estimate_gas,
            Bytes(func_data),
        );
        log::debug!("approve_spender(): transaction_obj: {:?}", transaction_obj);

        shared::blockchain_utils::sign_send_and_wait_txn(
            client.clone(),
            transaction_obj,
            from_wallet,
        )
        .await;
    }

    pub async fn wrap(
        &self,
        client: web3::Web3<Http>,
        from_wallet: &Wallet,
        amount: U256,
        gas_price: U256,
    ) {
        let estimate_gas = shared::blockchain_utils::estimate_gas(
            &self.contract(client.clone()),
            from_wallet,
            "deposit",
            (),
            web3::contract::Options {
                value: Some(amount),
                gas_price: Some(gas_price),
                ..Default::default()
            },
        )
        .await;
        log::debug!("wrap called estimate_gas: {:?}", estimate_gas);

        let func_data = shared::blockchain_utils::generate_func_data(
            &self.contract(client.clone()),
            "deposit",
            &[],
        );
        log::debug!("wrap(): deposit_data: {:?}", func_data);

        let nonce = from_wallet.nonce(client.clone()).await;
        log::debug!("wrap(): nonce: {:?}", nonce);

        let transaction_obj = shared::blockchain_utils::build_transaction_params(
            nonce,
            self.as_address().unwrap(),
            amount,
            gas_price,
            estimate_gas,
            Bytes(func_data),
        );
        log::debug!("wrap(): transaction_obj: {:?}", transaction_obj);

        shared::blockchain_utils::sign_send_and_wait_txn(
            client.clone(),
            transaction_obj,
            from_wallet,
        )
        .await;
    }

    pub async fn withdraw(
        &self,
        client: web3::Web3<Http>,
        wallet: &Wallet,
        withdraw_wallet: &WithdrawWallet,
        amount: U256,
        gas_price: U256,
    ) {
        let estimate_gas = shared::blockchain_utils::estimate_gas(
            &self.contract(client.clone()),
            wallet,
            "transfer",
            (withdraw_wallet.as_address(), amount),
            web3::contract::Options {
                gas_price: Some(gas_price),
                ..Default::default()
            },
        )
        .await;
        log::debug!("withdraw called estimate_gas: {:?}", estimate_gas);

        let func_data = shared::blockchain_utils::generate_func_data(
            &self.contract(client.clone()),
            "transfer",
            &[
                Token::Address(withdraw_wallet.as_address()),
                Token::Uint(amount),
            ],
        );
        log::debug!("withdraw(): func_data: {:?}", func_data);

        let nonce = wallet.nonce(client.clone()).await;
        log::debug!("withdraw(): nonce: {:?}", nonce);

        let transaction_obj = shared::blockchain_utils::build_transaction_params(
            nonce,
            self.as_address().unwrap(),
            U256::from(0_i32),
            gas_price,
            estimate_gas,
            Bytes(func_data),
        );
        log::debug!("withdraw(): transaction_obj: {:?}", transaction_obj);

        shared::blockchain_utils::sign_send_and_wait_txn(client.clone(), transaction_obj, wallet)
            .await;
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Assets(HashMap<String, Asset>);
impl Assets {
    pub fn hashmap(&self) -> &HashMap<String, Asset> {
        &self.0
    }

    pub fn get(&self, key: &str) -> &Asset {
        self.0.get(key).unwrap()
    }

    //TODO: use this function to get assets of the current network
    pub fn find_by_name_and_network(&self, name: &str, network: &str) -> Option<&Asset> {
        let result = self
            .hashmap()
            .iter()
            .filter(|(_, a)| a.name == name && a.network_id == network);

        if result.clone().count() > 1 {
            log::error!("Same asset multiples times for a network");
            return None;
        }

        let r = match result.last() {
            Some((_, a)) => Some(a),
            _ => None,
        };

        r
    }

    pub fn find_by_address(&self, address: &str) -> &Asset {
        let asset = match self.0.iter().filter(|(_, a)| a.address() == address).last() {
            Some((_, a)) => a,
            None => panic!("find_by_address() address: {} doesnt exist", address),
        };

        asset
    }
}
