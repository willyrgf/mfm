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
        );

    let cmd_matches = cmd.get_matches();
    println!("matches: {:?}", cmd_matches);
    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    match cmd_matches.subcommand() {
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
