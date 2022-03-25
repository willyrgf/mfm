use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use web3::{
    contract::Contract,
    transports::Http,
    types::{Address, U256},
    Web3,
};

use super::{asset::Asset, network::Network, wallet::Wallet, Config};

pub mod pacoca_auto_pool;
pub mod pacoca_vault;
pub mod pancake_swap_auto_cake_pool;
pub mod posi_farm_bnb_posi;
pub mod posi_farm_busd_posi;
pub mod position_stake_manager;
pub mod qi_dao_staking_pool;
pub mod qi_dao_staking_pool_qi_wmatic;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldFarm {
    name: String,
    contract_name: String,
    wallet_id: String,
    address: String,
    operation: String,
    network_id: String,
    asset_id: String,
    min_rewards_required: f64,
    //quoted_asset_id: String,
}

impl YieldFarm {
    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn network_id(&self) -> &str {
        self.network_id.as_str()
    }

    pub fn address(&self) -> String {
        self.address.clone()
    }

    pub fn get_asset<'a>(&self) -> &'a Asset {
        match Config::global()
            .assets
            .find_by_name_and_network(self.asset_id.as_str(), self.network_id.as_str())
        {
            Some(a) => a,
            _ => panic!("can't find asset"),
        }
    }

    pub fn get_min_rewards_required_u256(&self, asset_decimals: u8) -> U256 {
        let q = self.min_rewards_required;
        let qe = (q * 10_f64.powf(asset_decimals.into())) as u128;
        U256::from(qe)
    }

    pub fn get_wallet<'a>(&self) -> &'a Wallet {
        Config::global().wallets.get(self.wallet_id.as_str())
    }

    pub fn get_network<'a>(&self) -> &'a Network {
        Config::global().networks.get(self.network_id.as_str())
    }

    pub fn get_web3_client_http(&self) -> Web3<Http> {
        self.get_network().get_web3_client_http()
    }

    pub fn as_address(&self) -> Address {
        Address::from_str(self.address().as_str()).unwrap()
    }

    pub fn abi_path(&self) -> String {
        format!("./res/yield_farms/{}/abi.json", self.contract_name.as_str())
    }

    pub fn abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn contract(&self) -> Contract<Http> {
        let client = self.get_web3_client_http();
        let contract_address = self.as_address();
        let json_abi = self.abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub async fn get_pending_rewards(&self) -> U256 {
        match self.operation.as_str() {
            "posi_farm_bnb_posi" => posi_farm_bnb_posi::get_pending_rewards(self).await,
            "posi_farm_busd_posi" => posi_farm_busd_posi::get_pending_rewards(self).await,
            "cake_auto_pool" => pancake_swap_auto_cake_pool::get_pending_rewards(self).await,
            "pacoca_auto_pool" => pacoca_auto_pool::get_pending_rewards(self).await,
            "qi_dao_staking_pool_qi_wmatic" => {
                qi_dao_staking_pool_qi_wmatic::get_pending_rewards(self).await
            }
            _ => panic!("operation not implemented {:?}", self.operation),
        }
    }

    pub async fn harvest(&self) {
        match self.operation.as_str() {
            "posi_farm_bnb_posi" => posi_farm_bnb_posi::harvest(self).await,
            "posi_farm_busd_posi" => posi_farm_busd_posi::harvest(self).await,
            "cake_auto_pool" => pancake_swap_auto_cake_pool::harvest(self).await,
            "pacoca_auto_pool" => pacoca_auto_pool::harvest(self).await,
            _ => panic!("operation not implemented {:?}", self.operation),
        }
    }

    pub async fn deposit(&self, amount: U256) {
        match self.operation.as_str() {
            "cake_auto_pool" => pancake_swap_auto_cake_pool::deposit(self, amount).await,
            "pacoca_auto_pool" => pacoca_auto_pool::deposit(self, amount).await,
            _ => panic!("operation not implemented {:?}", self.operation),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldFarms(HashMap<String, YieldFarm>);

impl YieldFarms {
    pub fn hashmap(&self) -> &HashMap<String, YieldFarm> {
        &self.0
    }
    pub fn get(&self, key: &str) -> &YieldFarm {
        self.0.get(key).unwrap()
    }
}
