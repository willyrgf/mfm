use core::time;
use std::thread;

use crate::config::{
    asset::Asset, exchange::Exchange, network::Network, rebalancer::Rebalancer, wallet::Wallet,
    withdraw_wallet::WithdrawWallet, yield_farm::YieldFarm, Config,
};
use clap::{ArgMatches, Command};
use web3::{
    transports::Http,
    types::{TransactionReceipt, H256, U256},
};

pub mod allowance;
pub mod approve;
pub mod balances;
pub mod rebalancer;
pub mod swap;
pub mod transaction;
pub mod withdraw;
pub mod wrap;
pub mod yield_farm;

pub const CLI_NAME: &str = "mfm";

pub fn new() -> clap::Command<'static> {
    Command::new(CLI_NAME)
        .bin_name(CLI_NAME)
        .arg(
            clap::arg!(-c - -config_filename <PATH> "Config file path")
                .required(false)
                .default_value("config.yaml"),
        )
        .subcommand_required(true)
        .subcommand(wrap::generate_cmd())
        .subcommand(swap::generate_cmd())
        .subcommand(transaction::generate_cmd())
        .subcommand(allowance::generate_cmd())
        .subcommand(approve::generate_cmd())
        .subcommand(balances::generate_cmd())
        .subcommand(rebalancer::generate_cmd())
        .subcommand(yield_farm::generate_cmd())
        .subcommand(withdraw::generate_cmd())
}

pub async fn call_sub_commands(matches: &ArgMatches) {
    match matches.subcommand() {
        Some((wrap::WRAP_COMMAND, sub_matches)) => {
            wrap::call_sub_commands(sub_matches).await;
        }
        Some((swap::SWAP_COMMAND, sub_matches)) => {
            swap::call_sub_commands(sub_matches).await;
        }
        Some((allowance::ALLOWANCE_COMMAND, sub_matches)) => {
            allowance::call_sub_commands(sub_matches).await;
        }
        Some((approve::APPROVE_COMMAND, sub_matches)) => {
            approve::call_sub_commands(sub_matches).await;
        }
        Some((balances::BALANCES_COMMAND, sub_matches)) => {
            balances::call_sub_commands(sub_matches).await;
        }
        Some((rebalancer::REBALANCER_COMMAND, sub_matches)) => {
            rebalancer::call_sub_commands(sub_matches).await;
        }
        Some((transaction::TRANSACTION_COMMAND, sub_matches)) => {
            transaction::call_sub_commands(sub_matches).await;
        }
        Some((yield_farm::YIELD_FARM_COMMAND, sub_matches)) => {
            yield_farm::call_sub_commands(sub_matches).await;
        }
        Some((withdraw::WITHDRAW_COMMAND, sub_matches)) => {
            withdraw::call_sub_commands(sub_matches).await;
        }
        _ => panic!("command not registred"),
    }
}

pub async fn run(cmd: clap::Command<'static>) {
    let cmd_matches = cmd.get_matches();
    log::debug!("matches: {:?}", cmd_matches);

    match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    call_sub_commands(&cmd_matches).await
}

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

pub async fn wait_receipt(client: web3::Web3<Http>, tx_address: H256) -> TransactionReceipt {
    loop {
        match client.eth().transaction_receipt(tx_address).await {
            Ok(Some(receipt)) => return receipt,
            Ok(None) => {
                thread::sleep(time::Duration::from_secs(5));
                continue;
            }
            Err(e) => {
                log::error!("wait_receipt() err: {:?}", e);
                panic!()
            }
        }
    }
}
