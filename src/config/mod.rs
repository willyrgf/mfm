mod decrypt_wallet;
pub mod exchange;
pub mod network;
pub mod wallet;
pub mod withdraw_wallet;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

use crate::{
    asset::config::AssetsConfig, notification::config::Notifications,
    rebalancer::config::RebalancersConfig,
};
use exchange::Exchanges;
use network::Networks;
use wallet::Wallets;
use withdraw_wallet::WithdrawWallets;

static GLOBAL_CONFIG: OnceCell<Config> = OnceCell::new();

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
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
    pub rebalancers: Option<RebalancersConfig>,
    pub notifications: Option<Notifications>,
}

impl Config {
    pub fn global() -> &'static Config {
        GLOBAL_CONFIG.get().expect("CONFIG not loaded")
    }

    pub fn from_file(f: &str) -> Result<Self, anyhow::Error> {
        let reader = std::fs::File::open(f)
            .map_err(|e| anyhow::anyhow!("failed to open a file, err: {:?}", e))?;

        let mut config: Config = serde_yaml::from_reader(reader).map_err(|e| {
            anyhow::anyhow!("failed to deserialize the file to a Config, err: {:?}", e)
        })?;

        if config.wallets.any_encrypted() {
            config = decrypt_wallet::decrypt_wallets_from_config(config);
        }

        // ask user password for each wallet
        // decrypt private key
        // overwrite in memory config with the private key decrypted
        GLOBAL_CONFIG.set(config.clone()).unwrap();
        //TODO: before log, need filter some fields
        //tracing::debug!("from_file(): config: {:?}", config);

        Ok(config)
    }
}
