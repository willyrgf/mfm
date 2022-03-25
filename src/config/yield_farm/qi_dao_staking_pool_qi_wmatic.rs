use web3::types::U256;

use super::qi_dao_staking_pool;
use super::YieldFarm;

const POOL_ID: i32 = 4;

// pub async fn deposit(yield_farm: &YieldFarm, amount: U256) {
//     qi_dao_staking_pool::deposit(yield_farm, amount).await
// }

pub async fn get_pending_rewards(yield_farm: &YieldFarm) -> U256 {
    qi_dao_staking_pool::get_pending_rewards(POOL_ID, yield_farm).await
}

// pub async fn harvest(yield_farm: &YieldFarm) {
//     qi_dao_staking_pool::harvest(yield_farm).await
// }
