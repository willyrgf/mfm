use crate::{cmd, shared};
use clap::ArgMatches;
use prettytable::{cell, row, Table};
use web3::{ethabi::Token, types::U256};

pub const SWAP_COMMAND: &str = "swap";

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::get_exchange(args);
    let wallet = cmd::get_wallet(args);
    let client = exchange.get_network().get_web3_client_http();

    let input_token = cmd::get_token_input(args);
    log::debug!("input_token: {:?}", input_token);
    let output_token = cmd::get_token_output(args);
    log::debug!("output_token: {:?}", output_token);

    let input_token_decimals = input_token.decimals().await;
    let output_token_decimals = output_token.decimals().await;

    let amount_in = cmd::get_amount(args, input_token_decimals);
    let slippage = cmd::get_slippage(args, output_token_decimals);

    let asset_path = exchange
        .build_route_for(client.clone(), input_token, output_token)
        .await;
    let path_token: Vec<Token> = asset_path
        .clone()
        .into_iter()
        .map(|p| Token::Address(p))
        .collect::<Vec<_>>();
    let amount_min_out: U256 = exchange
        .get_amounts_out(client.clone(), amount_in, asset_path.clone())
        .await
        .last()
        .unwrap()
        .into();
    let gas_price = client.eth().gas_price().await.unwrap();
    log::debug!("amount_mint_out: {:?}", amount_min_out);

    let slippage_amount = (amount_min_out * slippage) / U256::exp10(output_token_decimals.into());
    log::debug!("slippage_amount {:?}", slippage_amount);

    let amount_out_slippage: U256 = amount_min_out - slippage_amount;
    log::debug!("amount_out_slippage : {:?}", amount_out_slippage);
    exchange
        .swap_tokens_for_tokens(
            client.clone(),
            wallet,
            gas_price,
            amount_in,
            amount_out_slippage,
            Token::Array(path_token),
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
        input_token.name(),
        shared::blockchain_utils::display_amount_to_float(amount_in, input_token_decimals),
        output_token.name(),
        shared::blockchain_utils::display_amount_to_float(
            amount_out_slippage,
            output_token_decimals
        ),
    ]);
    table.printstd();
}
