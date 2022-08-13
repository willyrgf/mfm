use crate::cmd::helpers;
use crate::{config::Config, shared};
use clap::ArgMatches;
use prettytable::{cell, row, table};
use web3::types::U256;

pub mod cmd;

#[tracing::instrument(name = "run balances")]
async fn run(args: &ArgMatches) {
    let config = Config::global();
    let wallet = helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    let hide_zero = helpers::get_hide_zero(args);

    let mut table = table!([
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
            .flat_map(|asset_config| asset_config.new_assets_list().unwrap())
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
