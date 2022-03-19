use std::str::FromStr;
use web3::{
    contract::Options,
    ethabi::Token,
    transports::Http,
    types::{Address, Bytes, TransactionParameters, U256},
};

use crate::{cmd, config::Config};

use super::YieldFarm;

pub async fn get_pending_rewards(
    pool_id: i32,
    config: &Config,
    yield_farm: &YieldFarm,
    client: web3::Web3<Http>,
) -> U256 {
    let contract = yield_farm.contract(client.clone());
    let wallet = yield_farm.get_wallet(config);
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

pub async fn harvest(
    pool_id: i32,
    config: &Config,
    yield_farm: &YieldFarm,
    client: web3::Web3<Http>,
) {
    let from_wallet = yield_farm.get_wallet(config);
    let gas_price = client.eth().gas_price().await.unwrap();
    let referrer_address: Address =
        Address::from_str("0x0000000000000000000000000000000000000000").unwrap();

    let estimate_gas = yield_farm
        .contract(client.clone())
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
    log::debug!("harvest called estimate_gas: {:?}", estimate_gas);

    let func_data = yield_farm
        .contract(client.clone())
        .abi()
        .function("deposit")
        .unwrap()
        .encode_input(&[
            Token::Uint(U256::from(pool_id)),
            Token::Uint(U256::from(0_i32)),
            Token::Address(referrer_address),
        ])
        .unwrap();
    log::debug!("harvest(): func_data: {:?}", func_data);

    let nonce = from_wallet.nonce(client.clone()).await;
    log::debug!("harvest(): nonce: {:?}", nonce);

    let transaction_obj = TransactionParameters {
        nonce: Some(nonce),
        to: Some(yield_farm.as_address()),
        value: U256::from(0_i32),
        gas_price: Some(gas_price),
        gas: estimate_gas,
        data: Bytes(func_data),
        ..Default::default()
    };
    log::debug!("harvest(): transaction_obj: {:?}", transaction_obj);

    let secret = from_wallet.secret();
    let signed_transaction = client
        .accounts()
        .sign_transaction(transaction_obj, &secret)
        .await
        .unwrap();
    log::debug!("harvest(): signed_transaction: {:?}", signed_transaction);

    let tx_address = client
        .eth()
        .send_raw_transaction(signed_transaction.raw_transaction)
        .await
        .unwrap();
    log::debug!("harvest(): tx_adress: {}", tx_address);

    let receipt = cmd::wait_receipt(client.clone(), tx_address).await;
    log::debug!("receipt: {:?}", receipt);
}
