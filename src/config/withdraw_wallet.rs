use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use web3::types::Address;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WithdrawWallet {
    address: String,
}

impl WithdrawWallet {
    pub fn address(&self) -> String {
        self.address.clone()
    }

    pub fn as_address(&self) -> Address {
        Address::from_str(&self.address()).unwrap()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WithdrawWallets(HashMap<String, WithdrawWallet>);

impl WithdrawWallets {
    pub fn get(&self, key: &str) -> &WithdrawWallet {
        self.0.get(key).unwrap()
    }
}
