use crate::{
    cmd::helpers,
    utils::{self, math},
};
use clap::ArgMatches;
use prettytable::{row, Table};
use web3::types::U256;

pub mod cmd;

#[tracing::instrument(name = "run swap")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let network = helpers::get_network(args)?;

    let input_asset = helpers::get_token_input_in_network_from_args(args, network.name()).unwrap();
    tracing::debug!("input_token: {:?}", input_asset);
    let output_asset =
        helpers::get_token_output_in_network_from_args(args, network.name()).unwrap();
    tracing::debug!("output_token: {:?}", output_asset);

    let input_asset_decimals = input_asset.decimals().await.unwrap();
    let output_asset_decimals = output_asset.decimals().await.unwrap();

    let amount_in = helpers::get_amount(args, input_asset_decimals)?;

    let exchange = match helpers::get_exchange(args) {
        Ok(e) => e,
        Err(_) => {
            tracing::warn!("exchange not found in args, try to use the best liquidity exchange");

            network
                .get_exchange_by_liquidity(&input_asset, &output_asset, amount_in)
                .await
                .unwrap()
        }
    };

    let from_wallet = helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let slippage = helpers::get_slippage(args)?;

    let asset_path_in = exchange.build_route_for(&input_asset, &output_asset).await;

    let amount_min_out: U256 = exchange
        .get_amounts_out(amount_in, asset_path_in.clone())
        .await
        .last()
        .unwrap()
        .into();
    tracing::debug!("amount_mint_out: {:?}", amount_min_out);

    let slippage_amount =
        math::get_slippage_amount(amount_min_out, slippage, output_asset_decimals);
    let amount_out_slippage = amount_min_out - slippage_amount;

    exchange
        .swap_tokens_for_tokens(
            from_wallet,
            amount_in,
            amount_out_slippage,
            input_asset.clone(),
            output_asset.clone(),
            Some(math::f64_to_u256(slippage, output_asset_decimals)),
        )
        .await;

    let mut table = Table::new();
    table.add_row(row![
        "From Asset",
        "From Asset Amount",
        "To Asset",
        "To Asset Amount",
    ]);
    table.add_row(row![
        input_asset.name(),
        utils::blockchain::display_amount_to_float(amount_in, input_asset_decimals),
        output_asset.name(),
        utils::blockchain::display_amount_to_float(amount_out_slippage, output_asset_decimals),
    ]);
    table.printstd();

    Ok(())
}
