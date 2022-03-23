use crate::cmd;
use clap::ArgMatches;

pub const APPROVE_COMMAND: &str = "approve";

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::get_exchange(args);
    let wallet = cmd::get_wallet(args);
    let asset = cmd::get_asset_in_network_from_args(args, exchange.network_id());

    let asset_decimals = asset.decimals().await;
    let amount = cmd::get_amount(args, asset_decimals);
    log::debug!("amount: {:?}", amount);

    asset
        .approve_spender(wallet, exchange.as_router_address().unwrap(), amount)
        .await;

    let remaning = asset
        .allowance(wallet.address(), exchange.as_router_address().unwrap())
        .await;
    log::debug!(
        "approved_spender allowance remaning to spend: {:?}, asset_decimals: {}",
        remaning,
        asset_decimals
    );
}
