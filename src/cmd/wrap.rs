use crate::{cmd, config};
use clap::ArgMatches;
use web3::types::U256;

pub const WRAP_COMMAND: &'static str = "wrap";

pub async fn handle_sub_commands(args: &ArgMatches, config: &config::Config) {
    let (_, client, wallet, _, network) =
        cmd::get_exchange_client_wallet_asset_network(args, config);

    let wrapped_asset = network.get_wrapped_asset(&config.assets);
    let wrapped_asset_decimals = wrapped_asset.decimals(client.clone()).await;

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
    log::debug!("nonce: {}", n);

    let gas_price = client.eth().gas_price().await.unwrap();

    wrapped_asset
        .wrap(client.clone(), wallet, amount_in, gas_price)
        .await;
}
