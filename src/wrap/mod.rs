use crate::cmd::helpers;
use clap::ArgMatches;

pub mod cmd;

#[tracing::instrument(name = "run wrap")]
async fn run(args: &ArgMatches) {
    let network = helpers::get_network(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    let wallet = helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let wrapped_asset = network.get_wrapped_asset().unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let wrapped_asset_decimals = wrapped_asset.decimals().await;

    let amount_in = helpers::get_amount(args, wrapped_asset_decimals).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    wrapped_asset
        .wrap(wallet, amount_in)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e);
            panic!()
        });
}
