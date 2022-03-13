use std::collections::HashMap;
use std::str::FromStr;

use rustc_hex::FromHexError;
use serde::{Deserialize, Serialize};
use web3::ethabi::Token;
use web3::{
    contract::{Contract, Options},
    transports::Http,
    types::{Address, Bytes, TransactionParameters, H160, U256},
};

use super::exchange::Exchange;
use super::network::{Network, Networks};
use super::wallet::Wallet;
use super::Config;
#[derive(Debug, PartialEq, Deserialize, Serialize)]
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

    pub fn exchange_id(&self) -> &str {
        self.exchange_id.as_str()
    }

    pub fn get_exchange<'a>(&self, config: &'a Config) -> &'a Exchange {
        config.exchanges.get(self.exchange_id())
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

    pub fn get_network<'a>(&self, networks: &'a Networks) -> &'a Network {
        networks.get(self.network_id.as_str())
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

        let route = config.routes.search(self, quoted);
        let assets_path = route.build_path(&config.assets);

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
        let estimate_gas = self
            .contract(client.clone())
            .estimate_gas(
                "approve",
                (spender, amount),
                from_wallet.address(),
                web3::contract::Options {
                    //value: Some(amount),
                    gas_price: Some(gas_price),
                    // gas: Some(500_000.into()),
                    // gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        log::debug!("approve_spender called estimate_gas: {:?}", estimate_gas);

        let func_data = self
            .contract(client.clone())
            .abi()
            .function("approve")
            .unwrap()
            .encode_input(&[Token::Address(spender), Token::Uint(amount)])
            .unwrap();
        log::debug!("approve_spender(): func_data: {:?}", func_data);

        let nonce = from_wallet.nonce(client.clone()).await;
        log::debug!("approve_spender(): nonce: {:?}", nonce);

        let transaction_obj = TransactionParameters {
            nonce: Some(nonce),
            to: Some(self.as_address().unwrap()),
            value: U256::from(0_i32),
            gas_price: Some(gas_price),
            gas: estimate_gas,
            data: Bytes(func_data),
            ..Default::default()
        };
        log::debug!("approve_spender(): transaction_obj: {:?}", transaction_obj);

        let secret = from_wallet.secret();
        let signed_transaction = client
            .accounts()
            .sign_transaction(transaction_obj, &secret)
            .await
            .unwrap();
        log::debug!(
            "approve_spender(): signed_transaction: {:?}",
            signed_transaction
        );

        let tx_address = client
            .eth()
            .send_raw_transaction(signed_transaction.raw_transaction)
            .await
            .unwrap();
        log::debug!("approve_spender(): tx_adress: {}", tx_address);
    }

    pub async fn wrap(
        &self,
        client: web3::Web3<Http>,
        from_wallet: &Wallet,
        amount: U256,
        gas_price: U256,
    ) {
        let estimate_gas = self
            .contract(client.clone())
            .estimate_gas(
                "deposit",
                (),
                from_wallet.address(),
                web3::contract::Options {
                    value: Some(amount),
                    gas_price: Some(gas_price),
                    // gas: Some(500_000.into()),
                    // gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        log::debug!("wrap called estimate_gas: {:?}", estimate_gas);

        let deposit_data = self
            .contract(client.clone())
            .abi()
            .function("deposit")
            .unwrap()
            .encode_input(&[])
            .unwrap();
        log::debug!("wrap(): deposit_data: {:?}", deposit_data);

        let nonce = from_wallet.nonce(client.clone()).await;
        log::debug!("wrap(): nonce: {:?}", nonce);

        let transaction_obj = TransactionParameters {
            nonce: Some(nonce),
            to: Some(self.as_address().unwrap()),
            value: amount,
            gas_price: Some(gas_price),
            gas: estimate_gas,
            data: Bytes(deposit_data),
            ..Default::default()
        };
        log::debug!("wrap(): transaction_obj: {:?}", transaction_obj);

        let secret = from_wallet.secret();
        let signed_transaction = client
            .accounts()
            .sign_transaction(transaction_obj, &secret)
            .await
            .unwrap();
        log::debug!("wrap(): signed_transaction: {:?}", signed_transaction);

        let tx_address = client
            .eth()
            .send_raw_transaction(signed_transaction.raw_transaction)
            .await
            .unwrap();
        log::debug!("wrap(): tx_adress: {}", tx_address);
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
