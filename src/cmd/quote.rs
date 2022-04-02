use crate::{cmd, shared};
use clap::{ArgMatches, Command};
use prettytable::{cell, row, Table};
use web3::types::U256;

pub const COMMAND: &str = "quote";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(COMMAND)
        .about("Get a quote for tokens to tokens")
        .arg(clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router").required(true))
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

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::helpers::get_exchange(args);

    let input_token =
        cmd::helpers::get_token_input_in_network_from_args(args, exchange.network_id());
    log::debug!("input_token: {:?}", input_token);
    let output_token =
        cmd::helpers::get_token_output_in_network_from_args(args, exchange.network_id());
    log::debug!("output_token: {:?}", output_token);

    let input_token_decimals = input_token.decimals().await;
    let output_token_decimals = output_token.decimals().await;

    let amount_in = cmd::helpers::get_amount(args, input_token_decimals);
    let slippage = cmd::helpers::get_slippage(args, output_token_decimals);

    let asset_path = exchange.build_route_for(&input_token, &output_token).await;

    log::debug!("asset_path: {:?}", asset_path);

    let amount_min_out: U256 = exchange
        .get_amounts_out(amount_in, asset_path.clone())
        //.get_amounts_out(amount_in, vec![output_token.as_address().unwrap()])
        .await
        .last()
        .unwrap()
        .into();

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
}
