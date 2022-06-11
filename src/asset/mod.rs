pub mod config;

use std::path::Path;
use std::str::FromStr;

use rustc_hex::FromHexError;
use web3::ethabi::Token;
use web3::Web3;
use web3::{
    contract::{Contract, Options},
    transports::Http,
    types::{Address, Bytes, H160, U256},
};

use crate::shared;

use crate::config::{
    exchange::Exchange, network::Network, wallet::Wallet, withdraw_wallet::WithdrawWallet, Config,
};
use config::AssetConfig;


//TODO: where put it???
include!(concat!(env!("OUT_DIR"), "/res.rs"));

#[derive(Debug, Clone)]
pub struct Asset {
    name: String,
    kind: String,
    network_id: String,
    address: String,
    exchange_id: String,
    slippage: f64,
    path_asset: String,
    network: Network,
}

impl Asset {
    pub fn new(asset_config: &AssetConfig, n: &Network) -> Self {
        let asset_network = asset_config.networks.get(n.get_name());
        let network: Network = (*n).clone();
        Asset {
            name: asset_network.name.clone(),
            kind: asset_config.kind.clone(),
            network_id: asset_network.network_id.clone(),
            address: asset_network.address.clone(),
            exchange_id: asset_network.exchange_id.clone(),
            slippage: asset_network.slippage,
            path_asset: asset_network.path_asset.clone(),
            network,
        }
    }

    pub fn slippage_u256(&self, asset_decimals: u8) -> U256 {
        //TODO: review u128
        let qe = ((&self.slippage / 100.0) * 10_f64.powf(asset_decimals.into())) as u128;
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

    pub fn get_path_asset(&self) -> Asset {
        Config::global()
            .assets
            .find_by_name_and_network(self.path_asset.as_str(), self.network_id.as_str())
            .unwrap()
    }

    pub fn get_exchange<'a>(&self) -> &'a Exchange {
        Config::global().exchanges.get(self.exchange_id())
    }

    pub fn abi_path(&self) -> String {
        let path = format!(
            "res/assets/{}/{}/{}/abi.json",
            self.network_id.as_str(),
            self.exchange_id.as_str(),
            self.name.as_str()
        );
        // TODO: move it to const static
        let fallback_path = "res/assets/erc20_abi.json".to_string();
        if Path::new(&path).exists() {
            return path;
        }
        fallback_path
    }

    pub fn abi_json_string(&self) -> String {
        let file_string = RES.get(&self.abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_str(file_string).unwrap();
        json.to_string()
    }

    pub fn get_network(&self) -> &Network {
        &self.network
    }

    pub fn get_web3_client_http(&self) -> Web3<Http> {
        self.get_network().get_web3_client_http()
    }

    pub fn contract(&self) -> Contract<Http> {
        let client = self.get_web3_client_http();
        let contract_address = self.as_address().unwrap();
        let json_abi = self.abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub async fn decimals(&self) -> u8 {
        let contract = &self.contract();
        let result = contract.query("decimals", (), None, Options::default(), None);
        let result_decimals: u8 = result.await.unwrap();
        result_decimals
    }

    pub async fn balance_of(&self, account: H160) -> U256 {
        let contract = &self.contract();
        let result = contract.query("balanceOf", (account,), None, Options::default(), None);
        let result_balance: U256 = result.await.unwrap();
        result_balance
    }

    pub fn build_path_for_coin(&self, coin_address: H160) -> Vec<H160> {
        vec![coin_address, self.as_address().unwrap()]
    }

    pub async fn balance_of_quoted_in(&self, wallet: &Wallet, quoted: &Asset) -> U256 {
        let account = wallet.address();
        let exchange = self.get_exchange();
        let base_balance = self.balance_of(account).await;

        if self.name() == quoted.name() {
            return base_balance;
        }

        let assets_path = exchange.build_route_for(self, quoted).await;

        exchange
            .get_amounts_out(base_balance, assets_path)
            .await
            .last()
            .unwrap()
            .into()
    }

    pub fn exist_max_tx_amount(&self) -> bool {
        self.contract().abi().function("_maxTxAmount").is_ok()
    }

    pub async fn max_tx_amount(&self) -> Option<U256> {
        if !self.exist_max_tx_amount() {
            return None;
        }

        match self
            .contract()
            .query("_maxTxAmount", (), None, Options::default(), None)
            .await
        {
            Ok(m) => Some(m),
            _ => None,
        }
    }

    pub async fn allowance(&self, owner: H160, spender: H160) -> U256 {
        let result: U256 = self
            .contract()
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

    pub async fn approve_spender(&self, from_wallet: &Wallet, spender: H160, amount: U256) {
        let client = self.get_web3_client_http();
        let gas_price = client.eth().gas_price().await.unwrap();

        let estimate_gas = shared::blockchain_utils::estimate_gas(
            &self.contract(),
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
            &self.contract(),
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

    pub async fn unwrap(&self, from_wallet: &Wallet, amount: U256) {
        let client = self.get_web3_client_http();
        let gas_price = client.eth().gas_price().await.unwrap();

        let estimate_gas = shared::blockchain_utils::estimate_gas(
            &self.contract(),
            from_wallet,
            "withdraw",
            (amount,),
            web3::contract::Options {
                gas_price: Some(gas_price),
                ..Default::default()
            },
        )
        .await;
        log::debug!("unwrap called estimate_gas: {:?}", estimate_gas);

        let func_data = shared::blockchain_utils::generate_func_data(
            &self.contract(),
            "withdraw",
            &[Token::Uint(amount)],
        );
        log::debug!("unwrap(): deposit_data: {:?}", func_data);

        let nonce = from_wallet.nonce(client.clone()).await;
        log::debug!("unwrap(): nonce: {:?}", nonce);

        let transaction_obj = shared::blockchain_utils::build_transaction_params(
            nonce,
            self.as_address().unwrap(),
            U256::from(0_i32),
            gas_price,
            estimate_gas,
            Bytes(func_data),
        );
        log::debug!("unwrap(): transaction_obj: {:?}", transaction_obj);

        shared::blockchain_utils::sign_send_and_wait_txn(
            client.clone(),
            transaction_obj,
            from_wallet,
        )
        .await;
    }

    pub async fn wrap(&self, from_wallet: &Wallet, amount: U256) {
        let client = self.get_web3_client_http();
        let gas_price = client.eth().gas_price().await.unwrap();

        let estimate_gas = shared::blockchain_utils::estimate_gas(
            &self.contract(),
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

        let func_data =
            shared::blockchain_utils::generate_func_data(&self.contract(), "deposit", &[]);
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

    pub async fn withdraw(&self, wallet: &Wallet, withdraw_wallet: &WithdrawWallet, amount: U256) {
        let client = self.get_web3_client_http();
        let gas_price = client.eth().gas_price().await.unwrap();

        let estimate_gas = shared::blockchain_utils::estimate_gas(
            &self.contract(),
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
            &self.contract(),
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
