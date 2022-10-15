use crate::{cmd::helpers, utils};
use clap::{ArgMatches, Command};
use prettytable::{row, Table};
use web3::types::U256;

pub const SWAP_COMMAND: &str = "swap";

pub fn generate_cmd() -> Command {
    Command::new(SWAP_COMMAND)
        .about("Swap Tokens for Tokens supporting fees on transfer")
        .arg(clap::arg!(-n --"network" <bsc> "Network to use, ex (bsc, polygon)").required(true))
        .arg(clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router").required(false))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(
            clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                .required(false),
        )
        .arg(clap::arg!(-i --"token_input" <TOKEN_INPUT> "Asset of input token").required(false))
        .arg(clap::arg!(-o --"token_output" <TOKEN_OUTPUT> "Asset of output token").required(false))
        .arg(
            clap::arg!(-s --"slippage" <SLIPPAGE> "Slippage (default 0.5)")
                .required(false)
                .default_value("0.5"),
        )
}

pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let network = helpers::get_network(args)?;

    let input_asset =
        helpers::get_token_input_in_network_from_args(args, network.get_name()).unwrap();
    tracing::debug!("input_token: {:?}", input_asset);
    let output_asset =
        helpers::get_token_output_in_network_from_args(args, network.get_name()).unwrap();
    tracing::debug!("output_token: {:?}", output_asset);

    let input_asset_decimals = input_asset.decimals().await.unwrap();
    let output_asset_decimals = output_asset.decimals().await.unwrap();

    let amount_in = helpers::get_amount(args, input_asset_decimals).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

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

    let wallet = helpers::get_wallet(args).unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let slippage = helpers::get_slippage(args, output_asset_decimals).unwrap();

    let asset_path_in = exchange.build_route_for(&input_asset, &output_asset).await;

    let amount_min_out: U256 = exchange
        .get_amounts_out(amount_in, asset_path_in.clone())
        .await
        .last()
        .unwrap()
        .into();
    tracing::debug!("amount_mint_out: {:?}", amount_min_out);

    let slippage_amount = (amount_min_out * slippage) / U256::exp10(output_asset_decimals.into());
    let amount_out_slippage = amount_min_out - slippage_amount;

    exchange
        .swap_tokens_for_tokens(
            wallet,
            amount_in,
            amount_out_slippage,
            input_asset.clone(),
            output_asset.clone(),
            Some(slippage),
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
