use crate::{cmd, config};
use clap::ArgMatches;

pub const WITHDRAW_COMMAND: &'static str = "withdraw";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let wallet = cmd::get_wallet(args, config);

    let network = cmd::get_network(args, config);
    let client = network.get_web3_client_http();

    let asset = cmd::get_asset(args, config);
    let asset_decimals = asset.decimals(client.clone()).await;
    let amount = cmd::get_amount(args, asset_decimals);

    let gas_price = client.eth().gas_price().await.unwrap();

    let withdraw_wallet = cmd::get_withdraw_wallet(args, config);

    asset
        .withdraw(client.clone(), wallet, withdraw_wallet, amount, gas_price)
        .await;
}
