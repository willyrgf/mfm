//! Wallet-file authentication configuration.
//!
//! This module intentionally models only local file-based key loading. Encrypted wallet file
//! support was removed, so callers must opt into plaintext loading explicitly.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Wallet file configuration used by [`super::Method::Wallet`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    /// Path to the wallet file on disk.
    pub private_key_path: PathBuf,
    /// Whether the file already contains plaintext key material.
    pub not_encrypted: Option<bool>,
}

impl PartialEq for Wallet {
    fn eq(&self, other: &Self) -> bool {
        self.private_key_path == other.private_key_path && self.not_encrypted == other.not_encrypted
    }
}

impl Eq for Wallet {}

impl Wallet {
    /// Reads the configured private key from disk.
    ///
    /// Encrypted wallet files are intentionally unsupported; callers must opt into plaintext
    /// loading with `not_encrypted: true`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if `not_encrypted` is not explicitly set to
    /// `true`.
    ///
    /// # Security
    ///
    /// The returned string contains raw secret material. Callers should minimize its lifetime,
    /// avoid logging it, and wrap it in a zeroizing container before broader use.
    pub fn read_private_key(
        &self,
        _password: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let private_key = std::fs::read_to_string(&self.private_key_path)?;

        if self.not_encrypted.unwrap_or(false) {
            return Ok(private_key.trim().to_string());
        }

        Err("Encrypted wallet files are no longer supported; set not_encrypted: true and store the key plaintext.".into())
    }
}
