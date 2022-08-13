use core::time;
use std::thread;

use web3::{
    contract::{tokens::Tokenize, Contract, Options},
    ethabi::Token,
    transports::Http,
    types::{
        Address, Bytes, SignedTransaction, TransactionParameters, TransactionReceipt, H256, U256,
    },
    Web3,
};

use crate::{asset::Asset, config::wallet::Wallet};

pub async fn estimate_gas<P>(
    contract: &Contract<Http>,
    from_wallet: &Wallet,
    func_name: &str,
    params: P,
    options: Options,
) -> Result<U256, anyhow::Error>
where
    P: Tokenize,
{
    // let gas_price = client.eth().gas_price().await.unwrap();
    let estimate_gas = contract
        .estimate_gas(func_name, params, from_wallet.address(), options)
        .await
        .map_err(|e| anyhow::anyhow!("failed to execute estimate_gas, got: {:?}", e))?;

    Ok((estimate_gas * (U256::from(10000_i32) + U256::from(1000_i32))) / U256::from(10000_i32))
}

pub fn generate_func_data(
    contract: &Contract<Http>,
    func_name: &str,
    input: &[Token],
) -> Result<Vec<u8>, anyhow::Error> {
    contract
        .abi()
        .function(func_name)
        .unwrap()
        .encode_input(input)
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to generate with this input to this func, got: {:?}",
                e
            )
        })
}

pub fn build_transaction_params(
    nonce: U256,
    to: Address,
    value: U256,
    gas_price: U256,
    estimate_gas: U256,
    func_data: Bytes,
) -> TransactionParameters {
    TransactionParameters {
        nonce: Some(nonce),
        to: Some(to),
        value,
        gas_price: Some(gas_price),
        gas: estimate_gas,
        data: func_data,
        ..Default::default()
    }
}

pub async fn sign_transaction(
    client: Web3<Http>,
    transaction_obj: TransactionParameters,
    from_wallet: &Wallet,
) -> SignedTransaction {
    let secret = from_wallet.secret();
    let signed_transaction = client
        .accounts()
        .sign_transaction(transaction_obj, &secret)
        .await
        .unwrap();
    signed_transaction
}

pub async fn send_raw_transaction(
    client: Web3<Http>,
    signed_transaction: SignedTransaction,
) -> Result<H256, anyhow::Error> {
    client
        .eth()
        .send_raw_transaction(signed_transaction.raw_transaction)
        .await
        .map_err(|e| anyhow::anyhow!("failed to send raw transaction, got: {:?}", e))
}

pub async fn sign_send_and_wait_txn(
    client: Web3<Http>,
    transaction_obj: TransactionParameters,
    from_wallet: &Wallet,
) -> Result<(), anyhow::Error> {
    let signed_transaction = sign_transaction(client.clone(), transaction_obj, from_wallet).await;
    let tx_address = send_raw_transaction(client.clone(), signed_transaction).await?;

    let receipt = wait_receipt(client.clone(), tx_address).await?;
    tracing::debug!("receipt: {:?}", receipt);

    Ok(())
}

pub async fn wait_receipt(
    client: web3::Web3<Http>,
    tx_address: H256,
) -> Result<TransactionReceipt, anyhow::Error> {
    loop {
        match client.eth().transaction_receipt(tx_address).await {
            Ok(Some(receipt)) => return Ok(receipt),
            Ok(None) => {
                thread::sleep(time::Duration::from_secs(5));
                continue;
            }
            Err(e) => return Err(anyhow::anyhow!(e)),
        }
    }
}

pub fn display_amount_to_float(amount: U256, decimals: u8) -> f64 {
    amount.low_u128() as f64 / 10_u64.pow(decimals.into()) as f64
}

pub async fn amount_in_quoted(asset_in: &Asset, asset_quoted: &Asset, amount_in: U256) -> U256 {
    let exchange = asset_in
        .get_network()
        .get_exchange_by_liquidity(asset_in,asset_quoted, amount_in)
        .await.
        unwrap_or_else(||{
            tracing::error!("move_parking_to_assets(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",asset_in,asset_quoted);
            panic!()
        });

    let path = exchange.build_route_for(asset_in, asset_quoted).await;
    match exchange.get_amounts_out(amount_in, path).await.last() {
        Some(p) => *p,
        None => U256::default(),
    }
}
