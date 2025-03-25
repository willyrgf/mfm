use crate::config::authentication::encryption::Encryption;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    pub private_key_path: PathBuf,
    pub not_encrypted: Option<bool>,
}

impl PartialEq for Wallet {
    fn eq(&self, other: &Self) -> bool {
        self.private_key_path == other.private_key_path && self.not_encrypted == other.not_encrypted
    }
}

impl Eq for Wallet {}

impl Wallet {
    pub fn read_private_key(
        &self,
        password: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let private_key = std::fs::read_to_string(&self.private_key_path)?;

        if self.not_encrypted.unwrap_or(false) {
            return Ok(private_key.trim().to_string());
        }

        let password = password.ok_or("Password required for encrypted private key")?;
        let mut encryption = Encryption::new(password);
        let decrypted = encryption.decrypt(&private_key)?;

        Ok(decrypted.as_str().to_string())
    }
}
