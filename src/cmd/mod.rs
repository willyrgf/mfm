use crate::config::Config;
use clap::{ArgMatches, Command};

pub mod allowance;
pub mod approve;
pub mod balances;
pub mod helpers;
pub mod quote;
pub mod rebalancer;
pub mod swap;
pub mod transaction;
pub mod unwrap;
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
        .subcommand(unwrap::generate_cmd())
        .subcommand(swap::generate_cmd())
        .subcommand(transaction::generate_cmd())
        .subcommand(allowance::generate_cmd())
        .subcommand(approve::generate_cmd())
        .subcommand(balances::generate_cmd())
        .subcommand(rebalancer::generate_cmd())
        .subcommand(yield_farm::generate_cmd())
        .subcommand(withdraw::generate_cmd())
        .subcommand(quote::generate_cmd())
}

pub async fn call_sub_commands(matches: &ArgMatches) {
    match matches.subcommand() {
        Some((wrap::WRAP_COMMAND, sub_matches)) => {
            wrap::call_sub_commands(sub_matches).await;
        }
        Some((unwrap::COMMAND, sub_matches)) => {
            unwrap::call_sub_commands(sub_matches).await;
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
        Some((quote::COMMAND, sub_matches)) => {
            quote::call_sub_commands(sub_matches).await;
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
