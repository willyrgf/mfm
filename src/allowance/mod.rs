use crate::{cmd::helpers, config::Config, utils};
use clap::ArgMatches;
use prettytable::{row, table};

pub mod cmd;

#[tracing::instrument(name = "run allowance")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let config = Config::global();
    let mut table = table!(["Exchange", "Asset", "Balance", "Allowance"]);

    let network = helpers::get_network(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    let wallet = helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    for exchange in network.get_exchanges().into_iter() {
        let assets_list = config.assets.hashmap().values().flat_map(|asset_config| {
            asset_config.new_assets_list().unwrap_or_else(|e| {
                tracing::error!(error = %e);
                panic!()
            })
        });

        futures::future::join_all(assets_list.map(|asset| async move {
            let balance_of = asset.balance_of(wallet.address()).await;
            let decimals = asset.decimals().await.unwrap();
            let allowance = asset
                .allowance(wallet.address(), exchange.as_router_address().unwrap())
                .await;
            (asset, balance_of, decimals, allowance, exchange)
        }))
        .await
        .into_iter()
        .for_each(|(asset, balance_of, decimals, allowance, exchange)| {
            table.add_row(row![
                exchange.name,
                asset.name(),
                utils::blockchain::display_amount_to_float(balance_of, decimals),
                utils::blockchain::display_amount_to_float(allowance, decimals),
            ]);
        });
    }

    table.printstd();

    Ok(())
}
