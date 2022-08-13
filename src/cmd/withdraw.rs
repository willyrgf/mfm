use crate::cmd;
use clap::{ArgMatches, Command};

pub const WITHDRAW_COMMAND: &str = "withdraw";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(WITHDRAW_COMMAND)
    .about("Withdraw to a wallet")
    .arg(
        clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
            .required(true),
    )
    .arg(
        clap::arg!(-n --"network" <bsc> "Network to wrap coin to token")
            .required(true),
    )
    .arg(
        clap::arg!(-t --"withdraw-wallet" <WITHDRAW_WALLET_NAME> "Withdraw wallet to receive the transfer")
            .required(true),
    )
    .arg(
        clap::arg!(-a --"asset" <ASSET> "Asset to withdraw")
            .required(true)
    )
    .arg(
        clap::arg!(-v --"amount" <VALUE> "Amount to withdraw")
            .required(true)
    )
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let wallet = cmd::helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    let withdraw_wallet = cmd::helpers::get_withdraw_wallet(args);

    let network = cmd::helpers::get_network(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let asset = cmd::helpers::get_asset_in_network_from_args(args, network.get_name());
    let asset_decimals = asset.decimals().await;
    let amount = cmd::helpers::get_amount(args, asset_decimals).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    asset
        .withdraw(wallet, &withdraw_wallet, amount)
        .await
        .unwrap();
}
