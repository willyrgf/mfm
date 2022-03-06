use core::panic;

use mfm::{cmd, config::Config};
use web3::ethabi::Token;
use web3::types::U256;

//TODO: handle with all unwraps
#[tokio::main]
async fn main() -> web3::contract::Result<()> {
    let cmd = cmd::new();

    // let args = Args::parse();
    let cmd_matches = cmd.get_matches();
    println!("matches: {:?}", cmd_matches);
    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    cmd::handle_sub_commands(cmd_matches, config);

    // match cmd_matches.subcommand() {
    //     Some(("approve_spender", args)) => {
    //         let exchange = match args.value_of("exchange") {
    //             Some(n) => config.exchanges.get(n),
    //             None => panic!("--exchange not supported"),
    //         };
    //         println!("exchange: {:?}", exchange);
    //         let network = exchange.get_network(&config.networks);

    //         let http = web3::transports::Http::new(network.rpc_url()).unwrap();
    //         let client = web3::Web3::new(http);

    //         let asset = match args.value_of("asset") {
    //             Some(i) => config.assets.get(i),
    //             None => panic!("--asset not supported"),
    //         };
    //         let asset_decimals = asset.decimals(client.clone()).await;
    //         let wallet = match args.value_of("wallet") {
    //             Some(w) => config.wallets.get(w),
    //             None => panic!("--wallet doesnt exist"),
    //         };
    //         //#TODO: need to review usage from i128
    //         let amount_in = match args.value_of("value") {
    //             Some(a) => {
    //                 let q = a.parse::<f64>().unwrap();
    //                 let qe = (q * 10_f64.powf(asset_decimals.into())) as i128;
    //                 U256::from(qe)
    //             }
    //             None => panic!("--value is missing"),
    //         };

    //         let gas_price = client.eth().gas_price().await.unwrap();
    //         println!("amount_int: {:?}", amount_in);

    //         asset
    //             .approve_spender(
    //                 client.clone(),
    //                 gas_price,
    //                 wallet,
    //                 exchange.as_router_address().unwrap(),
    //                 amount_in,
    //             )
    //             .await;
    //         let remaning = asset
    //             .allowance(
    //                 client.clone(),
    //                 wallet.address(),
    //                 exchange.as_router_address().unwrap(),
    //             )
    //             .await;
    //         println!(
    //             "approved_spender allowance remaning to spend: {:?}, asset_decimals: {}",
    //             remaning, asset_decimals
    //         );
    //     }
    //     Some(("allowance", args)) => {
    //         let exchange = match args.value_of("exchange") {
    //             Some(n) => config.exchanges.get(n),
    //             None => panic!("--exchange not supported"),
    //         };
    //         println!("exchange: {:?}", exchange);
    //         let network = exchange.get_network(&config.networks);

    //         let http = web3::transports::Http::new(network.rpc_url()).unwrap();
    //         let client = web3::Web3::new(http);

    //         let asset = match args.value_of("asset") {
    //             Some(i) => config.assets.get(i),
    //             None => panic!("--asset not supported"),
    //         };
    //         let wallet = match args.value_of("wallet") {
    //             Some(w) => config.wallets.get(w),
    //             None => panic!("--wallet doesnt exist"),
    //         };

    //         let asset_decimals = asset.decimals(client.clone()).await;
    //         let remaning = asset
    //             .allowance(
    //                 client.clone(),
    //                 wallet.address(),
    //                 exchange.as_router_address().unwrap(),
    //             )
    //             .await;
    //         println!(
    //             "allowance remaning to spend: {:?}, asset_decimals: {}",
    //             remaning, asset_decimals
    //         );
    //     }
    //     Some(("swaptt", args)) => {
    //         let exchange = match args.value_of("exchange") {
    //             Some(n) => config.exchanges.get(n),
    //             None => panic!("--exchange not supported"),
    //         };
    //         println!("exchange: {:?}", exchange);
    //         let network = exchange.get_network(&config.networks);

    //         let http = web3::transports::Http::new(network.rpc_url()).unwrap();
    //         let client = web3::Web3::new(http);

    //         let input_token = match args.value_of("token_input") {
    //             Some(i) => config.assets.get(i),
    //             None => panic!("--token_input not supported"),
    //         };
    //         println!("input_token: {:?}", input_token);

    //         let output_token = match args.value_of("token_output") {
    //             Some(i) => config.assets.get(i),
    //             None => panic!("--token_output not supported"),
    //         };
    //         println!("output_token: {:?}", output_token);

    //         let wallet = match args.value_of("wallet") {
    //             Some(w) => config.wallets.get(w),
    //             None => panic!("--wallet doesnt exist"),
    //         };

    //         let input_token_decimals = input_token.decimals(client.clone()).await;
    //         let output_token_decimals = output_token.decimals(client.clone()).await;
    //         //#TODO: review i128
    //         let amount_in = match args.value_of("amount") {
    //             Some(a) => {
    //                 let q = a.parse::<f64>().unwrap();
    //                 let qe = (q * 10_f64.powf(input_token_decimals.into())) as i128;
    //                 U256::from(qe)
    //             }
    //             None => panic!("missing amount"),
    //         };
    //         //#TODO: review i128
    //         let slippage = match args.value_of("slippage") {
    //             Some(a) => {
    //                 let q = a.parse::<f64>().unwrap();
    //                 let qe = ((q / 100.0) * 10_f64.powf(output_token_decimals.into())) as i64;
    //                 U256::from(qe)
    //             }
    //             None => panic!("missing slippage"),
    //         };

    //         let asset_path = config.routes.search(input_token, output_token);
    //         let path = asset_path.build_path(&config.assets);
    //         let path_token: Token = asset_path.build_path_using_tokens(&config.assets);
    //         let amount_min_out: U256 = exchange
    //             .get_amounts_out(client.clone(), amount_in, path.clone())
    //             .await
    //             .last()
    //             .unwrap()
    //             .into();
    //         let gas_price = client.eth().gas_price().await.unwrap();
    //         println!("amount_mint_out: {:?}", amount_min_out);

    //         let slippage_amount =
    //             (amount_min_out * slippage) / U256::exp10(output_token_decimals.into());
    //         println!("slippage_amount {:?}", slippage_amount);

    //         let amount_out_slippage: U256 = amount_min_out - slippage_amount;
    //         println!("amount_out_slippage : {:?}", amount_out_slippage);
    //         exchange
    //             .swap_tokens_for_tokens(
    //                 client.clone(),
    //                 wallet,
    //                 gas_price,
    //                 amount_in,
    //                 amount_out_slippage,
    //                 path_token,
    //             )
    //             .await;
    //     }
    //     Some(("wrap", args)) => {
    //         let network = match args.value_of("network") {
    //             Some(n) => config.networks.get(n),
    //             None => panic!("--network not supported"),
    //         };
    //         let http = web3::transports::Http::new(network.rpc_url()).unwrap();
    //         let client = web3::Web3::new(http);

    //         let wrapped_asset = network.get_wrapped_asset(&config.assets);
    //         let wrapped_asset_decimals = wrapped_asset.decimals(client.clone()).await;

    //         let wallet = match args.value_of("wallet") {
    //             Some(w) => config.wallets.get(w),
    //             None => panic!("--wallet doesnt exist"),
    //         };
    //         //#TODO: review usage of i128 for big numbers
    //         let amount_in = match args.value_of("amount") {
    //             Some(a) => {
    //                 let q = a.parse::<f64>().unwrap();
    //                 let qe = (q * 10_f64.powf(wrapped_asset_decimals.into())) as i128;
    //                 U256::from(qe)
    //             }
    //             None => {
    //                 let balance = client.eth().balance(wallet.address(), None).await.unwrap();
    //                 let min = network.get_min_balance_coin(wrapped_asset_decimals);
    //                 if min > balance {
    //                     panic!("balance: {} is not sufficient, min: {}", balance, min);
    //                 }
    //                 balance - min
    //             }
    //         };

    //         let n = wallet.nonce(client.clone()).await;
    //         println!("nonce: {}", n);

    //         let gas_price = client.eth().gas_price().await.unwrap();

    //         wrapped_asset
    //             .wrap(client.clone(), wallet, amount_in, gas_price)
    //             .await;
    //     }
    //     _ => panic!("cmd_matches None"),
    // };

    Ok(())
}
