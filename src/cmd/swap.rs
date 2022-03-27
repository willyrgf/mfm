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

    let input_asset =
        cmd::helpers::get_token_input_in_network_from_args(args, exchange.network_id());
    log::debug!("input_token: {:?}", input_asset);
    let output_asset =
        cmd::helpers::get_token_output_in_network_from_args(args, exchange.network_id());
    log::debug!("output_token: {:?}", output_asset);

    let input_asset_decimals = input_asset.decimals().await;
    let output_asset_decimals = output_asset.decimals().await;

    let amount_in = cmd::helpers::get_amount(args, input_asset_decimals);
    let slippage = cmd::helpers::get_slippage(args, output_asset_decimals);

    let asset_path_in = exchange.build_route_for(&input_asset, &output_asset).await;
    let asset_path_out = exchange.build_route_for(&output_asset, &input_asset).await;

    // TODO: move address->token transformation to swap
    let path_token: Vec<Token> = asset_path_in
        .clone()
        .into_iter()
        .map(Token::Address)
        .collect::<Vec<_>>();
    let amount_min_out: U256 = exchange
        .get_amounts_out(amount_in, asset_path_in.clone())
        .await
        .last()
        .unwrap()
        .into();
    log::debug!("amount_mint_out: {:?}", amount_min_out);

    // TODO: move it to a func an test it
    // check what input asset max tx amount limit is lower to use
    //
    // anonq 10000  = 1USD
    // anonqv2 1000 = 1USD
    // (anonq/anonqv2)*10000 = 10000
    // (anonqv2/anonqv2)*1000 = 1000 // use it
    //
    // anonq 10000  = 1USD
    // anonqv2 1000 = 20USD
    // (anonq/anonqv2)*10000 = 500 // use it
    // (anonqv2/anonqv2)*1000 = 1000
    //
    // amount_min_out = 0,05
    // amount_min_out*1000 = 50
    let i_max_tx_amount = input_asset.max_tx_amount().await;
    let o_max_tx_amount = output_asset.max_tx_amount().await;
    log::debug!("cmd::swap() i_max_tx_amount: {:?}", i_max_tx_amount);
    log::debug!("cmd::swap() o_max_tx_amount: {:?}", o_max_tx_amount);
    let limit_max_input = match (i_max_tx_amount, o_max_tx_amount) {
        (Some(il), Some(ol)) => {
            // anonq =        10_000 = 10000anonq
            // safemoon = 10_000_000 = 249410anonq
            // 10000*11000 = 111_000
            // 10_000*11000 = (10000*11000)*6,17 = 678_000

            // 10000000*6,17 = 61.7MM
            // 10000*1 = 10000

            let limit_amount_out: U256 = exchange
                .get_amounts_out(ol, asset_path_out.clone())
                .await
                .last()
                .unwrap()
                .into();

            log::debug!(
                "cmd::swap(): limit_amount_out: {:?}, limit_amount_out: {:?}",
                limit_amount_out,
                shared::blockchain_utils::display_amount_to_float(
                    limit_amount_out,
                    input_asset_decimals
                )
            );

            if il > limit_amount_out {
                Some(limit_amount_out)
            } else {
                Some(il)
            }
        }
        (None, Some(ol)) => {
            let limit_amount_in: U256 = exchange
                .get_amounts_out(ol, asset_path_out.clone())
                .await
                .last()
                .unwrap()
                .into();
            log::debug!("cmd::swap() limit_amount_in: {:?}", limit_amount_in);
            Some(limit_amount_in)
        }
        (Some(il), None) => Some(il),
        (None, None) => None,
    };

    log::debug!("cmd::swap() limit_max_output: {:?}", limit_max_input);

    let hops = match limit_max_input {
        Some(limit) => {
            let limited_amount_in = limit;
            // let amount_min_out: U256 = exchange
            // .get_amounts_out(amount_in, asset_path.clone())
            // .await
            // .last()
            // .unwrap()
            // .into();

            vec![(amount_in, amount_min_out)]
        }
        None => vec![(amount_in, amount_min_out)],
    };

    panic!();

    let slippage_amount = (amount_min_out * slippage) / U256::exp10(output_asset_decimals.into());
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
        input_asset.name(),
        shared::blockchain_utils::display_amount_to_float(amount_in, input_asset_decimals),
        output_asset.name(),
        shared::blockchain_utils::display_amount_to_float(
            amount_out_slippage,
            output_asset_decimals
        ),
    ]);
    table.printstd();
}
