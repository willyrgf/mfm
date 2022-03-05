pub mod asset;
pub mod exchange;
pub mod network;
pub mod route;
pub mod wallet;

use serde::{Deserialize, Serialize};
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub wallets: wallet::Wallets,
    pub assets: asset::Assets,
    pub networks: network::Networks,
    pub exchanges: exchange::Exchanges,
    pub routes: route::Routes,
}

impl Config {
    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();

        config
    }
}
