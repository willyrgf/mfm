use crate::config::Config;
use crate::{cmd::helpers, utils};
use clap::ArgMatches;
use prettytable::{row, table};
use web3::types::U256;

pub mod cmd;

#[tracing::instrument(name = "run balances")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let config = Config::global();
    let wallet = helpers::get_wallet(args)?;
    let hide_zero = helpers::get_hide_zero(args);

    let mut table = table!([
        "Network",
        "Asset",
        "Balance in float",
        "Balance",
        "Decimals"
    ]);

    let networks = config.networks.hashmap().values();

    for network in networks.clone() {
        let network_name = network.name();
        let symbol = network.symbol();
        let decimals = network.coin_decimals();
        let balance_of = network.balance_coin(wallet).await?;
        if !(hide_zero && balance_of == U256::from(0_i32)) {
            table.add_row(row![
                network_name,
                symbol,
                utils::blockchain::display_amount_to_float(balance_of, decimals),
                balance_of,
                decimals
            ]);
        }
    }

    for network in networks.clone() {
        let assets_list = config.assets.assets_by_network(network)?;
        for asset in assets_list.into_iter() {
            let balance_of = asset.balance_of(wallet.address()).await?;
            let decimals = asset.decimals().await?;
            if !(hide_zero && balance_of == U256::from(0_i32)) {
                table.add_row(row![
                    asset.network_id(),
                    asset.name(),
                    utils::blockchain::display_amount_to_float(balance_of, decimals),
                    balance_of,
                    decimals
                ]);
            }
        }
    }

    table.printstd();
    Ok(())
}
