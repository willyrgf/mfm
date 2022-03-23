use crate::{cmd, config::Config};
use clap::ArgMatches;

pub const APPROVE_COMMAND: &str = "approve";

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::get_exchange(args);
    let wallet = cmd::get_wallet(args);
    let asset = match args.value_of("asset") {
        Some(a) => Config::global()
            .assets
            .find_by_name_and_network(a, exchange.network_id())
            .unwrap(),
        None => panic!("can't find asset in the exchange network"),
    };

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
