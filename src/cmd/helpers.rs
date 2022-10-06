use crate::{
    asset::Asset,
    config::{
        exchange::Exchange, network::Network, wallet::Wallet, withdraw_wallet::WithdrawWallet,
        Config,
    },
    rebalancer::config::RebalancerConfig,
};
use anyhow::Context;
use clap::ArgMatches;
use web3::types::U256;

//TODO: add constants to all keys in value_of

#[tracing::instrument(name = "get exchange from command args")]
pub fn get_exchange(args: &ArgMatches) -> Result<&Exchange, anyhow::Error> {
    match args.get_one::<String>("exchange") {
        Some(n) => {
            let network = Config::global()
                .exchanges
                .get(n)
                .context("exchange not found")?;
            Ok(network)
        }
        None => Err(anyhow::anyhow!("--exchange is required")),
    }
}

#[tracing::instrument(name = "get network from command args")]
pub fn get_network(args: &ArgMatches) -> Result<&Network, anyhow::Error> {
    match args.get_one::<String>("network") {
        Some(n) => {
            let network = Config::global()
                .networks
                .get(n)
                .context("network not found")?;
            Ok(network)
        }
        None => Err(anyhow::anyhow!("--network is required")),
    }
}

#[tracing::instrument(name = "get wallet from command args")]
pub fn get_wallet(args: &ArgMatches) -> Result<&Wallet, anyhow::Error> {
    let config = Config::global();
    match args.get_one::<String>("wallet") {
        Some(n) => {
            let wallet = config.wallets.get(n).context("wallet not found")?;
            Ok(wallet)
        }
        None => Err(anyhow::anyhow!("--wallet is required")),
    }
}

pub fn get_asset_in_network_from_args(args: &ArgMatches, network_id: &str) -> Asset {
    match args.get_one::<String>("asset") {
        Some(a) => Config::global()
            .assets
            .find_by_name_and_network(a, network_id)
            .unwrap(),
        None => panic!("--asset not supported"),
    }
}

pub fn get_quoted_asset_in_network_from_args(
    args: &ArgMatches,
    network_id: &str,
) -> Result<Asset, anyhow::Error> {
    let config = Config::global();
    match args.get_one::<String>("quoted-asset") {
        Some(a) => config.assets.find_by_name_and_network(a, network_id),
        None => Err(anyhow::anyhow!("--quoted-asset is required")),
    }
}

pub fn get_force_harvest(args: &ArgMatches) -> bool {
    match args.get_one::<bool>("force-harvest") {
        Some(a) => *a,
        None => panic!("--force-harvest supported"),
    }
}

pub fn get_txn_id(args: &ArgMatches) -> &str {
    match args.get_one::<String>("txn_id") {
        Some(a) => a,
        None => panic!("--txn_id not supported"),
    }
}

#[tracing::instrument(name = "get amount from command args")]
pub fn get_amount(args: &ArgMatches, asset_decimals: u8) -> Result<U256, anyhow::Error> {
    //TODO: need to review usage from i128
    match args.get_one::<String>("amount") {
        Some(a) => {
            //TODO: move it to a helper function
            let q = a
                .parse::<f64>()
                .map_err(|e| anyhow::anyhow!("cant parse the amount value to f64, got {:?}", e))?;
            let qe = (q * 10_f64.powf(asset_decimals.into())) as i128;
            Ok(U256::from(qe))
        }
        None => Err(anyhow::anyhow!("--amount is required")),
    }
}

#[tracing::instrument(name = "get slippage from command args")]
pub fn get_slippage(args: &ArgMatches, asset_decimals: u8) -> Result<U256, anyhow::Error> {
    //TODO: review u128
    match args.get_one::<f64>("slippage") {
        Some(q) => {
            // let q = a.parse::<f64>().unwrap();
            let qe = ((q / 100.0) * 10_f64.powf(asset_decimals.into())) as u128;
            Ok(U256::from(qe))
        }
        None => Err(anyhow::anyhow!("--slippage is required")),
    }
}

#[tracing::instrument(name = "get input token in network from command args")]
pub fn get_token_input_in_network_from_args(
    args: &ArgMatches,
    network_id: &str,
) -> Result<Asset, anyhow::Error> {
    match args.get_one::<String>("token_input") {
        Some(i) => Config::global()
            .assets
            .find_by_name_and_network(i, network_id),
        None => Err(anyhow::anyhow!(
            "--token_input not supported on current network"
        )),
    }
}

#[tracing::instrument(name = "get output token in network from command args")]
pub fn get_token_output_in_network_from_args(
    args: &ArgMatches,
    network_id: &str,
) -> Result<Asset, anyhow::Error> {
    match args.get_one::<String>("token_output") {
        Some(i) => Config::global()
            .assets
            .find_by_name_and_network(i, network_id),
        None => Err(anyhow::anyhow!(
            "--token_output not supported on current network"
        )),
    }
}

pub fn get_rebalancer(args: &ArgMatches) -> RebalancerConfig {
    let config = Config::global();
    match args.get_one::<String>("name") {
        Some(i) => match config.rebalancers.clone() {
            Some(rebalancers) => rebalancers.get(i).clone(),
            None => {
                tracing::error!("get_rebalancer() rebalancers is not configured");
                panic!()
            }
        },
        None => panic!("--name not supported"),
    }
}

pub fn get_withdraw_wallet(args: &ArgMatches) -> WithdrawWallet {
    let config = Config::global();
    match args.get_one::<String>("withdraw-wallet") {
        Some(w) => match config.withdraw_wallets.clone() {
            Some(withdraw_wallets) => withdraw_wallets.get(w).clone(),
            None => {
                tracing::error!("get_withdraw_wallet() withdraw_wallet is not configured");
                panic!()
            }
        },
        None => panic!("--withdraw-wallet not supported"),
    }
}

#[tracing::instrument(name = "get hide zero from command args")]
pub fn get_hide_zero(args: &ArgMatches) -> bool {
    match args.get_one::<String>("hide-zero") {
        Some(b) => b.parse().unwrap_or(false),
        _ => false,
    }
}

mod test {
    use clap::{ArgMatches, Command};

    fn _get_arg_matches(cmd: Command, argv: &'static str) -> ArgMatches {
        cmd.try_get_matches_from(argv.split(' ').collect::<Vec<_>>())
            .unwrap()
    }

    #[test]
    fn get_hide_zero_working() {
        use super::get_hide_zero;

        let cmd = Command::new("app").arg(
            clap::arg!(-z --"hide-zero" <TRUE_FALSE> "hide zero balances")
                .required(false)
                .default_value("false"),
        );

        let test_cases = [
            ("app --hide-zero=true", true),
            ("app --hide-zero=false", false),
            ("app --hide-zero=invalid-false", false),
        ];

        for (argv, expected) in test_cases {
            let arg_matches = _get_arg_matches(cmd.clone(), argv);
            assert_eq!(get_hide_zero(&arg_matches), expected);
        }
    }

    #[test]
    fn get_network_working() {
        use super::get_network;
        use crate::config::Config;

        Config::from_file("test_config.yaml").unwrap();

        let cmd = Command::new("app")
            .arg(clap::arg!(-n --"network" <bsc> "Network to wrap coin to token").required(true));

        let test_cases = [
            ("app --network=bsc", true),
            ("app --network=false", false),
            ("app --network=invalid-false", false),
        ];

        for (argv, expected) in test_cases {
            let arg_matches = _get_arg_matches(cmd.clone(), argv);
            assert_eq!(get_network(&arg_matches).is_ok(), expected);
        }
    }
}
