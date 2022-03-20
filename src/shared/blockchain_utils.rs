use web3::{
    contract::{
        tokens::{Tokenizable, Tokenize},
        Contract, Options,
    },
    transports::Http,
    types::U256,
};

use crate::config::wallet::Wallet;

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

    estimate_gas
}
