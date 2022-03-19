pub mod asset;
pub mod exchange;
pub mod network;
pub mod rebalancer;
pub mod route;
pub mod wallet;
pub mod withdraw_wallet;
pub mod yield_farm;

use serde::{Deserialize, Serialize};

use asset::Assets;
use exchange::Exchanges;
use network::Networks;
use rebalancer::Rebalancers;
use route::Routes;
use wallet::Wallets;
use withdraw_wallet::WithdrawWallets;
use yield_farm::YieldFarms;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub wallets: Wallets,
    pub withdraw_wallets: WithdrawWallets,
    pub assets: Assets,
    pub networks: Networks,
    pub exchanges: Exchanges,
    pub routes: Routes,
    pub rebalancers: Rebalancers,
    pub yield_farms: YieldFarms,
}

impl Config {
    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();
        log::debug!("from_file(): config: {:?}", config);

        config
    }
}
