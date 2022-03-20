use crate::{cmd, config};
use clap::ArgMatches;
use prettytable::{cell, row, Table};

pub const YIELD_FARM_COMMAND: &str = "yield-farm";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let yield_farm = cmd::get_yield_farm(args, config);
    let network = yield_farm.get_network(config);
    let client = network.get_web3_client_http();
    let yield_farm_asset = yield_farm.get_asset(config);
    let yield_farm_asset_decimals = yield_farm_asset.decimals(client.clone()).await;

    let pending_rewards = yield_farm.get_pending_rewards(config, client.clone()).await;
    let min_rewards_required = yield_farm.get_min_rewards_required_u256(yield_farm_asset_decimals);
    log::info!("yield_farm pending rewards: {:?}", pending_rewards);
    log::info!(
        "yield_farm min rewards required: {:?}",
        min_rewards_required
    );

    if pending_rewards >= min_rewards_required {
        log::info!("harvesting yield farm: {:?}", yield_farm);
        yield_farm.harvest(config, client.clone()).await;
    }

    let mut table = Table::new();

    table.add_row(row!["Amount in pending rewards", pending_rewards]);
    table.printstd();
}
