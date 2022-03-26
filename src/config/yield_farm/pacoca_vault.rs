use web3::types::U256;

use super::pancake_swap_auto_cake_pool;
use super::YieldFarm;

pub async fn deposit(yield_farm: &YieldFarm, amount: U256) {
    pancake_swap_auto_cake_pool::deposit(yield_farm, amount).await
}

pub async fn get_pending_rewards(yield_farm: &YieldFarm) -> U256 {
    pancake_swap_auto_cake_pool::get_pending_rewards(yield_farm).await
}

pub async fn harvest(yield_farm: &YieldFarm) {
    pancake_swap_auto_cake_pool::harvest(yield_farm).await
}

pub async fn get_deposited_amount(yield_farm: &YieldFarm) -> U256 {
    pancake_swap_auto_cake_pool::get_deposited_amount(yield_farm).await
}
