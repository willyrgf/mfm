use web3::types::U256;

use super::position_stake_manager;
use super::YieldFarm;

const POOL_ID: i32 = 2;

pub async fn get_pending_rewards(yield_farm: &YieldFarm) -> U256 {
    position_stake_manager::get_pending_rewards(POOL_ID, yield_farm).await
}

pub async fn harvest(yield_farm: &YieldFarm) {
    position_stake_manager::harvest(POOL_ID, yield_farm).await
}
