pub mod asset;
pub mod exchange;
pub mod network;
pub mod route;
pub mod wallet;

use serde::{Deserialize, Serialize};

use asset::Assets;
use exchange::Exchanges;
use network::Networks;
use route::Routes;
use wallet::Wallets;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub wallets: Wallets,
    pub assets: Assets,
    pub networks: Networks,
    pub exchanges: Exchanges,
    pub routes: Routes,
}

impl Config {
    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();

        config
    }

    // pub fn get_exchange_client_wallet_asset(
    //     &self,
    //     exchange: &str,
    //     wallet: &str,
    //     asset: &str,
    // ) -> (&Exchange, &Web3<Http>, &Wallet, &Asset) {
    //     let e = self.exchanges.get(exchange);
    //     let n = e.get_network(&self.networks);

    //     let c = Web3::new(Http::new(n.rpc_url()).unwrap());

    //     let a = self.assets.get(asset);

    //     let w = self.wallets.get(wallet);
    //     (e, &c, w, a)
    // }
}
