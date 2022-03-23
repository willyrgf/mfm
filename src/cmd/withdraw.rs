use crate::cmd;
use clap::ArgMatches;

pub const WITHDRAW_COMMAND: &str = "withdraw";

pub async fn call_sub_commands(args: &ArgMatches) {
    let wallet = cmd::get_wallet(args);
    let network = cmd::get_network(args);
    let withdraw_wallet = cmd::get_withdraw_wallet(args);

    let asset = cmd::get_asset_in_network_from_args(args, network.get_name());
    let asset_decimals = asset.decimals().await;
    let amount = cmd::get_amount(args, asset_decimals);

    asset.withdraw(wallet, withdraw_wallet, amount).await;
}
