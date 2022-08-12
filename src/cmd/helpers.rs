use crate::{
    asset::Asset,
    config::{
        exchange::Exchange, network::Network, wallet::Wallet, withdraw_wallet::WithdrawWallet,
        yield_farm::YieldFarm, Config,
    },
    rebalancer::config::RebalancerConfig,
};
use clap::ArgMatches;
use web3::types::U256;

//TODO: add constants to all keys in value_of

pub fn get_exchange(args: &ArgMatches) -> &Exchange {
    let config = Config::global();
    match args.value_of("exchange") {
        Some(n) => config.exchanges.get(n),
        None => panic!("--exchange not supported"),
    }
}

pub fn get_network(args: &ArgMatches) -> Option<&Network> {
    match args.value_of("network") {
        Some(n) => Config::global().networks.get(n),
        None => None,
    }
}

pub fn get_wallet(args: &ArgMatches) -> &Wallet {
    let config = Config::global();
    match args.value_of("wallet") {
        Some(w) => config.wallets.get(w),
        None => panic!("--wallet doesnt exist"),
    }
}

pub fn get_asset_in_network_from_args(args: &ArgMatches, network_id: &str) -> Asset {
    match args.value_of("asset") {
        Some(a) => Config::global()
            .assets
            .find_by_name_and_network(a, network_id)
            .unwrap(),
        None => panic!("--asset not supported"),
    }
}

pub fn get_quoted_asset_in_network_from_args(args: &ArgMatches, network_id: &str) -> Option<Asset> {
    let config = Config::global();
    match args.value_of("quoted-asset") {
        Some(a) => config.assets.find_by_name_and_network(a, network_id),
        None => None,
    }
}

pub fn get_force_harvest(args: &ArgMatches) -> bool {
    match args.value_of("force-harvest") {
        Some(a) => a.parse::<bool>().unwrap(),
        None => panic!("--force-harvest supported"),
    }
}

pub fn get_txn_id(args: &ArgMatches) -> &str {
    match args.value_of("txn_id") {
        Some(a) => a,
        None => panic!("--txn_id not supported"),
    }
}

pub fn get_amount(args: &ArgMatches, asset_decimals: u8) -> U256 {
    //TODO: need to review usage from i128
    match args.value_of("amount") {
        Some(a) => {
            //TODO: move it to a helper function
            let q = a.parse::<f64>().unwrap();
            let qe = (q * 10_f64.powf(asset_decimals.into())) as i128;
            U256::from(qe)
        }
        None => panic!("--amount not supported"),
    }
}

pub fn get_token_input_in_network_from_args(args: &ArgMatches, network_id: &str) -> Asset {
    match args.value_of("token_input") {
        Some(i) => Config::global()
            .assets
            .find_by_name_and_network(i, network_id)
            .unwrap(),
        None => panic!("--token_input not supported on current network"),
    }
}

pub fn get_token_output_in_network_from_args(args: &ArgMatches, network_id: &str) -> Asset {
    match args.value_of("token_output") {
        Some(i) => Config::global()
            .assets
            .find_by_name_and_network(i, network_id)
            .unwrap(),
        None => panic!("--token_output not supported on current network"),
    }
}

pub fn get_slippage(args: &ArgMatches, asset_decimals: u8) -> U256 {
    //TODO: review u128
    match args.value_of("slippage") {
        Some(a) => {
            let q = a.parse::<f64>().unwrap();
            let qe = ((q / 100.0) * 10_f64.powf(asset_decimals.into())) as u128;
            U256::from(qe)
        }
        None => panic!("missing slippage"),
    }
}

pub fn get_rebalancer(args: &ArgMatches) -> RebalancerConfig {
    let config = Config::global();
    match args.value_of("name") {
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
    match args.value_of("withdraw-wallet") {
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

pub fn get_yield_farm(args: &ArgMatches) -> YieldFarm {
    let config = Config::global();
    match args.value_of("yield-farm") {
        Some(y) => match config.yield_farms.clone() {
            Some(yield_farms) => yield_farms.get(y).clone(),
            None => {
                tracing::error!("get_yield_farm() yield_farm is not configured");
                panic!()
            }
        },
        None => panic!("--yield-farm not supported"),
    }
}

pub fn get_hide_zero(args: &ArgMatches) -> bool {
    match args.value_of("hide-zero") {
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

        let cmd = crate::balances::cmd::generate();

        let test_cases = [
            ("balances --wallet zero --hide-zero=true", true),
            ("balances --wallet zero --hide-zero=false", false),
            ("balances --wallet zero --hide-zero=invalid-false", false),
        ];

        for (argv, expected) in test_cases {
            let arg_matches = _get_arg_matches(cmd.clone(), argv);
            assert_eq!(get_hide_zero(&arg_matches), expected);
        }
    }
}
