use crate::signing;

use rustc_hex::FromHex;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};
use web3::{
    transports::Http,
    types::{Address, U256},
};

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
    pub fn secret(&self) -> SecretKey {
        SecretKey::from_str(&self.private_key()).unwrap()
    }
    pub fn public(&self) -> PublicKey {
        let secp = Secp256k1::new();
        let s = self.secret();
        PublicKey::from_secret_key(&secp, &s)
    }
    pub fn address(&self) -> Address {
        signing::public_key_address(&self.public())
    }
    pub async fn nonce(&self, client: web3::Web3<Http>) -> U256 {
        let n = client
            .eth()
            .transaction_count(self.address(), None)
            .await
            .unwrap();
        n
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Wallets(HashMap<String, Wallet>);
impl Wallets {
    pub fn get(&self, key: &str) -> &Wallet {
        self.0.get(key).unwrap()
    }
}
