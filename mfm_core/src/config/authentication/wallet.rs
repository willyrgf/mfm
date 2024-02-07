use serde_derive::{Deserialize, Serialize};

use crate::password::SafePassword;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    pub private_key: SafePassword,
    pub not_encrypted: Option<bool>,
    pub env_password: Option<String>,
}
