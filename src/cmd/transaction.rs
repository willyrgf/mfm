use crate::{cmd, config};
use clap::ArgMatches;

pub const TRANSACTION_COMMAND: &'static str = "transaction";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let network = cmd::get_network(args, config);
    let _client = network.get_web3_client_http();

    // let transaction_receipt = client.eth().transaction_receipt();

    // let asset_decimals = asset.decimals(client.clone()).await;
    // let remaning = asset
    //     .allowance(
    //         client.clone(),
    //         wallet.address(),
    //         exchange.as_router_address().unwrap(),
    //     )
    //     .await;
    // log::debug!(
    //     "allowance remaning to spend: {:?}, asset_decimals: {}",
    //     remaning,
    //     asset_decimals
    // );
}
