use crate::{cmd, config::Config, shared};
use clap::{ArgMatches, Command};
use prettytable::{cell, row, Table};
use web3::types::U256;

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

#[tracing::instrument(name = "balances call command")]
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

    futures::future::join_all(
        config
            .assets
            .hashmap()
            .values()
            .flat_map(|asset_config| asset_config.new_assets_list())
            .map(|asset| async move {
                let balance_of = asset.balance_of(wallet.address()).await;
                let decimals = asset.decimals().await;
                (asset, balance_of, decimals)
            }),
    )
    .await
    .into_iter()
    .for_each(|(asset, balance_of, decimals)| {
        if !(hide_zero && balance_of == U256::from(0_i32)) {
            table.add_row(row![
                asset.network_id(),
                asset.name(),
                shared::blockchain_utils::display_amount_to_float(balance_of, decimals),
                balance_of,
                decimals
            ]);
        }
    });

    table.printstd();
}
