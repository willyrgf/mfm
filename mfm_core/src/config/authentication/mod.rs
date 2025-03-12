pub mod encryption;
pub mod wallet;

use self::wallet::Wallet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "wallet")]
pub enum Method {
    #[serde(rename = "wallet")]
    Wallet(Wallet),
    #[serde(rename = "meta_mask")]
    MetaMask,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Methods(Vec<Method>);

impl Methods {
    pub fn get_methods(&self) -> &Vec<Method> {
        &self.0
    }
}
