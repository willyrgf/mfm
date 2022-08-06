// pub mod asset;
mod decrypt_wallet;
pub mod exchange;
pub mod network;
// pub mod rebalancer;
pub mod wallet;
pub mod withdraw_wallet;
pub mod yield_farm;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

use crate::{asset::config::AssetsConfig, rebalancer::config::RebalancersConfig};
use exchange::Exchanges;
use network::Networks;
use wallet::Wallets;
use withdraw_wallet::WithdrawWallets;
use yield_farm::YieldFarms;

use self::decrypt_wallet::decrypt_wallets_from_config;

static GLOBAL_CONFIG: OnceCell<Config> = OnceCell::new();

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Server {
    pub api_url: String,
    pub api_token: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub server: Option<Server>,
    pub wallets: Wallets,
    pub withdraw_wallets: Option<WithdrawWallets>,
    pub networks: Networks,
    pub exchanges: Exchanges,
    pub assets: AssetsConfig,
    pub yield_farms: Option<YieldFarms>,
    pub rebalancers: Option<RebalancersConfig>,
}

impl Config {
    pub fn global() -> &'static Config {
        GLOBAL_CONFIG.get().expect("CONFIG not loaded")
    }

    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let mut config: Config = serde_yaml::from_reader(reader).unwrap();
        // TODO: modify the config checking for wallets that are encrypted
        if config.wallets.any_encrypted() {
            config = decrypt_wallets_from_config(config)
        }
        // ask user password for each wallet
        // decrypt private key
        // overwrite in memory config with the private key decrypted
        GLOBAL_CONFIG.set(config.clone()).unwrap();
        //TODO: before log, need filter some fields
        //tracing::debug!("from_file(): config: {:?}", config);

        config
    }
}
