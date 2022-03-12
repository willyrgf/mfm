use crate::{cmd, config};
use clap::ArgMatches;
use web3::types::U256;

pub const SWAP_COMMAND: &'static str = "swap";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let exchange = cmd::get_exchange(args, config);
    let wallet = cmd::get_wallet(args, config);
    let client = exchange
        .get_network(&config.networks)
        .get_web3_client_http();

    let input_token = cmd::get_token_input(args, config);
    log::debug!("input_token: {:?}", input_token);
    let output_token = cmd::get_token_output(args, config);
    log::debug!("output_token: {:?}", output_token);

    let input_token_decimals = input_token.decimals(client.clone()).await;
    let output_token_decimals = output_token.decimals(client.clone()).await;

    let amount_in = cmd::get_amount(args, input_token_decimals);
    let slippage = cmd::get_slippage(args, output_token_decimals);

    let asset_path = config.routes.search(input_token, output_token);
    let path = asset_path.build_path(&config.assets);
    let path_token = asset_path.build_path_using_tokens(&config.assets);
    let amount_min_out: U256 = exchange
        .get_amounts_out(client.clone(), amount_in, path.clone())
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
            path_token,
        )
        .await;
}
