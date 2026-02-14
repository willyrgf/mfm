//! # Keystore Module
//!
//! Minimal secure keystore for Ethereum keys and mnemonics.
//!
//! This is a simplified, focused implementation that provides only essential
//! functionality for storing and retrieving Ethereum private keys and mnemonics.
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
pub mod error;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy_primitives::Address;
use argon2::{Argon2, Params};
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic;
use chrono::{DateTime, Utc};
use fs2::FileExt;
use k256::{ecdsa::SigningKey, SecretKey};
use rand::rngs::OsRng;
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use tiny_keccak::{Hasher, Keccak};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub use error::KeystoreError;

const KEYSTORE_FILE_VERSION: u8 = 2;
const FILE_INTEGRITY_CONTEXT: &[u8] = b"mfm_keystore_file_integrity_v1";
const MNEMONIC_PAYLOAD_VERSION: u8 = 1;
const DEFAULT_AUTO_LOCK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MIN_PASSWORD_LEN: usize = 12;
const MAX_KEYSTORE_SIZE: usize = 10 * 1024 * 1024; // 10MB
const MIN_KEYSTORE_SIZE: usize = 100; // Minimum JSON structure
const MAX_ENTRIES: usize = 10_000;
const MAX_ENTRY_DATA: usize = 1024 * 1024; // 1MiB
const MAX_AUDIT_LOG_ENTRIES: usize = 4_096;
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
    /// Whether secret export APIs are enabled.
    ///
    /// Exporting private keys/mnemonics increases exfiltration risk and is disabled by default.
    /// This flag is only effective when the crate is compiled with
    /// `dangerous-secret-export`.
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
pub enum KeyType {
    PrivateKey,
    Mnemonic { derivation_path: String },
}

impl KeyType {
    fn sanitized_for_output(&self) -> Self {
        self.clone()
    }
}

/// Key entry stored in keystore
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEntry {
    pub id: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub key_type: KeyType,
    pub encrypted_data: Vec<u8>,
    pub nonce: [u8; 12],
    pub created_at: DateTime<Utc>,
}

/// Key metadata for listing operations
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub id: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub key_type: KeyType,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    Unlock,
    Lock,
    ImportPrivateKey { id: Uuid },
    ImportMnemonic { id: Uuid },
    GetPrivateKey { id: Uuid },
    ExportPrivateKey { id: Uuid },
    ExportMnemonic { id: Uuid },
    DeleteKey { id: Uuid },
    ChangePassword,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditLogEntry {
    pub timestamp: DateTime<Utc>,
    pub event: AuditEvent,
    pub success: bool,
}

/// On-disk keystore file format
#[derive(Clone, Serialize, Deserialize)]
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
struct ArgonParams {
    salt: [u8; 32],
    memory_kb: u32,
    iterations: u32,
    parallelism: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MnemonicPayload {
    version: u8,
    mnemonic: String,
    #[serde(default)]
    passphrase: Option<String>,
}

impl MnemonicPayload {
    fn new(mnemonic: String, passphrase: Option<String>) -> Self {
        Self {
            version: MNEMONIC_PAYLOAD_VERSION,
            mnemonic,
            passphrase,
        }
    }
}

impl Zeroize for MnemonicPayload {
    fn zeroize(&mut self) {
        self.version = 0;
        self.mnemonic.zeroize();
        if let Some(passphrase) = &mut self.passphrase {
            passphrase.zeroize();
        }
        self.passphrase = None;
    }
}

struct MnemonicMaterial {
    mnemonic: Zeroizing<String>,
    passphrase: Option<Zeroizing<String>>,
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

    /// Get Ethereum address for this key
    pub fn ethereum_address(&self) -> Result<Address, KeystoreError> {
        let secret_key = SecretKey::from_slice(self.key_bytes.as_ref())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;
        let public_key = secret_key.public_key();

        // Compute Ethereum address from public key
        use k256::elliptic_curve::sec1::ToEncodedPoint;
        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]); // Skip 0x04 prefix
        let mut hash = [0u8; 32];
        keccak.finalize(&mut hash);

        // Note: SecretKey implements ZeroizeOnDrop and will be zeroized when dropped
        Ok(Address::from_slice(&hash[12..]))
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

/// Minimal secure keystore for Ethereum keys and mnemonics
#[derive(Debug)]
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
    // Thread safety marker - prevents Send + Sync
    _not_thread_safe: *const (),
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
            self.log_audit(AuditEvent::Unlock, false);
            return Err(err);
        }

        let result = if self.path.exists() {
            // Unlock existing keystore
            self.unlock_existing(password)
        } else {
            // Initialize new keystore
            self.initialize_new(password)
        };

        self.log_audit(AuditEvent::Unlock, result.is_ok());
        result
    }

    /// Lock keystore (clear master key from memory)
    pub fn lock(&mut self) {
        self.log_audit(AuditEvent::Lock, true);
        self.master_key = None;
        self.unlocked_at = None;
    }

    /// Set auto-lock timeout for unlocked sessions.
    ///
    /// `None` disables auto-lock.
    pub fn set_auto_lock_timeout(&mut self, timeout: Option<Duration>) {
        self.auto_lock_timeout = timeout;
    }

    pub fn audit_log(&self) -> &[AuditLogEntry] {
        &self.audit_log
    }

    fn log_audit(&mut self, event: AuditEvent, success: bool) {
        self.audit_log.push(AuditLogEntry {
            timestamp: Utc::now(),
            event,
            success,
        });
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

            self.entries.push(entry);
            self.save_to_disk()?;

            // Zeroize the key array
            key_array.zeroize();

            Ok(id)
        })();

        self.log_audit(AuditEvent::ImportPrivateKey { id }, result.is_ok());
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

            // Derive private key from mnemonic - wrap seed in Zeroizing for automatic cleanup
            let seed = Zeroizing::new(mnemonic.to_seed(passphrase.unwrap_or("")));

            // Limit XPrv scope to ensure it's dropped quickly
            let secure_key = {
                let derived_xprv = XPrv::derive_from_path(*seed, &derivation_path_obj)?;
                SecureKey::new(derived_xprv.private_key().to_bytes().into())
            };

            let address = secure_key.ethereum_address()?;

            let payload = Zeroizing::new(MnemonicPayload::new(
                mnemonic.to_string(),
                passphrase.map(ToOwned::to_owned),
            ));
            let mnemonic_bytes = Zeroizing::new(serde_json::to_vec(&*payload)?);

            // Create encrypted entry
            let mut nonce = [0u8; 12];
            OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
                KeystoreError::CryptoError("Failed to generate random nonce".to_string())
            })?;

            let encrypted_data =
                self.encrypt_data(master_key, &nonce, &mnemonic_bytes, id.as_bytes())?;

            let entry = KeyEntry {
                id,
                alias,
                address,
                key_type: KeyType::Mnemonic {
                    derivation_path: derivation_path.to_string(),
                },
                encrypted_data,
                nonce,
                created_at: Utc::now(),
            };

            self.entries.push(entry);
            self.save_to_disk()?;

            Ok(id)
        })();

        self.log_audit(AuditEvent::ImportMnemonic { id }, result.is_ok());
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
                KeyType::PrivateKey => {
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
                KeyType::Mnemonic {
                    derivation_path, ..
                } => {
                    let material = self.decrypt_mnemonic_material(master_key, entry)?;
                    let mnemonic = Mnemonic::from_str(material.mnemonic.as_str())?;
                    let derivation_path_obj = DerivationPath::from_str(derivation_path)?;

                    // Derive private key from mnemonic - wrap seed in Zeroizing for automatic cleanup
                    let passphrase = material
                        .passphrase
                        .as_ref()
                        .map_or("", |passphrase| passphrase.as_str());
                    let seed = Zeroizing::new(mnemonic.to_seed(passphrase));

                    // Limit XPrv scope to ensure it's dropped quickly
                    let secure_key = {
                        let derived_xprv = XPrv::derive_from_path(*seed, &derivation_path_obj)?;
                        SecureKey::new(derived_xprv.private_key().to_bytes().into())
                    };

                    Ok(secure_key)
                }
            }
        })();

        match result {
            Ok(secure_key) => {
                self.log_audit(AuditEvent::GetPrivateKey { id }, true);

                if let Err(err) = self.save_to_disk() {
                    // If we can't persist the audit entry, treat the overall operation as failed
                    // (the key must not be returned without a durable audit trail).
                    if let Some(last) = self.audit_log.last_mut() {
                        if matches!(last.event, AuditEvent::GetPrivateKey { id: eid } if eid == id)
                        {
                            last.success = false;
                        }
                    }
                    return Err(err);
                }

                Ok(secure_key)
            }
            Err(err) => {
                self.log_audit(AuditEvent::GetPrivateKey { id }, false);
                Err(err)
            }
        }
    }

    /// Export the private key as a hex string (0x-prefixed).
    ///
    /// - For `KeyType::PrivateKey`, this returns the stored private key.
    /// - For `KeyType::Mnemonic`, this derives the private key and exports it.
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
                KeyType::PrivateKey => {
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
                KeyType::Mnemonic {
                    derivation_path, ..
                } => {
                    let material = self.decrypt_mnemonic_material(master_key, entry)?;
                    let mnemonic = Mnemonic::from_str(material.mnemonic.as_str())?;
                    let derivation_path_obj = DerivationPath::from_str(derivation_path)?;

                    let passphrase = material
                        .passphrase
                        .as_ref()
                        .map_or("", |passphrase| passphrase.as_str());
                    let seed = Zeroizing::new(mnemonic.to_seed(passphrase));
                    let derived_xprv = XPrv::derive_from_path(*seed, &derivation_path_obj)?;

                    let key_bytes = derived_xprv.private_key().to_bytes();
                    Ok(Zeroizing::new(format!(
                        "0x{}",
                        hex::encode(key_bytes.as_slice())
                    )))
                }
            }
        })();

        match result {
            Ok(private_key_hex) => {
                self.log_audit(AuditEvent::ExportPrivateKey { id }, true);

                if let Err(err) = self.save_to_disk() {
                    if let Some(last) = self.audit_log.last_mut() {
                        if matches!(
                            last.event,
                            AuditEvent::ExportPrivateKey { id: eid } if eid == id
                        ) {
                            last.success = false;
                        }
                    }
                    return Err(err);
                }

                Ok(private_key_hex)
            }
            Err(err) => {
                self.log_audit(AuditEvent::ExportPrivateKey { id }, false);
                Err(err)
            }
        }
    }

    /// Export the mnemonic phrase.
    #[cfg(feature = "dangerous-secret-export")]
    pub fn export_mnemonic(&mut self, id: Uuid) -> Result<Zeroizing<String>, KeystoreError> {
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
                KeyType::PrivateKey => Err(KeystoreError::InvalidInput(
                    "export_mnemonic only works for mnemonic keys".to_string(),
                )),
                KeyType::Mnemonic { .. } => {
                    let material = self.decrypt_mnemonic_material(master_key, entry)?;
                    Ok(material.mnemonic)
                }
            }
        })();

        match result {
            Ok(mnemonic) => {
                self.log_audit(AuditEvent::ExportMnemonic { id }, true);

                if let Err(err) = self.save_to_disk() {
                    if let Some(last) = self.audit_log.last_mut() {
                        if matches!(last.event, AuditEvent::ExportMnemonic { id: eid } if eid == id)
                        {
                            last.success = false;
                        }
                    }
                    return Err(err);
                }

                Ok(mnemonic)
            }
            Err(err) => {
                self.log_audit(AuditEvent::ExportMnemonic { id }, false);
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

            let initial_len = self.entries.len();
            self.entries.retain(|entry| entry.id != id);

            if self.entries.len() == initial_len {
                return Err(KeystoreError::KeyNotFound(id));
            }

            self.save_to_disk()?;
            Ok(())
        })();

        self.log_audit(AuditEvent::DeleteKey { id }, result.is_ok());
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

            self.save_to_disk()?;
            Ok(())
        })();

        self.log_audit(AuditEvent::ChangePassword, result.is_ok());
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

    fn decrypt_mnemonic_material(
        &self,
        master_key: &[u8; 32],
        entry: &KeyEntry,
    ) -> Result<MnemonicMaterial, KeystoreError> {
        let decrypted_data = self.decrypt_data(
            master_key,
            &entry.nonce,
            &entry.encrypted_data,
            entry.id.as_bytes(),
        )?;

        if let Ok(payload) = serde_json::from_slice::<MnemonicPayload>(decrypted_data.as_ref()) {
            if payload.version != MNEMONIC_PAYLOAD_VERSION {
                return Err(KeystoreError::InvalidInput(format!(
                    "Unsupported mnemonic payload version: {}",
                    payload.version
                )));
            }

            return Ok(MnemonicMaterial {
                mnemonic: Zeroizing::new(payload.mnemonic),
                passphrase: payload.passphrase.map(Zeroizing::new),
            });
        }

        Err(KeystoreError::InvalidMnemonic(
            "Unsupported mnemonic payload format".to_string(),
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

        // Verify password by checking stored verification hash
        let computed_verification = self.create_verification_hash(&master_key)?;
        let stored_verification = self
            .master_key_verification
            .ok_or(KeystoreError::InvalidPassword)?;

        if computed_verification.ct_eq(&stored_verification).into() {
            let verified_file = self.verify_file_integrity(&master_key)?;

            self.entries = verified_file.entries;
            self.audit_log = verified_file.audit_log;
            self.kdf_params = Some(verified_file.kdf_params);
            self.master_key_verification = Some(verified_file.master_key_verification);
            self.file_integrity_mac = Some(verified_file.file_integrity_mac);
            self.master_key = Some(master_key);
            self.unlocked_at = Some(Instant::now());
            Ok(())
        } else {
            Err(KeystoreError::InvalidPassword)
        }
    }

    fn derive_master_key(
        &self,
        password: &str,
        params: &ArgonParams,
    ) -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
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

        cipher.encrypt(nonce, payload).map_err(KeystoreError::from)
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

        cipher
            .decrypt(nonce, payload)
            .map(Zeroizing::new)
            .map_err(KeystoreError::from)
    }

    fn save_to_disk(&mut self) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        let parent = self.ensure_parent_directory_safe()?;
        let _lock = self.acquire_mutation_lock(&parent)?;
        self.verify_no_external_modification()?;
        self.ensure_master_key_available()?;

        let kdf_params = self
            .kdf_params
            .as_ref()
            .ok_or(KeystoreError::InvalidInput("No KDF parameters".to_string()))?;

        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

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

        // Serialize to get canonical byte representation
        let json_data_without_mac = serde_json::to_string_pretty(&keystore_file_without_mac)?;

        // Compute MAC over the canonical serialized data (excluding the placeholder MAC)
        let file_integrity_mac =
            self.compute_file_integrity_mac(master_key, json_data_without_mac.as_bytes())?;

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

        let json_data = serde_json::to_vec_pretty(&keystore_file)?;
        self.atomic_write_keystore_file(&parent, &json_data)?;
        self.file_integrity_mac = Some(file_integrity_mac);

        Ok(())
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;
        self.early_file_validation()?;

        let data = fs::read(&self.path)?;
        let keystore_file: KeystoreFile = serde_json::from_slice(&data)?;

        if keystore_file.version != KEYSTORE_FILE_VERSION {
            return Err(KeystoreError::InvalidInput(format!(
                "Unsupported keystore version: {}",
                keystore_file.version
            )));
        }
        self.validate_keystore_shape(&keystore_file)?;

        self.kdf_params = Some(keystore_file.kdf_params);
        self.master_key_verification = Some(keystore_file.master_key_verification);
        self.file_integrity_mac = Some(keystore_file.file_integrity_mac);
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

        let data = fs::read(&self.path)
            .map_err(|_| KeystoreError::InvalidInput("Cannot read keystore file".to_string()))?;

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

        // Parse JSON structure to ensure it's valid
        let keystore_file: KeystoreFile = serde_json::from_slice(&data).map_err(|_| {
            KeystoreError::InvalidInput("Malformed keystore file - invalid JSON".to_string())
        })?;

        // Basic structure validation
        if keystore_file.version != KEYSTORE_FILE_VERSION {
            return Err(KeystoreError::InvalidInput(format!(
                "Unsupported keystore version: {}",
                keystore_file.version
            )));
        }
        self.validate_keystore_shape(&keystore_file)?;

        Ok(())
    }

    fn verify_file_integrity(&self, master_key: &[u8; 32]) -> Result<KeystoreFile, KeystoreError> {
        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;

        // Read the file again for verification
        let data = fs::read(&self.path)?;

        // Parse to extract the data without the MAC for verification
        let keystore_file: KeystoreFile = serde_json::from_slice(&data)?;
        if keystore_file.version != KEYSTORE_FILE_VERSION {
            return Err(KeystoreError::InvalidInput(format!(
                "Unsupported keystore version: {}",
                keystore_file.version
            )));
        }
        self.validate_keystore_shape(&keystore_file)?;

        if let Some(expected_mac) = self.file_integrity_mac {
            if expected_mac.ct_ne(&keystore_file.file_integrity_mac).into() {
                return Err(KeystoreError::InvalidInput(
                    "Concurrent modification detected while unlocking keystore".to_string(),
                ));
            }
        }
        let stored_mac = keystore_file.file_integrity_mac;

        // Create the same structure used during save (with placeholder MAC)
        let keystore_file_without_mac = KeystoreFile {
            version: keystore_file.version,
            kdf_params: keystore_file.kdf_params.clone(),
            master_key_verification: keystore_file.master_key_verification,
            audit_log: keystore_file.audit_log.clone(),
            entries: keystore_file.entries.clone(),
            file_integrity_mac: [0u8; 32], // Same placeholder used during save
        };

        let json_data_without_mac = serde_json::to_string_pretty(&keystore_file_without_mac)?;
        let computed_mac =
            self.compute_file_integrity_mac(master_key, json_data_without_mac.as_bytes())?;

        // Constant-time comparison to prevent timing attacks
        if computed_mac.ct_ne(&stored_mac).into() {
            return Err(KeystoreError::InvalidInput(
                "File integrity verification failed - keystore may have been tampered with"
                    .to_string(),
            ));
        }

        Ok(keystore_file)
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
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(MutationLockGuard { file })
    }

    fn verify_no_external_modification(&self) -> Result<(), KeystoreError> {
        if !self.path.exists() {
            if self.file_integrity_mac.is_some() {
                return Err(KeystoreError::InvalidInput(
                    "Concurrent modification detected while writing keystore".to_string(),
                ));
            }
            return Ok(());
        }

        let expected_mac = self.file_integrity_mac.ok_or(KeystoreError::InvalidInput(
            "Refusing to overwrite existing keystore without integrity state".to_string(),
        ))?;

        let data = fs::read(&self.path)?;
        if data.len() < MIN_KEYSTORE_SIZE || data.len() > MAX_KEYSTORE_SIZE {
            return Err(KeystoreError::InvalidInput(
                "Concurrent modification detected while writing keystore".to_string(),
            ));
        }
        let current_file: KeystoreFile = serde_json::from_slice(&data).map_err(|_| {
            KeystoreError::InvalidInput(
                "Concurrent modification detected while writing keystore".to_string(),
            )
        })?;

        if current_file.file_integrity_mac.ct_ne(&expected_mac).into() {
            return Err(KeystoreError::InvalidInput(
                "Concurrent modification detected while writing keystore".to_string(),
            ));
        }

        Ok(())
    }

    fn atomic_write_keystore_file(&self, parent: &Path, data: &[u8]) -> Result<(), KeystoreError> {
        if self.path.exists() {
            self.ensure_target_path_is_safe()?;
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
            self.log_audit(AuditEvent::Lock, true);
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
