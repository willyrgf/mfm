use serde::{Deserialize, Serialize};
use tari_utilities::SafePassword;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    #[serde(deserialize_with = "deserialize_safe_password")]
    pub private_key: SafePassword,
    pub not_encrypted: bool,
    pub env_password: Option<String>,
}

impl PartialEq for Wallet {
    fn eq(&self, other: &Self) -> bool {
        self.not_encrypted == other.not_encrypted && self.env_password == other.env_password
    }
}

impl Eq for Wallet {}

fn deserialize_safe_password<'de, D>(deserializer: D) -> Result<SafePassword, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let password: String = Deserialize::deserialize(deserializer)?;
    Ok(SafePassword::from(password))
}
