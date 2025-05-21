// modules
pub mod error;
#[cfg(test)]
mod tests;

// keystore impl
use error::KeystoreError;
use k256::ecdsa::signature::hazmat::{PrehashVerifier, PrehashSigner};
use k256::ecdsa::VerifyingKey;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy_primitives::{Address, B256, B256 as H256}; // Removed U256
use alloy_signer::{Signature as AlloySignature, Error as AlloySignerError, Signer};
use argon2::{self, Argon2}; // Import argon2 module for Params
use async_trait::async_trait;
use atomicwrites::{AtomicFile, OverwriteBehavior};
use subtle::ConstantTimeEq;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _}; // For base64 encoding
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic; // Ensure Seed is not imported from bip39
use chrono::{DateTime, Utc};
use dirs_next;
use fs2::FileExt; // For file locking
use hex; // For encoding salt
use k256::{ecdsa::SigningKey, SecretKey}; // Removed PublicKey
use rand_core::{CryptoRng, OsRng, RngCore}; // Added OsRng
use ring::hkdf; // For HKDF key derivation
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write; // Added for f.write_all
use std::path::PathBuf;
use std::str::FromStr; // For DerivationPath::from_str
use std::sync::{Arc, RwLock, atomic::{AtomicU32, Ordering}};
use std::time::{Duration, Instant};
use tiny_keccak::{Hasher, Keccak}; // For Keccak-256
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing}; // Added Zeroizing struct

// --- Structs for Keystore Data ---

// V1 Structures (for deserializing old format)
#[derive(Deserialize, Debug, Clone)]
struct EncryptedKeyEntryV1 {
    id: Uuid,
    alias: Option<String>,
    address: Address,
    encrypted_pk: String,
    nonce: String, // Base64 random nonce
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Deserialize, Debug, Clone)]
struct KeystoreFileV1 {
    version: String,
    master_kdf: String,
    master_kdf_params: MasterKdfParams, // Assuming MasterKdfParams is compatible for V1
    entries: Vec<EncryptedKeyEntryV1>,
}

// Current (V2) Structures
#[derive(Serialize, Deserialize, Debug, Clone, Zeroize, ZeroizeOnDrop)] // Zeroize for salt is fine
pub struct MasterKdfParams {
    pub salt: String, // hex_encoded_salt
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub output_len: usize,
}

// Uuid and DateTime<Utc> do not implement Zeroize/ZeroizeOnDrop by default.
// String fields (encrypted_pk, nonce) do. Alias is Option<String>.
// Address is a fixed-size array, which should be fine.
// Removing ZeroizeOnDrop from EncryptedKeyEntry derive.
// String fields within will handle their own zeroization as String implements ZeroizeOnDrop.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedKeyEntry {
    pub id: Uuid,                  // Not secret
    pub alias: Option<String>,     // String part will be zeroized on drop if Some
    pub address: Address,          // Not secret
    pub encrypted_pk: String, // base64 encoded encrypted private key - String implements ZeroizeOnDrop
    pub encryption_counter: u64,   // Counter for nonce derivation
    pub created_at: DateTime<Utc>, // Not secret
    pub updated_at: DateTime<Utc>, // Not secret
                              // Potentially other metadata like derivation path if applicable, key type, etc.
}

// KeystoreFile's ZeroizeOnDrop will apply to its String fields.
// MasterKdfParams also derives ZeroizeOnDrop for its salt.
// Vec<EncryptedKeyEntry> elements (Strings) will be zeroized when they are dropped.
#[derive(Serialize, Deserialize, Debug, ZeroizeOnDrop)]
struct KeystoreFile {
    version: String,    // String implements ZeroizeOnDrop
    password_verification_tag: String, // String implements ZeroizeOnDrop
    master_kdf: String, // String implements ZeroizeOnDrop
    master_kdf_params: MasterKdfParams,
    #[zeroize(skip)] // Skip entries vec, as EncryptedKeyEntry doesn't derive ZeroizeOnDrop itself.
    // Sensitive String fields within EncryptedKeyEntry will self-zeroize.
    entries: Vec<EncryptedKeyEntry>,
}

// Information about a key, returned by list_keys
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyInfo {
    pub id: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Configuration for keystore behavior
#[derive(Debug, Clone)]
pub struct KeystoreConfig {
    // KDF parameters
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub output_len: usize,
    // Session management
    pub auto_lock_timeout: Duration,
    // Rate limiting
    pub unlock_min_delay: Duration,
    pub unlock_max_attempts: u32,
    pub unlock_backoff_factor: f32,
}

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            // Default KDF parameters
            m_cost: 65536,
            t_cost: 3,
            p_cost: 1,
            output_len: 32,
            // Default session management
            auto_lock_timeout: Duration::from_secs(300), // 5 minutes
            // Default rate limiting
            unlock_min_delay: Duration::from_millis(500),
            unlock_max_attempts: 5,
            unlock_backoff_factor: 2.0,
        }
    }
}

#[async_trait]
impl Signer for EphemeralSigner {
    fn address(&self) -> Address {
        let verifying_key = self.key.as_ref().verifying_key(); // AsRef to get k256::SigningKey
        let uncompressed_pk = verifying_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hashed_pk = [0u8; 32];
        keccak.finalize(&mut hashed_pk);
        let address_bytes: [u8; 20] = hashed_pk[12..]
            .try_into()
            .expect("Address derivation failed unexpectedly"); // Should not panic if logic is correct
        Address::from(address_bytes)
    }

    async fn sign_hash(&self, hash: &B256) -> Result<AlloySignature, AlloySignerError> {
        let signature_k256: k256::ecdsa::recoverable::Signature = self
            .key
            .as_ref() // Get &k256::SigningKey from &ZeroizingSigningKey
            .sign_prehash_recoverable(hash.as_slice())
            .map_err(|e| AlloySignerError::Other(Box::new(e)))?;

        let r_bytes: [u8; 32] = signature_k256.r().to_bytes().into();
        let s_bytes: [u8; 32] = signature_k256.s().to_bytes().into();
        let parity_id: u8 = signature_k256.recovery_id().to_byte();

        // Ensure parity_id is 0 or 1 as expected by alloy_signer::Signature::from_scalars_and_parity
        // k256 recovery_id can be 0, 1, 2, or 3.
        // Ethereum's V is typically 27 + recovery_id or (for EIP-155) 35 + 2*chain_id + recovery_id.
        // alloy_signer's Signature::from_scalars_and_parity expects parity_id to be 0 or 1.
        // k256::ecdsa::recoverable::Signature's recovery_id().to_byte() is 0 or 1 if normalized, or 0,1,2,3.
        // We need to ensure we provide what alloy expects.
        // Let's assume alloy_signer handles the conversion from a simple 0/1 parity.
        // If signature_k256.recovery_id() is already 0 or 1, this is fine.
        // k256::ecdsa::recoverable::Signature normalizes the S value and recovery ID.
        // So, recovery_id().to_byte() should be 0 or 1.

        let alloy_sig = AlloySignature::from_scalars_and_parity(r_bytes, s_bytes, parity_id)
            .map_err(|e| AlloySignerError::Other(Box::new(e)))?; // Convert the string error

        Ok(alloy_sig)
    }
}

// Wrapper for a signing key that ensures zeroization on drop.
#[derive(Debug)]
pub struct EphemeralSigner {
    pub key: ZeroizingSigningKey,
}

impl Drop for EphemeralSigner {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

// Wrapper for k256::SigningKey that implements ZeroizeOnDrop
#[derive(Debug)]
pub struct ZeroizingSigningKey(SigningKey);

impl ZeroizeOnDrop for ZeroizingSigningKey {}

impl Zeroize for ZeroizingSigningKey {
    fn zeroize(&mut self) {
        // self.0 is of type k256::SigningKey.
        // Its `to_bytes()` method returns a `k256::elliptic_curve::generic_array::GenericArray<u8, k256::elliptic_curve::FieldSize<k256::Secp256k1>>`.
        // We need to get a mutable slice to its contents and zeroize that.
        let mut secret_bytes = self.0.to_bytes();
        secret_bytes.as_mut_slice().zeroize();
    }
}

impl From<SigningKey> for ZeroizingSigningKey {
    fn from(key: SigningKey) -> Self {
        ZeroizingSigningKey(key)
    }
}

impl AsRef<SigningKey> for ZeroizingSigningKey {
    fn as_ref(&self) -> &SigningKey {
        &self.0
    }
}

// Allow direct access to the inner SigningKey when needed
impl std::ops::Deref for ZeroizingSigningKey {
    type Target = SigningKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// --- Keystore Struct ---

#[derive(Debug, Default)] // Default can be useful for initialization
struct KeystoreInner {
    master_key: Option<zeroize::Zeroizing<Vec<u8>>>,
    entries: Vec<EncryptedKeyEntry>, // V2 entries
    master_kdf_params: Option<MasterKdfParams>,
    is_unlocked: bool,
    last_activity_at: Option<Instant>,
    last_unlock_attempt: Option<Instant>,
    password_verification_tag: Option<String>, // V2 field
    v1_data_to_migrate: Option<KeystoreFileV1>, // For migration
    migration_pending: bool,                    // For migration
}

// Enum to hold loaded data, distinguishing between V1 and V2
#[derive(Debug)]
enum LoadedKeystoreData {
    V1(KeystoreFileV1),
    V2(KeystoreFile), // This is the current KeystoreFile struct (V2)
}

#[derive(Debug)]
pub struct Keystore {
    file_path: PathBuf, // Stays as is, Send+Sync
    config: KeystoreConfig,    // Stays as is, Send+Sync
    unlock_attempts: AtomicU32, // Stays as is, Send+Sync

    // Wrapped mutable state
    inner: std::sync::Arc<std::sync::RwLock<KeystoreInner>>,
}

// --- Keystore Implementation ---

impl Keystore {
    const DEFAULT_KEYSTORE_FILENAME: &'static str = "keystore_v1.json"; // Will be updated by versioning
    const APP_DIR_NAME: &'static str = "mfm";
    const KEYSTORE_VERSION_V1: &'static str = "1.0.0"; // For explicit V1 load check
    const KEYSTORE_VERSION_V2: &'static str = "2.0.0";
    const PASSWORD_VERIFICATION_PLAINTEXT: &'static [u8] = b"MFM_KEYSTORE_VERIFY_OK";
    const PASSWORD_VERIFICATION_AAD: &'static [u8] = b"mfm-keystore-password-verification-aad";
    const PASSWORD_VERIFICATION_NONCE_BYTES: [u8; 12] = [0u8; 12]; // Fixed nonce
    const HKDF_ENTRY_KEY_INFO_PREFIX: &'static [u8] = b"mfm-entry-key:";
    const HKDF_NONCE_SALT: &'static [u8] = b"mfm-keystore-nonce-salt";
    const HKDF_NONCE_INFO_PREFIX: &'static [u8] = b"mfm-nonce:";

    pub fn new(custom_path: Option<PathBuf>) -> Result<Self, KeystoreError> {
        Self::new_with_config(custom_path, KeystoreConfig::default())
    }

    pub fn new_with_config(
        custom_path: Option<PathBuf>,
        config: KeystoreConfig,
    ) -> Result<Self, KeystoreError> {
        let file_path = match custom_path {
            Some(path) => path,
            None => {
                let data_dir = dirs_next::data_dir().ok_or_else(|| {
                    KeystoreError::FsError("Could not determine system data directory".to_string())
                })?;
                let app_data_dir = data_dir.join(Self::APP_DIR_NAME);
                if !app_data_dir.exists() {
                    fs::create_dir_all(&app_data_dir).map_err(KeystoreError::Io)?;
                }
                app_data_dir.join(Self::DEFAULT_KEYSTORE_FILENAME)
            }
        };

        Ok(Self {
            file_path,
            config,
            unlock_attempts: AtomicU32::new(0),
            inner: Arc::new(RwLock::new(KeystoreInner::default())),
        })
    }

    pub fn initialize_or_load(&self, password: Option<&str>) -> Result<(), KeystoreError> {
        match self.load_from_disk_internal()? {
            Some(LoadedKeystoreData::V2(v2_data)) => {
                let mut inner = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
                inner.entries = v2_data.entries;
                inner.master_kdf_params = Some(v2_data.master_kdf_params);
                inner.password_verification_tag = Some(v2_data.password_verification_tag);
                inner.is_unlocked = false;
                inner.master_key = None;
                inner.migration_pending = false;
                inner.v1_data_to_migrate = None;
            }
            Some(LoadedKeystoreData::V1(v1_data)) => {
                let mut inner = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
                inner.v1_data_to_migrate = Some(v1_data);
                inner.migration_pending = true;
                // Keystore remains locked and unusable until migration is performed.
                // Fields like entries, master_kdf_params, password_verification_tag will be populated during migration.
                inner.is_unlocked = false; 
            }
            None => { // No file exists, initialize new V2 keystore
                if let Some(p) = password {
                    if p.is_empty() {
                        return Err(KeystoreError::InvalidPassword);
                    }
                    let mut inner = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
                    let kdf_params = Self::generate_kdf_params_with_config(&mut OsRng, &self.config)?;
                    let temp_master_key = self.derive_master_key(p, &kdf_params, &self.config)?;
                    let verification_tag = Self::generate_and_encrypt_verification_tag_static(&temp_master_key)?;
                    temp_master_key.zeroize();

                    inner.master_kdf_params = Some(kdf_params);
                    inner.password_verification_tag = Some(verification_tag);
                    inner.entries = Vec::new(); // Ensure entries is empty for a new keystore
                    inner.is_unlocked = false;
                    inner.migration_pending = false;
                    inner.v1_data_to_migrate = None;
                    
                    // Drop the lock before save_to_disk
                    drop(inner);
                    self.save_to_disk()?; // This will save as V2
                }
                // If no password, it remains uninitialized and migration_pending is false.
            }
        }
        Ok(())
    }

    pub fn unlock(&self, password: &str) -> Result<(), KeystoreError> {
        // Rate limiting implementation - needs to access inner.last_unlock_attempt
        self.enforce_unlock_rate_limiting()?; // Will be refactored

        let mut inner = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;

        if inner.is_unlocked {
            inner.last_activity_at = Some(Instant::now());
            return Ok(());
        }

        if password.is_empty() {
            // increment_unlock_attempts accesses self.unlock_attempts (Atomic) and inner.last_unlock_attempt
            // It's called here without holding the lock on inner, which is fine for self.unlock_attempts
            // but not for inner.last_unlock_attempt. Refactor increment_unlock_attempts.
            drop(inner); // Release lock before calling potentially re-entrant method
            self.increment_unlock_attempts()?;
            return Err(KeystoreError::InvalidPassword);
        }

        // Clone kdf_params to avoid holding the lock during derive_master_key
        let kdf_params_clone = inner.master_kdf_params.clone().ok_or_else(|| {
            KeystoreError::FsError("Keystore is not initialized with KDF parameters.".to_string())
        })?;
        
        // Clone password_verification_tag
        let pv_tag_clone = inner.password_verification_tag.clone();

        // Drop the lock before potentially long running crypto operations
        drop(inner);

        let mut derived_key =
            self.derive_master_key(password, &kdf_params_clone, &self.config)?;

        // Pass pv_tag_clone to decrypt_and_verify_password_tag
        let verification_ok =
            self.decrypt_and_verify_password_tag(&derived_key, pv_tag_clone)?;

        if !verification_ok {
            derived_key.zeroize();
            self.increment_unlock_attempts()?;
            return Err(KeystoreError::InvalidPassword);
        }

        // Re-acquire write lock to update keystore state
        let mut inner_write = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
        inner_write.master_key = Some(zeroize::Zeroizing::new(derived_key));
        inner_write.is_unlocked = true;
        inner_write.last_activity_at = Some(Instant::now());
        self.unlock_attempts.store(0, Ordering::SeqCst);
        Ok(())
    }

    // Enforce rate limiting for unlock attempts
    fn enforce_unlock_rate_limiting(&self) -> Result<(), KeystoreError> {
        let attempts = self.unlock_attempts.load(Ordering::SeqCst);
        let mut inner = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?; // Needs write for last_unlock_attempt

        if attempts > 0 {
            if let Some(last_attempt) = inner.last_unlock_attempt {
                let elapsed = last_attempt.elapsed();
                // calculate_backoff_delay now needs self.config
                let required_delay = self.calculate_backoff_delay(attempts, &self.config);

                if elapsed < required_delay {
                    return Err(KeystoreError::FsError(format!(
                        "Too many unlock attempts. Please wait {} seconds before trying again.",
                        (required_delay - elapsed).as_secs()
                    )));
                }
            }
        }
        inner.last_unlock_attempt = Some(Instant::now());
        Ok(())
    }

    // Calculate exponential backoff delay
    fn calculate_backoff_delay(&self, attempts: u32, config: &KeystoreConfig) -> Duration {
        if attempts >= self.config.unlock_max_attempts {
            // Maximum backoff reached
            Duration::from_secs(30)
        } else {
            let factor = config.unlock_backoff_factor.powi(attempts as i32);
            let millis = (config.unlock_min_delay.as_millis() as f32 * factor) as u64;
            Duration::from_millis(millis)
        }
    }

    // Increment unlock attempts counter
    fn increment_unlock_attempts(&self) -> Result<(), KeystoreError> { // Now returns Result
        self.unlock_attempts.fetch_add(1, Ordering::SeqCst);
        let mut inner = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
        inner.last_unlock_attempt = Some(Instant::now());
        Ok(())
    }

    pub fn lock(&self) {
        let mut inner = match self.inner.write() {
            Ok(guard) => guard,
            Err(_) => { /* Handle LockPoisoned, maybe log or panic depending on policy */ return }
        };
        inner.master_key = None; // This will zeroize the key due to Zeroizing wrapper
        inner.is_unlocked = false;
        inner.last_activity_at = Some(Instant::now()); // Record lock time as last activity
    }

    // Check if auto-lock should be triggered
    fn check_auto_lock(&self) { // Remains &self
        let (should_lock, lock_timeout) = {
            let inner_read = match self.inner.read() {
                Ok(guard) => guard,
                Err(_) => return // Or handle LockPoisoned
            };
            if inner_read.is_unlocked {
                if let Some(last_activity) = inner_read.last_activity_at {
                    (last_activity.elapsed() > self.config.auto_lock_timeout, Some(self.config.auto_lock_timeout))
                } else { (false, None) }
            } else { (false, None) }
        }; // Read lock is dropped here

        if should_lock {
            if lock_timeout.is_some() { // Ensure config was accessible
                 self.lock(); // lock() will acquire its own write lock
            }
        }
    }

    // Update last activity timestamp
    fn update_activity_timestamp(&self) { // Remains &self
        let mut inner = match self.inner.write() {
            Ok(guard) => guard,
            Err(_) => return // Or handle LockPoisoned
        };
        inner.last_activity_at = Some(Instant::now());
    }

    pub fn import_mnemonic(
        &self,
        alias: Option<String>,
        phrase: &str,
        passphrase: Option<&str>,
        path_str: &str, // e.g., "m/44'/60'/0'/0/0"
    ) -> Result<(Uuid, Address), KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        let master_key_bytes_clone;
        let current_entries_aliases;
        {
            let inner_read = self.inner.read().map_err(|_| KeystoreError::LockPoisoned)?;
            if !inner_read.is_unlocked || inner_read.master_key.is_none() {
                return Err(KeystoreError::Locked);
            }
            // Clone necessary data for operations outside the lock
            master_key_bytes_clone = inner_read.master_key.as_ref().unwrap().as_slice().to_vec(); // Assuming master_key is Some
            current_entries_aliases = inner_read.entries.iter().filter_map(|e| e.alias.clone()).collect::<Vec<String>>();
        }


        if phrase.is_empty() {
            return Err(KeystoreError::InvalidPrivateKey);
        }

        let mnemonic = Mnemonic::parse(phrase)?;
        let seed_bytes_array = mnemonic.to_seed(passphrase.unwrap_or(""));
        let seed_bytes_zeroizing = Zeroizing::new(seed_bytes_array.to_vec());
        let derivation_path = DerivationPath::from_str(path_str)
            .map_err(|e| KeystoreError::InvalidPath(format!("Failed to parse derivation path: {}", e)))?;
        let xprv = XPrv::derive_from_path(seed_bytes_zeroizing.as_slice(), &derivation_path)
            .map_err(KeystoreError::Bip32)?;
        let bip32_signing_key = xprv.private_key();
        let secret_bytes_from_bip32 = bip32_signing_key.to_bytes();
        let secret_bytes_zeroizing = Zeroizing::new(secret_bytes_from_bip32.to_vec());
        let secret_key = SecretKey::from_slice(secret_bytes_zeroizing.as_slice())
            .map_err(|_| KeystoreError::DerivationFailed)?;
        let pk_bytes_for_encryption = Zeroizing::new(secret_key.to_bytes().to_vec());
        let signing_key = SigningKey::from(&secret_key);
        let public_key = signing_key.verifying_key();
        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hashed_pk = [0u8; 32];
        keccak.finalize(&mut hashed_pk);
        let address_bytes: [u8; 20] = hashed_pk[12..].try_into().map_err(|_| KeystoreError::DerivationFailed)?;
        let address = Address::from(address_bytes);

        if let Some(ref new_alias) = alias {
            if current_entries_aliases.contains(new_alias) {
                 return Err(KeystoreError::AliasExists(new_alias.clone()));
            }
        }

        let id = Uuid::new_v4();
        let encryption_counter = 1;
        // Pass cloned master_key_bytes_clone to encrypt_pk if it needs it, or if encrypt_pk is static-like
        // For now, assuming encrypt_pk needs master_key_bytes and other params directly
        // and that it can be called without holding a lock on `inner` if it's refactored.
        // Let's assume encrypt_pk needs a master_key, id, address, counter.
        // We will need to pass master_key_bytes_clone to it.

        let encrypted_pk_vec = self.encrypt_pk_static( // Assuming a static version or one that takes master_key
            &pk_bytes_for_encryption,
            &id,
            &address,
            encryption_counter,
            &master_key_bytes_clone, // Pass cloned master key
            self.master_kdf_params.as_ref().ok_or(KeystoreError::MissingMasterKdfParams)?.clone() // Pass cloned KDF params
        )?;


        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            encryption_counter,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        {
            let mut inner_write = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
            inner_write.entries.push(entry);
            inner_write.last_activity_at = Some(Instant::now());
        }

        self.save_to_disk()?; // save_to_disk will acquire its own lock
        Ok((id, address))
    }

    pub fn import_private_key_hex(
        &self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        self.check_auto_lock();
        let master_key_bytes_clone;
        let current_entries_aliases;
        {
            let inner_read = self.inner.read().map_err(|_| KeystoreError::LockPoisoned)?;
            if !inner_read.is_unlocked || inner_read.master_key.is_none() {
                return Err(KeystoreError::Locked);
            }
            master_key_bytes_clone = inner_read.master_key.as_ref().unwrap().as_slice().to_vec();
            current_entries_aliases = inner_read.entries.iter().filter_map(|e| e.alias.clone()).collect::<Vec<String>>();
        }

        if pk_hex.is_empty() {
            return Err(KeystoreError::InvalidPrivateKey);
        }
        let pk_bytes = Zeroizing::new(hex::decode(pk_hex)?);
        if pk_bytes.len() != 32 {
            return Err(KeystoreError::InvalidPrivateKey);
        }
        let secret_key = SecretKey::from_slice(pk_bytes.as_slice()).map_err(|_| KeystoreError::InvalidPrivateKey)?;
        let signing_key = SigningKey::from(secret_key);
        let public_key = signing_key.verifying_key();
        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hashed_pk = [0u8; 32];
        keccak.finalize(&mut hashed_pk);
        let address_bytes: [u8; 20] = hashed_pk[12..].try_into().map_err(|_| KeystoreError::DerivationFailed)?;
        let address = Address::from(address_bytes);

        if let Some(ref new_alias) = alias {
             if current_entries_aliases.contains(new_alias) {
                return Err(KeystoreError::AliasExists(new_alias.clone()));
            }
        }

        let id = Uuid::new_v4();
        let encryption_counter = 1;
        // Similar to import_mnemonic, pass cloned master_key_bytes to encrypt_pk_static
        let encrypted_pk_vec = self.encrypt_pk_static(
            pk_bytes.as_slice(),
            &id,
            &address,
            encryption_counter,
            &master_key_bytes_clone,
            self.master_kdf_params.as_ref().ok_or(KeystoreError::MissingMasterKdfParams)?.clone()
        )?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            encryption_counter,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        {
            let mut inner_write = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
            inner_write.entries.push(entry);
            inner_write.last_activity_at = Some(Instant::now());
        }
        self.save_to_disk()?;
        Ok((id, address))
    }

    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        self.update_activity_timestamp(); // Acquires write lock
        let inner_read = self.inner.read().map_err(|_| KeystoreError::LockPoisoned)?;
        let key_infos = inner_read
            .entries
            .iter()
            .map(|entry| KeyInfo {
                id: entry.id,
                alias: entry.alias.clone(),
                address: entry.address,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
            })
            .collect();
        Ok(key_infos)
    }

    pub fn get_signer(&self, uuid: Uuid) -> Result<EphemeralSigner, KeystoreError> {
        self.check_auto_lock();
        let master_key_bytes_clone;
        let entry_clone;
        {
            let inner_read = self.inner.read().map_err(|_| KeystoreError::LockPoisoned)?;
            if !inner_read.is_unlocked || inner_read.master_key.is_none() {
                return Err(KeystoreError::Locked);
            }
            master_key_bytes_clone = inner_read.master_key.as_ref().unwrap().as_slice().to_vec();
            entry_clone = inner_read
                .entries
                .iter()
                .find(|e| e.id == uuid)
                .cloned() // Clone the entry to release the lock sooner
                .ok_or(KeystoreError::KeyNotFound(uuid))?;
        } // Read lock dropped here

        let encrypted_pk_bytes = BASE64_STANDARD
            .decode(&entry_clone.encrypted_pk)
            .map_err(|_e| KeystoreError::InvalidFormat)?;

        // Assuming decrypt_pk_static takes master_key_bytes, id, address, counter, and kdf_params
        let decrypted_pk_zeroizing_vec = self.decrypt_pk_static(
            &encrypted_pk_bytes,
            &entry_clone.id,
            &entry_clone.address,
            entry_clone.encryption_counter,
            &master_key_bytes_clone,
            self.master_kdf_params.as_ref().ok_or(KeystoreError::MissingMasterKdfParams)?.clone()
        )?;

        let secret_key = SecretKey::from_slice(decrypted_pk_zeroizing_vec.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;
        let signing_key = SigningKey::from(secret_key);
        let z_signing_key = ZeroizingSigningKey::from(signing_key);

        self.update_activity_timestamp(); // Acquires write lock
        Ok(EphemeralSigner { key: z_signing_key })
    }

    // Verify a signature against a message hash using the key identified by uuid
    pub fn verify_signature(
        &self,
        uuid: Uuid,
        message_hash: H256,
        signature: AlloySignature,
    ) -> Result<bool, KeystoreError> {
        self.check_auto_lock();
        // Get the signer. This will handle locking for get_signer and update_activity_timestamp.
        let signer = self.get_signer(uuid)?; // This already updates activity.
        let verifying_key = signer.key.as_ref().verifying_key();
        let r_bytes = signature.r().to_be_bytes::<32>();
        let s_bytes = signature.s().to_be_bytes::<32>();
        let mut signature_bytes = [0u8; 64];
        signature_bytes[..32].copy_from_slice(&r_bytes);
        signature_bytes[32..].copy_from_slice(&s_bytes);
        let k256_signature = k256::ecdsa::Signature::from_slice(&signature_bytes)
            .map_err(|_| KeystoreError::SignatureVerificationFailed)?;
        let message_bytes = message_hash.as_slice();
        let result = verifying_key.verify_prehash(message_bytes, &k256_signature).is_ok();
        Ok(result)
    }

    pub fn delete_key(&self, uuid: Uuid) -> Result<(), KeystoreError> {
        self.check_auto_lock();
        {
            let mut inner_write = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;
            let initial_len = inner_write.entries.len();
            inner_write.entries.retain(|entry| entry.id != uuid);
            if inner_write.entries.len() == initial_len {
                return Err(KeystoreError::KeyNotFound(uuid));
            }
            inner_write.last_activity_at = Some(Instant::now());
        }
        self.save_to_disk()?;
        Ok(())
    }

    pub fn change_password(
        &self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        self.check_auto_lock();

        // Verify old password by attempting an unlock (unlock handles its own locking)
        // We need to ensure that unlock doesn't hold a lock when change_password needs one.
        // The current unlock implementation tries to minimize lock duration.
        self.unlock(old_password)?; // This will set master_key if successful.

        if new_password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        let mut inner_write = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;

        let new_kdf_params = Self::generate_kdf_params_with_config(&mut OsRng, &self.config)?;

        // Derive new master key. derive_master_key takes config.
        let new_master_key_vec = self.derive_master_key(new_password, &new_kdf_params, &self.config)?;
        let new_master_key_zeroizing = Zeroizing::new(new_master_key_vec.clone()); // Clone for verification tag

        // Generate the new password verification tag
        let verification_tag =
            Self::generate_and_encrypt_verification_tag_static(&new_master_key_vec)?;
        inner_write.password_verification_tag = Some(verification_tag);
        
        // Store the current entries (clone to work on them outside the main lock for a moment if needed, though here we re-encrypt)
        let old_entries = inner_write.entries.clone();
        let old_master_key_option = inner_write.master_key.take(); // Take the old master key

        // Drop main write lock before re-encryption loop if decrypt/encrypt are to be static-like.
        // However, we need to update entries, so we'll hold it.
        // If decrypt_pk/encrypt_pk were static, we'd need to pass master_key_bytes.
        // The current master_key is now new_master_key_zeroizing.
        inner_write.master_key = Some(new_master_key_zeroizing.clone()); // Set new master key for encrypt_pk

        let mut re_encrypted_entries = Vec::new();

        // We need the old master key bytes for decryption.
        let old_master_key_bytes = old_master_key_option.ok_or(KeystoreError::Locked)?.as_slice().to_vec();

        for old_entry in old_entries {
            let encrypted_pk_bytes = BASE64_STANDARD.decode(&old_entry.encrypted_pk)
                .map_err(|_| KeystoreError::InvalidFormat)?;

            // Decrypt with old key (needs old_master_key_bytes, entry details, old kdf_params)
            // Assuming decrypt_pk_static needs: enc_pk, id, addr, counter, old_master_key, old_kdf_params
            let decrypted_pk = self.decrypt_pk_static(
                &encrypted_pk_bytes,
                &old_entry.id,
                &old_entry.address,
                old_entry.encryption_counter,
                &old_master_key_bytes, // Pass old master key bytes
                inner_write.master_kdf_params.as_ref().unwrap().clone() // Pass old kdf_params
            )?;

            let new_encryption_counter = old_entry.encryption_counter.checked_add(1)
                .ok_or_else(|| KeystoreError::FsError("Encryption counter overflow".to_string()))?;

            // Encrypt with new key (needs new_master_key_bytes, entry details, new kdf_params)
            // Assuming encrypt_pk_static needs: dec_pk, id, addr, counter, new_master_key, new_kdf_params
            let new_encrypted_pk = self.encrypt_pk_static(
                &decrypted_pk,
                &old_entry.id,
                &old_entry.address,
                new_encryption_counter,
                &new_master_key_vec, // Pass new master key bytes
                new_kdf_params.clone() // Pass new kdf_params
            )?;

            let new_entry = EncryptedKeyEntry {
                id: old_entry.id,
                alias: old_entry.alias,
                address: old_entry.address,
                encrypted_pk: BASE64_STANDARD.encode(&new_encrypted_pk),
                encryption_counter: new_encryption_counter,
                created_at: old_entry.created_at,
                updated_at: Utc::now(),
            };
            re_encrypted_entries.push(new_entry);
        }

        inner_write.entries = re_encrypted_entries;
        inner_write.master_kdf_params = Some(new_kdf_params);
        inner_write.last_activity_at = Some(Instant::now());
        // The new master_key is already set in inner_write.master_key

        drop(inner_write); // Release lock before save_to_disk
        self.save_to_disk()?;
        Ok(())
    }

    fn save_to_disk(&self) -> Result<(), KeystoreError> {
        // Create parent directories if they don't exist
        if let Some(parent) = self.file_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(KeystoreError::Io)?;
            }
        }

        let lock_file_path = self.file_path.with_extension("json.lock");
        let lock_file = fs::OpenOptions::new().write(true).create(true).open(&lock_file_path).map_err(KeystoreError::Io)?;
        lock_file.lock_exclusive().map_err(|e| KeystoreError::FsError(format!("Failed to acquire exclusive lock on lock file: {}", e)))?;

        let write_result = (|| {
            let inner_read = self.inner.read().map_err(|_| KeystoreError::LockPoisoned)?;
            let kdf_params = inner_read.master_kdf_params.as_ref().ok_or(KeystoreError::FsError(
                "Keystore is not initialized with KDF parameters for V2 save.".to_string(),
            ))?;
            let pv_tag = inner_read.password_verification_tag.as_ref().ok_or(KeystoreError::FsError(
                "Password verification tag missing for V2 save.".to_string()
            ))?;
            let keystore_data = KeystoreFile {
                version: Self::KEYSTORE_VERSION_V2.to_string(),
                password_verification_tag: pv_tag.clone(),
                master_kdf: "argon2id".to_string(),
                master_kdf_params: kdf_params.clone(),
                entries: inner_read.entries.clone(),
            };
            let json_data = serde_json::to_string_pretty(&keystore_data)?;
            let atomic_file = AtomicFile::new(&self.file_path, OverwriteBehavior::AllowOverwrite);
            atomic_file.write(|f| f.write_all(json_data.as_bytes()).map_err(KeystoreError::Io))
                .map_err(|e| KeystoreError::FsError(format!("Failed to write keystore file: {}", e)))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&self.file_path).map_err(KeystoreError::Io)?.permissions();
                perms.set_mode(0o600);
                fs::set_permissions(&self.file_path, perms).map_err(KeystoreError::Io)?;
            }
            #[cfg(windows)]
            {
                // Convert self.file_path to an OsStr for the windows function
                set_windows_file_permissions(self.file_path.as_os_str()).map_err(|e| {
                    // Potentially log this error but don't make it fatal if keystore write itself was okay?
                    // For now, let's map it to FsError, though a dedicated error might be better.
                    KeystoreError::FsError(format!("Failed to set Windows file permissions: {}", e))
                })?;
            }
            Ok(())
        })();

        let unlock_result = lock_file.unlock().map_err(|e| KeystoreError::FsError(format!("Failed to unlock lock file: {}", e)));
        fs::remove_file(&lock_file_path).ok();
        write_result?;
        unlock_result?;
        Ok(())
    }

    // Internal loading logic, returns an enum indicating V1 or V2 data, or None if no file.
    fn load_from_disk_internal(&self) -> Result<Option<LoadedKeystoreData>, KeystoreError> {
        if !self.file_path.exists() {
            return Ok(None);
        }

        let lock_file_path = self.file_path.with_extension("json.lock");
        let lock_file = fs::OpenOptions::new().read(true).write(true).create(true).open(&lock_file_path).map_err(KeystoreError::Io)?;
        lock_file.lock_shared().map_err(|e| KeystoreError::FsError(format!("Failed to acquire shared lock on lock file: {}", e)))?;

        let read_result = (|| {
            let file_content = fs::read_to_string(&self.file_path)?;
            
            // Try to deserialize as V2 first (current KeystoreFile format)
            match serde_json::from_str::<KeystoreFile>(&file_content) {
                Ok(v2_data) if v2_data.version == Self::KEYSTORE_VERSION_V2 => {
                    // Validate V2 specific fields
                    if v2_data.master_kdf_params.output_len < 32 {
                         return Err(KeystoreError::InvalidKeystoreFormat("V2 Keystore KDF output length is less than 32 bytes.".to_string()));
                    }
                    if v2_data.master_kdf != "argon2id" {
                         return Err(KeystoreError::UnsupportedKdf(v2_data.master_kdf.clone()));
                    }
                    return Ok(Some(LoadedKeystoreData::V2(v2_data)));
                }
                _ => { // If V2 fails or version doesn't match, try V1
                    match serde_json::from_str::<KeystoreFileV1>(&file_content) {
                        Ok(v1_data) if v1_data.version == Self::KEYSTORE_VERSION_V1 => {
                             // Validate V1 specific fields (if any beyond schema)
                            if v1_data.master_kdf_params.output_len < 32 {
                                return Err(KeystoreError::InvalidKeystoreFormat("V1 Keystore KDF output length is less than 32 bytes.".to_string()));
                            }
                            if v1_data.master_kdf != "argon2id" { // Assuming V1 also used argon2id
                                return Err(KeystoreError::UnsupportedKdf(v1_data.master_kdf.clone()));
                            }
                            Ok(Some(LoadedKeystoreData::V1(v1_data)))
                        }
                        _ => Err(KeystoreError::InvalidKeystoreFormat("Unable to parse keystore as V1 or V2 format.".to_string())),
                    }
                }
            }
        })();

        let unlock_result = lock_file.unlock().map_err(|e| KeystoreError::FsError(format!("Failed to unlock lock file: {}", e)));
        
        // Propagate read error first, then unlock error
        let loaded_data = read_result?;
        unlock_result?;
        
        Ok(loaded_data)
    }
    
    // This method is now effectively replaced by load_from_disk_internal and initialize_or_load's direct handling
    // For now, we'll keep it as a stub or remove it if all callers are updated.
    // Let's assume it's no longer directly called by public API after initialize_or_load refactor.
    fn load_from_disk(&self) -> Result<(), KeystoreError> {
        // This method is now a bit redundant. The actual loading and population
        // happens in initialize_or_load via load_from_disk_internal.
        // If this was meant for a "reload" functionality, it would need to be re-thought.
        // For now, let's make it a no-op or return an error indicating it shouldn't be used directly.
        // Returning Ok(()) for now to avoid breaking existing internal calls if any, but it's effectively unused.
        // The primary loading logic is now in initialize_or_load via load_from_disk_internal.
        Ok(())
    }

    fn decrypt_v1_entry_payload(
        master_key_bytes: &[u8],
        kdf_params_v1: &MasterKdfParams, // Passed for derive_entry_key_static
        entry_v1: &EncryptedKeyEntryV1,
        // Keystore constants are available via Self::
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let entry_key = Keystore::derive_entry_key_static_helper(master_key_bytes, &entry_v1.id, kdf_params_v1)?;
        
        let nonce_v1_bytes = BASE64_STANDARD.decode(&entry_v1.nonce)
            .map_err(|_| KeystoreError::MigrationV1DataError)?; // Specific error for nonce decoding
        
        if nonce_v1_bytes.len() != 12 {
            return Err(KeystoreError::MigrationV1DataError); // Nonce length error
        }
        let nonce_array: [u8; 12] = nonce_v1_bytes.try_into().unwrap(); // Should not panic due to length check
        let nonce = Nonce::from_slice(&nonce_array);

        let aad = Keystore::create_aad_static(&entry_v1.id, &entry_v1.address);
        
        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);

        let encrypted_pk_bytes = BASE64_STANDARD.decode(&entry_v1.encrypted_pk)
            .map_err(|_| KeystoreError::MigrationV1DataError)?;

        cipher.decrypt(nonce, aes_gcm::aead::Payload { msg: &encrypted_pk_bytes, aad: &aad })
            .map(Zeroizing::new)
            .map_err(|_| KeystoreError::MigrationV1DataError) // Decryption failure
    }


    pub fn migrate_v1_keystore(&self, password: &str) -> Result<(), KeystoreError> {
        let mut inner = self.inner.write().map_err(|_| KeystoreError::LockPoisoned)?;

        if !inner.migration_pending || inner.v1_data_to_migrate.is_none() {
            return Err(KeystoreError::MigrationNotPending);
        }

        let v1_data = inner.v1_data_to_migrate.take().unwrap(); // Safe due to check above

        // Derive Master Key (V1)
        let mut v1_master_key_bytes = self.derive_master_key(password, &v1_data.master_kdf_params, &self.config)?;
        
        // Generate Password Verification Tag for V2
        let v2_password_verification_tag = Self::generate_and_encrypt_verification_tag_static(&v1_master_key_bytes)?;

        // Migrate Entries
        let mut new_v2_entries = Vec::new();
        for entry_v1 in v1_data.entries.iter() {
            let decrypted_pk_bytes = Self::decrypt_v1_entry_payload(
                &v1_master_key_bytes,
                &v1_data.master_kdf_params,
                entry_v1,
            )?;

            let new_encryption_counter = 1u64; // Start V2 counters at 1

            // Re-encrypt with new (V2) nonce mechanism, using the same master key for now
            let new_encrypted_pk_vec = self.encrypt_pk_static(
                &decrypted_pk_bytes,
                &entry_v1.id,
                &entry_v1.address,
                new_encryption_counter,
                &v1_master_key_bytes, // Using the derived V1 master key
                v1_data.master_kdf_params.clone(), // KDF params remain the same for this step
            )?;
            
            let v2_entry = EncryptedKeyEntry {
                id: entry_v1.id,
                alias: entry_v1.alias.clone(),
                address: entry_v1.address,
                encrypted_pk: BASE64_STANDARD.encode(&new_encrypted_pk_vec),
                encryption_counter: new_encryption_counter,
                created_at: entry_v1.created_at,
                updated_at: Utc::now(), // Update timestamp to migration time
            };
            new_v2_entries.push(v2_entry);
        }
        
        // Update KeystoreInner
        inner.entries = new_v2_entries;
        inner.master_kdf_params = Some(v1_data.master_kdf_params.clone());
        inner.password_verification_tag = Some(v2_password_verification_tag);
        inner.is_unlocked = false; // Keystore remains locked after migration
        inner.master_key = None;   // Clear any temporary master key
        inner.migration_pending = false;
        // v1_data_to_migrate is already None due to take()
        inner.last_activity_at = Some(Instant::now());

        v1_master_key_bytes.zeroize();
        drop(inner); // Release lock before save_to_disk

        self.save_to_disk()?; // Save as V2 format
        Ok(())
    }


    // Static-like helper for encrypt_pk
    fn encrypt_pk_static(
        &self, // Still takes &self if it needs things like derive_entry_key, which might need master_kdf_params from inner
        pk_bytes: &[u8],
        id: &Uuid,
        address: &Address,
        encryption_counter: u64,
        master_key_bytes: &[u8], // Explicitly pass master key
        kdf_params: MasterKdfParams, // Explicitly pass KDF params for derive_entry_key's salt
    ) -> Result<Vec<u8>, KeystoreError> {
        // Derive per-entry encryption key using HKDF
        // derive_entry_key needs to be refactored to take kdf_params or salt directly
        let entry_key = self.derive_entry_key_static(master_key_bytes, id, kdf_params)?;
        let nonce_array = self.derive_aes_gcm_nonce_static(master_key_bytes, id, encryption_counter)?; // Assuming static version

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_array);
        let aad = self.create_aad(id, address); // create_aad is simple, can remain as is or be static

        cipher.encrypt(nonce, aes_gcm::aead::Payload { msg: pk_bytes, aad: &aad })
            .map_err(|e| KeystoreError::AesGcm(format!("Encryption failed: {}", e)))
    }

    // Static-like helper for decrypt_pk
    fn decrypt_pk_static(
        &self,
        encrypted_pk_bytes: &[u8],
        id: &Uuid,
        address: &Address,
        encryption_counter: u64,
        master_key_bytes: &[u8], // Explicitly pass master key
        kdf_params: MasterKdfParams, // Explicitly pass KDF params
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let entry_key = self.derive_entry_key_static(master_key_bytes, id, kdf_params)?;
        let nonce_array = self.derive_aes_gcm_nonce_static(master_key_bytes, id, encryption_counter)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_array);
        let aad = self.create_aad(id, address);

        let decrypted_bytes = cipher.decrypt(nonce, aes_gcm::aead::Payload { msg: encrypted_pk_bytes, aad: &aad })
            .map_err(|_| KeystoreError::EntryDecryptionFailed)?;
        Ok(Zeroizing::new(decrypted_bytes))
    }

    // Decrypt and verify the password verification tag
    fn decrypt_and_verify_password_tag(
        &self,
        derived_master_key_bytes: &[u8],
        password_verification_tag_clone: Option<String>, // Pass cloned tag
    ) -> Result<bool, KeystoreError> {
        let b64_tag = password_verification_tag_clone.ok_or(KeystoreError::InvalidFormat)?;

        let tag_bytes = BASE64_STANDARD.decode(&b64_tag).map_err(|_| KeystoreError::InvalidFormat)?;
        let key = Key::<Aes256Gcm>::from_slice(derived_master_key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&Self::PASSWORD_VERIFICATION_NONCE_BYTES);
        let decrypted_plaintext_result = cipher.decrypt(nonce, aes_gcm::aead::Payload { msg: &tag_bytes, aad: Self::PASSWORD_VERIFICATION_AAD });
        match decrypted_plaintext_result {
            Ok(plaintext_bytes) => Ok(plaintext_bytes == Self::PASSWORD_VERIFICATION_PLAINTEXT),
            Err(_) => Ok(false),
        }
    }

    // Create Additional Authenticated Data from entry metadata
    fn create_aad(&self, id: &Uuid, address: &Address) -> Vec<u8> {
        let mut aad = Vec::with_capacity(16 + 20); // UUID (16 bytes) + Address (20 bytes)
        aad.extend_from_slice(id.as_bytes());
        aad.extend_from_slice(address.as_slice());
        aad
    }

    // Derive per-entry encryption key using HKDF - made static-like
    // Helper for derive_entry_key_static to avoid &self if possible, or clarify its need
    fn derive_entry_key_static_helper(
        master_key: &[u8],
        id: &Uuid,
        kdf_params: &MasterKdfParams,
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let salt_bytes = hex::decode(&kdf_params.salt).map_err(|_| {
            KeystoreError::InvalidKeystoreFormat("Failed to decode KDF salt for entry key derivation (static_helper).".to_string())
        })?;
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt_bytes);
        let prk = salt.extract(master_key);
        let mut info_vec = Vec::with_capacity(Keystore::HKDF_ENTRY_KEY_INFO_PREFIX.len() + id.as_bytes().len());
        info_vec.extend_from_slice(Keystore::HKDF_ENTRY_KEY_INFO_PREFIX);
        info_vec.extend_from_slice(id.as_bytes());
        let mut okm = vec![0u8; 32];
        prk.expand(&[&info_vec], hkdf::HKDF_SHA256).map_err(|_| KeystoreError::DerivationFailed)?.fill(&mut okm).map_err(|_| KeystoreError::DerivationFailed)?;
        Ok(Zeroizing::new(okm))
    }

    fn derive_entry_key_static(
        &self, 
        master_key: &[u8],
        id: &Uuid,
        kdf_params: MasterKdfParams, 
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        Self::derive_entry_key_static_helper(master_key, id, &kdf_params)
    }


    // Derive AES-GCM nonce using HKDF - made static-like
    // Helper for derive_aes_gcm_nonce_static
    fn derive_aes_gcm_nonce_static_helper(
        master_key_bytes: &[u8],
        id: &Uuid,
        counter: u64,
    ) -> Result<[u8; 12], KeystoreError> {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, Keystore::HKDF_NONCE_SALT);
        let prk = salt.extract(master_key_bytes);
        let mut info_vec = Vec::with_capacity(Keystore::HKDF_NONCE_INFO_PREFIX.len() + id.as_bytes().len() + std::mem::size_of::<u64>());
        info_vec.extend_from_slice(Keystore::HKDF_NONCE_INFO_PREFIX);
        info_vec.extend_from_slice(id.as_bytes());
        info_vec.extend_from_slice(&counter.to_be_bytes());
        let mut okm = [0u8; 12];
        prk.expand(&[&info_vec], hkdf::HKDF_SHA256).map_err(|_| KeystoreError::DerivationFailed)?.fill(&mut okm).map_err(|_| KeystoreError::DerivationFailed)?;
        Ok(okm)
    }
    
    fn derive_aes_gcm_nonce_static(
        &self, 
        master_key_bytes: &[u8],
        id: &Uuid,
        counter: u64,
    ) -> Result<[u8; 12], KeystoreError> {
        Self::derive_aes_gcm_nonce_static_helper(master_key_bytes, id, counter)
    }

    // Static helper for create_aad
    fn create_aad_static(id: &Uuid, address: &Address) -> Vec<u8> {
        let mut aad = Vec::with_capacity(16 + 20); // UUID (16 bytes) + Address (20 bytes)
        aad.extend_from_slice(id.as_bytes());
        aad.extend_from_slice(address.as_slice());
        aad
                if last_activity.elapsed() > self.config.auto_lock_timeout { // self.config is fine
                    // self.lock() needs to be called without holding the read lock
                    // This is handled by the new structure of check_auto_lock
                }
            }
        }
    }

    // Update last activity timestamp
    fn update_activity_timestamp(&mut self) {
        self.last_activity_at = Some(Instant::now());
    }

    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        phrase: &str,
        passphrase: Option<&str>,
        path_str: &str, // e.g., "m/44'/60'/0'/0/0"
    ) -> Result<(Uuid, Address), KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }
        if phrase.is_empty() {
            // Or a more specific error like InvalidMnemonic
            return Err(KeystoreError::InvalidPrivateKey);
        }

        let mnemonic = Mnemonic::parse(phrase)?;
        // Get seed bytes directly from mnemonic using to_seed()
        let seed_bytes_array = mnemonic.to_seed(passphrase.unwrap_or("")); // Returns [u8; 64]

        // Create a zeroizing copy of the seed bytes
        let seed_bytes_zeroizing = Zeroizing::new(seed_bytes_array.to_vec());

        let derivation_path = DerivationPath::from_str(path_str).map_err(|e| {
            KeystoreError::InvalidPath(format!("Failed to parse derivation path: {}", e))
        })?;

        // Derive the private key using BIP32
        // XPrv::derive_from_path takes AsRef<[u8]> for seed
        let xprv = XPrv::derive_from_path(seed_bytes_zeroizing.as_slice(), &derivation_path)
            .map_err(KeystoreError::Bip32)?;

        // xprv.private_key() returns a SigningKey from bip32's re-exported k256.
        // We need to get its bytes and reconstruct with our project's k256::SecretKey.
        let bip32_signing_key = xprv.private_key();
        let secret_bytes_from_bip32 = bip32_signing_key.to_bytes(); // This is GenericArray from bip32's k256

        // Create a zeroizing buffer for the private key bytes
        let secret_bytes_zeroizing = Zeroizing::new(secret_bytes_from_bip32.to_vec());

        let secret_key = SecretKey::from_slice(secret_bytes_zeroizing.as_slice())
            .map_err(|_| KeystoreError::DerivationFailed)?; // Convert to our k256::SecretKey

        let pk_bytes_for_encryption = Zeroizing::new(secret_key.to_bytes().to_vec()); // Get bytes from our k256::SecretKey

        // From here, similar to import_private_key_hex
        let signing_key = SigningKey::from(&secret_key); // Use our project's k256::SecretKey
        let public_key = signing_key.verifying_key();

        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hashed_pk = [0u8; 32];
        keccak.finalize(&mut hashed_pk);
        let address_bytes: [u8; 20] = hashed_pk[12..]
            .try_into()
            .map_err(|_| KeystoreError::DerivationFailed)?;
        let address = Address::from(address_bytes);

        if let Some(ref new_alias) = alias {
            if self
                .entries
                .iter()
                .any(|e| e.alias.as_ref() == Some(new_alias))
            {
                return Err(KeystoreError::AliasExists(new_alias.clone()));
            }
        }

        let id = Uuid::new_v4();
        let mut nonce_bytes = [0u8; 12];
        // pk_bytes_for_encryption is GenericArray, convert to slice for encrypt_pk
        let encryption_counter = 1; // Initial counter for new key
        let encrypted_pk_vec = self.encrypt_pk(
            pk_bytes_for_encryption.as_slice(),
            &id,
            &address,
            encryption_counter,
        )?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            encryption_counter,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.entries.push(entry);
        self.save_to_disk()?;
        self.update_activity_timestamp();

        Ok((id, address))
    }

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }
        if pk_hex.is_empty() {
            return Err(KeystoreError::InvalidPrivateKey);
        }

        // Use Zeroizing for the decoded private key bytes
        let pk_bytes = Zeroizing::new(hex::decode(pk_hex)?);
        if pk_bytes.len() != 32 {
            // Secp256k1 private keys are 32 bytes
            return Err(KeystoreError::InvalidPrivateKey);
        }

        // Validate and derive public key + address
        let secret_key = SecretKey::from_slice(pk_bytes.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?; // k256 error is not std::Error
        let signing_key = SigningKey::from(secret_key); // This is infallible if SecretKey is valid
        let public_key = signing_key.verifying_key();

        // Derive EVM address from public key
        let uncompressed_pk = public_key.to_encoded_point(false); // false for uncompressed
                                                                  // uncompressed_pk[0] is 0x04. We need to hash uncompressed_pk[1..]
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]); // Skip the 0x04 prefix
        let mut hashed_pk = [0u8; 32];
        keccak.finalize(&mut hashed_pk);
        let address_bytes: [u8; 20] = hashed_pk[12..].try_into().map_err(|_| {
            KeystoreError::DerivationFailed // Should not happen if logic is correct
        })?;
        let address = Address::from(address_bytes);

        // Check for alias conflict
        if let Some(ref new_alias) = alias {
            if self
                .entries
                .iter()
                .any(|e| e.alias.as_ref() == Some(new_alias))
            {
                return Err(KeystoreError::AliasExists(new_alias.clone()));
            }
        }

        let id = Uuid::new_v4();
        let mut nonce_bytes = [0u8; 12];
        let encryption_counter = 1; // Initial counter for new key
        let encrypted_pk_vec =
            self.encrypt_pk(pk_bytes.as_slice(), &id, &address, encryption_counter)?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            encryption_counter,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.entries.push(entry);
        self.save_to_disk()?;
        self.update_activity_timestamp();

        Ok((id, address))
    }

    pub fn list_keys(&mut self) -> Result<Vec<KeyInfo>, KeystoreError> {
        // Listing keys does not require the keystore to be unlocked as it only exposes metadata.
        // However, we still update the activity timestamp
        self.update_activity_timestamp();

        let key_infos = self
            .entries
            .iter()
            .map(|entry| KeyInfo {
                id: entry.id,
                alias: entry.alias.clone(),
                address: entry.address,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
            })
            .collect();
        Ok(key_infos)
    }

    // Note: The plan mentions PrivateKeySigner, which comes from alloy-signer-local.
    // We'll need to ensure this is correctly typed and handled.
    // For now, returning a SigningKey directly for compatibility with tests.
    pub fn get_signer(&mut self, uuid: Uuid) -> Result<EphemeralSigner, KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }

        let entry = self
            .entries
            .iter()
            .find(|e| e.id == uuid)
            .ok_or(KeystoreError::KeyNotFound(uuid))?;

        let encrypted_pk_bytes = BASE64_STANDARD
            .decode(&entry.encrypted_pk)
            .map_err(|_e| KeystoreError::InvalidFormat)?; // Underscore e

        let decrypted_pk_zeroizing_vec = self.decrypt_pk(
            &encrypted_pk_bytes,
            &entry.id,
            &entry.address,
            entry.encryption_counter,
        )?;

        let secret_key = SecretKey::from_slice(decrypted_pk_zeroizing_vec.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?; // Should be valid if encryption/decryption worked

        let signing_key = SigningKey::from(secret_key);
        let z_signing_key = ZeroizingSigningKey::from(signing_key);

        self.update_activity_timestamp();
        Ok(EphemeralSigner { key: z_signing_key })
    }

    // Verify a signature against a message hash using the key identified by uuid
    pub fn verify_signature(
        &mut self,
        uuid: Uuid,
        message_hash: H256,
        signature: Signature,
    ) -> Result<bool, KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        // Get the signer for this key
        let signing_key = self.get_signer(uuid)?;

        // Get the verifying key from the signing key
        let verifying_key = signing_key.verifying_key();

        // Convert the alloy signature to k256 signature format
        let r_bytes = signature.r().to_be_bytes::<32>();
        let s_bytes = signature.s().to_be_bytes::<32>();

        // Combine r and s into a signature
        let mut signature_bytes = [0u8; 64];
        signature_bytes[..32].copy_from_slice(&r_bytes);
        signature_bytes[32..].copy_from_slice(&s_bytes);

        // Create a recoverable signature
        let k256_signature = k256::ecdsa::Signature::from_slice(&signature_bytes)
            .map_err(|_| KeystoreError::SignatureVerificationFailed)?;

        // Verify the signature
        let message_bytes = message_hash.as_slice();
        let result = verifying_key
            .verify_prehash(message_bytes, &k256_signature)
            .is_ok();

        self.update_activity_timestamp();
        Ok(result)
    }

    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.id != uuid);

        if self.entries.len() == initial_len {
            return Err(KeystoreError::KeyNotFound(uuid));
        }

        self.save_to_disk()?;
        self.update_activity_timestamp();
        Ok(())
    }

    pub fn change_password(
        &mut self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        // Verify the old password is correct by attempting to unlock
        if !self.is_unlocked {
            self.unlock(old_password)?;
        }

        if new_password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        // Generate new KDF parameters
        let new_kdf_params = Self::generate_kdf_params_with_config(&mut OsRng, &self.config)?;

        // Derive new master key
        let new_master_key = self.derive_master_key(new_password, &new_kdf_params)?;
        let new_master_key = Zeroizing::new(new_master_key);

        // Generate the new password verification tag with the new master key
        let verification_tag =
            self.generate_and_encrypt_verification_tag(new_master_key.as_slice())?;
        self.password_verification_tag = Some(verification_tag);

        // Store the current entries
        let old_entries = self.entries.clone();

        // Clear entries for re-encryption
        self.entries.clear();

        // Temporarily store the old master key
        let old_master_key = self.master_key.take();

        // Re-encrypt all entries with the new master key
        for old_entry in old_entries {
            // Decrypt with old key
            self.master_key = old_master_key.clone();
            let encrypted_pk_bytes = BASE64_STANDARD
                .decode(&old_entry.encrypted_pk)
                .map_err(|_| KeystoreError::InvalidFormat)?;

            let nonce_vec = BASE64_STANDARD
                .decode(&old_entry.nonce)
                .map_err(|_| KeystoreError::InvalidFormat)?;

            let nonce_bytes: [u8; 12] = nonce_vec
                .try_into()
                .map_err(|_| KeystoreError::InvalidFormat)?;

            let decrypted_pk = self.decrypt_pk(
                &encrypted_pk_bytes,
                &old_entry.id,
                &old_entry.address,
                old_entry.encryption_counter,
            )?;

            // Encrypt with new key
            self.master_key = Some(new_master_key.clone());
            let new_encryption_counter = old_entry.encryption_counter.checked_add(1).ok_or_else(|| {
                KeystoreError::FsError("Encryption counter overflow".to_string())
            })?;

            let new_encrypted_pk = self.encrypt_pk(
                &decrypted_pk,
                &old_entry.id,
                &old_entry.address,
                new_encryption_counter,
            )?;

            // Create new entry with re-encrypted key
            let new_entry = EncryptedKeyEntry {
                id: old_entry.id,
                alias: old_entry.alias,
                address: old_entry.address,
                encrypted_pk: BASE64_STANDARD.encode(&new_encrypted_pk),
                encryption_counter: new_encryption_counter,
                created_at: old_entry.created_at,
                updated_at: Utc::now(),
            };

            self.entries.push(new_entry);
        }

        // Set the master key to the new key
        self.master_key = Some(new_master_key);

        // Update KDF parameters
        self.master_kdf_params = Some(new_kdf_params);

        // Save the updated keystore
        self.save_to_disk()?;
        self.update_activity_timestamp();

        Ok(())
    }

    fn save_to_disk(&mut self) -> Result<(), KeystoreError> {
        // Create parent directories if they don't exist
        if let Some(parent) = self.file_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(KeystoreError::Io)?;
            }
        }

        // Derive lock file path
        let lock_file_path = self.file_path.with_extension("json.lock");

        // Open/create the lock file
        let lock_file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&lock_file_path)
            .map_err(KeystoreError::Io)?;

        // Acquire exclusive lock on the lock file
        lock_file
            .lock_exclusive()
            .map_err(|e| KeystoreError::FsError(format!("Failed to acquire exclusive lock on lock file: {}", e)))?;

        let write_result = (|| {
            let kdf_params = self
                .master_kdf_params
            .as_ref()
            .ok_or(KeystoreError::FsError(
                "Keystore is not initialized with KDF parameters.".to_string(),
            ))?;

        let keystore_data = KeystoreFile {
            version: "1.0.0".to_string(),
            password_verification_tag: self
                .password_verification_tag
                .clone()
                .ok_or_else(|| {
                    KeystoreError::FsError(
                        "Password verification tag missing during save".to_string(),
                    )
                })?,
            master_kdf: "argon2id".to_string(),
            master_kdf_params: kdf_params.clone(),
            entries: self.entries.clone(),
        };

        let json_data = serde_json::to_string_pretty(&keystore_data)?;

        // Use AtomicFile for atomic writes
        let atomic_file = AtomicFile::new(&self.file_path, OverwriteBehavior::AllowOverwrite);
        atomic_file
            .write(|f| f.write_all(json_data.as_bytes()).map_err(KeystoreError::Io))
            .map_err(|e| KeystoreError::FsError(format!("Failed to write keystore file: {}", e)))?;

        // Set file permissions to 0o600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&self.file_path)
                .map_err(KeystoreError::Io)?
                .permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&self.file_path, perms).map_err(KeystoreError::Io)?;
        }

            Ok(())
        })();

        // Ensure lock is released even if write_result is an error
        let unlock_result = lock_file
            .unlock()
            .map_err(|e| KeystoreError::FsError(format!("Failed to unlock lock file: {}", e)));
        
        // Attempt to remove the lock file
        fs::remove_file(&lock_file_path).ok(); 

        // Propagate any error from the write operation first, then from unlock
        write_result?;
        unlock_result?;

        Ok(())
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        if !self.file_path.exists() {
            // If the file doesn't exist, it might be a fresh keystore.
            // The initialize_or_load function will handle creating a new one if needed.
            // For load_from_disk, we assume it should exist if called.
            // Or, we can return Ok and let initialize_or_load decide.
            // For now, let's indicate it's not an error state for load_from_disk itself,
            // but that no data was loaded. The caller can then decide.
            // A better approach might be for initialize_or_load to check existence first.
            // Let's make it return Ok if not found, and initialize_or_load will create new.
            return Ok(());
        }

        // Derive lock file path
        let lock_file_path = self.file_path.with_extension("json.lock");

        // Open/create the lock file (needed for create(true))
        let lock_file = fs::OpenOptions::new()
            .read(true)
            .write(true) // Required for create true
            .create(true)
            .open(&lock_file_path)
            .map_err(KeystoreError::Io)?;

        // Acquire shared lock on the lock file
        lock_file
            .lock_shared()
            .map_err(|e| KeystoreError::FsError(format!("Failed to acquire shared lock on lock file: {}", e)))?;

        let read_result = (|| {
            let file_content = fs::read_to_string(&self.file_path)?;
        let keystore_data: KeystoreFile = serde_json::from_str(&file_content)?;

        // Validate version and KDF
        if keystore_data.version != "1.0.0" {
            // Check if we support migration from this version
            if keystore_data.version == "1.1.0" {
                // Implement migration logic here
                // For now, just return an error
                return Err(KeystoreError::FsError(format!(
                    "Keystore version {} requires migration. Please update your software.",
                    keystore_data.version
                )));
            } else {
                return Err(KeystoreError::InvalidFormat);
            }
        }

        if keystore_data.master_kdf != "argon2id" {
            // Clone master_kdf because KeystoreFile implements Drop
            return Err(KeystoreError::UnsupportedKdf(
                keystore_data.master_kdf.clone(),
            ));
        }

        // Clone fields because KeystoreFile implements Drop
        self.entries = keystore_data.entries.clone();
        if keystore_data.master_kdf_params.output_len < 32 {
            return Err(KeystoreError::InvalidKeystoreFormat(
                "Keystore KDF output length is less than the required 32 bytes.".to_string(),
            ));
        }
        self.master_kdf_params = Some(keystore_data.master_kdf_params.clone());
        self.password_verification_tag = Some(keystore_data.password_verification_tag.clone());
        // Keystore remains locked after loading. Unlock is a separate step.
        self.is_unlocked = false;
        self.master_key = None;

            Ok(())
        })();

        // Ensure lock is released even if read_result is an error
        let unlock_result = lock_file
            .unlock()
            .map_err(|e| KeystoreError::FsError(format!("Failed to unlock lock file: {}", e)));

        read_result?;
        unlock_result?;

        Ok(())
    }

    // fn encrypt_pk(&self, pk: &[u8], nonce: &[u8]) -> Result<Vec<u8>, KeystoreError> {
    fn encrypt_pk(
        &self,
        pk_bytes: &[u8],
        id: &Uuid,
        address: &Address,
        encryption_counter: u64,
    ) -> Result<Vec<u8>, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_slice();

        // Derive per-entry encryption key using HKDF
        let entry_key = self.derive_entry_key(master_key_bytes, id)?;
        let nonce_array = self.derive_aes_gcm_nonce(master_key_bytes, id, encryption_counter)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_array);

        // Create AAD from entry metadata
        let aad = self.create_aad(id, address);

        cipher
            .encrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: pk_bytes,
                    aad: &aad,
                },
            )
            .map_err(|e| KeystoreError::AesGcm(format!("Encryption failed: {}", e)))
    }

    fn decrypt_pk(
        &self,
        encrypted_pk_bytes: &[u8],
        id: &Uuid,
        address: &Address,
        encryption_counter: u64,
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_slice();

        // Derive per-entry encryption key using HKDF
        let entry_key = self.derive_entry_key(master_key_bytes, id)?;
        let nonce_array = self.derive_aes_gcm_nonce(master_key_bytes, id, encryption_counter)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_array);

        // Create AAD from entry metadata
        let aad = self.create_aad(id, address);

        let decrypted_bytes = cipher
            .decrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: encrypted_pk_bytes,
                    aad: &aad,
                },
            )
            .map_err(|_| KeystoreError::EntryDecryptionFailed)?;

        Ok(Zeroizing::new(decrypted_bytes))
    }

    // Decrypt and verify the password verification tag
    fn decrypt_and_verify_password_tag(
        &self,
        derived_master_key_bytes: &[u8],
    ) -> Result<bool, KeystoreError> {
        let b64_tag = self
            .password_verification_tag
            .as_ref()
            .ok_or(KeystoreError::InvalidFormat)?; // Or specific error

        let tag_bytes = BASE64_STANDARD
            .decode(b64_tag)
            .map_err(|_| KeystoreError::InvalidFormat)?;

        let key = Key::<Aes256Gcm>::from_slice(derived_master_key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&Self::PASSWORD_VERIFICATION_NONCE_BYTES);

        let decrypted_plaintext_result = cipher.decrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: &tag_bytes,
                aad: Self::PASSWORD_VERIFICATION_AAD,
            },
        );

        match decrypted_plaintext_result {
            Ok(plaintext_bytes) => {
                // Ensure lengths match before comparison to prevent subtle issues, though ct_eq handles different lengths.
                // However, for a fixed verification tag, lengths should always be identical if decrypted correctly.
                if plaintext_bytes.len() == Self::PASSWORD_VERIFICATION_PLAINTEXT.len() {
                    Ok(plaintext_bytes.ct_eq(Self::PASSWORD_VERIFICATION_PLAINTEXT).into())
                } else {
                    Ok(false) // Length mismatch implies it's not the tag
                }
            }
            Err(_) => Ok(false), // Decryption failed, so tag verification failed
        }
    }

    // Generate and encrypt a verification tag for the master key
    // Made static as it doesn't depend on KeystoreInner state if master_key_bytes is passed.
    fn generate_and_encrypt_verification_tag_static(
        master_key_bytes: &[u8],
    ) -> Result<String, KeystoreError> {
        let key = Key::<Aes256Gcm>::from_slice(master_key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&Self::PASSWORD_VERIFICATION_NONCE_BYTES);

        let ciphertext = cipher
            .encrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: Self::PASSWORD_VERIFICATION_PLAINTEXT,
                    aad: Self::PASSWORD_VERIFICATION_AAD,
                },
            )
            .map_err(|e| KeystoreError::AesGcm(format!("Encryption failed: {}", e)))?;

        Ok(BASE64_STANDARD.encode(&ciphertext))
    }

    // Create Additional Authenticated Data from entry metadata
    fn create_aad(&self, id: &Uuid, address: &Address) -> Vec<u8> {
        let mut aad = Vec::with_capacity(16 + 20); // UUID (16 bytes) + Address (20 bytes)
        aad.extend_from_slice(id.as_bytes());
        aad.extend_from_slice(address.as_slice());
        aad
    }

    // Derive per-entry encryption key using HKDF
    fn derive_entry_key(
        &self,
        master_key: &[u8],
        id: &Uuid,
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let kdf_params = self
            .master_kdf_params
            .as_ref()
            .ok_or(KeystoreError::MissingMasterKdfParams)?;
        let salt_bytes = hex::decode(&kdf_params.salt).map_err(|_| {
            KeystoreError::InvalidKeystoreFormat(
                "Failed to decode KDF salt for entry key derivation.".to_string(),
            )
        })?;
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt_bytes);
        let prk = salt.extract(master_key);

        let mut info_vec =
            Vec::with_capacity(Self::HKDF_ENTRY_KEY_INFO_PREFIX.len() + id.as_bytes().len());
        info_vec.extend_from_slice(Self::HKDF_ENTRY_KEY_INFO_PREFIX);
        info_vec.extend_from_slice(id.as_bytes());

        let mut okm = vec![0u8; 32]; // 32 bytes for AES-256
        prk.expand(&[&info_vec], hkdf::HKDF_SHA256)
            .map_err(|_| KeystoreError::DerivationFailed)?
            .fill(&mut okm)
            .map_err(|_| KeystoreError::DerivationFailed)?;

        Ok(Zeroizing::new(okm))
    }

    // Derive AES-GCM nonce using HKDF
    fn derive_aes_gcm_nonce(
        &self,
        master_key_bytes: &[u8],
        id: &Uuid,
        counter: u64,
    ) -> Result<[u8; 12], KeystoreError> {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, Self::HKDF_NONCE_SALT);
        let prk = salt.extract(master_key_bytes);

        let mut info_vec = Vec::with_capacity(
            Self::HKDF_NONCE_INFO_PREFIX.len() + id.as_bytes().len() + std::mem::size_of::<u64>(),
        );
        info_vec.extend_from_slice(Self::HKDF_NONCE_INFO_PREFIX);
        info_vec.extend_from_slice(id.as_bytes());
        info_vec.extend_from_slice(&counter.to_be_bytes());

        let mut okm = [0u8; 12]; // 12 bytes for AES-GCM nonce
        prk.expand(&[&info_vec], hkdf::HKDF_SHA256)
            .map_err(|_| KeystoreError::DerivationFailed)?
            .fill(&mut okm)
            .map_err(|_| KeystoreError::DerivationFailed)?;

        Ok(okm)
    }

    // fn derive_master_key(&self, password: &str, params: &MasterKdfParams) -> Result<Vec<u8>, KeystoreError> {
    // Now takes config as an argument
    fn derive_master_key(
        &self, // Still takes &self if it needs other non-inner fields, or for consistency
        password: &str,
        kdf_params: &MasterKdfParams,
        config: &KeystoreConfig, // Pass config explicitly
    ) -> Result<Vec<u8>, KeystoreError> {
        let salt = hex::decode(&kdf_params.salt)
            .map_err(|e| KeystoreError::Argon2Error(format!("Failed to decode salt: {}", e)))?;

        let params = argon2::Params::new(
            kdf_params.m_cost,
            kdf_params.t_cost,
            kdf_params.p_cost,
            Some(kdf_params.output_len),
        )
        .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?; // Ensure argon2::Error is converted

        let argon2_context = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13, // Argon2 version 1.3
            params,
        );

        let mut output_key_material = vec![0u8; kdf_params.output_len];
        argon2_context
            .hash_password_into(
                password.as_bytes(),
                &salt,
                &mut output_key_material,
            )
            .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?;

        Ok(output_key_material)
    }

    fn generate_kdf_params(
        rng: &mut (impl CryptoRng + RngCore),
    ) -> Result<MasterKdfParams, KeystoreError> {
        Self::generate_kdf_params_with_config(rng, &KeystoreConfig::default())
    }

    fn generate_kdf_params_with_config(
        rng: &mut (impl CryptoRng + RngCore),
        config: &KeystoreConfig,
    ) -> Result<MasterKdfParams, KeystoreError> {
        if config.output_len < 32 {
            return Err(KeystoreError::ConfigError(
                "KDF output length in config must be at least 32 bytes.".to_string(),
            ));
        }
        let mut salt_bytes = [0u8; 16]; // 16-byte salt
        rng.try_fill_bytes(&mut salt_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate salt: {}", e)))?;

        Ok(MasterKdfParams {
            salt: hex::encode(salt_bytes),
            m_cost: config.m_cost,
            t_cost: config.t_cost,
            p_cost: config.p_cost,
            output_len: config.output_len,
        })
    }
}

// Ensure the module is declared in mfm_core/src/lib.rs or mfm_core/src/keystore/mod.rs
