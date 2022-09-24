use crate::{cmd, config::Config, utils};
use clap::{ArgMatches, Command};
use prettytable::{row, table};

pub const ALLOWANCE_COMMAND: &str = "allowance";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(ALLOWANCE_COMMAND)
        .about("Get allowance for an network and wallet")
        .arg(clap::arg!(-n --"network" <bsc> "Network to use, ex (bsc, polygon)").required(true))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let config = Config::global();
    let mut table = table!(["Exchange", "Asset", "Balance", "Allowance"]);

    let network = cmd::helpers::get_network(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    let wallet = cmd::helpers::get_wallet(args).unwrap_or_else(|e| {
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
}
