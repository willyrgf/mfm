use clap::Parser;
use mfm::config::Config;
use web3::types::U256;

// multiverse finance machine cli
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    // filename with the mfm configurations
    #[clap(short, long, default_value = "config.yaml")]
    config_filename: String,
}

//TODO: handle with all unwraps

#[tokio::main]
async fn main() -> web3::contract::Result<()> {
    let args = Args::parse();
    let config = Config::from_file(&args.config_filename);
    let wallet = config.wallets.get("test-wallet");
    let bsc_network = config.networks.get("bsc");

    let http = web3::transports::Http::new(bsc_network.rpc_url()).unwrap();
    let client = web3::Web3::new(http);

    let n = wallet.nonce(client.clone()).await;
    println!("nonce: {}", n);

    // let exchange_contract = exchange.router_contract(client.clone());

    let wbnb = config.assets.get("wbnb");
    let exchange = config.exchanges.get(wbnb.exchange_id());
    let decimals_wbnb = wbnb.decimals(client.clone()).await;
    let quantity = 0.005;
    let quantity_exp = (quantity * 10_f64.powf(decimals_wbnb.into())) as i64;
    let gas_price = client.eth().gas_price().await.unwrap();

    let amount_in = U256::from(quantity_exp);

    exchange
        .wrap(client.clone(), wbnb, wallet, amount_in, gas_price)
        .await;

    // // TODO: move it to a async func and let main without async
    // for (_, asset) in config.assets.hashmap().iter() {
    //     let balance_of = asset.balance_of(client.clone(), account_address).await;
    //     let decimals = asset.decimals(client.clone()).await;
    //     println!(
    //         "asset: {}, balance_of: {:?}, decimals: {}",
    //         asset.name(),
    //         balance_of,
    //         decimals
    //     );

    //     if asset.name() == "wbnb2" {
    //         let wbnb = config.assets.get("wbnb");
    //         let busd = config.assets.get("busd");
    //         let path = vec![
    //             asset.as_address().unwrap(),
    //             wbnb.as_address().unwrap(),
    //             busd.as_address().unwrap(),
    //         ];
    //         let route = config.routes.search(asset, busd);
    //         println!(
    //             "route: {:?}; real_path: {:?}",
    //             route,
    //             route.build_path(&config.assets)
    //         );
    //         println!("path: {:?}", path);
    //         let gas_price = client.eth().gas_price().await.unwrap();
    //         // TODO: validate quantity_exp?
    //         let quantity = 0.005;
    //         let quantity_exp = (quantity * 10_f64.powf(decimals_wbnb.into())) as i64;

    //         let amount_in = U256::from(quantity_exp);
    //         println!("amount_in: {} ", amount_in);
    //         exchange
    //             .wrap(client.clone(), wbnb, wallet, amount_in, gas_price)
    //             .await;

    //         // let paths = busd.build_path_for_coin(wbnb.as_address().unwrap());
    //         // let amounts_out = exchange
    //         //     .get_amounts_out(client.clone(), amount_in, vec![wbnb.as_address().unwrap()])
    //         //     .await;
    //         // println!("amounts_out: {:?}", amounts_out);

    //         // let u256_default = U256::default();
    //         // let amount_out: U256 = amounts_out.last().unwrap_or(&u256_default).into();

    //         println!("gas_price: {}", gas_price);
    //         println!("decimals: {}", decimals);
    //         println!("decimals_wbnb: {}", decimals_wbnb);

    //         let wrapped_addr: Address = exchange_contract
    //             .query("WETH", (), None, Options::default(), None)
    //             .await
    //             .unwrap();
    //         println!("wrapped_addr: {}", wrapped_addr);
    //         // let token_address: Vec<Token> = paths
    //         //     .into_iter()
    //         //     .map(|p| Token::Address(p))
    //         //     .collect::<Vec<_>>();

    //         // let valid_timestamp = get_valid_timestamp(300000);

    //         // let estimate_gas = wbnb
    //         //     .contract(client.clone())
    //         //     .estimate_gas(
    //         //         "deposit",
    //         //         (),
    //         //         account_address,
    //         //         web3::contract::Options {
    //         //             value: Some(amount_in),
    //         //             gas_price: Some(gas_price),
    //         //             gas: Some(500_000.into()),
    //         //             // gas: Some(gas_price),
    //         //             ..Default::default()
    //         //         },
    //         //     )
    //         //     .await
    //         //     .unwrap();
    //         // println!("estimate_gas: {:?}", estimate_gas);

    //         // let estimate_gas = exchange_contract
    //         //     .estimate_gas(
    //         //         "swapExactETHForTokensSupportingFeeOnTransferTokens",
    //         //         (
    //         //             amount_out,
    //         //             // paths,
    //         //             vec![wbnb.as_address().unwrap()],
    //         //             account_address,
    //         //             U256::from_dec_str(&valid_timestamp.to_string()).unwrap(),
    //         //         ),
    //         //         account_address,
    //         //         web3::contract::Options {
    //         //             value: Some(amount_in),
    //         //             gas_price: Some(gas_price),
    //         //             gas: Some(500_000.into()),
    //         //             // gas: Some(gas_price),
    //         //             ..Default::default()
    //         //         },
    //         //     )
    //         //     .await
    //         //     .unwrap();
    //         // println!("estimate_gas: {:?}", estimate_gas);

    //         // let estimate_gas = exchange_contract
    //         //     .estimate_gas(
    //         //         "swapTokensForExactTokens",
    //         //         (
    //         //             amount_out,
    //         //             amount_in,
    //         //             paths,
    //         //             account_address,
    //         //             U256::from_dec_str(&valid_timestamp.to_string()).unwrap(),
    //         //         ),
    //         //         account_address,
    //         //         web3::contract::Options {
    //         //             // value: Some(amount_in),
    //         //             gas_price: Some(gas_price),
    //         //             // gas: Some(500_000.into()),
    //         //             // gas: Some(gas_price),
    //         //             ..Default::default()
    //         //         },
    //         //     )
    //         //     .await
    //         //     .unwrap();
    //         // println!("estimate_gas: {:?}", estimate_gas);

    //         // let estimate_gas_in_bnb = estimate_gas * gas_price;
    //         // println!("estimate_gas_in_bnb: {:?}", estimate_gas_in_bnb);

    //         // let swap_data = exchange_contract
    //         //     .abi()
    //         //     .functions
    //         //     .get("swapExactETHForTokensSupportingFeeOnTransferTokens")
    //         //     .unwrap()
    //         //     .get(0)
    //         //     .unwrap()
    //         //     .encode_input(
    //         //         &(
    //         //             Token::Int(amount_out),
    //         //             Token::Array(token_address),
    //         //             Token::Address(account_address),
    //         //             Token::Int(U256::from(300000000000000_u64)),
    //         //         )
    //         //             .into_tokens(),
    //         //     )
    //         //     .unwrap();

    //         // println!("swap_data: {:?}", swap_data);

    //         //     let mut iteration = 0;
    //         //     let mut estimate_gas = U256::default();
    //         //     while iteration < 10 {
    //         //         estimate_gas = match client
    //         //             .eth()
    //         //             .estimate_gas(
    //         //                 web3::types::CallRequest {
    //         //                     from: Some(account_address),
    //         //                     to: Some(exchange.as_router_address().unwrap()),
    //         //                     gas_price: Some(gas_price),
    //         //                     data: swap_data.
    //         //                     ..Default::default()
    //         //                 },
    //         //                 None,
    //         //             )
    //         //             .await
    //         //         {
    //         //             Ok(e) => e,
    //         //             Err(err) => {
    //         //                 println!("estimate_gas err: {}", err);
    //         //                 U256::default()
    //         //             }
    //         //         };
    //         //         iteration = iteration + 1;
    //         //         thread::sleep(Duration::new(5, 0));
    //         //     }
    //         //     println!("estimate_gas: {}", estimate_gas)
    //     }
    // }

    Ok(())
}

// pub struct Crypto {
//     name: String,
// }

// pub struct Defi {
//     contract_abi: String,
//     // steps: v
// }

// pub enum Handle {
//     Crypto(Crypto),
//     Defi(Defi),
// }

// impl Handle {
//     pub fn next_step() {}
// }

// wbnb 18
// 0.000000000000000001
// 1000000000000000000
// 100000000000000
//               10000
// (0,1 * 1)*1e18
//
// let quantity = 0.1;
