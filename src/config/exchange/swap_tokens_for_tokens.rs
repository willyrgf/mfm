use crate::shared;

use web3::{
    contract::Error,
    ethabi::Token,
    types::{Bytes, TransactionParameters, U256},
};

use crate::config::wallet::Wallet;

use super::Exchange;

pub async fn estimate_gas(
    exchange: &Exchange,
    from_wallet: &Wallet,
    amount_in: U256,
    amount_min_out: U256,
    asset_path: Token,
) -> Result<U256, Error> {
    let client = exchange.get_web3_client_http();
    let gas_price = client.eth().gas_price().await.unwrap();
    let valid_timestamp = exchange.get_valid_timestamp(30000000);
    exchange
        .router_contract()
        .estimate_gas(
            "swapExactTokensForTokensSupportingFeeOnTransferTokens",
            // "swapExactTokensForTokens",
            (
                amount_in,
                amount_min_out,
                asset_path.clone(),
                from_wallet.address(),
                U256::from_dec_str(&valid_timestamp.to_string()).unwrap(),
            ),
            from_wallet.address(),
            web3::contract::Options {
                value: Some(U256::from(0_i32)),
                gas_price: Some(gas_price),
                // gas: Some(500_000.into()),
                // gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await
}

pub async fn swap(
    exchange: &Exchange,
    from_wallet: &Wallet,
    amount_in: U256,
    amount_min_out: U256,
    asset_path: Token,
) {
    let client = exchange.get_web3_client_http();
    let gas_price = client.eth().gas_price().await.unwrap();
    let valid_timestamp = exchange.get_valid_timestamp(30000000);
    let estimate_gas = match exchange
        .router_contract()
        .estimate_gas(
            "swapExactTokensForTokensSupportingFeeOnTransferTokens",
            // "swapExactTokensForTokens",
            (
                amount_in,
                amount_min_out,
                asset_path.clone(),
                from_wallet.address(),
                U256::from_dec_str(&valid_timestamp.to_string()).unwrap(),
            ),
            from_wallet.address(),
            web3::contract::Options {
                value: Some(U256::from(0_i32)),
                gas_price: Some(gas_price),
                // gas: Some(500_000.into()),
                // gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await
    {
        Ok(e) => e,
        Err(err) => {
            // TODO: return error
            log::error!("swap(): estimate_gas(): err: {}, asset_path: {:?}, amount_in: {:?}, amount_min_out: {:?}", err, asset_path, amount_in, amount_min_out);
            panic!()
        }
    };

    log::debug!("swap_tokens_for_tokens estimate_gas: {}", estimate_gas);

    let func_data = exchange
        .router_contract()
        .abi()
        .function("swapExactTokensForTokensSupportingFeeOnTransferTokens")
        // .function("swapExactTokensForTokens")
        .unwrap()
        .encode_input(&[
            Token::Uint(amount_in),
            Token::Uint(amount_min_out),
            asset_path,
            Token::Address(from_wallet.address()),
            Token::Uint(U256::from_dec_str(&valid_timestamp.to_string()).unwrap()),
        ])
        .unwrap();
    log::debug!("swap_tokens_for_tokens(): func_data: {:?}", func_data);

    let nonce = from_wallet.nonce(client.clone()).await;
    log::debug!("swap_tokens_for_tokens(): nonce: {:?}", nonce);

    let estimate_with_margin =
        (estimate_gas * (U256::from(10000_i32) + U256::from(1000_i32))) / U256::from(10000_i32);
    let transaction_obj = TransactionParameters {
        nonce: Some(nonce),
        to: Some(exchange.as_router_address().unwrap()),
        value: U256::from(0_i32),
        gas_price: Some(gas_price),
        gas: estimate_with_margin,
        data: Bytes(func_data),
        ..Default::default()
    };
    log::debug!(
        "swap_tokens_for_tokens(): transaction_obj: {:?}",
        transaction_obj
    );

    shared::blockchain_utils::sign_send_and_wait_txn(client.clone(), transaction_obj, from_wallet)
        .await;
}
