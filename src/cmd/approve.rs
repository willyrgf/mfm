use crate::cmd;
use clap::{ArgMatches, Command};

pub const APPROVE_COMMAND: &str = "approve";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(APPROVE_COMMAND)
        .about("Approve token spending (needed to swap tokens)")
        //TODO: add a custom spender arg to add another spenders lide yield-farms
        .arg(
            clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router as spender")
                .required(true),
        )
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(clap::arg!(-a --"asset" <ASSET> "Asset to approve spender").required(true))
        .arg(clap::arg!(-v --"amount" <VALUE> "Amount to allow spending").required(true))
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::helpers::get_exchange(args);
    let wallet = cmd::helpers::get_wallet(args);
    let asset = cmd::helpers::get_asset_in_network_from_args(args, exchange.network_id());

    let asset_decimals = asset.decimals().await;
    let amount = cmd::helpers::get_amount(args, asset_decimals);
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
