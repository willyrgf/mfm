use crate::cmd;
use clap::ArgMatches;

pub const APPROVE_COMMAND: &str = "approve";

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::get_exchange(args);
    let wallet = cmd::get_wallet(args);
    let asset = cmd::get_asset(args);

    let client = exchange.get_network().get_web3_client_http();

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
