use crate::config;
use clap::ArgMatches;
use web3::types::U256;

pub const APPROVE_COMMAND: &'static str = "approve";

pub async fn handle_sub_commands(args: &ArgMatches, config: &config::Config) {
    let exchange = match args.value_of("exchange") {
        Some(n) => config.exchanges.get(n),
        None => panic!("--exchange not supported"),
    };
    log::debug!("exchange: {:?}", exchange);
    let network = exchange.get_network(&config.networks);

    let http = web3::transports::Http::new(network.rpc_url()).unwrap();
    let client = web3::Web3::new(http);

    let asset = match args.value_of("asset") {
        Some(i) => config.assets.get(i),
        None => panic!("--asset not supported"),
    };
    let asset_decimals = asset.decimals(client.clone()).await;
    let wallet = match args.value_of("wallet") {
        Some(w) => config.wallets.get(w),
        None => panic!("--wallet doesnt exist"),
    };
    //#TODO: need to review usage from i128
    let amount_in = match args.value_of("value") {
        Some(a) => {
            let q = a.parse::<f64>().unwrap();
            let qe = (q * 10_f64.powf(asset_decimals.into())) as i128;
            U256::from(qe)
        }
        None => panic!("--value is missing"),
    };

    let gas_price = client.eth().gas_price().await.unwrap();
    log::debug!("amount_int: {:?}", amount_in);

    asset
        .approve_spender(
            client.clone(),
            gas_price,
            wallet,
            exchange.as_router_address().unwrap(),
            amount_in,
        )
        .await;
    let remaning = asset
        .allowance(
            client.clone(),
            wallet.address(),
            exchange.as_router_address().unwrap(),
        )
        .await;
    log::debug!(
        "approved_spender allowance remaning to spend: {:?}, asset_decimals: {}",
        remaning,
        asset_decimals
    );
}
