use crate::{
    cmd,
    config::{self, yield_farm::YieldFarm},
};
use clap::{ArgMatches, Command};
use prettytable::{cell, row, Table};

pub const YIELD_FARM_COMMAND: &str = "yield-farm";
pub const YIELD_FARM_RUN_COMMAND: &str = "run";
pub const YIELD_FARM_INFO_COMMAND: &str = "info";

pub fn generate_info_cmd<'a>() -> Command<'a> {
    Command::new(YIELD_FARM_INFO_COMMAND)
        .about("Get info about all or one yield farm")
        .arg(
            clap::arg!(-y --"yield-farm" <YIELD_FARM_NAME> "Yield farm name in config file")
                .required(false),
        )
}

pub fn generate_run_cmd<'a>() -> Command<'a> {
    Command::new(YIELD_FARM_RUN_COMMAND)
        .about("Run harvest on all or one yield farm")
        .arg(
            clap::arg!(-y --"yield-farm" <YIELD_FARM_NAME> "Yield farm name in config file")
                .required(false),
        )
        .arg(
            clap::arg!(-f --"force-harvest" <true_or_false> "Will ignore the min rewards required")
                .required(false)
                .default_value("false"),
        )
}

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(YIELD_FARM_COMMAND)
        .about("Haverst some YieldFarm")
        .subcommand(generate_info_cmd())
        .subcommand(generate_run_cmd())
}

pub fn get_farms_to_look<'a>(
    args: &'a ArgMatches,
    config: &'a config::Config,
) -> Vec<&'a YieldFarm> {
    let farms_to_look: Vec<&YieldFarm> = match args.value_of("yield-farm") {
        Some(y) => vec![config.yield_farms.get(y)],
        None => config
            .yield_farms
            .hashmap()
            .iter()
            .map(|(_k, v)| v)
            .collect::<Vec<&YieldFarm>>(),
    };

    farms_to_look
}

pub async fn call_info_cmd(args: &ArgMatches, config: &config::Config) {
    let mut table = Table::new();
    table.add_row(row![
        "Farm",
        "Pending rewards",
        "Asset",
        "Decimals",
        "Min rewards required"
    ]);
    for yield_farm in get_farms_to_look(args, config) {
        let network = yield_farm.get_network(config);
        let client = network.get_web3_client_http();
        let yield_farm_asset = yield_farm.get_asset(config);
        let yield_farm_asset_decimals = yield_farm_asset.decimals(client.clone()).await;

        let pending_rewards = yield_farm.get_pending_rewards(config, client.clone()).await;
        let min_rewards_required =
            yield_farm.get_min_rewards_required_u256(yield_farm_asset_decimals);
        table.add_row(row![
            yield_farm.name(),
            pending_rewards,
            yield_farm_asset.name(),
            yield_farm_asset_decimals,
            min_rewards_required
        ]);
    }
    table.printstd();
}

pub async fn call_run_cmd(args: &ArgMatches, config: &config::Config) {
    let mut table = Table::new();
    let force_harvest = cmd::get_force_harvest(args);
    table.add_row(row![
        "Harvested",
        "Farm",
        "Pending rewards",
        "Asset",
        "Decimals",
        "Min rewards required"
    ]);
    for yield_farm in get_farms_to_look(args, config) {
        let network = yield_farm.get_network(config);
        let client = network.get_web3_client_http();
        let yield_farm_asset = yield_farm.get_asset(config);
        let yield_farm_asset_decimals = yield_farm_asset.decimals(client.clone()).await;

        let pending_rewards = yield_farm.get_pending_rewards(config, client.clone()).await;
        let min_rewards_required =
            yield_farm.get_min_rewards_required_u256(yield_farm_asset_decimals);
        log::info!("yield_farm pending rewards: {:?}", pending_rewards);
        log::info!(
            "yield_farm min rewards required: {:?}",
            min_rewards_required
        );

        let can_harvest = pending_rewards >= min_rewards_required;
        if can_harvest || force_harvest {
            log::info!("harvesting yield farm: {:?}", yield_farm);
            yield_farm.harvest(config, client.clone()).await;
        }

        table.add_row(row![
            (can_harvest || force_harvest),
            yield_farm.name(),
            pending_rewards,
            yield_farm_asset.name(),
            yield_farm_asset_decimals,
            min_rewards_required
        ]);
    }
    table.printstd();
}

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    match args.subcommand() {
        Some((YIELD_FARM_RUN_COMMAND, sub_args)) => {
            call_run_cmd(sub_args, config).await;
        }

        Some((YIELD_FARM_INFO_COMMAND, sub_args)) => {
            call_info_cmd(sub_args, config).await;
        }
        _ => {
            log::error!("no sub cmd found");
            panic!("sub_cmd_not_found");
        }
    }
}
