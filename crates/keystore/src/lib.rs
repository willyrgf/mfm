#![warn(missing_docs)]
//! Bounded secret custody primitive for MFM.
//!
//! `Keystore` intentionally contains an `Rc` marker and is therefore neither `Send` nor `Sync`.
//! A caller that needs an async signer must retain this owner on a dedicated thread and expose a
//! bounded command handle; this crate never turns the key material into a generic `Send` future.

use std::collections::BTreeMap;
use std::rc::Rc;

use mfm_ids::StableId;
use zeroize::{Zeroize, Zeroizing};

/// Redaction-safe keystore error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeystoreError {
    /// The public identifier or bounded operation was invalid.
    #[error("keystore operation is invalid")]
    Invalid,
    /// The requested entry was not present.
    #[error("keystore entry was not found")]
    NotFound,
}

/// Minimal non-secret keystore configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeystoreConfig {
    /// Maximum number of retained entries.
    pub maximum_entries: usize,
}

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            maximum_entries: 64,
        }
    }
}

/// Public key metadata without private material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// Stable key identity.
    pub key_id: StableId,
}

struct SecretBytes(Zeroizing<Vec<u8>>);

impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Non-`Send`, non-`Sync` secret owner.
pub struct Keystore {
    config: KeystoreConfig,
    entries: BTreeMap<StableId, SecretBytes>,
    _thread_affinity: Rc<()>,
}

impl Keystore {
    /// Creates an empty in-memory keystore.
    pub fn new(config: KeystoreConfig) -> Result<Self, KeystoreError> {
        if config.maximum_entries == 0 {
            return Err(KeystoreError::Invalid);
        }
        Ok(Self {
            config,
            entries: BTreeMap::new(),
            _thread_affinity: Rc::new(()),
        })
    }

    /// Inserts one bounded secret into the thread-affine owner.
    pub fn insert(
        &mut self,
        key_id: StableId,
        secret: Zeroizing<Vec<u8>>,
    ) -> Result<KeyInfo, KeystoreError> {
        if secret.is_empty()
            || secret.len() > 16 * 1024
            || (!self.entries.contains_key(&key_id)
                && self.entries.len() >= self.config.maximum_entries)
        {
            return Err(KeystoreError::Invalid);
        }
        self.entries.insert(key_id.clone(), SecretBytes(secret));
        Ok(KeyInfo { key_id })
    }

    /// Lists only public key identities.
    pub fn list(&self) -> Vec<KeyInfo> {
        self.entries
            .keys()
            .cloned()
            .map(|key_id| KeyInfo { key_id })
            .collect()
    }

    /// Removes one secret entry and immediately drops its zeroizing owner.
    pub fn remove(&mut self, key_id: &StableId) -> Result<(), KeystoreError> {
        self.entries
            .remove(key_id)
            .map(|_| ())
            .ok_or(KeystoreError::NotFound)
    }
}
