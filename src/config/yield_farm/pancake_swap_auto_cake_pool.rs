use std::str::FromStr;
use web3::{
    contract::{Contract, Options},
    ethabi::Token,
    transports::Http,
    types::{Address, Bytes, TransactionParameters, U256},
};

use crate::{
    cmd,
    config::{wallet::Wallet, Config},
};

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

// pub async fn get_pending_rewards_amounts() -> (U256) {

// }

pub async fn get_pending_rewards(
    config: &Config,
    yield_farm: &YieldFarm,
    client: web3::Web3<Http>,
) -> U256 {
    let asset = yield_farm.get_asset(config);
    let asset_decimals = asset.decimals(client.clone()).await;
    let contract = yield_farm.contract(client.clone());
    let wallet = yield_farm.get_wallet(config);

    let price_per_full_share: U256 = get_price_per_full_share(contract.clone()).await;
    let total_pending_cake_rewards: U256 = get_total_pending_cake_rewards(contract.clone()).await;

    let (shares, last_deposited_time, cake_at_last_user_action, last_user_action_time): (
        U256,
        U256,
        U256,
        U256,
    ) = get_user_info(contract.clone(), wallet).await;

    log::debug!("price_per_full_share: {:?}", price_per_full_share);
    log::debug!(
        "total_pending_cake_rewards: {:?}",
        total_pending_cake_rewards
    );
    log::debug!("shares: {:?}", shares);
    log::debug!("last_deposited_time: {:?}", last_deposited_time);
    log::debug!("cake_at_last_user_action: {:?}", cake_at_last_user_action);
    log::debug!("last_user_action_time: {:?}", last_user_action_time);

    let amount_in_cake = (shares * price_per_full_share) / U256::exp10(asset_decimals.into());
    log::debug!("amount_in_cake: {:?}", amount_in_cake);
    // log::debug!("price_full_share: {:?}", price_full_share);
    let pending_rewards = amount_in_cake - cake_at_last_user_action;
    log::debug!("pending_rewards: {:?}", pending_rewards);
    let pending_shares =
        ((pending_rewards * U256::exp10(asset_decimals.into())) / price_per_full_share);

    log::debug!("pending_shares: {:?}", pending_shares);
    let pending_rewards_from_shares =
        (pending_shares * price_per_full_share) / U256::exp10(asset_decimals.into());
    log::debug!(
        "pending_rewards_from_shares: {:?}",
        pending_rewards_from_shares
    );

    pending_rewards
}

pub async fn harvest(config: &Config, yield_farm: &YieldFarm, client: web3::Web3<Http>) {
    let asset = yield_farm.get_asset(config);
    let asset_decimals = asset.decimals(client.clone()).await;
    let contract = yield_farm.contract(client.clone());
    let from_wallet = yield_farm.get_wallet(config);
    let price_per_full_share: U256 = get_price_per_full_share(contract.clone()).await;
    let (shares, _, cake_at_last_user_action, _): (U256, U256, U256, U256) =
        get_user_info(contract.clone(), from_wallet).await;
    let amount_in_cake = (shares * price_per_full_share) / U256::exp10(asset_decimals.into());
    let pending_rewards = amount_in_cake - cake_at_last_user_action;
    let pending_shares =
        ((pending_rewards * U256::exp10(asset_decimals.into())) / price_per_full_share);

    let gas_price = client.eth().gas_price().await.unwrap();

    let estimate_gas = yield_farm
        .contract(client.clone())
        .estimate_gas(
            "withdraw",
            pending_shares,
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
        .function("withdraw")
        .unwrap()
        .encode_input(&[Token::Uint(pending_shares)])
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
