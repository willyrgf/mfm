use crate::config::{
    asset::Asset, exchange::Exchange, network::Network, rebalancer::Rebalancer, wallet::Wallet,
    withdraw_wallet::WithdrawWallet, yield_farm::YieldFarm, Config,
};
use clap::ArgMatches;
use web3::types::U256;

//TODO: add constants to all keys in value_of
//

pub fn get_exchange<'a>(args: &'a ArgMatches) -> &'a Exchange {
    let config = Config::global();
    match args.value_of("exchange") {
        Some(n) => config.exchanges.get(n),
        None => panic!("--exchange not supported"),
    }
}

pub fn get_network<'a>(args: &'a ArgMatches) -> &'a Network {
    let config = Config::global();
    match args.value_of("network") {
        Some(n) => config.networks.get(n),
        None => panic!("--network not supported"),
    }
}

pub fn get_wallet<'a>(args: &'a ArgMatches) -> &'a Wallet {
    let config = Config::global();
    match args.value_of("wallet") {
        Some(w) => config.wallets.get(w),
        None => panic!("--wallet doesnt exist"),
    }
}

pub fn get_asset<'a>(args: &'a ArgMatches) -> &'a Asset {
    let config = Config::global();
    match args.value_of("asset") {
        Some(a) => config.assets.get(a),
        None => panic!("--asset not supported"),
    }
}

pub fn get_asset_in_network_from_args<'a>(args: &'a ArgMatches, network_id: &str) -> &'a Asset {
    match args.value_of("asset") {
        Some(a) => Config::global()
            .assets
            .find_by_name_and_network(a, network_id)
            .unwrap(),
        None => panic!("--asset not supported"),
    }
}

pub fn get_quoted_asset_in_network_from_args<'a>(
    args: &'a ArgMatches,
    network_id: &str,
) -> Option<&'a Asset> {
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

pub fn get_token_input<'a>(args: &'a ArgMatches) -> &'a Asset {
    let config = Config::global();
    match args.value_of("token_input") {
        Some(i) => config.assets.get(i),
        None => panic!("--token_input not supported"),
    }
}

pub fn get_token_input_in_network_from_args<'a>(
    args: &'a ArgMatches,
    network_id: &str,
) -> &'a Asset {
    match args.value_of("token_input") {
        Some(i) => Config::global()
            .assets
            .find_by_name_and_network(i, network_id)
            .unwrap(),
        None => panic!("--token_input not supported on current network"),
    }
}

pub fn get_token_output<'a>(args: &'a ArgMatches) -> &'a Asset {
    let config = Config::global();
    match args.value_of("token_output") {
        Some(i) => config.assets.get(i),
        None => panic!("--token_output not supported"),
    }
}

pub fn get_token_output_in_network_from_args<'a>(
    args: &'a ArgMatches,
    network_id: &str,
) -> &'a Asset {
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

pub fn get_rebalancer<'a>(args: &'a ArgMatches) -> &'a Rebalancer {
    let config = Config::global();
    match args.value_of("name") {
        Some(i) => config.rebalancers.get(i),
        None => panic!("--name not supported"),
    }
}

pub fn get_withdraw_wallet<'a>(args: &'a ArgMatches) -> &'a WithdrawWallet {
    let config = Config::global();
    match args.value_of("withdraw-wallet") {
        Some(w) => config.withdraw_wallets.get(w),
        None => panic!("--withdraw-wallet not supported"),
    }
}

pub fn get_yield_farm(args: &ArgMatches) -> &YieldFarm {
    let config = Config::global();
    match args.value_of("yield-farm") {
        Some(w) => config.yield_farms.get(w),
        None => panic!("--yield-farm not supported"),
    }
}
