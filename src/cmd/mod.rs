use crate::config;
use clap::{ArgMatches, Command};

pub mod allowance;
pub mod approve;
pub mod swap;
pub mod wrap;

pub fn new() -> clap::Command<'static> {
    Command::new("mfm")
        .bin_name("mfm")
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
            Command::new("approve_spender")
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
                    clap::arg!(-v --"value" <VALUE> "Value to allow spending")
                        .required(true)
                )
        )
}

pub async fn handle_sub_commands(matches: &ArgMatches, config: &config::Config) {
    match matches.subcommand() {
        Some((wrap::WRAP_COMMAND, sub_matches)) => {
            wrap::handle_sub_commands(sub_matches, config).await;
        }
        Some((swap::SWAP_COMMAND, sub_matches)) => {
            swap::handle_sub_commands(sub_matches, config).await;
        }
        Some((allowance::ALLOWANCE_COMMAND, sub_matches)) => {
            swap::handle_sub_commands(sub_matches, config).await;
        }
        Some((approve::APPROVE_COMMAND, sub_matches)) => {
            swap::handle_sub_commands(sub_matches, config).await;
        }
        _ => panic!("command not registred"),
    }
}
