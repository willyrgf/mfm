use crate::cmd;
use clap::{ArgMatches, Command};

pub const TRANSACTION_COMMAND: &str = "transaction";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new("transaction")
        .about("Get transaction details")
        .arg(clap::arg!(-n --"network" <bsc> "Network to search transaction").required(true))
}

//TODO: finish it
pub async fn call_sub_commands(args: &ArgMatches) {
    let network = cmd::helpers::get_network(args).unwrap();
    // let network = match cmd::helpers::get_network(args) {
    //     Some(n) => n,
    //     None => {
    //         tracing::error!("--network not found");
    //         panic!()
    //     }
    // };

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
    // tracing::debug!(
    //     "allowance remaning to spend: {:?}, asset_decimals: {}",
    //     remaning,
    //     asset_decimals
    // );
}
