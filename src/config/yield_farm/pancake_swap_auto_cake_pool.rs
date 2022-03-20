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
    shared,
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

pub async fn get_pending_rewards_amounts(
    contract: &Contract<Http>,
    wallet: &Wallet,
    asset_decimals: u8,
) -> (U256, U256, U256) {
    let price_per_full_share: U256 = get_price_per_full_share(contract.clone()).await;
    let total_pending_cake_rewards: U256 = get_total_pending_cake_rewards(contract.clone()).await;
    let (shares, _, cake_at_last_user_action, _): (U256, U256, U256, U256) =
        get_user_info(contract.clone(), wallet).await;
    let amount_in_cake = (shares * price_per_full_share) / U256::exp10(asset_decimals.into());
    let pending_rewards = amount_in_cake - cake_at_last_user_action;
    let pending_shares =
        ((pending_rewards * U256::exp10(asset_decimals.into())) / price_per_full_share);

    (pending_shares, pending_rewards, amount_in_cake)
}

pub async fn get_pending_rewards(
    config: &Config,
    yield_farm: &YieldFarm,
    client: web3::Web3<Http>,
) -> U256 {
    let asset = yield_farm.get_asset(config);
    let asset_decimals = asset.decimals(client.clone()).await;
    let contract = yield_farm.contract(client.clone());
    let wallet = yield_farm.get_wallet(config);

    let (_, pending_rewards, _): (U256, U256, U256) =
        get_pending_rewards_amounts(&contract, &wallet, asset_decimals).await;

    pending_rewards
}

pub async fn harvest(config: &Config, yield_farm: &YieldFarm, client: web3::Web3<Http>) {
    let asset = yield_farm.get_asset(config);
    let asset_decimals = asset.decimals(client.clone()).await;
    let contract = yield_farm.contract(client.clone());
    let from_wallet = yield_farm.get_wallet(config);
    let (pending_shares, _, _): (U256, U256, U256) =
        get_pending_rewards_amounts(&contract, &from_wallet, asset_decimals).await;

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
