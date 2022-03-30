use web3::{
    contract::Options,
    ethabi::Token,
    types::{Bytes, U256},
};

use crate::shared;

use super::YieldFarm;

pub async fn get_pending_rewards(yield_farm: &YieldFarm) -> U256 {
    let contract = yield_farm.contract();
    let wallet = yield_farm.get_wallet();

    let pending_rewards = contract
        .query(
            "pendingReward",
            (wallet.address(),),
            None,
            Options::default(),
            None,
        )
        .await
        .unwrap();

    pending_rewards
}

pub async fn get_deposited_amount(yield_farm: &YieldFarm) -> U256 {
    // let asset_decimals = asset.decimals().await;
    let contract = yield_farm.contract();
    let wallet = yield_farm.get_wallet();

    let (deposited_amount, _): (U256, U256) = contract
        .query(
            "userInfo",
            (wallet.address(),),
            None,
            Options::default(),
            None,
        )
        .await
        .unwrap();

    deposited_amount
}

pub async fn deposit(yield_farm: &YieldFarm, amount: U256) {
    let client = yield_farm.get_web3_client_http();
    let contract = yield_farm.contract();
    let from_wallet = yield_farm.get_wallet();

    let gas_price = client.eth().gas_price().await.unwrap();
    let estimate_gas = shared::blockchain_utils::estimate_gas(
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
    log::debug!("harvest called estimate_gas: {:?}", estimate_gas);
    // let estimate_gas = (estimate_gas_from_helper * (U256::from(30000_i32) + U256::from(3000_i32)))
    //     / U256::from(30000_i32);

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
    let contract = yield_farm.contract();
    let from_wallet = yield_farm.get_wallet();

    let gas_price = client.eth().gas_price().await.unwrap();
    let estimate_gas = shared::blockchain_utils::estimate_gas(
        &contract,
        from_wallet,
        "deposit",
        U256::from(0_i32),
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
        &[Token::Uint(U256::from(0_i32))],
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
