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

use crate::{
    asset::Asset,
    config::{
        exchange::{Exchange, ZERO_ADDRESS},
        wallet::Wallet,
    },
};

pub async fn estimate_gas<P>(
    contract: &Contract<Http>,
    from_wallet: &Wallet,
    func_name: &str,
    params: P,
    options: Options,
) -> U256
where
    P: Tokenize,
{
    // let gas_price = client.eth().gas_price().await.unwrap();
    let estimate_gas = contract
        .estimate_gas(func_name, params, from_wallet.address(), options)
        .await
        .unwrap();

    (estimate_gas * (U256::from(10000_i32) + U256::from(1000_i32))) / U256::from(10000_i32)
}

pub fn generate_func_data(contract: &Contract<Http>, func_name: &str, input: &[Token]) -> Vec<u8> {
    // let gas_price = client.eth().gas_price().await.unwrap();
    let func_data = contract
        .abi()
        .function(func_name)
        .unwrap()
        .encode_input(input)
        .unwrap();

    func_data
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
) -> H256 {
    let tx_address = client
        .eth()
        .send_raw_transaction(signed_transaction.raw_transaction)
        .await
        .unwrap();

    tx_address
}

pub async fn sign_send_and_wait_txn(
    client: Web3<Http>,
    transaction_obj: TransactionParameters,
    from_wallet: &Wallet,
) {
    let signed_transaction = sign_transaction(client.clone(), transaction_obj, from_wallet).await;
    let tx_address = send_raw_transaction(client.clone(), signed_transaction).await;

    let receipt = wait_receipt(client.clone(), tx_address).await;
    log::debug!("receipt: {:?}", receipt);
}

pub async fn wait_receipt(client: web3::Web3<Http>, tx_address: H256) -> TransactionReceipt {
    loop {
        match client.eth().transaction_receipt(tx_address).await {
            Ok(Some(receipt)) => return receipt,
            Ok(None) => {
                thread::sleep(time::Duration::from_secs(5));
                continue;
            }
            Err(e) => {
                log::error!("wait_receipt() err: {:?}", e);
                panic!()
            }
        }
    }
}

pub fn display_amount_to_float(amount: U256, decimals: u8) -> f64 {
    amount.low_u128() as f64 / 10_u64.pow(decimals.into()) as f64
}

pub async fn amount_in_quoted(asset_in: &Asset, asset_quoted: &Asset, amount_in: U256) -> U256 {
    let exchange = asset_in.get_exchange();
    let path = exchange.build_route_for(asset_in, asset_quoted).await;
    match exchange.get_amounts_out(amount_in, path).await.last() {
        Some(p) => *p,
        None => U256::default(),
    }
}

pub async fn exchange_better_liquidity<'a>(
    input_asset: &'a Asset,
    output_asset: &'a Asset,
) -> Option<&'a Exchange> {
    let input_asset_exchange = input_asset.get_exchange();
    let output_asset_exchange = output_asset.get_exchange();
    let input_path_asset = input_asset.get_path_asset();
    let output_path_asset = output_asset.get_path_asset();

    let input_path_asset_address_input_asset_exchange = match input_asset_exchange
        .get_factory_pair(input_asset, &input_path_asset)
        .await
    {
        Some(a) if a.to_string().as_str() != ZERO_ADDRESS => Some(a),
        _ => None,
    };

    let output_path_asset_address_input_asset_exchange = match input_asset_exchange
        .get_factory_pair(&input_path_asset, output_asset)
        .await
    {
        Some(a) if a.to_string().as_str() != ZERO_ADDRESS => Some(a),
        _ => None,
    };

    let input_path_asset_address_output_asset_exchange = match output_asset_exchange
        .get_factory_pair(input_asset, &input_path_asset)
        .await
    {
        Some(a) if a.to_string().as_str() != ZERO_ADDRESS => Some(a),
        _ => None,
    };

    let output_path_asset_address_output_asset_exchange = match output_asset_exchange
        .get_factory_pair(&input_path_asset, output_asset)
        .await
    {
        Some(a) if a.to_string().as_str() != ZERO_ADDRESS => Some(a),
        _ => None,
    };

    match (
        input_path_asset_address_input_asset_exchange,
        output_path_asset_address_input_asset_exchange,
        input_path_asset_address_output_asset_exchange,
        output_path_asset_address_output_asset_exchange,
    ) {
        (Some(_), Some(_), Some(_), Some(_)) => {
            // TODO: continue from here
            // cargo watch -x 'run -- quote -e quickswap_v2 -a 1 -i grt -o usdc'
        }
        (None, None, Some(_), Some(_)) => return Some(output_asset_exchange),
        (Some(_), Some(_), None, None) => return Some(input_asset_exchange),
        _ => {
            log::error!("")
        }
    };

    // input_exchange:
    //

    unimplemented!()
}

// TODO: check the factory for a pair address with liquidity
// use pair contract to get reserve and check which exchange have the better liquidity
// https://github.com/Uniswap/v2-core/blob/master/contracts/UniswapV2Pair.sol
pub fn exchange_to_use<'a>(asset_in: &'a Asset, asset_out: &'a Asset) -> &'a Exchange {
    if asset_in.exchange_id() == asset_out.exchange_id() {
        asset_in.get_exchange()
    } else {
        asset_out.get_exchange()
    }
}
