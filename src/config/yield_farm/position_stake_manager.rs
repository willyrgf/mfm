use std::str::FromStr;
use web3::{
    contract::Options,
    ethabi::Token,
    types::{Address, Bytes, TransactionParameters, U256},
};

use crate::shared;

use super::YieldFarm;

pub async fn get_pending_rewards(pool_id: i32, yield_farm: &YieldFarm) -> U256 {
    let contract = yield_farm.contract();
    let wallet = yield_farm.get_wallet();
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

pub async fn harvest(pool_id: i32, yield_farm: &YieldFarm) {
    let client = yield_farm.get_web3_client_http();
    let from_wallet = yield_farm.get_wallet();
    let gas_price = client.eth().gas_price().await.unwrap();
    let referrer_address: Address =
        Address::from_str("0x0000000000000000000000000000000000000000").unwrap();

    let estimate_gas = yield_farm
        .contract()
        .estimate_gas(
            "deposit",
            (U256::from(pool_id), U256::from(0_i32), referrer_address),
            from_wallet.address(),
            web3::contract::Options {
                //value: Some(amount),
                gas_price: Some(gas_price),
                // gas: Some(500_000.into()),
                // gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tracing::debug!("harvest called estimate_gas: {:?}", estimate_gas);

    let func_data = yield_farm
        .contract()
        .abi()
        .function("deposit")
        .unwrap()
        .encode_input(&[
            Token::Uint(U256::from(pool_id)),
            Token::Uint(U256::from(0_i32)),
            Token::Address(referrer_address),
        ])
        .unwrap();
    tracing::debug!("harvest(): func_data: {:?}", func_data);

    let nonce = from_wallet.nonce(client.clone()).await.unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
    tracing::debug!("harvest(): nonce: {:?}", nonce);

    let transaction_obj = TransactionParameters {
        nonce: Some(nonce),
        to: Some(yield_farm.as_address()),
        value: U256::from(0_i32),
        gas_price: Some(gas_price),
        gas: estimate_gas,
        data: Bytes(func_data),
        ..Default::default()
    };
    tracing::debug!("harvest(): transaction_obj: {:?}", transaction_obj);

    let secret = from_wallet.secret();
    let signed_transaction = client
        .accounts()
        .sign_transaction(transaction_obj, &secret)
        .await
        .unwrap();
    tracing::debug!("harvest(): signed_transaction: {:?}", signed_transaction);

    let tx_address = client
        .eth()
        .send_raw_transaction(signed_transaction.raw_transaction)
        .await
        .unwrap();
    tracing::debug!("harvest(): tx_adress: {}", tx_address);

    let receipt = shared::blockchain_utils::wait_receipt(client.clone(), tx_address).await;
    tracing::debug!("receipt: {:?}", receipt);
}
