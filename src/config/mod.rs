pub mod asset;
pub mod exchange;
pub mod network;
pub mod rebalancer;
pub mod wallet;
pub mod withdraw_wallet;
pub mod yield_farm;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

use asset::Assets;
use exchange::Exchanges;
use network::Networks;
use rebalancer::Rebalancers;
use wallet::Wallets;
use withdraw_wallet::WithdrawWallets;
use yield_farm::YieldFarms;

static GLOBAL_CONFIG: OnceCell<Config> = OnceCell::new();

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub wallets: Wallets,
    pub withdraw_wallets: WithdrawWallets,
    pub assets: Assets,
    pub networks: Networks,
    pub exchanges: Exchanges,
    pub rebalancers: Rebalancers,
    pub yield_farms: YieldFarms,
}

impl Config {
    pub fn global() -> &'static Config {
        GLOBAL_CONFIG.get().expect("CONFIG not loaded")
    }

    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();
        GLOBAL_CONFIG.set(config.clone()).unwrap();
        //TODO: before log, need filter some fields
        //log::debug!("from_file(): config: {:?}", config);

        config
    }
}
