use web3::{contract::Options, transports::Http, types::U256};

use crate::config::Config;

use super::position_stake_manager;
use super::YieldFarm;

const POOL_ID: i32 = 2;

pub async fn get_pending_rewards(
    config: &Config,
    yield_farm: &YieldFarm,
    client: web3::Web3<Http>,
) -> U256 {
    position_stake_manager::get_pending_rewards(POOL_ID, config, yield_farm, client).await
}

pub async fn harvest(config: &Config, yield_farm: &YieldFarm, client: web3::Web3<Http>) {
    position_stake_manager::harvest(POOL_ID, config, yield_farm, client).await
}
