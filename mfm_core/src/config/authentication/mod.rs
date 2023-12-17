pub mod wallet;

use serde::{Deserialize, Serialize};

use self::wallet::Wallet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Method {
    Wallet(Wallet),
    MetaMask, // TODO: the next auth method
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Methods(Vec<Method>);
