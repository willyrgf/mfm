use crate::{cmd, config};
use clap::ArgMatches;

pub const ALLOWANCE_COMMAND: &'static str = "allowance";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let exchange = cmd::get_exchange(args, config);
    let wallet = cmd::get_wallet(args, config);
    let asset = cmd::get_asset(args, config);
    let client = exchange
        .get_network(&config.networks)
        .get_web3_client_http();

    let asset_decimals = asset.decimals(client.clone()).await;
    let remaning = asset
        .allowance(
            client.clone(),
            wallet.address(),
            exchange.as_router_address().unwrap(),
        )
        .await;
    log::debug!(
        "allowance remaning to spend: {:?}, asset_decimals: {}",
        remaning,
        asset_decimals
    );
}
