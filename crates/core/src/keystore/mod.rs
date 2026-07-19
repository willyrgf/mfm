//! # Keystore Module
//!
//! Minimal secure keystore for Ethereum private keys and one-time mnemonic imports.
//!
//! This is a simplified, focused implementation that provides only essential
//! functionality for storing and retrieving Ethereum private keys. Mnemonics are accepted only as
//! import inputs for deriving one selected key; the mnemonic phrase and BIP-39 passphrase are not
//! stored.
//!
//! ## Quick Start
//!
//! ```rust
//! use mfm_core::keystore::{Keystore, KeystoreConfig};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Use minimal config for doctest (don't use in production!)
//!     let unsafe_fast_config = KeystoreConfig {
//!         argon2_memory_kb: 64,    // 64KB - minimal for doctest
//!         argon2_iterations: 1,    // 1 iteration - minimal
//!         argon2_parallelism: 1,
//!         allow_secret_exports: false,
//!     };
//!     let temp_dir = tempfile::tempdir()?;
//!     let keystore_path = temp_dir.path().join("keystore.json");
//!
//!     let mut keystore = Keystore::new_with_config(&keystore_path, unsafe_fast_config)?;
//!     keystore.unlock("secure_password")?;
//!
//!     // Import a private key
//!     let key_id = keystore.import_private_key(
//!         Some("my-wallet".to_string()),
//!         "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
//!     )?;
//!
//!     // List all keys
//!     let keys = keystore.list_keys()?;
//!     println!("Stored {} keys", keys.len());
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Security Model
//!
//! - **Full disk encryption** (including swap) assumed to be enabled
//! - **Single-threaded usage** - not designed for concurrent access
//! - **Local-only operation** - no network features or remote storage
//! - **Trusted application environment** - assumes application is not compromised
//! - **No secret-bearing persistence outside the keystore file** - callers must not log or store
//!   exported secrets elsewhere
//!
//! ## Design Principles
//!
//! - **Simplicity over feature completeness**: Only essential functionality
//! - **Security by default**: Secure configurations are the default
//! - **Minimal attack surface**: Fewer features mean fewer vulnerabilities
//! - **Clear separation of concerns**: Each component has a single responsibility
//!
/// Error types produced by keystore operations.
pub mod error;

mod secure_key;
pub use self::secure_key::SecureKey;
mod model;
use self::model::{ArgonParams, KeystoreFile, KeystoreHeader};
pub use self::model::{AuditEvent, AuditLogEntry, KeyEntry, KeyInfo, KeyType, KeystoreConfig};
mod lifecycle;
mod operations;
mod persistence;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Argon2, Params};
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic;
use chrono::{DateTime, Utc};
use rand::rngs::OsRng;
use rand::TryRngCore;
#[cfg(test)]
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use zeroize::Zeroizing;

#[cfg(test)]
use alloy_primitives::Address;

pub use error::KeystoreError;

const KEYSTORE_FILE_VERSION: u8 = 3;
const FILE_INTEGRITY_CONTEXT: &[u8] = b"mfm_keystore_file_integrity_v1";
const DEFAULT_AUTO_LOCK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MIN_KDF_MEMORY_KB: u32 = 64;
const MIN_KDF_ITERATIONS: u32 = 1;
const MIN_KDF_PARALLELISM: u32 = 1;
const MIN_PASSWORD_LEN: usize = 12;
const MAX_KEYSTORE_SIZE: usize = 10 * 1024 * 1024; // 10MB
const MIN_KEYSTORE_SIZE: usize = 100; // Minimum JSON structure
const MAX_ENTRIES: usize = 10_000;
const MAX_ENTRY_DATA: usize = 1024 * 1024; // 1MiB
const MAX_AUDIT_LOG_ENTRIES: usize = 4_096;
const KEYSTORE_FILE_FIELDS: &[&str] = &[
    "version",
    "kdf_params",
    "master_key_verification",
    "audit_log",
    "entries",
    "file_integrity_mac",
];
const PASSWORD_REJECT_LIST: &[&str] = &[
    "password",
    "password123",
    "123456789012",
    "qwerty123456",
    "letmein123456",
    "changeme123456",
    "adminadmin12",
];

/// Minimal secure keystore for Ethereum private keys and one-time mnemonic imports.
pub struct Keystore {
    path: PathBuf,
    config: KeystoreConfig,
    master_key: Option<Zeroizing<[u8; 32]>>,
    unlocked_at: Option<Instant>,
    auto_lock_timeout: Option<Duration>,
    entries: Vec<KeyEntry>,
    audit_log: Vec<AuditLogEntry>,
    kdf_params: Option<ArgonParams>,
    master_key_verification: Option<[u8; 32]>,
    file_integrity_mac: Option<[u8; 32]>,
    #[cfg(test)]
    fail_next_write: Cell<bool>,
    // Thread safety marker - prevents Send + Sync
    _not_thread_safe: *const (),
}

impl Drop for Keystore {
    fn drop(&mut self) {
        // Ensure master key is zeroized on drop
        self.master_key = None;
    }
}

// Note: Thread safety is prevented by the raw pointer field _not_thread_safe
// Raw pointers are !Send + !Sync by default

#[cfg(test)]
mod tests;
