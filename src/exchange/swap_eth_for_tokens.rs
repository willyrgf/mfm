use crate::utils;

use web3::{
    ethabi::Token,
    types::{Bytes, TransactionParameters, U256},
};

use crate::config::wallet::Wallet;

use super::config::ExchangeConfig;

pub async fn swap(
    exchange: &ExchangeConfig,
    from_wallet: &Wallet,
    amount_in: U256,
    amount_min_out: U256,
    asset_path: Token,
) {
    let client = exchange.get_web3_client_http();
    let gas_price = client.eth().gas_price().await.unwrap();
    let valid_timestamp = exchange.get_valid_timestamp(30000000);
    let estimate_gas = exchange
        .router_contract()
        .estimate_gas(
            "swapExactETHForTokensSupportingFeeOnTransferTokens",
            // "swapExactTokensForTokens",
            (
                amount_min_out,
                asset_path.clone(),
                from_wallet.address(),
                U256::from_dec_str(&valid_timestamp.to_string()).unwrap(),
            ),
            from_wallet.address(),
            web3::contract::Options {
                value: Some(amount_in),
                gas_price: Some(gas_price),
                // gas: Some(500_000.into()),
                // gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let func_data = exchange
        .router_contract()
        .abi()
        .function("swapExactETHForTokensSupportingFeeOnTransferTokens")
        // .function("swapExactTokensForTokens")
        .unwrap()
        .encode_input(&[
            Token::Uint(amount_min_out),
            asset_path,
            Token::Address(from_wallet.address()),
            Token::Uint(U256::from_dec_str(&valid_timestamp.to_string()).unwrap()),
        ])
        .unwrap();

    let nonce = from_wallet.nonce(client.clone()).await.unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let estimate_with_margin =
        (estimate_gas * (U256::from(10000_i32) + U256::from(1000_i32))) / U256::from(10000_i32);
    let transaction_obj = TransactionParameters {
        nonce: Some(nonce),
        to: Some(exchange.as_router_address().unwrap()),
        value: amount_in,
        gas_price: Some(gas_price),
        gas: estimate_with_margin,
        data: Bytes(func_data),
        ..Default::default()
    };

    utils::blockchain::sign_send_and_wait_txn(client.clone(), transaction_obj, from_wallet)
        .await
        .unwrap();
}
