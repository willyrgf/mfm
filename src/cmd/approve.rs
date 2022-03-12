use crate::{cmd, config};
use clap::ArgMatches;

pub const APPROVE_COMMAND: &'static str = "approve";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let exchange = cmd::get_exchange(args, config);
    let wallet = cmd::get_wallet(args, config);
    let asset = cmd::get_asset(args, config);
    let client = exchange
        .get_network(&config.networks)
        .get_web3_client_http();

    let asset_decimals = asset.decimals(client.clone()).await;
    let amount = cmd::get_amount(args, asset_decimals);
    log::debug!("amount: {:?}", amount);

    let gas_price = client.eth().gas_price().await.unwrap();

    asset
        .approve_spender(
            client.clone(),
            gas_price,
            wallet,
            exchange.as_router_address().unwrap(),
            amount,
        )
        .await;

    let remaning = asset
        .allowance(
            client.clone(),
            wallet.address(),
            exchange.as_router_address().unwrap(),
        )
        .await;
    log::debug!(
        "approved_spender allowance remaning to spend: {:?}, asset_decimals: {}",
        remaning,
        asset_decimals
    );
}
