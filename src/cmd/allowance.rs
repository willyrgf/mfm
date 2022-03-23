use crate::cmd;
use clap::{ArgMatches, Command};

pub const ALLOWANCE_COMMAND: &str = "allowance";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(ALLOWANCE_COMMAND)
        .about("Get allowance for an token")
        .arg(clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router").required(true))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(clap::arg!(-a --"asset" <ASSET> "Asset to check allowance").required(true))
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::get_exchange(args);
    let wallet = cmd::get_wallet(args);
    let asset = cmd::get_asset_in_network_from_args(args, exchange.network_id());

    let asset_decimals = asset.decimals().await;
    let remaning = asset
        .allowance(wallet.address(), exchange.as_router_address().unwrap())
        .await;
    log::debug!(
        "allowance remaning to spend: {:?}, asset_decimals: {}",
        remaning,
        asset_decimals
    );
}
