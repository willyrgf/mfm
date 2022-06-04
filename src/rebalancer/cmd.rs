use crate::{
    cmd,
    config::Config,
    rebalancer::{self, config::Strategy, generate_asset_rebalances},
    shared,
    shared::blockchain_utils::{amount_in_quoted, display_amount_to_float},
};
use clap::{ArgMatches, Command};
use prettytable::{cell, row, Table};
use web3::types::U256;

pub const REBALANCER_COMMAND: &str = "rebalancer";
pub const REBALANCER_RUN_COMMAND: &str = "run";
pub const REBALANCER_INFO_COMMAND: &str = "info";

pub fn generate_info_cmd() -> Command<'static> {
    Command::new(REBALANCER_INFO_COMMAND)
        .about("Infos about rebalancer")
        .arg(
            clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                .required(true),
        )
}

pub fn generate_run_cmd() -> Command<'static> {
    Command::new(REBALANCER_RUN_COMMAND)
        .about("Run rebalancer")
        .arg(
            clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                .required(true),
        )
}

pub fn generate_cmd() -> Command<'static> {
    Command::new(REBALANCER_COMMAND)
        .about("Fires a rebalancer")
        .subcommand(generate_run_cmd())
        .subcommand(generate_info_cmd())
}

pub async fn call_sub_commands(args: &ArgMatches) {
    match args.subcommand() {
        Some((REBALANCER_RUN_COMMAND, sub_args)) => {
            cmd_run(sub_args).await;
        }

        Some((REBALANCER_INFO_COMMAND, sub_args)) => {
            cmd_info(sub_args).await;
        }
        _ => {
            log::error!("no sub cmd found");
            panic!("sub_cmd_not_found");
        }
    }
}

async fn cmd_run(args: &ArgMatches) {
    let config = cmd::helpers::get_rebalancer(args);
    log::debug!(
        "rebalancer::cmd::call_sub_commands(): rebalancer_config: {:?}",
        config
    );

    rebalancer::validate(config).await;

    match config.strategy() {
        Strategy::FullParking => {
            log::debug!("rebalancer::cmd::call_sub_commands() Strategy::FullParking");
            rebalancer::run_full_parking(config).await;
        }
        Strategy::DiffParking => {
            log::debug!("rebalancer::cmd::call_sub_commands() Strategy::DiffParking");
            rebalancer::run_diff_parking(config).await;
        }
    }
}

async fn cmd_info(args: &ArgMatches) {
    log::debug!("cmd_info()");

    let hide_zero = true;
    let config = cmd::helpers::get_rebalancer(args);
    let asset_quoted = &config.get_quoted_asset();

    let mut table = Table::new();
    table.add_row(row![
        "Network",
        "Asset",
        "Balance",
        "Quoted In",
        "Balance in quoted",
        "Amount to trade",
        "Quoted amount to trade"
    ]);

    generate_asset_rebalances(config)
        .await
        .iter()
        .for_each(|ar| {
            let balance_of = ar.asset_balances.balance;
            let asset = ar.asset_balances.asset.clone();
            let decimals = ar.asset_balances.asset_decimals;
            let amount_in_quoted = ar.asset_balances.quoted_balance;
            let asset_quoted_decimals = ar.asset_balances.quoted_asset_decimals;

            let sign = match ar.kind.as_str() {
                "to_parking" => "-",
                _ => "+",
            };

            if !(hide_zero && balance_of == U256::from(0_i32)) {
                table.add_row(row![
                    asset.network_id(),
                    asset.name(),
                    display_amount_to_float(balance_of, decimals),
                    config.quoted_in(),
                    display_amount_to_float(amount_in_quoted, asset_quoted_decimals),
                    //TODO: put it in a method of ar
                    format!(
                        "{}{}",
                        sign,
                        display_amount_to_float(ar.asset_amount_to_trade, decimals)
                    ),
                    format!(
                        "{}{}",
                        sign,
                        display_amount_to_float(ar.quoted_amount_to_trade, asset_quoted_decimals)
                    )
                ]);
            }
        });

    table.printstd();
}
