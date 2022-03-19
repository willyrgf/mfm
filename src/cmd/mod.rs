use core::time;
use std::thread;

use crate::config::{
    asset::Asset, exchange::Exchange, network::Network, rebalancer::Rebalancer, wallet::Wallet,
    withdraw_wallet::WithdrawWallet, Config,
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

pub const CLI_NAME: &'static str = "mfm";

pub fn new() -> clap::Command<'static> {
    Command::new(CLI_NAME)
        .bin_name(CLI_NAME)
        .arg(
            clap::arg!(-c - -config_filename <PATH> "Config file path")
                .required(false)
                .default_value("config.yaml"),
        )
        .subcommand_required(true)
        .subcommand(
            Command::new(wrap::WRAP_COMMAND)
                .about("Wrap a coin to a token")
                .arg(
                    clap::arg!(--"network" <bsc> "Network to wrap coin to token")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"amount" <AMMOUNT> "Amount to wrap coin into token, default: (balance-min_balance_coin)")
                        .required(false)
                        ,
                ),
        )
        .subcommand(
            Command::new(swap::SWAP_COMMAND)
                .about("Swap Tokens for Tokens supporting fees on transfer")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                        .required(false)
                )
                .arg(
                    clap::arg!(-i --"token_input" <TOKEN_INPUT> "Asset of input token")
                        .required(false)
                )
                .arg(
                    clap::arg!(-o --"token_output" <TOKEN_OUTPUT> "Asset of output token")
                        .required(false)
                )
                .arg(
                    clap::arg!(-s --"slippage" <SLIPPAGE> "Slippage (default 0.5)")
                        .required(false)
                        .default_value("0.5")
                )
        )
        .subcommand(
            Command::new("transaction")
                .about("Get transaction details")
                .arg(
                    clap::arg!(-n --"network" <bsc> "Network to search transaction")
                        .required(true),
                )
        )
        .subcommand(
            Command::new("allowance")
                .about("Get allowance for an token")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"asset" <ASSET> "Asset to check allowance")
                        .required(true)
                )
        )
        .subcommand(
            Command::new("approve")
                .about("Approve token spending (needed to swap tokens)")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router as spender")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"asset" <ASSET> "Asset to approve spender")
                        .required(true)
                )
                .arg(
                    clap::arg!(-v --"amount" <VALUE> "Amount to allow spending")
                        .required(true)
                )
        )
        .subcommand(
            Command::new("balances")
                .about("Check balances from all assets listed on config")
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
        )
        .subcommand(
            Command::new("rebalancer")
                .about("Fires a rebalancer")
                .arg(
                    clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                        .required(true),
                )
        )
        .subcommand(
                    Command::new(withdraw::WITHDRAW_COMMAND)
                        .about("Withdraw to a wallet")
                        .arg(
                            clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                                .required(true),
                        )
                .arg(
                    clap::arg!(-n --"network" <bsc> "Network to wrap coin to token")
                        .required(true),
                )
                .arg(
                    clap::arg!(-t --"withdraw-wallet" <WITHDRAW_WALLET_NAME> "Withdraw wallet to receive the transfer")
                        .required(true),
                )
                        .arg(
                            clap::arg!(-a --"asset" <ASSET> "Asset to withdraw")
                                .required(true)
                        )
                        .arg(
                            clap::arg!(-v --"amount" <VALUE> "Amount to withdraw")
                                .required(true)
                        )
                )
}

pub async fn call_sub_commands(matches: &ArgMatches, config: &Config) {
    match matches.subcommand() {
        Some((wrap::WRAP_COMMAND, sub_matches)) => {
            wrap::call_sub_commands(sub_matches, config).await;
        }
        Some((swap::SWAP_COMMAND, sub_matches)) => {
            swap::call_sub_commands(sub_matches, config).await;
        }
        Some((allowance::ALLOWANCE_COMMAND, sub_matches)) => {
            allowance::call_sub_commands(sub_matches, config).await;
        }
        Some((approve::APPROVE_COMMAND, sub_matches)) => {
            approve::call_sub_commands(sub_matches, config).await;
        }
        Some((balances::BALANCES_COMMAND, sub_matches)) => {
            balances::call_sub_commands(sub_matches, config).await;
        }
        Some((rebalancer::REBALANCER_COMMAND, sub_matches)) => {
            rebalancer::call_sub_commands(sub_matches, config).await;
        }
        Some((transaction::TRANSACTION_COMMAND, sub_matches)) => {
            transaction::call_sub_commands(sub_matches, config).await;
        }
        Some((withdraw::WITHDRAW_COMMAND, sub_matches)) => {
            withdraw::call_sub_commands(sub_matches, config).await;
        }
        _ => panic!("command not registred"),
    }
}

pub async fn run(cmd: clap::Command<'static>) {
    let cmd_matches = cmd.get_matches();
    log::debug!("matches: {:?}", cmd_matches);

    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    call_sub_commands(&cmd_matches, &config).await
}

//TODO: add constants to all keys in value_of
//

pub fn get_exchange<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a Exchange {
    match args.value_of("exchange") {
        Some(n) => config.exchanges.get(n),
        None => panic!("--exchange not supported"),
    }
}

pub fn get_network<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a Network {
    match args.value_of("network") {
        Some(n) => config.networks.get(n),
        None => panic!("--network not supported"),
    }
}

pub fn get_wallet<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a Wallet {
    match args.value_of("wallet") {
        Some(w) => config.wallets.get(w),
        None => panic!("--wallet doesnt exist"),
    }
}

pub fn get_asset<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a Asset {
    match args.value_of("asset") {
        Some(a) => config.assets.get(a),
        None => panic!("--asset not supported"),
    }
}

pub fn get_txn_id<'a>(args: &'a ArgMatches) -> &'a str {
    match args.value_of("txn_id") {
        Some(a) => a,
        None => panic!("--txn_id not supported"),
    }
}

pub fn get_amount<'a>(args: &'a ArgMatches, asset_decimals: u8) -> U256 {
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

pub fn get_token_input<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a Asset {
    match args.value_of("token_input") {
        Some(i) => config.assets.get(i),
        None => panic!("--token_input not supported"),
    }
}

pub fn get_token_output<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a Asset {
    match args.value_of("token_output") {
        Some(i) => config.assets.get(i),
        None => panic!("--token_output not supported"),
    }
}

pub fn get_slippage<'a>(args: &'a ArgMatches, asset_decimals: u8) -> U256 {
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

pub fn get_rebalancer<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a Rebalancer {
    match args.value_of("name") {
        Some(i) => config.rebalancers.get(i),
        None => panic!("--name not supported"),
    }
}

pub fn get_withdraw_wallet<'a>(args: &'a ArgMatches, config: &'a Config) -> &'a WithdrawWallet {
    match args.value_of("withdraw-wallet") {
        Some(w) => config.withdraw_wallets.get(w),
        None => panic!("--withdraw-wallet not supported"),
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
