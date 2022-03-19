use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use web3::{
    contract::Contract,
    transports::Http,
    types::{Address, U256},
};

use super::{asset::Asset, network::Network, wallet::Wallet, Config};

pub mod pancake_swap_auto_cake_pool;
pub mod posi_farm_bnb_posi;
pub mod posi_farm_busd_posi;
pub mod position_stake_manager;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldFarm {
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
    pub fn address(&self) -> String {
        self.address.clone()
    }

    pub fn get_asset<'a>(&self, config: &'a Config) -> &'a Asset {
        let asset = config.assets.get(self.asset_id.as_str());
        asset
    }

    pub fn get_min_rewards_required_u256(&self, asset_decimals: u8) -> U256 {
        let q = self.min_rewards_required;
        let qe = (q * 10_f64.powf(asset_decimals.into())) as u128;
        U256::from(qe)
    }

    // pub fn get_quoted_asset<'a>(&self, config: &'a Config) -> &'a Asset {
    //     let quoted_asset = config.assets.get(self.quoted_asset_id.as_str());
    //     quoted_asset
    // }

    pub fn get_wallet<'a>(&self, config: &'a Config) -> &'a Wallet {
        let wallet = config.wallets.get(self.wallet_id.as_str());
        wallet
    }

    pub fn get_network<'a>(&self, config: &'a Config) -> &'a Network {
        let network = config.networks.get(self.network_id.as_str());
        network
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

    pub fn contract(&self, client: web3::Web3<Http>) -> Contract<Http> {
        let contract_address = self.as_address();
        let json_abi = self.abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub async fn get_pending_rewards(&self, config: &Config, client: web3::Web3<Http>) -> U256 {
        match self.operation.as_str() {
            "posi_farm_bnb_posi" => {
                posi_farm_bnb_posi::get_pending_rewards(config, &self, client.clone()).await
            }
            "posi_farm_busd_posi" => {
                posi_farm_busd_posi::get_pending_rewards(config, &self, client.clone()).await
            }
            "cake_auto_pool" => {
                pancake_swap_auto_cake_pool::get_pending_rewards(config, &self, client.clone())
                    .await
            }
            _ => panic!("operation not implemented {:?}", self.operation),
        }
    }

    pub async fn harvest(&self, config: &Config, client: web3::Web3<Http>) {
        match self.operation.as_str() {
            "posi_farm_bnb_posi" => {
                posi_farm_bnb_posi::harvest(config, &self, client.clone()).await
            }
            "posi_farm_busd_posi" => {
                posi_farm_busd_posi::harvest(config, &self, client.clone()).await
            }
            "cake_auto_pool" => {
                pancake_swap_auto_cake_pool::harvest(config, &self, client.clone()).await
            }
            _ => panic!("operation not implemented {:?}", self.operation),
        }
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldFarms(HashMap<String, YieldFarm>);

impl YieldFarms {
    pub fn get(&self, key: &str) -> &YieldFarm {
        self.0.get(key).unwrap()
    }
}
