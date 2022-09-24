use crate::{signing, utils::password::SafePassword};

use rustc_hex::FromHex;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};
use web3::{
    transports::Http,
    types::{Address, U256},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    pub(crate) private_key: SafePassword,
    pub(crate) env_password: Option<String>,
    pub(crate) encrypted: Option<bool>,
}

impl Wallet {
    pub fn private_key(&self) -> Zeroizing<String> {
        let bytes = self.private_key.reveal().to_vec();
        Zeroizing::new(String::from_utf8(bytes).unwrap())
    }
    pub fn secret(&self) -> SecretKey {
        // TODO: add a wrap to zeroize the secret key too
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
    pub async fn nonce(&self, client: web3::Web3<Http>) -> Result<U256, anyhow::Error> {
        client
            .eth()
            .transaction_count(self.address(), None)
            .await
            .map_err(|e| anyhow::anyhow!("failed to fetch nonce, got: {:?}", e))
    }

    pub async fn coin_balance(&self, client: web3::Web3<Http>) -> U256 {
        match client.eth().balance(self.address(), None).await {
            Ok(n) => n,
            Err(_) => U256::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Wallets(pub(crate) HashMap<String, Wallet>);
impl Wallets {
    pub fn get(&self, key: &str) -> Option<&Wallet> {
        self.0.get(key)
    }

    pub fn hashmap(&self) -> &HashMap<String, Wallet> {
        &self.0
    }

    pub fn any_encrypted(&self) -> bool {
        self.0
            .values()
            .map(|wallet| wallet.encrypted.unwrap_or(false))
            .any(|b| b)
    }
}
