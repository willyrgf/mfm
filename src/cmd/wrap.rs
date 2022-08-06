use crate::cmd;
use clap::{ArgMatches, Command};
use web3::types::U256;

//TODO: Need to review this,  may we can use swaptokenstoeth
// because in another networks the deposit does not act like another ones
pub const WRAP_COMMAND: &str = "wrap";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(WRAP_COMMAND)
    .about("Wrap a coin to a token")
    .arg(
        clap::arg!(-n --"network" <bsc> "Network to wrap coin to token")
            .required(true),
    )
    .arg(
        clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
            .required(true),
    )
    .arg(
        clap::arg!(-a --"amount" <AMMOUNT> "Amount to wrap coin into token, default: (balance-min_balance_coin)")
            .required(false)
            ,
    )
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let network = match cmd::helpers::get_network(args) {
        Some(n) => n,
        None => {
            tracing::error!("--network not found");
            panic!()
        }
    };
    let wallet = cmd::helpers::get_wallet(args);
    let client = network.get_web3_client_http();

    let wrapped_asset = network.get_wrapped_asset();
    let wrapped_asset_decimals = wrapped_asset.decimals().await;

    //TODO: doc the calc and the None case
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
                //TODO: review all panics like that
                panic!("balance: {} is not sufficient, min: {}", balance, min);
            }
            balance - min
        }
    };

    let n = wallet.nonce(client.clone()).await;
    tracing::debug!("nonce: {}", n);

    wrapped_asset.wrap(wallet, amount_in).await;
    // exchange
    //     .swap_eth_for_tokens(wallet, amount_in, amount_in, wrapped_asset)
    //     .await
}
