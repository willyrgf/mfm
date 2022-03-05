use std::collections::HashMap;

use rustc_hex::{FromHex};
use serde::{Deserialize, Serialize};
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Wallet {
    private_key: String,
}

impl Wallet {
    pub fn to_raw(&self) -> Vec<u8> {
        self.private_key.from_hex().unwrap()
    }
    pub fn private_key(&self) -> String {
        self.private_key.clone()
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Wallets(HashMap<String, Wallet>);
impl Wallets {
    pub fn get(&self, key: &str) -> &Wallet {
        self.0.get(key).unwrap()
    }
}