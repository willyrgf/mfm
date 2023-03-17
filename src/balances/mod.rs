use crate::cmd::helpers;
use crate::config::Config;
use crate::utils::scalar::BigDecimal;
use clap::ArgMatches;
use prettytable::{row, table};
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
            .networks
            .hashmap()
            .values()
            .map(|network| async move {
                let balance_of = match network
                    .get_web3_client_http()
                    .eth()
                    .balance(wallet.address(), None)
                    .await
                {
                    Ok(n) => n,
                    Err(_) => U256::default(),
                };
                (
                    network.name(),
                    network.symbol(),
                    balance_of,
                    network.coin_decimals(),
                )
            }),
    )
    .await
    .into_iter()
    .for_each(|(network_name, symbol, balance_of, decimals)| {
        let balance_of_bd = BigDecimal::from_unsigned_u256(&balance_of, decimals.into());
        let balance_of_f64 = balance_of_bd.with_scale(decimals.into()).to_f64().unwrap();

        if !(hide_zero && balance_of == U256::from(0_i32)) {
            table.add_row(row![
                network_name,
                symbol,
                balance_of_f64,
                balance_of,
                decimals
            ]);
        }
    });

    futures::future::join_all(
        config
            .assets
            .hashmap()
            .values()
            .flat_map(|asset_config| asset_config.new_assets_list().unwrap())
            .map(|asset| async move {
                let balance_of = asset.balance_of(wallet.address()).await;
                let decimals = asset.decimals().await.unwrap();
                (asset, balance_of, decimals)
            }),
    )
    .await
    .into_iter()
    .for_each(|(asset, balance_of, decimals)| {
        let balance_of_bd = BigDecimal::from_unsigned_u256(&balance_of, decimals.into());
        let balance_of_f64 = balance_of_bd.with_scale(decimals.into()).to_f64().unwrap();

        if !(hide_zero && balance_of == U256::from(0_i32)) {
            table.add_row(row![
                asset.network_id(),
                asset.name(),
                balance_of_f64,
                balance_of,
                decimals
            ]);
        }
    });

    table.printstd();
}
