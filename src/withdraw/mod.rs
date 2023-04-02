use crate::cmd::helpers;
use clap::ArgMatches;

pub mod cmd;

#[tracing::instrument(name = "run withdraw")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let wallet = helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    let withdraw_wallet = helpers::get_withdraw_wallet(args);

    let network = helpers::get_network(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let asset = helpers::get_asset_in_network_from_args(args, network.name());
    let asset_decimals = asset.decimals().await.unwrap();
    let amount = helpers::get_amount(args, asset_decimals).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    asset
        .withdraw(wallet, &withdraw_wallet, amount)
        .await
        .unwrap();

    Ok(())
}
