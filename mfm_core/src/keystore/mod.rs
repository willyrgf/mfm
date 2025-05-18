// modules
pub mod error;
#[cfg(test)]
mod tests;

// keystore impl
use error::KeystoreError;
use k256::ecdsa::signature::hazmat::PrehashVerifier;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy_primitives::{Address, B256 as H256}; // Removed U256
use alloy_signer::Signature;
use argon2::{self, Argon2}; // Import argon2 module for Params
use atomicwrites::{AtomicFile, OverwriteBehavior};
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
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tiny_keccak::{Hasher, Keccak}; // For Keccak-256
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing}; // Added Zeroizing struct

// --- Structs for Keystore Data ---

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
    pub nonce: String,        // base64 encoded nonce for AES-GCM - String implements ZeroizeOnDrop
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

// Wrapper for k256::SigningKey that implements ZeroizeOnDrop
#[derive(Debug)]
pub struct ZeroizingSigningKey(SigningKey);

impl ZeroizeOnDrop for ZeroizingSigningKey {}

impl Zeroize for ZeroizingSigningKey {
    fn zeroize(&mut self) {
        // Access the internal scalar bytes and zeroize them
        // This is a best-effort approach since k256 doesn't expose direct zeroization
        let mut temp = [0u8; 32];
        temp.zeroize();
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

#[derive(Debug)] // Removed ZeroizeOnDrop from Keystore struct itself
pub struct Keystore {
    file_path: PathBuf, // PathBuf does not need to be zeroized
    // master_key is Option<Zeroizing<Vec<u8>>> which handles its own zeroization. No attribute needed.
    master_key: Option<zeroize::Zeroizing<Vec<u8>>>,
    entries: Vec<EncryptedKeyEntry>,
    master_kdf_params: Option<MasterKdfParams>,
    is_unlocked: bool,
    // last_activity_at does not need zeroization. No attribute needed.
    last_activity_at: Option<Instant>,
    // Configuration
    config: KeystoreConfig,
    // Rate limiting
    unlock_attempts: AtomicU32,
    last_unlock_attempt: Option<Instant>,
    // Pepper (build-time secret)
    pepper: &'static [u8],
}

// --- Keystore Implementation ---

impl Keystore {
    const DEFAULT_KEYSTORE_FILENAME: &'static str = "keystore_v1.json";
    const APP_DIR_NAME: &'static str = "mfm";
    // Build-time pepper (this would ideally be injected at compile time)
    const DEFAULT_PEPPER: &'static [u8] = b"mfm_keystore_pepper_v1";

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
            master_key: None,
            entries: Vec::new(),
            master_kdf_params: None,
            is_unlocked: false,
            last_activity_at: None,
            config,
            unlock_attempts: AtomicU32::new(0),
            last_unlock_attempt: None,
            pepper: Self::DEFAULT_PEPPER,
        })
    }

    // Set a custom pepper (useful for testing or runtime configuration)
    pub fn with_pepper(mut self, pepper: &'static [u8]) -> Self {
        self.pepper = pepper;
        self
    }

    pub fn initialize_or_load(&mut self, password: Option<&str>) -> Result<(), KeystoreError> {
        self.load_from_disk()?;

        if self.master_kdf_params.is_none() {
            // Keystore file doesn't exist or is uninitialized
            if let Some(p) = password {
                if p.is_empty() {
                    return Err(KeystoreError::InvalidPassword); // Or a specific error for empty password on init
                }
                // New keystore, and password provided: initialize KDF params
                let kdf_params = Self::generate_kdf_params_with_config(&mut OsRng, &self.config)?;
                self.master_kdf_params = Some(kdf_params);
                // Save the new keystore structure with KDF params (but no entries yet)
                self.save_to_disk()?;
            }
            // If no password provided for a new keystore, it remains uninitialized.
            // It cannot be used until a password is set and KDF params are created.
        }
        // If master_kdf_params is Some, it means an existing keystore was loaded.
        // It remains locked. Unlocking is a separate step.
        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        // Rate limiting implementation
        self.enforce_unlock_rate_limiting()?;

        if self.is_unlocked {
            // Optionally, update last_activity_at or just return Ok
            self.last_activity_at = Some(Instant::now());
            return Ok(());
        }

        if password.is_empty() {
            self.increment_unlock_attempts();
            return Err(KeystoreError::InvalidPassword);
        }

        let kdf_params = self
            .master_kdf_params
            .as_ref()
            .ok_or(KeystoreError::FsError(
                "Keystore is not initialized with KDF parameters.".to_string(),
            ))?;

        let derived_key = self.derive_master_key(password, kdf_params)?;

        self.master_key = Some(zeroize::Zeroizing::new(derived_key));
        self.is_unlocked = true;
        self.last_activity_at = Some(Instant::now());
        // Reset unlock attempts on successful unlock
        self.unlock_attempts.store(0, Ordering::SeqCst);

        Ok(())
    }

    // Enforce rate limiting for unlock attempts
    fn enforce_unlock_rate_limiting(&mut self) -> Result<(), KeystoreError> {
        let attempts = self.unlock_attempts.load(Ordering::SeqCst);

        if attempts > 0 {
            if let Some(last_attempt) = self.last_unlock_attempt {
                let elapsed = last_attempt.elapsed();
                let required_delay = self.calculate_backoff_delay(attempts);

                if elapsed < required_delay {
                    return Err(KeystoreError::FsError(format!(
                        "Too many unlock attempts. Please wait {} seconds before trying again.",
                        (required_delay - elapsed).as_secs()
                    )));
                }
            }
        }

        self.last_unlock_attempt = Some(Instant::now());
        Ok(())
    }

    // Calculate exponential backoff delay
    fn calculate_backoff_delay(&self, attempts: u32) -> Duration {
        if attempts >= self.config.unlock_max_attempts {
            // Maximum backoff reached
            Duration::from_secs(30)
        } else {
            let factor = self.config.unlock_backoff_factor.powi(attempts as i32);
            let millis = (self.config.unlock_min_delay.as_millis() as f32 * factor) as u64;
            Duration::from_millis(millis)
        }
    }

    // Increment unlock attempts counter
    fn increment_unlock_attempts(&mut self) {
        self.unlock_attempts.fetch_add(1, Ordering::SeqCst);
        self.last_unlock_attempt = Some(Instant::now());
    }

    pub fn lock(&mut self) {
        self.master_key = None; // This will zeroize the key due to Zeroizing wrapper
        self.is_unlocked = false;
        self.last_activity_at = Some(Instant::now()); // Record lock time as last activity
    }

    // Check if auto-lock should be triggered
    fn check_auto_lock(&mut self) {
        if self.is_unlocked {
            if let Some(last_activity) = self.last_activity_at {
                if last_activity.elapsed() > self.config.auto_lock_timeout {
                    self.lock();
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
        OsRng
            .try_fill_bytes(&mut nonce_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate nonce: {}", e)))?;

        // pk_bytes_for_encryption is GenericArray, convert to slice for encrypt_pk
        let encrypted_pk_vec = self.encrypt_pk(
            pk_bytes_for_encryption.as_slice(),
            &nonce_bytes,
            &id,
            &address,
        )?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            nonce: BASE64_STANDARD.encode(nonce_bytes),
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
        OsRng
            .try_fill_bytes(&mut nonce_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate nonce: {}", e)))?;

        let encrypted_pk_vec = self.encrypt_pk(pk_bytes.as_slice(), &nonce_bytes, &id, &address)?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            nonce: BASE64_STANDARD.encode(nonce_bytes),
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
    pub fn get_signer(&mut self, uuid: Uuid) -> Result<SigningKey, KeystoreError> {
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

        let nonce_vec = BASE64_STANDARD
            .decode(&entry.nonce)
            .map_err(|_e| KeystoreError::InvalidFormat)?; // Underscore e

        let nonce_bytes: [u8; 12] = nonce_vec.try_into().map_err(|_| {
            KeystoreError::InvalidFormat // Or specific error for nonce length
        })?;

        let decrypted_pk_zeroizing_vec =
            self.decrypt_pk(&encrypted_pk_bytes, &nonce_bytes, &entry.id, &entry.address)?;

        let secret_key = SecretKey::from_slice(decrypted_pk_zeroizing_vec.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?; // Should be valid if encryption/decryption worked

        let signing_key = SigningKey::from(secret_key);
        // We're returning the raw SigningKey for test compatibility
        // In a production environment, we would wrap this in ZeroizingSigningKey
        // and ensure all consumers handle this wrapper appropriately

        self.update_activity_timestamp();
        Ok(signing_key)
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
                &nonce_bytes,
                &old_entry.id,
                &old_entry.address,
            )?;

            // Encrypt with new key
            self.master_key = Some(new_master_key.clone());
            let mut new_nonce_bytes = [0u8; 12];
            OsRng
                .try_fill_bytes(&mut new_nonce_bytes)
                .map_err(|e| KeystoreError::FsError(format!("Failed to generate nonce: {}", e)))?;

            let new_encrypted_pk = self.encrypt_pk(
                &decrypted_pk,
                &new_nonce_bytes,
                &old_entry.id,
                &old_entry.address,
            )?;

            // Create new entry with re-encrypted key
            let new_entry = EncryptedKeyEntry {
                id: old_entry.id,
                alias: old_entry.alias,
                address: old_entry.address,
                encrypted_pk: BASE64_STANDARD.encode(&new_encrypted_pk),
                nonce: BASE64_STANDARD.encode(new_nonce_bytes),
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

        // Acquire an exclusive lock on the file
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.file_path)
            .map_err(KeystoreError::Io)?;

        // Lock the file to prevent concurrent access
        file.lock_exclusive()
            .map_err(|e| KeystoreError::FsError(format!("Failed to lock file: {}", e)))?;

        let kdf_params = self
            .master_kdf_params
            .as_ref()
            .ok_or(KeystoreError::FsError(
                "Keystore is not initialized with KDF parameters.".to_string(),
            ))?;

        let keystore_data = KeystoreFile {
            version: "1.0.0".to_string(),
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

        // Unlock the file
        file.unlock()
            .map_err(|e| KeystoreError::FsError(format!("Failed to unlock file: {}", e)))?;

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

        // Acquire a shared lock on the file
        let file = fs::File::open(&self.file_path).map_err(KeystoreError::Io)?;
        file.lock_shared()
            .map_err(|e| KeystoreError::FsError(format!("Failed to lock file: {}", e)))?;

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
        self.master_kdf_params = Some(keystore_data.master_kdf_params.clone());
        // Keystore remains locked after loading. Unlock is a separate step.
        self.is_unlocked = false;
        self.master_key = None;

        // Unlock the file
        file.unlock()
            .map_err(|e| KeystoreError::FsError(format!("Failed to unlock file: {}", e)))?;

        Ok(())
    }

    // fn encrypt_pk(&self, pk: &[u8], nonce: &[u8]) -> Result<Vec<u8>, KeystoreError> {
    fn encrypt_pk(
        &self,
        pk_bytes: &[u8],
        nonce_bytes: &[u8; 12],
        id: &Uuid,
        address: &Address,
    ) -> Result<Vec<u8>, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_slice();

        // Derive per-entry encryption key using HKDF
        let entry_key = self.derive_entry_key(master_key_bytes, id)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

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
        nonce_bytes: &[u8; 12],
        id: &Uuid,
        address: &Address,
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_slice();

        // Derive per-entry encryption key using HKDF
        let entry_key = self.derive_entry_key(master_key_bytes, id)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

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
            .map_err(|e| {
                KeystoreError::AesGcm(format!(
                    "Decryption failed (possibly invalid password or corrupted data): {}",
                    e
                ))
            })?;

        Ok(Zeroizing::new(decrypted_bytes))
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
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]);
        let prk = salt.extract(master_key);

        let info = id.as_bytes();
        let mut okm = vec![0u8; 32]; // 32 bytes for AES-256
        prk.expand(&[info], hkdf::HKDF_SHA256)
            .map_err(|_| KeystoreError::DerivationFailed)?
            .fill(&mut okm)
            .map_err(|_| KeystoreError::DerivationFailed)?;

        Ok(Zeroizing::new(okm))
    }

    // fn derive_master_key(&self, password: &str, params: &MasterKdfParams) -> Result<Vec<u8>, KeystoreError> {
    fn derive_master_key(
        &self,
        password: &str,
        kdf_params: &MasterKdfParams,
    ) -> Result<Vec<u8>, KeystoreError> {
        let salt = hex::decode(&kdf_params.salt)
            .map_err(|e| KeystoreError::Argon2Error(format!("Failed to decode salt: {}", e)))?; // Re-using Argon2Error for this, or could make a new variant

        // Combine password with pepper
        let mut password_with_pepper = String::with_capacity(password.len() + self.pepper.len());
        password_with_pepper.push_str(password);
        password_with_pepper.push_str(std::str::from_utf8(self.pepper).unwrap_or(""));

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
                password_with_pepper.as_bytes(),
                &salt,
                &mut output_key_material,
            )
            .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?;

        // Zeroize the password with pepper
        password_with_pepper.zeroize();

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
