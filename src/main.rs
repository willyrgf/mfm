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

    Ok(())
}
