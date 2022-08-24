use crate::cmd::helpers;
use clap::ArgMatches;

pub mod cmd;

#[tracing::instrument(name = "run wrap")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let network = helpers::get_network(args)?;
    let wallet = helpers::get_wallet(args)?;

    let wrapped_asset = network.get_wrapped_asset()?;
    let wrapped_asset_decimals = wrapped_asset.decimals().await?;

    let amount_in = helpers::get_amount(args, wrapped_asset_decimals)?;

    wrapped_asset.wrap(wallet, amount_in).await
}
