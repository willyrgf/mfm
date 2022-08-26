use crate::{cmd::helpers, shared};
use clap::ArgMatches;
use prettytable::{cell, row, Table};
use web3::types::U256;

pub mod cmd;

#[tracing::instrument(name = "run quote")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let exchange = helpers::get_exchange(args)?;

    let input_token = helpers::get_token_input_in_network_from_args(args, exchange.network_id())?;
    let output_token = helpers::get_token_output_in_network_from_args(args, exchange.network_id())?;

    let input_token_decimals = input_token.decimals().await?;
    let output_token_decimals = output_token.decimals().await?;

    let amount_in = helpers::get_amount(args, input_token_decimals)?;
    let slippage = helpers::get_slippage(args, output_token_decimals)?;

    let asset_path = exchange.build_route_for(&input_token, &output_token).await;

    let amount_min_out: U256 = exchange
        .get_amounts_out(amount_in, asset_path.clone())
        .await
        .last()
        .unwrap()
        .into();

    // TODO: move this calc for the new mod of U256
    let slippage_amount = (amount_min_out * slippage) / U256::exp10(output_token_decimals.into());
    let amount_out_slippage: U256 = amount_min_out - slippage_amount;

    let mut table = Table::new();
    table.add_row(row![
        "Exchange",
        "From Asset",
        "From Asset Amount",
        "To Asset",
        "To Asset Amount",
        "To Asset Amount with Slippage",
    ]);
    table.add_row(row![
        exchange.name(),
        input_token.name(),
        shared::blockchain_utils::display_amount_to_float(amount_in, input_token_decimals),
        output_token.name(),
        shared::blockchain_utils::display_amount_to_float(amount_min_out, output_token_decimals),
        shared::blockchain_utils::display_amount_to_float(
            amount_out_slippage,
            output_token_decimals
        ),
    ]);
    table.printstd();

    Ok(())
}
