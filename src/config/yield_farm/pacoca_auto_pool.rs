use web3::types::U256;

use super::pacoca_vault;
use super::YieldFarm;

pub async fn get_pending_rewards(yield_farm: &YieldFarm) -> U256 {
    pacoca_vault::get_pending_rewards(yield_farm).await
}

pub async fn harvest(yield_farm: &YieldFarm) {
    pacoca_vault::harvest(yield_farm).await
}
