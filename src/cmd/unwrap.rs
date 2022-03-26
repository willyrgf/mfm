use crate::cmd;
use clap::{ArgMatches, Command};
//TODO: Need to review this,  may we can use swaptokenstoeth
// because in another networks the deposit does not act like another ones
pub const COMMAND: &str = "unwrap";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(COMMAND)
        .about("Unwrap a wrapped coin to coin")
        .arg(clap::arg!(-n --"network" <bsc> "Network to unwrap token to coin").required(true))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(clap::arg!(-a --"amount" <AMMOUNT> "Amount to unwrap token into coin").required(false))
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let wallet = cmd::helpers::get_wallet(args);

    let network = match cmd::helpers::get_network(args) {
        Some(n) => n,
        None => {
            log::error!("--network not found");
            panic!()
        }
    };

    let wrapped_asset = network.get_wrapped_asset();
    let wrapped_asset_decimals = wrapped_asset.decimals().await;

    let amount_in = cmd::helpers::get_amount(args, wrapped_asset_decimals);

    wrapped_asset.unwrap(wallet, amount_in).await;
}
