use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct WithdrawWallet {
    address: String,
}

impl WithdrawWallet {
    pub fn address(&self) -> String {
        self.address.clone()
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct WithdrawWallets(HashMap<String, WithdrawWallet>);

impl WithdrawWallets {
    pub fn get(&self, key: &str) -> &WithdrawWallet {
        self.0.get(key).unwrap()
    }
}
