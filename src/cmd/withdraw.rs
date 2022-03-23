use crate::cmd;
use clap::ArgMatches;

pub const WITHDRAW_COMMAND: &str = "withdraw";

pub async fn call_sub_commands(args: &ArgMatches) {
    let wallet = cmd::get_wallet(args);

    let network = cmd::get_network(args);
    let client = network.get_web3_client_http();

    let asset = cmd::get_asset(args);
    let asset_decimals = asset.decimals().await;
    let amount = cmd::get_amount(args, asset_decimals);

    let gas_price = client.eth().gas_price().await.unwrap();

    let withdraw_wallet = cmd::get_withdraw_wallet(args);

    asset
        .withdraw(client.clone(), wallet, withdraw_wallet, amount, gas_price)
        .await;
}
