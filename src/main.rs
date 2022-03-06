use clap::Command;
use mfm::config::Config;
use web3::types::U256;

//TODO: handle with all unwraps
#[tokio::main]
async fn main() -> web3::contract::Result<()> {
    // let args = Args::parse();

    let cmd = Command::new("mfm")
        .bin_name("mfm")
        .arg(
            clap::arg!(-c - -config_filename <PATH> "Config file path")
                .required(false)
                .default_value("config.yaml"),
        )
        .subcommand_required(true)
        .subcommand(
            Command::new("wrap")
                .about("Wrap a coin to a token")
                .arg(
                    clap::arg!(--"network" <bsc> "Network to wrap coin to token")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"amount" <AMMOUNT> "Amount to wrap coin into token, default: (balance-min_balance_coin)")
                        .required(false)
                        ,
                ),
        )
        .subcommand(
            Command::new("swaptt")
                .about("Swap Tokens for Tokens")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                        .required(false)
                )
                .arg(
                    clap::arg!(-i --"token_input" <TOKEN_INPUT> "Asset of input token")
                        .required(false)
                )
                .arg(
                    clap::arg!(-o --"token_output" <TOKEN_OUTPUT> "Asset of output token")
                        .required(false)
                ),

        );

    let cmd_matches = cmd.get_matches();
    println!("matches: {:?}", cmd_matches);
    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    match cmd_matches.subcommand() {
        Some(("swaptt", args)) => {
            let exchange = match args.value_of("exchange") {
                Some(n) => config.exchanges.get(n),
                None => panic!("--exchange not supported"),
            };
            println!("exchange: {:?}", exchange);
            let network = exchange.get_network(&config.networks);

            let http = web3::transports::Http::new(network.rpc_url()).unwrap();
            let client = web3::Web3::new(http);

            let input_token = match args.value_of("token_input") {
                Some(i) => config.assets.get(i),
                None => panic!("--token_input not supported"),
            };
            println!("input_token: {:?}", input_token);

            let output_token = match args.value_of("token_output") {
                Some(i) => config.assets.get(i),
                None => panic!("--token_output not supported"),
            };
            println!("output_token: {:?}", output_token);

            let wallet = match args.value_of("wallet") {
                Some(w) => config.wallets.get(w),
                None => panic!("--wallet doesnt exist"),
            };

            let input_token_decimals = input_token.decimals(client.clone()).await;
            let amount_in = match args.value_of("amount") {
                Some(a) => {
                    let q = a.parse::<f64>().unwrap();
                    let qe = (q * 10_f64.powf(input_token_decimals.into())) as i64;
                    U256::from(qe)
                }
                None => panic!("missing amount"),
            };

            let asset_path = config.routes.search(input_token, output_token);
            let path = asset_path.build_path(&config.assets);
            let amount_min_out = exchange
                .get_amounts_out(client.clone(), amount_in, path.clone())
                .await
                .last()
                .unwrap()
                .into();
            let gas_price = client.eth().gas_price().await.unwrap();

            println!("amount_mint_out: {:?}", amount_min_out);
            println!("path : {:?}", path);
            exchange
                .swap_tokens_for_tokens(
                    client.clone(),
                    wallet,
                    gas_price,
                    amount_in,
                    amount_min_out,
                    path,
                )
                .await;
        }
        Some(("wrap", args)) => {
            let network = match args.value_of("network") {
                Some(n) => config.networks.get(n),
                None => panic!("--network not supported"),
            };
            let http = web3::transports::Http::new(network.rpc_url()).unwrap();
            let client = web3::Web3::new(http);

            let wrapped_asset = network.get_wrapped_asset(&config.assets);
            let wrapped_asset_decimals = wrapped_asset.decimals(client.clone()).await;

            let wallet = match args.value_of("wallet") {
                Some(w) => config.wallets.get(w),
                None => panic!("--wallet doesnt exist"),
            };
            let amount_in = match args.value_of("amount") {
                Some(a) => {
                    let q = a.parse::<f64>().unwrap();
                    let qe = (q * 10_f64.powf(wrapped_asset_decimals.into())) as i64;
                    U256::from(qe)
                }
                None => {
                    let balance = client.eth().balance(wallet.address(), None).await.unwrap();
                    let min = network.get_min_balance_coin(wrapped_asset_decimals);
                    if min > balance {
                        panic!("balance: {} is not sufficient, min: {}", balance, min);
                    }
                    balance - min
                }
            };

            let n = wallet.nonce(client.clone()).await;
            println!("nonce: {}", n);

            let gas_price = client.eth().gas_price().await.unwrap();

            wrapped_asset
                .wrap(client.clone(), wallet, amount_in, gas_price)
                .await;
        }
        _ => panic!("cmd_matches None"),
    };

    Ok(())
}
