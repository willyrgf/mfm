use crate::{cmd::helpers, config::Config};
use clap::ArgMatches;

pub mod cmd;

#[tracing::instrument(name = "run approve-all", skip(args))]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let wallet = helpers::get_wallet(args)?;
    let network = helpers::get_network(args)?;
    let config = Config::global();

    let assets = config.assets.assets_by_network(network)?;
    let exchanges = network.get_exchanges();

    for asset in assets {
        let asset_decimals = asset.decimals().await.unwrap();
        let amount = helpers::get_amount(args, asset_decimals).unwrap_or(asset.balance_of(wallet.address()).await?);
        tracing::debug!("amount: {:?}", amount);

        for exchange in exchanges.iter() {
            let allowance_amount = asset.allowance(wallet.address(), exchange.as_router_address().unwrap()).await;
            if allowance_amount >= amount {
                tracing::info!("current amount allowed is greater or equal --amount, to {} on {} for {} with spender {}", asset.name(), exchange.name(), amount, wallet.address());
                continue;
            }

            tracing::info!("running approve_spender to {} on {} for {} with spender {}", asset.name(), exchange.name(), amount, wallet.address());
            asset
                .approve_spender(wallet, exchange.as_router_address().unwrap(), amount)
                .await
                .unwrap();

            let remaning = asset
                .allowance(wallet.address(), exchange.as_router_address().unwrap())
                .await;
            tracing::info!(
                "approved_spender allowance remaning on {}, to spend: {:?}, asset_decimals: {}",
                asset.name(),
                remaning,
                asset_decimals
            );
        }
    }
    Ok(())
}
