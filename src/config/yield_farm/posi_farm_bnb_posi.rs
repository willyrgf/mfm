use web3::{contract::Options, transports::Http, types::U256};

use crate::config::Config;

use super::YieldFarm;

pub fn harvest() {
    log::debug!("posi_farm_bnb_posi");
}

pub async fn get_pending_rewards(
    config: &Config,
    yield_farm: &YieldFarm,
    client: web3::Web3<Http>,
) -> U256 {
    let contract = yield_farm.contract(client.clone());
    let wallet = yield_farm.get_wallet(config);
    let pool_id = 2;
    let result = contract.query(
        "pendingPosition",
        (U256::from(pool_id), wallet.address()),
        None,
        Options::default(),
        None,
    );
    let pending_balance: U256 = result.await.unwrap();
    pending_balance
}
