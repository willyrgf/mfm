use crate::{cmd, config::Config, shared};
use clap::ArgMatches;
use prettytable::{cell, row, Table};
//use web3::types::U256;

pub const BALANCES_COMMAND: &str = "balances";

pub async fn call_sub_commands(args: &ArgMatches) {
    let config = Config::global();
    let wallet = cmd::get_wallet(args, config);
    let mut table = Table::new();
    table.add_row(row![
        "Network",
        "Asset",
        "Balance in float",
        "Balance",
        "Decimals"
    ]);
    for asset in config.assets.hashmap().values() {
        let client = asset.get_network(&config.networks).get_web3_client_http();
        let balance_of = asset.balance_of(client.clone(), wallet.address()).await;
        let decimals = asset.decimals(client.clone()).await;
        table.add_row(row![
            asset.network_id(),
            asset.name(),
            shared::blockchain_utils::display_amount_to_float(balance_of, decimals),
            balance_of,
            decimals
        ]);
        //let asset_decimals = asset.decimals(client.clone()).await;
        //let amount_balance: f64 = (balance_of / U256::exp10(asset_decimals.into())).into();
        log::info!("{} -> balance {}", asset.name(), balance_of)
    }

    table.printstd();
}
