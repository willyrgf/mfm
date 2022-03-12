use crate::config;
use clap::ArgMatches;

pub const ALLOWANCE_COMMAND: &'static str = "allowance";

pub async fn handle_sub_commands(args: &ArgMatches, config: &config::Config) {
    let exchange = match args.value_of("exchange") {
        Some(n) => config.exchanges.get(n),
        None => panic!("--exchange not supported"),
    };
    println!("exchange: {:?}", exchange);
    let network = exchange.get_network(&config.networks);

    let http = web3::transports::Http::new(network.rpc_url()).unwrap();
    let client = web3::Web3::new(http);

    let asset = match args.value_of("asset") {
        Some(i) => config.assets.get(i),
        None => panic!("--asset not supported"),
    };
    let wallet = match args.value_of("wallet") {
        Some(w) => config.wallets.get(w),
        None => panic!("--wallet doesnt exist"),
    };

    let asset_decimals = asset.decimals(client.clone()).await;
    let remaning = asset
        .allowance(
            client.clone(),
            wallet.address(),
            exchange.as_router_address().unwrap(),
        )
        .await;
    println!(
        "allowance remaning to spend: {:?}, asset_decimals: {}",
        remaning, asset_decimals
    );
}
