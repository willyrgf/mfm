use serde_derive::{Deserialize, Serialize};

use crate::password::{deserialize_safe_password, SafePassword};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    #[serde(deserialize_with = "deserialize_safe_password")]
    pub private_key: SafePassword,
    pub not_encrypted: bool,
    pub env_password: Option<String>,
}
