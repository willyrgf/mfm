use crate::{cmd, config};
use clap::ArgMatches;

pub const ALLOWANCE_COMMAND: &'static str = "allowance";

pub async fn handle_sub_commands(args: &ArgMatches, config: &config::Config) {
    let (exchange, client, wallet, asset) = cmd::get_exchange_client_wallet_asset(args, config);

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
