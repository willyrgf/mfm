use crate::{cmd, config};
use clap::ArgMatches;
use web3::types::U256;

pub const BALANCES_COMMAND: &'static str = "balances";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let wallet = cmd::get_wallet(args, config);
    for (_, asset) in config.assets.hashmap() {
        let client = asset.get_network(&config.networks).get_web3_client_http();
        let balance_of = asset.balance_of(client.clone(), wallet.address()).await;
        //let asset_decimals = asset.decimals(client.clone()).await;
        //let amount_balance: f64 = (balance_of / U256::exp10(asset_decimals.into())).into();
        log::info!("{} -> balance {}", asset.name(), balance_of)
    }
}
