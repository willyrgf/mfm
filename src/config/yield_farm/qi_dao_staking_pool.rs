use web3::{contract::Options, types::U256};

use super::YieldFarm;

// pub async fn deposit(yield_farm: &YieldFarm, amount: U256) {
//     panic!("not implemented")
// }

pub async fn get_pending_rewards(pool_id: i32, yield_farm: &YieldFarm) -> U256 {
    let contract = yield_farm.contract();
    let wallet = yield_farm.get_wallet();
    let result = contract.query(
        "pending",
        (U256::from(pool_id), wallet.address()),
        None,
        Options::default(),
        None,
    );
    let pending_balance: U256 = result.await.unwrap();
    pending_balance
}

// pub async fn harvest(yield_farm: &YieldFarm) {
//     panic!("not implemented")
// }
