use crate::config;
use clap::ArgMatches;
use web3::types::U256;

pub const SWAP_COMMAND: &'static str = "swap";

pub async fn handle_sub_commands(args: &ArgMatches, config: &config::Config) {
    let exchange = match args.value_of("exchange") {
        Some(n) => config.exchanges.get(n),
        None => panic!("--exchange not supported"),
    };
    log::debug!("exchange: {:?}", exchange);
    let network = exchange.get_network(&config.networks);

    let http = web3::transports::Http::new(network.rpc_url()).unwrap();
    let client = web3::Web3::new(http);

    let input_token = match args.value_of("token_input") {
        Some(i) => config.assets.get(i),
        None => panic!("--token_input not supported"),
    };
    log::debug!("input_token: {:?}", input_token);

    let output_token = match args.value_of("token_output") {
        Some(i) => config.assets.get(i),
        None => panic!("--token_output not supported"),
    };
    log::debug!("output_token: {:?}", output_token);

    let wallet = match args.value_of("wallet") {
        Some(w) => config.wallets.get(w),
        None => panic!("--wallet doesnt exist"),
    };

    let input_token_decimals = input_token.decimals(client.clone()).await;
    let output_token_decimals = output_token.decimals(client.clone()).await;
    //#TODO: review i128
    let amount_in = match args.value_of("amount") {
        Some(a) => {
            let q = a.parse::<f64>().unwrap();
            let qe = (q * 10_f64.powf(input_token_decimals.into())) as i128;
            U256::from(qe)
        }
        None => panic!("missing amount"),
    };
    //#TODO: review i128
    let slippage = match args.value_of("slippage") {
        Some(a) => {
            let q = a.parse::<f64>().unwrap();
            let qe = ((q / 100.0) * 10_f64.powf(output_token_decimals.into())) as i64;
            U256::from(qe)
        }
        None => panic!("missing slippage"),
    };

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
