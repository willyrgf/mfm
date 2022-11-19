use crate::cmd::helpers;
use clap::ArgMatches;

pub mod cmd;

#[tracing::instrument(name = "run approve")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let exchange = helpers::get_exchange(args).unwrap();
    let wallet = helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    let asset = helpers::get_asset_in_network_from_args(args, exchange.network_id());

    let asset_decimals = asset.decimals().await.unwrap();
    let amount = helpers::get_amount(args, asset_decimals).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    tracing::debug!("amount: {:?}", amount);

    asset
        .approve_spender(wallet, exchange.as_router_address().unwrap(), amount)
        .await
        .unwrap();

    let remaning = asset
        .allowance(wallet.address(), exchange.as_router_address().unwrap())
        .await;
    tracing::debug!(
        "approved_spender allowance remaning to spend: {:?}, asset_decimals: {}",
        remaning,
        asset_decimals
    );

    Ok(())
}
