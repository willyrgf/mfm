use crate::{cmd, config};
use clap::ArgMatches;

pub const YIELD_FARM_COMMAND: &'static str = "yield-farm";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let yield_farm = cmd::get_yield_farm(args, config);

    let network = yield_farm.get_network(config);
    let client = network.get_web3_client_http();

    //let gas_price = client.eth().gas_price().await.unwrap();
    log::debug!("yield_farm: {:?}", yield_farm);

    let pending_rewards = yield_farm.get_pending_rewards(config, client.clone()).await;
    log::debug!("yield_farm pending rewards: {:?}", pending_rewards);

    yield_farm.harvest(config, client.clone()).await;
}
