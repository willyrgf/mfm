use web3::{
    contract::{Contract, Options},
    ethabi::Token,
    transports::Http,
    types::{Bytes, U256},
};

use crate::{config::wallet::Wallet, shared};

use super::YieldFarm;

pub async fn get_price_per_full_share(contract: Contract<Http>) -> U256 {
    contract
        .query("getPricePerFullShare", (), None, Options::default(), None)
        .await
        .unwrap()
}

pub async fn get_total_pending_cake_rewards(contract: Contract<Http>) -> U256 {
    contract
        .query(
            "calculateTotalPendingCakeRewards",
            (),
            None,
            Options::default(),
            None,
        )
        .await
        .unwrap()
}

pub async fn get_user_info(contract: Contract<Http>, wallet: &Wallet) -> (U256, U256, U256, U256) {
    let (shares, last_deposited_time, cake_at_last_user_action, last_user_action_time): (
        U256,
        U256,
        U256,
        U256,
    ) = contract
        .query("userInfo", wallet.address(), None, Options::default(), None)
        .await
        .unwrap();

    (
        shares,
        last_deposited_time,
        cake_at_last_user_action,
        last_user_action_time,
    )
}

pub async fn get_pending_rewards_amounts(
    contract: &Contract<Http>,
    wallet: &Wallet,
    asset_decimals: u8,
) -> (U256, U256, U256) {
    let price_per_full_share: U256 = get_price_per_full_share(contract.clone()).await;
    // let total_pending_cake_rewards: U256 = get_total_pending_cake_rewards(contract.clone()).await;
    let (shares, _, cake_at_last_user_action, _): (U256, U256, U256, U256) =
        get_user_info(contract.clone(), wallet).await;
    let amount_in_cake = (shares * price_per_full_share) / U256::exp10(asset_decimals.into());
    let pending_rewards = amount_in_cake - cake_at_last_user_action;
    let pending_shares =
        (pending_rewards * U256::exp10(asset_decimals.into())) / price_per_full_share;

    (pending_shares, pending_rewards, amount_in_cake)
}

pub async fn get_pending_rewards(yield_farm: &YieldFarm) -> U256 {
    let asset = match yield_farm.get_reward_asset() {
        Some(a) => a,
        None => {
            log::error!("missing reward asset in assets");
            return U256::from(0_i32);
        }
    };
    let asset_decimals = asset.decimals().await;
    let contract = yield_farm.contract();
    let wallet = yield_farm.get_wallet();

    let (_, pending_rewards, _): (U256, U256, U256) =
        get_pending_rewards_amounts(&contract, wallet, asset_decimals).await;

    pending_rewards
}

pub async fn get_deposited_amount(yield_farm: &YieldFarm) -> U256 {
    let asset = match yield_farm.get_deposit_asset() {
        Some(a) => a,
        None => {
            log::error!("missing reward asset in assets");
            return U256::from(0_i32);
        }
    };
    let asset_decimals = asset.decimals().await;
    let contract = yield_farm.contract();
    let wallet = yield_farm.get_wallet();

    let (_, _, deposited_amount): (U256, U256, U256) =
        get_pending_rewards_amounts(&contract, wallet, asset_decimals).await;

    deposited_amount
}

pub async fn deposit(yield_farm: &YieldFarm, amount: U256) {
    let client = yield_farm.get_web3_client_http();
    let contract = yield_farm.contract();
    let from_wallet = yield_farm.get_wallet();

    let gas_price = client.eth().gas_price().await.unwrap();
    let estimate_gas_from_helper = shared::blockchain_utils::estimate_gas(
        &contract,
        from_wallet,
        "deposit",
        amount,
        web3::contract::Options {
            gas_price: Some(gas_price),
            ..Default::default()
        },
    )
    .await;
    log::debug!(
        "harvest called estimate_gas: {:?}",
        estimate_gas_from_helper
    );
    let estimate_gas = (estimate_gas_from_helper * (U256::from(30000_i32) + U256::from(3000_i32)))
        / U256::from(30000_i32);

    let func_data =
        shared::blockchain_utils::generate_func_data(&contract, "deposit", &[Token::Uint(amount)]);
    log::debug!("harvest(): func_data: {:?}", func_data);

    let nonce = from_wallet.nonce(client.clone()).await;
    log::debug!("harvest(): nonce: {:?}", nonce);

    let transaction_obj = shared::blockchain_utils::build_transaction_params(
        nonce,
        yield_farm.as_address(),
        U256::from(0_i32),
        gas_price,
        estimate_gas,
        Bytes(func_data),
    );
    log::debug!("harvest(): transaction_obj: {:?}", transaction_obj);

    shared::blockchain_utils::sign_send_and_wait_txn(client.clone(), transaction_obj, from_wallet)
        .await;
}

pub async fn harvest(yield_farm: &YieldFarm) {
    let client = yield_farm.get_web3_client_http();
    let asset = match yield_farm.get_reward_asset() {
        Some(a) => a,
        None => {
            log::error!("missing reward asset in assets");
            return;
        }
    };
    let asset_decimals = asset.decimals().await;
    let contract = yield_farm.contract();
    let from_wallet = yield_farm.get_wallet();
    let (pending_shares, _, _): (U256, U256, U256) =
        get_pending_rewards_amounts(&contract, from_wallet, asset_decimals).await;

    let gas_price = client.eth().gas_price().await.unwrap();
    let estimate_gas = shared::blockchain_utils::estimate_gas(
        &contract,
        from_wallet,
        "withdraw",
        pending_shares,
        web3::contract::Options {
            gas_price: Some(gas_price),
            ..Default::default()
        },
    )
    .await;
    log::debug!("harvest called estimate_gas: {:?}", estimate_gas);

    let func_data = shared::blockchain_utils::generate_func_data(
        &contract,
        "withdraw",
        &[Token::Uint(pending_shares)],
    );
    log::debug!("harvest(): func_data: {:?}", func_data);

    let nonce = from_wallet.nonce(client.clone()).await;
    log::debug!("harvest(): nonce: {:?}", nonce);

    let transaction_obj = shared::blockchain_utils::build_transaction_params(
        nonce,
        yield_farm.as_address(),
        U256::from(0_i32),
        gas_price,
        estimate_gas,
        Bytes(func_data),
    );
    log::debug!("harvest(): transaction_obj: {:?}", transaction_obj);

    shared::blockchain_utils::sign_send_and_wait_txn(client.clone(), transaction_obj, from_wallet)
        .await;
}
