use crate::{cmd, shared};
use clap::{ArgMatches, Command};
use prettytable::{cell, row, Table};
use web3::{ethabi::Token, types::U256};

pub const SWAP_COMMAND: &str = "swap";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(SWAP_COMMAND)
        .about("Swap Tokens for Tokens supporting fees on transfer")
        .arg(clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router").required(true))
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

pub async fn call_sub_commands(args: &ArgMatches) {
    let exchange = cmd::helpers::get_exchange(args);
    let wallet = cmd::helpers::get_wallet(args);

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

    let asset_path = exchange.build_route_for(input_token, output_token).await;
    let path_token: Vec<Token> = asset_path
        .clone()
        .into_iter()
        .map(Token::Address)
        .collect::<Vec<_>>();
    let amount_min_out: U256 = exchange
        .get_amounts_out(amount_in, asset_path.clone())
        .await
        .last()
        .unwrap()
        .into();
    log::debug!("amount_mint_out: {:?}", amount_min_out);

    let slippage_amount = (amount_min_out * slippage) / U256::exp10(output_token_decimals.into());
    log::debug!("slippage_amount {:?}", slippage_amount);

    let amount_out_slippage: U256 = amount_min_out - slippage_amount;
    log::debug!("amount_out_slippage : {:?}", amount_out_slippage);
    exchange
        .swap_tokens_for_tokens(
            wallet,
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
