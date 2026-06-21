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
//! ## Documentation
//!
//! For comprehensive documentation including security considerations, usage patterns,
//! and troubleshooting, see [`KEYSTORE.md`](./KEYSTORE.md).
/// Error types produced by keystore operations.
pub mod error;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy_primitives::{Address, PrimitiveSignature};
use argon2::{Argon2, Params};
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic;
use chrono::{DateTime, Utc};
use fs2::FileExt;
use k256::{ecdsa::SigningKey, SecretKey};
use rand::rngs::OsRng;
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::cell::Cell;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::crypto::{EthereumKeyError, EthereumPrivateKey};

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

/// Simplified configuration with secure defaults
#[derive(Debug, Clone)]
pub struct KeystoreConfig {
    /// Argon2 memory cost in KB (default: 1GB = 1048576)
    pub argon2_memory_kb: u32,
    /// Argon2 time cost in iterations (default: 8)
    pub argon2_iterations: u32,
    /// Argon2 parallelism (default: 1)
    pub argon2_parallelism: u32,
    /// Whether private-key export APIs are enabled.
    ///
    /// Exporting private keys increases exfiltration risk and is disabled by default. This flag is
    /// only effective when the crate is compiled with `dangerous-secret-export`.
    pub allow_secret_exports: bool,
}

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            argon2_memory_kb: 1_048_576, // 1GB - production secure
            argon2_iterations: 8,        // 8 iterations - secure default
            argon2_parallelism: 1,       // Single threaded
            allow_secret_exports: false,
        }
    }
}

impl KeystoreConfig {
    /// Production-grade secure configuration
    pub fn production() -> Self {
        Self::default()
    }

    /// Development configuration (faster but less secure)
    #[cfg(test)]
    pub fn development() -> Self {
        Self {
            argon2_memory_kb: 8192, // 8MB for faster tests
            argon2_iterations: 2,   // 2 iterations
            argon2_parallelism: 1,
            allow_secret_exports: false,
        }
    }

    /// Integration test configuration (very fast but insecure - DO NOT USE IN PRODUCTION)
    pub fn insecure_integration_test() -> Self {
        Self {
            argon2_memory_kb: 64, // 64KB - minimal for fast tests
            argon2_iterations: 1, // 1 iteration - minimal
            argon2_parallelism: 1,
            allow_secret_exports: false,
        }
    }
}

/// Key type for different storage formats
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum KeyType {
    /// Raw secp256k1 private key material.
    PrivateKey,
    /// Private key derived once from BIP-39 input during import.
    HdDerived {
        /// Derivation path used for the one-time import derivation.
        derivation_path: String,
    },
}

impl KeyType {
    fn sanitized_for_output(&self) -> Self {
        self.clone()
    }
}

/// Key entry stored in keystore
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyEntry {
    /// Stable identifier for the stored key entry.
    pub id: Uuid,
    /// Optional human-readable alias.
    pub alias: Option<String>,
    /// Derived Ethereum address for the key material.
    pub address: Address,
    /// Stored key format.
    pub key_type: KeyType,
    /// Encrypted key payload bytes.
    pub encrypted_data: Vec<u8>,
    /// AES-GCM nonce used to encrypt `encrypted_data`.
    pub nonce: [u8; 12],
    /// Creation timestamp in UTC.
    pub created_at: DateTime<Utc>,
}

/// Key metadata for listing operations
#[derive(Debug, Clone)]
pub struct KeyInfo {
    /// Stable identifier for the stored key entry.
    pub id: Uuid,
    /// Optional human-readable alias.
    pub alias: Option<String>,
    /// Derived Ethereum address for the key material.
    pub address: Address,
    /// Stored key format.
    pub key_type: KeyType,
    /// Creation timestamp in UTC.
    pub created_at: DateTime<Utc>,
}

impl From<&KeyEntry> for KeyInfo {
    fn from(entry: &KeyEntry) -> Self {
        Self {
            id: entry.id,
            alias: entry.alias.clone(),
            address: entry.address,
            key_type: entry.key_type.sanitized_for_output(),
            created_at: entry.created_at,
        }
    }
}

/// Audit events appended to the in-memory and persisted keystore audit log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum AuditEvent {
    /// An unlock attempt occurred.
    Unlock,
    /// The keystore was locked.
    Lock,
    /// A private key import was attempted.
    ImportPrivateKey {
        /// Identifier assigned to the imported entry.
        id: Uuid,
    },
    /// A mnemonic import was attempted.
    ImportMnemonic {
        /// Identifier assigned to the imported entry.
        id: Uuid,
    },
    /// A private key retrieval was attempted for signing.
    GetPrivateKey {
        /// Identifier of the requested entry.
        id: Uuid,
    },
    /// A private key export was attempted.
    ExportPrivateKey {
        /// Identifier of the requested entry.
        id: Uuid,
    },
    /// A key deletion was attempted.
    DeleteKey {
        /// Identifier of the deleted entry.
        id: Uuid,
    },
    /// A password rotation was attempted.
    ChangePassword,
    /// Old audit records were compacted to keep the persisted log bounded.
    AuditLogCompacted {
        /// Number of oldest audit records represented by this summary.
        dropped_entries: u64,
    },
}

/// One persisted audit-log record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditLogEntry {
    /// UTC timestamp when the event was recorded.
    pub timestamp: DateTime<Utc>,
    /// Operation that was attempted.
    pub event: AuditEvent,
    /// Whether the attempted operation succeeded.
    pub success: bool,
}

/// On-disk keystore file format
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeystoreFile {
    version: u8,
    kdf_params: ArgonParams,
    master_key_verification: [u8; 32], // HMAC for password verification
    #[serde(default)]
    audit_log: Vec<AuditLogEntry>,
    entries: Vec<KeyEntry>,
    file_integrity_mac: [u8; 32], // HMAC over the entire file contents for integrity
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArgonParams {
    salt: [u8; 32],
    memory_kb: u32,
    iterations: u32,
    parallelism: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeystoreHeader {
    version: u8,
    kdf_params: ArgonParams,
    master_key_verification: [u8; 32],
    file_integrity_mac: [u8; 32],
}

/// Secure key wrapper that zeroizes on drop
pub struct SecureKey {
    key_bytes: Zeroizing<[u8; 32]>,
}

impl SecureKey {
    fn new(key_bytes: [u8; 32]) -> Self {
        Self {
            key_bytes: Zeroizing::new(key_bytes),
        }
    }

    /// Sign a 32-byte hash (returns k256::Signature)
    pub fn sign_hash(&self, hash: &[u8; 32]) -> Result<k256::ecdsa::Signature, KeystoreError> {
        let secret_key = SecretKey::from_slice(self.key_bytes.as_ref())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;
        let signing_key = SigningKey::from(&secret_key);

        use k256::ecdsa::signature::hazmat::PrehashSigner;
        let result = signing_key
            .sign_prehash(hash)
            .map_err(|e| KeystoreError::CryptoError(format!("Signing failed: {e}")));

        // Note: SecretKey and SigningKey implement ZeroizeOnDrop automatically
        // via the k256 crate, so they will be zeroized when dropped
        result
    }

    /// Sign a 32-byte hash and return an Ethereum recoverable signature.
    pub fn sign_hash_recoverable(
        &self,
        hash: &[u8; 32],
    ) -> Result<PrimitiveSignature, KeystoreError> {
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(self.key_bytes.as_ref());
        let key = EthereumPrivateKey::from_secret_bytes(key_bytes)
            .map_err(keystore_error_from_ethereum_key)?;
        key.sign_hash_recoverable(hash)
            .map_err(keystore_error_from_ethereum_key)
    }

    /// Get Ethereum address for this key
    pub fn ethereum_address(&self) -> Result<Address, KeystoreError> {
        ethereum_address_from_key_bytes(self.key_bytes.as_ref())
    }

    /// Get public key
    pub fn public_key(&self) -> Result<k256::PublicKey, KeystoreError> {
        let secret_key = SecretKey::from_slice(self.key_bytes.as_ref())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;
        // Note: SecretKey implements ZeroizeOnDrop and will be zeroized when dropped
        Ok(secret_key.public_key())
    }
}

impl ZeroizeOnDrop for SecureKey {}

fn ethereum_address_from_key_bytes(key_bytes: &[u8]) -> Result<Address, KeystoreError> {
    let key_bytes: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| KeystoreError::InvalidPrivateKey)?;
    let key = EthereumPrivateKey::from_secret_bytes(key_bytes)
        .map_err(keystore_error_from_ethereum_key)?;
    key.address().map_err(keystore_error_from_ethereum_key)
}

fn keystore_error_from_ethereum_key(err: EthereumKeyError) -> KeystoreError {
    match err {
        EthereumKeyError::InvalidHex
        | EthereumKeyError::InvalidLength
        | EthereumKeyError::InvalidPrivateKey => KeystoreError::InvalidPrivateKey,
        EthereumKeyError::SigningFailed => {
            KeystoreError::CryptoError("failed to sign prehashed payload".to_string())
        }
    }
}

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

impl std::fmt::Debug for Keystore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keystore")
            .field("path", &self.path)
            .field("config", &self.config)
            .field(
                "unlocked",
                &(self.master_key.is_some() && !self.has_unlock_expired()),
            )
            .field("auto_lock_timeout", &self.auto_lock_timeout)
            .field("entry_count", &self.entries.len())
            .field("audit_log_count", &self.audit_log.len())
            .field("kdf_params_loaded", &self.kdf_params.is_some())
            .finish()
    }
}

struct MutationLockGuard {
    file: std::fs::File,
}

impl Drop for MutationLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Keystore {
    /// Create or load keystore from file
    pub fn new(path: impl AsRef<Path>) -> Result<Self, KeystoreError> {
        Self::new_with_config(path, KeystoreConfig::default())
    }

    /// Create or load keystore with custom configuration
    pub fn new_with_config(
        path: impl AsRef<Path>,
        config: KeystoreConfig,
    ) -> Result<Self, KeystoreError> {
        let path = path.as_ref().to_path_buf();

        let mut keystore = Self {
            path,
            config,
            master_key: None,
            unlocked_at: None,
            auto_lock_timeout: Some(DEFAULT_AUTO_LOCK_TIMEOUT),
            entries: Vec::new(),
            audit_log: Vec::new(),
            kdf_params: None,
            master_key_verification: None,
            file_integrity_mac: None,
            #[cfg(test)]
            fail_next_write: Cell::new(false),
            _not_thread_safe: std::ptr::null(),
        };

        // Load existing keystore if file exists
        if keystore.path.exists() {
            keystore.load_from_disk()?;
        }

        Ok(keystore)
    }

    /// Initialize new keystore or unlock existing one
    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        // Early file validation to detect malformed files before expensive operations
        if let Err(err) = self.early_file_validation() {
            self.append_audit_event(AuditEvent::Unlock, false);
            return Err(err);
        }

        let result = if self.path.exists() {
            // Unlock existing keystore
            self.unlock_existing(password)
        } else {
            // Initialize new keystore
            self.initialize_new(password)
        };

        self.append_audit_event(AuditEvent::Unlock, result.is_ok());
        result
    }

    /// Lock keystore (clear master key from memory)
    pub fn lock(&mut self) {
        self.append_audit_event(AuditEvent::Lock, true);
        self.master_key = None;
        self.unlocked_at = None;
    }

    /// Set auto-lock timeout for unlocked sessions.
    ///
    /// `None` disables auto-lock.
    pub fn set_auto_lock_timeout(&mut self, timeout: Option<Duration>) {
        self.auto_lock_timeout = timeout;
    }

    /// Returns the bounded recent audit log for this keystore instance.
    ///
    /// When the log reaches [`MAX_AUDIT_LOG_ENTRIES`], older records are represented by an
    /// [`AuditEvent::AuditLogCompacted`] summary entry.
    pub fn audit_log(&self) -> &[AuditLogEntry] {
        &self.audit_log
    }

    #[cfg(test)]
    fn fail_next_write_for_test(&self) {
        self.fail_next_write.set(true);
    }

    fn append_audit_event(&mut self, event: AuditEvent, success: bool) {
        let timestamp = Utc::now();
        if self.audit_log.len() >= MAX_AUDIT_LOG_ENTRIES {
            self.compact_audit_log_for_append(timestamp);
        }

        self.audit_log.push(AuditLogEntry {
            timestamp,
            event,
            success,
        });
        debug_assert!(self.audit_log.len() <= MAX_AUDIT_LOG_ENTRIES);
    }

    fn compact_audit_log_for_append(&mut self, timestamp: DateTime<Utc>) {
        if self.audit_log.len() < MAX_AUDIT_LOG_ENTRIES {
            return;
        }

        let first_is_compaction = matches!(
            self.audit_log.first().map(|entry| &entry.event),
            Some(AuditEvent::AuditLogCompacted { .. })
        );

        if first_is_compaction {
            let drop_count = self
                .audit_log
                .len()
                .saturating_sub(MAX_AUDIT_LOG_ENTRIES.saturating_sub(1));
            if drop_count > 0 {
                self.audit_log.drain(1..1 + drop_count);
            }
            if let Some(first) = self.audit_log.first_mut() {
                if let AuditEvent::AuditLogCompacted { dropped_entries } = &mut first.event {
                    *dropped_entries = dropped_entries.saturating_add(drop_count as u64);
                    first.timestamp = timestamp;
                    first.success = true;
                }
            }
        } else {
            let drop_count = (self.audit_log.len() + 2)
                .saturating_sub(MAX_AUDIT_LOG_ENTRIES)
                .min(self.audit_log.len());
            if drop_count > 0 {
                self.audit_log.drain(0..drop_count);
            }
            self.audit_log.insert(
                0,
                AuditLogEntry {
                    timestamp,
                    event: AuditEvent::AuditLogCompacted {
                        dropped_entries: drop_count as u64,
                    },
                    success: true,
                },
            );
        }
    }

    /// Import private key (hex format)
    pub fn import_private_key(
        &mut self,
        alias: Option<String>,
        private_key_hex: &str,
    ) -> Result<Uuid, KeystoreError> {
        let id = Uuid::new_v4();
        let result: Result<Uuid, KeystoreError> = (|| {
            self.ensure_master_key_available()?;
            let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

            // Validate and parse private key
            let private_key_hex = private_key_hex.trim_start_matches("0x");
            if private_key_hex.len() != 64 {
                return Err(KeystoreError::InvalidPrivateKey);
            }

            let mut private_key_bytes =
                hex::decode(private_key_hex).map_err(|_| KeystoreError::InvalidPrivateKey)?;

            if private_key_bytes.len() != 32 {
                private_key_bytes.zeroize(); // Zeroize invalid data
                return Err(KeystoreError::InvalidPrivateKey);
            }

            let mut key_array = [0u8; 32];
            key_array.copy_from_slice(&private_key_bytes);

            // Zeroize the Vec<u8> immediately after copying
            private_key_bytes.zeroize();

            // Validate that it's a valid secp256k1 private key (not zero, not >= curve order)
            if key_array == [0u8; 32] {
                return Err(KeystoreError::InvalidPrivateKey);
            }

            // Check if the key is valid by creating a SecretKey
            SecretKey::from_slice(&key_array).map_err(|_| KeystoreError::InvalidPrivateKey)?;

            // Calculate Ethereum address
            let secure_key = SecureKey::new(key_array);
            let address = secure_key.ethereum_address()?;

            // Create encrypted entry
            let mut nonce = [0u8; 12];
            OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
                KeystoreError::CryptoError("Failed to generate random nonce".to_string())
            })?;

            let encrypted_data =
                self.encrypt_data(master_key, &nonce, &key_array, id.as_bytes())?;

            let entry = KeyEntry {
                id,
                alias,
                address,
                key_type: KeyType::PrivateKey,
                encrypted_data,
                nonce,
                created_at: Utc::now(),
            };

            let previous_entry_len = self.entries.len();
            let previous_audit_log = self.audit_log.clone();
            self.entries.push(entry);
            self.append_audit_event(AuditEvent::ImportPrivateKey { id }, true);
            if let Err(err) = self.save_to_disk() {
                self.entries.truncate(previous_entry_len);
                self.audit_log = previous_audit_log;
                key_array.zeroize();
                return Err(err);
            }

            // Zeroize the key array only after the durable write has consumed it.
            key_array.zeroize();
            Ok(id)
        })();
        result
    }

    /// Import mnemonic with derivation path
    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        mnemonic: &str,
        derivation_path: &str,
        passphrase: Option<&str>,
    ) -> Result<Uuid, KeystoreError> {
        let id = Uuid::new_v4();

        let result: Result<Uuid, KeystoreError> = (|| {
            self.ensure_master_key_available()?;
            let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

            // Validate mnemonic
            let mnemonic = Mnemonic::from_str(mnemonic)?;

            // Validate derivation path
            let derivation_path_obj = DerivationPath::from_str(derivation_path)?;

            // Derive the selected private key once. The mnemonic and passphrase are import inputs,
            // not persisted wallet material.
            let seed = Zeroizing::new(mnemonic.to_seed(passphrase.unwrap_or("")));
            let derived_key: Zeroizing<[u8; 32]> = {
                let derived_xprv = XPrv::derive_from_path(*seed, &derivation_path_obj)?;
                Zeroizing::new(derived_xprv.private_key().to_bytes().into())
            };

            let address = ethereum_address_from_key_bytes(derived_key.as_ref())?;

            // Create encrypted entry
            let mut nonce = [0u8; 12];
            OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
                KeystoreError::CryptoError("Failed to generate random nonce".to_string())
            })?;

            let encrypted_data =
                self.encrypt_data(master_key, &nonce, &*derived_key, id.as_bytes())?;

            let entry = KeyEntry {
                id,
                alias,
                address,
                key_type: KeyType::HdDerived {
                    derivation_path: derivation_path.to_string(),
                },
                encrypted_data,
                nonce,
                created_at: Utc::now(),
            };

            let previous_entry_len = self.entries.len();
            let previous_audit_log = self.audit_log.clone();
            self.entries.push(entry);
            self.append_audit_event(AuditEvent::ImportMnemonic { id }, true);
            if let Err(err) = self.save_to_disk() {
                self.entries.truncate(previous_entry_len);
                self.audit_log = previous_audit_log;
                return Err(err);
            }

            Ok(id)
        })();
        result
    }

    /// Get private key for signing
    pub fn get_private_key(&mut self, id: Uuid) -> Result<SecureKey, KeystoreError> {
        let result: Result<SecureKey, KeystoreError> = (|| {
            self.ensure_master_key_available()?;
            let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

            let entry = self
                .entries
                .iter()
                .find(|e| e.id == id)
                .ok_or(KeystoreError::KeyNotFound(id))?;

            match &entry.key_type {
                KeyType::PrivateKey | KeyType::HdDerived { .. } => {
                    let decrypted_data = self.decrypt_data(
                        master_key,
                        &entry.nonce,
                        &entry.encrypted_data,
                        entry.id.as_bytes(),
                    )?;
                    if decrypted_data.len() != 32 {
                        return Err(KeystoreError::InvalidPrivateKey);
                    }
                    let mut key_bytes = [0u8; 32];
                    key_bytes.copy_from_slice(&decrypted_data);
                    Ok(SecureKey::new(key_bytes))
                }
            }
        })();

        match result {
            Ok(secure_key) => {
                let previous_audit_log = self.audit_log.clone();
                self.append_audit_event(AuditEvent::GetPrivateKey { id }, true);
                if let Err(err) = self.save_to_disk() {
                    self.audit_log = previous_audit_log;
                    return Err(err);
                }

                Ok(secure_key)
            }
            Err(err) => Err(err),
        }
    }

    /// Export the private key as a hex string (0x-prefixed).
    ///
    /// - For `KeyType::PrivateKey`, this returns the stored private key.
    /// - For `KeyType::HdDerived`, this returns the one-time derived private key.
    #[cfg(feature = "dangerous-secret-export")]
    pub fn export_private_key(&mut self, id: Uuid) -> Result<Zeroizing<String>, KeystoreError> {
        let result: Result<Zeroizing<String>, KeystoreError> = (|| {
            self.ensure_secret_exports_enabled()?;
            self.ensure_master_key_available()?;
            let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

            let entry = self
                .entries
                .iter()
                .find(|e| e.id == id)
                .ok_or(KeystoreError::KeyNotFound(id))?;

            match &entry.key_type {
                KeyType::PrivateKey | KeyType::HdDerived { .. } => {
                    let decrypted_data = self.decrypt_data(
                        master_key,
                        &entry.nonce,
                        &entry.encrypted_data,
                        entry.id.as_bytes(),
                    )?;
                    if decrypted_data.len() != 32 {
                        return Err(KeystoreError::InvalidPrivateKey);
                    }
                    Ok(Zeroizing::new(format!(
                        "0x{}",
                        hex::encode(decrypted_data.as_slice())
                    )))
                }
            }
        })();

        match result {
            Ok(private_key_hex) => {
                let previous_audit_log = self.audit_log.clone();
                self.append_audit_event(AuditEvent::ExportPrivateKey { id }, true);
                if let Err(err) = self.save_to_disk() {
                    self.audit_log = previous_audit_log;
                    return Err(err);
                }

                Ok(private_key_hex)
            }
            Err(err) => {
                if self.master_key.is_some() && !self.has_unlock_expired() {
                    let previous_audit_log = self.audit_log.clone();
                    self.append_audit_event(AuditEvent::ExportPrivateKey { id }, false);
                    if self.save_to_disk().is_err() {
                        self.audit_log = previous_audit_log;
                    }
                }
                Err(err)
            }
        }
    }

    /// List stored keys (metadata only). Requires an unlocked session.
    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        self.ensure_unlocked_for_read()?;
        Ok(self.entries.iter().map(KeyInfo::from).collect())
    }

    /// Remove key from keystore
    pub fn delete_key(&mut self, id: Uuid) -> Result<(), KeystoreError> {
        let result: Result<(), KeystoreError> = (|| {
            self.ensure_master_key_available()?;

            let previous_entries = self.entries.clone();
            let previous_audit_log = self.audit_log.clone();
            let initial_len = previous_entries.len();
            self.entries.retain(|entry| entry.id != id);

            if self.entries.len() == initial_len {
                return Err(KeystoreError::KeyNotFound(id));
            }

            self.append_audit_event(AuditEvent::DeleteKey { id }, true);
            if let Err(err) = self.save_to_disk() {
                self.entries = previous_entries;
                self.audit_log = previous_audit_log;
                return Err(err);
            }
            Ok(())
        })();
        result
    }

    /// Change keystore password by re-encrypting all entries with a new derived master key.
    pub fn change_password(
        &mut self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        let result: Result<(), KeystoreError> = (|| {
            self.ensure_master_key_available()?;
            self.validate_password_policy(new_password)?;
            let previous_entries = self.entries.clone();
            let previous_audit_log = self.audit_log.clone();
            let previous_kdf_params = self.kdf_params.clone();
            let previous_master_key_verification = self.master_key_verification;
            let previous_master_key = self.master_key.clone();
            let previous_unlocked_at = self.unlocked_at;
            let previous_file_integrity_mac = self.file_integrity_mac;

            let kdf_params = self
                .kdf_params
                .as_ref()
                .ok_or(KeystoreError::InvalidPassword)?;

            let stored_verification = self
                .master_key_verification
                .ok_or(KeystoreError::InvalidPassword)?;

            // Verify old password.
            let old_master_key = self.derive_master_key(old_password, kdf_params)?;
            let computed_verification = self.create_verification_hash(&old_master_key)?;
            if computed_verification.ct_ne(&stored_verification).into() {
                return Err(KeystoreError::InvalidPassword);
            }

            // Generate new salt and derive new master key.
            let mut salt = [0u8; 32];
            OsRng.try_fill_bytes(&mut salt).map_err(|_| {
                KeystoreError::CryptoError("Failed to generate random salt".to_string())
            })?;

            let new_kdf_params = ArgonParams {
                salt,
                memory_kb: self.config.argon2_memory_kb,
                iterations: self.config.argon2_iterations,
                parallelism: self.config.argon2_parallelism,
            };

            let new_master_key = self.derive_master_key(new_password, &new_kdf_params)?;
            let new_verification = self.create_verification_hash(&new_master_key)?;

            // Re-encrypt entries one-by-one to avoid holding all plaintexts in memory.
            let mut new_entries = Vec::with_capacity(self.entries.len());
            for entry in &self.entries {
                let plaintext = self.decrypt_data(
                    &old_master_key,
                    &entry.nonce,
                    &entry.encrypted_data,
                    entry.id.as_bytes(),
                )?;

                let mut nonce = [0u8; 12];
                OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
                    KeystoreError::CryptoError("Failed to generate random nonce".to_string())
                })?;

                let encrypted =
                    self.encrypt_data(&new_master_key, &nonce, &plaintext, entry.id.as_bytes())?;
                let mut updated_entry = entry.clone();
                updated_entry.encrypted_data = encrypted;
                updated_entry.nonce = nonce;
                new_entries.push(updated_entry);
            }
            self.entries = new_entries;

            // Update master key + KDF params in memory and persist.
            self.kdf_params = Some(new_kdf_params);
            self.master_key_verification = Some(new_verification);
            self.master_key = Some(new_master_key);
            self.unlocked_at = Some(Instant::now());
            self.append_audit_event(AuditEvent::ChangePassword, true);

            if let Err(err) = self.save_to_disk_after_rekey(&old_master_key) {
                self.entries = previous_entries;
                self.audit_log = previous_audit_log;
                self.kdf_params = previous_kdf_params;
                self.master_key_verification = previous_master_key_verification;
                self.master_key = previous_master_key;
                self.unlocked_at = previous_unlocked_at;
                self.file_integrity_mac = previous_file_integrity_mac;
                return Err(err);
            }
            Ok(())
        })();
        result
    }

    // Private helper methods

    #[cfg(feature = "dangerous-secret-export")]
    fn ensure_secret_exports_enabled(&self) -> Result<(), KeystoreError> {
        if self.config.allow_secret_exports {
            return Ok(());
        }

        Err(KeystoreError::OperationNotPermitted(
            "Secret export operations are disabled by policy".to_string(),
        ))
    }

    fn initialize_new(&mut self, password: &str) -> Result<(), KeystoreError> {
        self.validate_password_policy(password)?;

        // Generate salt for KDF
        let mut salt = [0u8; 32];
        OsRng.try_fill_bytes(&mut salt).map_err(|_| {
            KeystoreError::CryptoError("Failed to generate random salt".to_string())
        })?;

        let kdf_params = ArgonParams {
            salt,
            memory_kb: self.config.argon2_memory_kb,
            iterations: self.config.argon2_iterations,
            parallelism: self.config.argon2_parallelism,
        };

        // Derive master key
        let master_key = self.derive_master_key(password, &kdf_params)?;

        // Create verification data
        let master_key_verification = self.create_verification_hash(&master_key)?;

        self.master_key = Some(master_key);
        self.unlocked_at = Some(Instant::now());
        self.kdf_params = Some(kdf_params);
        self.master_key_verification = Some(master_key_verification);

        // Save empty keystore to disk (this computes and stores file_integrity_mac).
        self.save_to_disk()?;

        Ok(())
    }

    fn unlock_existing(&mut self, password: &str) -> Result<(), KeystoreError> {
        let kdf_params = self
            .kdf_params
            .as_ref()
            .ok_or(KeystoreError::InvalidPassword)?;

        // Derive master key with stored parameters
        let master_key = self.derive_master_key(password, kdf_params)?;
        let verified_file = self.verify_file_integrity(&master_key)?;

        let computed_verification = self.create_verification_hash(&master_key)?;
        if computed_verification
            .ct_ne(&verified_file.master_key_verification)
            .into()
        {
            Err(KeystoreError::InvalidPassword)
        } else {
            self.entries = verified_file.entries;
            self.audit_log = verified_file.audit_log;
            self.kdf_params = Some(verified_file.kdf_params);
            self.master_key_verification = Some(verified_file.master_key_verification);
            self.file_integrity_mac = Some(verified_file.file_integrity_mac);
            self.master_key = Some(master_key);
            self.unlocked_at = Some(Instant::now());
            Ok(())
        }
    }

    fn derive_master_key(
        &self,
        password: &str,
        params: &ArgonParams,
    ) -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
        self.validate_kdf_params(params)?;
        let argon2_params = Params::new(
            params.memory_kb,
            params.iterations,
            params.parallelism,
            Some(32), // output length
        )?;

        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2_params,
        );

        let mut output = Zeroizing::new([0u8; 32]);
        argon2.hash_password_into(password.as_bytes(), &params.salt, output.as_mut())?;

        Ok(output)
    }

    fn create_verification_hash(&self, master_key: &[u8; 32]) -> Result<[u8; 32], KeystoreError> {
        use ring::hmac;
        let key = hmac::Key::new(hmac::HMAC_SHA256, master_key);
        let tag = hmac::sign(&key, b"keystore_verification_v1");
        let mut verification = [0u8; 32];
        verification.copy_from_slice(tag.as_ref());
        Ok(verification)
    }

    fn compute_file_integrity_mac(
        &self,
        master_key: &[u8; 32],
        file_data: &[u8],
    ) -> Result<[u8; 32], KeystoreError> {
        use ring::hmac;
        let key = hmac::Key::new(hmac::HMAC_SHA256, master_key);
        let mut ctx = hmac::Context::with_key(&key);
        ctx.update(FILE_INTEGRITY_CONTEXT);
        ctx.update(file_data);
        let tag = ctx.sign();
        let mut mac = [0u8; 32];
        mac.copy_from_slice(tag.as_ref());
        Ok(mac)
    }

    // TODO: check nonce-reuse-robust XChaCha20-Poly1305
    fn encrypt_data(
        &self,
        master_key: &[u8; 32],
        nonce: &[u8; 12],
        data: &[u8],
        additional_data: &[u8],
    ) -> Result<Vec<u8>, KeystoreError> {
        let key = Key::<Aes256Gcm>::from_slice(master_key);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce);

        use aes_gcm::aead::Payload;
        let payload = Payload {
            msg: data,
            aad: additional_data,
        };

        Ok(cipher.encrypt(nonce, payload)?)
    }

    fn decrypt_data(
        &self,
        master_key: &[u8; 32],
        nonce: &[u8; 12],
        encrypted_data: &[u8],
        additional_data: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let key = Key::<Aes256Gcm>::from_slice(master_key);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce);

        use aes_gcm::aead::Payload;
        let payload = Payload {
            msg: encrypted_data,
            aad: additional_data,
        };

        Ok(Zeroizing::new(cipher.decrypt(nonce, payload)?))
    }

    fn save_to_disk(&mut self) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        let parent = self.ensure_parent_directory_safe()?;
        let _lock = self.acquire_mutation_lock(&parent)?;
        self.ensure_master_key_available()?;
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        self.verify_current_file_matches_memory_mac(master_key)?;
        let file_integrity_mac = self.write_current_keystore_file_locked(&parent, master_key)?;
        self.file_integrity_mac = Some(file_integrity_mac);

        Ok(())
    }

    fn save_to_disk_after_rekey(
        &mut self,
        current_file_master_key: &[u8; 32],
    ) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        let parent = self.ensure_parent_directory_safe()?;
        let _lock = self.acquire_mutation_lock(&parent)?;
        self.verify_current_file_matches_memory_mac(current_file_master_key)?;
        self.ensure_master_key_available()?;
        let new_master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        let file_integrity_mac =
            self.write_current_keystore_file_locked(&parent, new_master_key)?;
        self.file_integrity_mac = Some(file_integrity_mac);

        Ok(())
    }

    fn write_current_keystore_file_locked(
        &self,
        parent: &Path,
        master_key: &[u8; 32],
    ) -> Result<[u8; 32], KeystoreError> {
        let kdf_params = self
            .kdf_params
            .as_ref()
            .ok_or(KeystoreError::InvalidInput("No KDF parameters".to_string()))?;

        let master_key_verification =
            self.master_key_verification
                .ok_or(KeystoreError::InvalidInput(
                    "No verification hash".to_string(),
                ))?;

        // Create keystore file structure without MAC first for canonical serialization
        let keystore_file_without_mac = KeystoreFile {
            version: KEYSTORE_FILE_VERSION,
            kdf_params: kdf_params.clone(),
            master_key_verification,
            audit_log: self.audit_log.clone(),
            entries: self.entries.clone(),
            file_integrity_mac: [0u8; 32], // Placeholder MAC
        };
        self.validate_keystore_shape(&keystore_file_without_mac)?;

        let mac_preimage = Self::canonical_file_mac_preimage_from_file(&keystore_file_without_mac)?;

        // Compute MAC over the canonical serialized data with the MAC slot zeroed.
        let file_integrity_mac = self.compute_file_integrity_mac(master_key, &mac_preimage)?;

        // Create final keystore file with the computed MAC
        let keystore_file = KeystoreFile {
            version: KEYSTORE_FILE_VERSION,
            kdf_params: kdf_params.clone(),
            master_key_verification,
            audit_log: self.audit_log.clone(),
            entries: self.entries.clone(),
            file_integrity_mac,
        };
        self.validate_keystore_shape(&keystore_file)?;

        let json_data = serde_json::to_vec(&keystore_file)?;
        self.atomic_write_keystore_file(parent, &json_data)?;

        Ok(file_integrity_mac)
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;
        self.early_file_validation()?;

        let value = self.read_keystore_file_value()?;
        let header = self.parse_bounded_header(&value)?;

        self.kdf_params = Some(header.kdf_params);
        self.master_key_verification = Some(header.master_key_verification);
        self.file_integrity_mac = Some(header.file_integrity_mac);
        self.entries.clear();
        self.audit_log.clear();
        self.master_key = None;
        self.unlocked_at = None;

        Ok(())
    }

    fn early_file_validation(&self) -> Result<(), KeystoreError> {
        // Early validation with dummy key to detect obviously malformed files
        // before expensive password derivation
        if !self.path.exists() {
            return Ok(()); // New keystore, nothing to validate
        }

        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;

        let value = self.read_keystore_file_value()?;
        self.parse_bounded_header(&value)?;

        Ok(())
    }

    fn verify_file_integrity(&self, master_key: &[u8; 32]) -> Result<KeystoreFile, KeystoreError> {
        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;

        let value = self.read_keystore_file_value()?;
        let header = self.parse_bounded_header(&value)?;

        if let Some(expected_mac) = self.file_integrity_mac {
            if expected_mac.ct_ne(&header.file_integrity_mac).into() {
                return Err(KeystoreError::InvalidInput(
                    "Concurrent modification detected while unlocking keystore".to_string(),
                ));
            }
        }
        let stored_mac = header.file_integrity_mac;
        let mac_preimage = Self::canonical_file_mac_preimage_from_value(value.clone())?;
        let computed_mac = self.compute_file_integrity_mac(master_key, &mac_preimage)?;

        // Constant-time comparison to prevent timing attacks
        if computed_mac.ct_ne(&stored_mac).into() {
            return Err(KeystoreError::InvalidInput(
                "File integrity verification failed - keystore may have been tampered with"
                    .to_string(),
            ));
        }

        let keystore_file: KeystoreFile = serde_json::from_value(value).map_err(|_| {
            KeystoreError::InvalidInput(
                "Malformed authenticated keystore payload - likely corrupted".to_string(),
            )
        })?;
        self.validate_keystore_shape(&keystore_file)?;

        Ok(keystore_file)
    }

    fn read_keystore_file_value(&self) -> Result<serde_json::Value, KeystoreError> {
        let data = fs::read(&self.path)
            .map_err(|_| KeystoreError::InvalidInput("Cannot read keystore file".to_string()))?;
        Self::parse_keystore_file_value(&data)
    }

    fn parse_keystore_file_value(data: &[u8]) -> Result<serde_json::Value, KeystoreError> {
        if data.len() > MAX_KEYSTORE_SIZE {
            return Err(KeystoreError::InvalidInput(
                "Keystore file too large - possible DoS attempt".to_string(),
            ));
        }

        if data.len() < MIN_KEYSTORE_SIZE {
            return Err(KeystoreError::InvalidInput(
                "Keystore file too small - likely corrupted".to_string(),
            ));
        }

        let value: serde_json::Value = serde_json::from_slice(data).map_err(|_| {
            KeystoreError::InvalidInput("Malformed keystore file - invalid JSON".to_string())
        })?;
        Self::validate_top_level_file_shape(&value)?;
        Ok(value)
    }

    fn validate_top_level_file_shape(
        value: &serde_json::Value,
    ) -> Result<&serde_json::Map<String, serde_json::Value>, KeystoreError> {
        let object = value.as_object().ok_or_else(|| {
            KeystoreError::InvalidInput("Keystore file must be a JSON object".to_string())
        })?;

        for key in object.keys() {
            if !KEYSTORE_FILE_FIELDS.contains(&key.as_str()) {
                return Err(KeystoreError::InvalidInput(format!(
                    "Unknown keystore file field: {key}"
                )));
            }
        }

        for key in KEYSTORE_FILE_FIELDS {
            if !object.contains_key(*key) {
                return Err(KeystoreError::InvalidInput(format!(
                    "Missing keystore file field: {key}"
                )));
            }
        }

        Ok(object)
    }

    fn parse_bounded_header(
        &self,
        value: &serde_json::Value,
    ) -> Result<KeystoreHeader, KeystoreError> {
        let object = Self::validate_top_level_file_shape(value)?;
        let header_value = serde_json::json!({
            "version": object.get("version").cloned().unwrap_or(serde_json::Value::Null),
            "kdf_params": object.get("kdf_params").cloned().unwrap_or(serde_json::Value::Null),
            "master_key_verification": object
                .get("master_key_verification")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "file_integrity_mac": object
                .get("file_integrity_mac")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        });
        let header: KeystoreHeader = serde_json::from_value(header_value).map_err(|_| {
            KeystoreError::InvalidInput(
                "Malformed keystore header - invalid KDF or MAC fields".to_string(),
            )
        })?;

        if header.version != KEYSTORE_FILE_VERSION {
            return Err(KeystoreError::InvalidInput(format!(
                "Unsupported keystore version: {}",
                header.version
            )));
        }
        self.validate_kdf_params(&header.kdf_params)?;
        Ok(header)
    }

    fn validate_kdf_params(&self, params: &ArgonParams) -> Result<(), KeystoreError> {
        if params.memory_kb < MIN_KDF_MEMORY_KB || params.memory_kb > self.config.argon2_memory_kb {
            return Err(KeystoreError::InvalidInput(
                "KDF memory cost out of bounds".to_string(),
            ));
        }
        if params.iterations < MIN_KDF_ITERATIONS
            || params.iterations > self.config.argon2_iterations
        {
            return Err(KeystoreError::InvalidInput(
                "KDF iteration count out of bounds".to_string(),
            ));
        }
        if params.parallelism < MIN_KDF_PARALLELISM
            || params.parallelism > self.config.argon2_parallelism
        {
            return Err(KeystoreError::InvalidInput(
                "KDF parallelism out of bounds".to_string(),
            ));
        }
        Ok(())
    }

    fn canonical_file_mac_preimage_from_file(
        keystore_file: &KeystoreFile,
    ) -> Result<Vec<u8>, KeystoreError> {
        let value = serde_json::to_value(keystore_file)?;
        Self::canonical_file_mac_preimage_from_value(value)
    }

    fn canonical_file_mac_preimage_from_value(
        mut value: serde_json::Value,
    ) -> Result<Vec<u8>, KeystoreError> {
        let object = value.as_object_mut().ok_or_else(|| {
            KeystoreError::InvalidInput("Keystore file must be a JSON object".to_string())
        })?;
        object.insert(
            "file_integrity_mac".to_string(),
            serde_json::to_value([0u8; 32])?,
        );
        Ok(serde_json::to_vec(&value)?)
    }

    fn ensure_target_path_is_safe(&self) -> Result<(), KeystoreError> {
        if !self.path.exists() {
            return Ok(());
        }

        let metadata = fs::symlink_metadata(&self.path)?;
        if metadata.file_type().is_symlink() {
            return Err(KeystoreError::InvalidInput(
                "Refusing to use symlinked keystore path".to_string(),
            ));
        }
        if !metadata.file_type().is_file() {
            return Err(KeystoreError::InvalidInput(
                "Refusing to use non-regular keystore path".to_string(),
            ));
        }

        Ok(())
    }

    fn ensure_parent_directory_safe(&self) -> Result<PathBuf, KeystoreError> {
        let parent = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        if parent.exists() {
            let metadata = fs::symlink_metadata(&parent)?;
            if metadata.file_type().is_symlink() {
                return Err(KeystoreError::InvalidInput(
                    "Refusing to use symlinked parent directory".to_string(),
                ));
            }
            if !metadata.file_type().is_dir() {
                return Err(KeystoreError::InvalidInput(
                    "Keystore parent path must be a directory".to_string(),
                ));
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();
                let world_writable = (mode & 0o002) != 0;
                let sticky = (mode & 0o1000) != 0;
                if world_writable && !sticky {
                    return Err(KeystoreError::InvalidInput(
                        "Refusing unsafe parent directory permissions".to_string(),
                    ));
                }
            }
        } else {
            fs::create_dir_all(&parent)?;
            Self::set_restrictive_permissions_for_directory(&parent)?;
        }

        Ok(parent)
    }

    fn validate_keystore_shape(&self, keystore_file: &KeystoreFile) -> Result<(), KeystoreError> {
        if keystore_file.entries.len() > MAX_ENTRIES {
            return Err(KeystoreError::InvalidInput(
                "Too many entries - possible DoS attempt".to_string(),
            ));
        }
        if keystore_file.audit_log.len() > MAX_AUDIT_LOG_ENTRIES {
            return Err(KeystoreError::InvalidInput(
                "Audit log too large - possible DoS attempt".to_string(),
            ));
        }
        if keystore_file
            .entries
            .iter()
            .any(|e| e.encrypted_data.len() > MAX_ENTRY_DATA)
        {
            return Err(KeystoreError::InvalidInput(
                "Entry too large – likely corrupted".to_string(),
            ));
        }

        let mut ids = HashSet::with_capacity(keystore_file.entries.len());
        for entry in &keystore_file.entries {
            if !ids.insert(entry.id) {
                return Err(KeystoreError::InvalidInput(
                    "Duplicate key entry id detected".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn mutation_lock_path(&self, parent: &Path) -> PathBuf {
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("keystore");
        parent.join(format!(".{file_name}.lock"))
    }

    fn acquire_mutation_lock(&self, parent: &Path) -> Result<MutationLockGuard, KeystoreError> {
        let lock_path = self.mutation_lock_path(parent);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(MutationLockGuard { file })
    }

    fn concurrent_modification_error() -> KeystoreError {
        KeystoreError::InvalidInput(
            "Concurrent modification detected while writing keystore".to_string(),
        )
    }

    fn verify_current_file_matches_memory_mac(
        &self,
        master_key: &[u8; 32],
    ) -> Result<(), KeystoreError> {
        if !self.path.exists() {
            if self.file_integrity_mac.is_some() {
                return Err(Self::concurrent_modification_error());
            }
            return Ok(());
        }

        let expected_mac = self
            .file_integrity_mac
            .ok_or_else(Self::concurrent_modification_error)?;

        let data = fs::read(&self.path).map_err(|_| Self::concurrent_modification_error())?;
        let value = Self::parse_keystore_file_value(&data)
            .map_err(|_| Self::concurrent_modification_error())?;
        let header = self
            .parse_bounded_header(&value)
            .map_err(|_| Self::concurrent_modification_error())?;
        let current_file: KeystoreFile = serde_json::from_value(value.clone())
            .map_err(|_| Self::concurrent_modification_error())?;
        self.validate_keystore_shape(&current_file)
            .map_err(|_| Self::concurrent_modification_error())?;

        let stored_mac = header.file_integrity_mac;
        let mac_preimage = Self::canonical_file_mac_preimage_from_value(value)
            .map_err(|_| Self::concurrent_modification_error())?;
        let computed_mac = self
            .compute_file_integrity_mac(master_key, &mac_preimage)
            .map_err(|_| Self::concurrent_modification_error())?;

        let authenticated_match = computed_mac.ct_eq(&stored_mac)
            & computed_mac.ct_eq(&expected_mac)
            & stored_mac.ct_eq(&expected_mac);
        if !bool::from(authenticated_match) {
            return Err(Self::concurrent_modification_error());
        }

        Ok(())
    }

    fn atomic_write_keystore_file(&self, parent: &Path, data: &[u8]) -> Result<(), KeystoreError> {
        if self.path.exists() {
            self.ensure_target_path_is_safe()?;
        }

        #[cfg(test)]
        if self.fail_next_write.replace(false) {
            return Err(KeystoreError::FileError(
                "injected keystore write failure".to_string(),
            ));
        }

        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("keystore");
        let tmp_path = parent.join(format!(".{file_name}.tmp-{}", Uuid::new_v4()));

        let write_result = (|| -> Result<(), KeystoreError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)?;
            Self::set_restrictive_permissions_for_file(&file)?;

            file.write_all(data)?;
            file.sync_all()?;
            drop(file);

            fs::rename(&tmp_path, &self.path)?;
            Self::set_restrictive_permissions_for_path(&self.path)?;

            #[cfg(unix)]
            {
                let parent_dir = OpenOptions::new().read(true).open(parent)?;
                parent_dir.sync_all()?;
            }
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_result
    }

    #[cfg(unix)]
    fn set_restrictive_permissions_for_file(file: &std::fs::File) -> Result<(), KeystoreError> {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_restrictive_permissions_for_file(_file: &std::fs::File) -> Result<(), KeystoreError> {
        Ok(())
    }

    #[cfg(unix)]
    fn set_restrictive_permissions_for_path(path: &Path) -> Result<(), KeystoreError> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_restrictive_permissions_for_path(_path: &Path) -> Result<(), KeystoreError> {
        Ok(())
    }

    #[cfg(unix)]
    fn set_restrictive_permissions_for_directory(path: &Path) -> Result<(), KeystoreError> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_restrictive_permissions_for_directory(_path: &Path) -> Result<(), KeystoreError> {
        Ok(())
    }

    fn ensure_unlocked_for_read(&self) -> Result<(), KeystoreError> {
        if self.master_key.is_none() || self.has_unlock_expired() {
            return Err(KeystoreError::Locked);
        }
        Ok(())
    }

    fn ensure_master_key_available(&mut self) -> Result<(), KeystoreError> {
        if self.has_unlock_expired() && self.master_key.is_some() {
            self.append_audit_event(AuditEvent::Lock, true);
            self.master_key = None;
            self.unlocked_at = None;
        }
        if self.master_key.is_some() {
            self.unlocked_at = Some(Instant::now());
        }
        if self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }
        Ok(())
    }

    fn has_unlock_expired(&self) -> bool {
        match (self.unlocked_at, self.auto_lock_timeout) {
            (Some(unlocked_at), Some(timeout)) => unlocked_at.elapsed() >= timeout,
            _ => false,
        }
    }

    fn validate_password_policy(&self, password: &str) -> Result<(), KeystoreError> {
        if password.len() < MIN_PASSWORD_LEN {
            return Err(KeystoreError::InvalidInput(format!(
                "Password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }

        let lowered = password.to_ascii_lowercase();
        if PASSWORD_REJECT_LIST
            .iter()
            .any(|candidate| lowered == *candidate)
        {
            return Err(KeystoreError::InvalidInput(
                "Password is too weak".to_string(),
            ));
        }

        if let Some(first) = lowered.chars().next() {
            if lowered.chars().all(|ch| ch == first) {
                return Err(KeystoreError::InvalidInput(
                    "Password is too weak".to_string(),
                ));
            }
        }

        if password.trim().is_empty() {
            return Err(KeystoreError::InvalidInput(
                "Password is too weak".to_string(),
            ));
        }

        Ok(())
    }
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
