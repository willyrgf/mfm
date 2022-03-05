use clap::{Command, Parser};
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
            Command::new("wrap").about("Wrap a coin to a token").arg(
                clap::arg!(--"network" <bsc>)
                    .required(false)
                    .allow_invalid_utf8(true),
            ),
        );

    let cmd_matches = cmd.get_matches();
    println!("matches: {:?}", cmd_matches);
    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    match cmd_matches.subcommand() {
        Some(("wrap", _matches)) => {
            let wallet = config.wallets.get("test-wallet");
            let bsc_network = config.networks.get("bsc");

            let http = web3::transports::Http::new(bsc_network.rpc_url()).unwrap();
            let client = web3::Web3::new(http);

            let n = wallet.nonce(client.clone()).await;
            println!("nonce: {}", n);

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
        }
        _ => panic!("cmd_matches None"),
    };

    Ok(())
}
