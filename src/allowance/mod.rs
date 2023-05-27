use crate::{cmd::helpers, config::Config, utils};
use clap::ArgMatches;
use prettytable::{row, table};

pub mod cmd;

#[tracing::instrument(name = "run allowance")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let config = Config::global();
    let mut table = table!(["Exchange", "Asset", "Balance", "Allowance"]);

    let network = helpers::get_network(args)?;
    let wallet = helpers::get_wallet(args)?;

    for exchange in network.get_exchanges().into_iter() {
        let assets_list = config.assets.assets_by_network(network)?;

        for asset in assets_list.into_iter() {
            let balance_of = asset.balance_of(wallet.address()).await?;
            let decimals = asset.decimals().await?;
            let allowance = asset
                .allowance(wallet.address(), exchange.as_router_address().unwrap())
                .await;

            table.add_row(row![
                exchange.name,
                asset.name(),
                utils::blockchain::display_amount_to_float(balance_of, decimals),
                utils::blockchain::display_amount_to_float(allowance, decimals),
            ]);
        }
    }

    table.printstd();

    Ok(())
}
