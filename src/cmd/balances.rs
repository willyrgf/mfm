use crate::{cmd, config::Config, shared};
use clap::{ArgMatches, Command};
use prettytable::{cell, row, Table};
use web3::types::U256;
//use web3::types::U256;

pub const BALANCES_COMMAND: &str = "balances";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(BALANCES_COMMAND)
        .about("Check balances from all assets listed on config")
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(
            clap::arg!(-z --"hide-zero" <TRUE_FALSE> "hide zero balances")
                .required(false)
                .default_value("false"),
        )
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let config = Config::global();
    let wallet = cmd::helpers::get_wallet(args);
    let hide_zero = match args.value_of("hide-zero") {
        Some(b) => b.parse::<bool>().unwrap(),
        _ => false,
    };

    let mut table = Table::new();
    table.add_row(row![
        "Network",
        "Asset",
        "Balance in float",
        "Balance",
        "Decimals"
    ]);
    for asset in config.assets.hashmap().values() {
        let balance_of = asset.balance_of(wallet.address()).await;
        let decimals = asset.decimals().await;
        if !(hide_zero && balance_of == U256::from(0_i32)) {
            table.add_row(row![
                asset.network_id(),
                asset.name(),
                shared::blockchain_utils::display_amount_to_float(balance_of, decimals),
                balance_of,
                decimals
            ]);
        }
        //let asset_decimals = asset.decimals(client.clone()).await;
        //let amount_balance: f64 = (balance_of / U256::exp10(asset_decimals.into())).into();
        log::info!("{} -> balance {}", asset.name(), balance_of)
    }

    table.printstd();
}
