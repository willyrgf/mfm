/// Minimal secure keystore for Ethereum keys and mnemonics
///
/// This is a simplified, focused implementation that provides only essential
/// functionality for storing and retrieving Ethereum private keys and mnemonics.
///
/// # Security Assumptions
///
/// - Full disk encryption (including swap) is enabled
/// - Single-threaded usage (not Send + Sync)
/// - Local-only operation (no network features)
/// - Trusted application environment
///
/// # Design Principles
///
/// - Simplicity over feature completeness
/// - Security by default with minimal configuration
/// - Clear separation of concerns
/// - Minimal attack surface
pub mod error;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy_primitives::Address;
use argon2::{Argon2, Params};
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic;
use chrono::{DateTime, Utc};
use k256::{ecdsa::SigningKey, SecretKey};
use rand::rngs::OsRng;
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use subtle::ConstantTimeEq;
use tiny_keccak::{Hasher, Keccak};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub use error::KeystoreError;

const KEYSTORE_FILE_VERSION: u8 = 1;

/// Simplified configuration with secure defaults
#[derive(Debug, Clone)]
pub struct KeystoreConfig {
    /// Argon2 memory cost in KB (default: 1GB = 1048576)
    pub argon2_memory_kb: u32,
    /// Argon2 time cost in iterations (default: 8)
    pub argon2_iterations: u32,
    /// Argon2 parallelism (default: 1)
    pub argon2_parallelism: u32,
}

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            argon2_memory_kb: 1_048_576, // 1GB - production secure
            argon2_iterations: 8,        // 8 iterations - secure default
            argon2_parallelism: 1,       // Single threaded
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
        }
    }
}

/// Key type for different storage formats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyType {
    PrivateKey,
    Mnemonic { derivation_path: String },
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
            key_type: entry.key_type.clone(),
            created_at: entry.created_at,
        }
    }
}

/// On-disk keystore file format
#[derive(Serialize, Deserialize)]
struct KeystoreFile {
    version: u8,
    kdf_params: ArgonParams,
    master_key_verification: [u8; 32], // HMAC for password verification
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
            .map_err(|e| KeystoreError::CryptoError(format!("Signing failed: {}", e)));

        // Note: SecretKey and SigningKey implement ZeroizeOnDrop automatically
        // via the k256 crate, so they will be zeroized when dropped
        result
    }

    /// Get Ethereum address for this key
    pub fn ethereum_address(&self) -> Address {
        let secret_key = SecretKey::from_slice(self.key_bytes.as_ref()).unwrap();
        let public_key = secret_key.public_key();

        // Compute Ethereum address from public key
        use k256::elliptic_curve::sec1::ToEncodedPoint;
        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]); // Skip 0x04 prefix
        let mut hash = [0u8; 32];
        keccak.finalize(&mut hash);

        // Note: SecretKey implements ZeroizeOnDrop and will be zeroized when dropped
        Address::from_slice(&hash[12..])
    }

    /// Get public key
    pub fn public_key(&self) -> k256::PublicKey {
        let secret_key = SecretKey::from_slice(self.key_bytes.as_ref()).unwrap();
        // Note: SecretKey implements ZeroizeOnDrop and will be zeroized when dropped
        secret_key.public_key()
    }
}

impl ZeroizeOnDrop for SecureKey {}

/// Minimal secure keystore for Ethereum keys and mnemonics
#[derive(Debug)]
pub struct Keystore {
    path: PathBuf,
    config: KeystoreConfig,
    master_key: Option<Zeroizing<[u8; 32]>>,
    entries: Vec<KeyEntry>,
    kdf_params: Option<ArgonParams>,
    master_key_verification: Option<[u8; 32]>,
    file_integrity_mac: Option<[u8; 32]>,
    // Thread safety marker - prevents Send + Sync
    _not_thread_safe: *const (),
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
            entries: Vec::new(),
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
        self.early_file_validation()?;

        if self.path.exists() {
            // Unlock existing keystore
            self.unlock_existing(password)
        } else {
            // Initialize new keystore
            self.initialize_new(password)
        }
    }

    /// Lock keystore (clear master key from memory)
    pub fn lock(&mut self) {
        self.master_key = None;
    }

    /// Import private key (hex format)
    pub fn import_private_key(
        &mut self,
        alias: Option<String>,
        private_key_hex: &str,
    ) -> Result<Uuid, KeystoreError> {
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
        let address = secure_key.ethereum_address();

        // Create encrypted entry
        let id = Uuid::new_v4();
        let mut nonce = [0u8; 12];
        OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
            KeystoreError::CryptoError("Failed to generate random nonce".to_string())
        })?;

        let encrypted_data = self.encrypt_data(master_key, &nonce, &key_array, id.as_bytes())?;

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
        let mut key_array = key_array;
        key_array.zeroize();

        Ok(id)
    }

    /// Import mnemonic with derivation path
    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        mnemonic: &str,
        derivation_path: &str,
    ) -> Result<Uuid, KeystoreError> {
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

        // Validate mnemonic
        let mnemonic = Mnemonic::from_str(mnemonic)?;

        // Validate derivation path
        let derivation_path_obj = DerivationPath::from_str(derivation_path)?;

        // Derive private key from mnemonic - wrap seed in Zeroizing for automatic cleanup
        let seed_bytes = mnemonic.to_seed("");
        let mut seed = Zeroizing::new(seed_bytes);

        // Limit XPrv scope to ensure it's dropped quickly
        let mut private_key_bytes = {
            let derived_xprv = XPrv::derive_from_path(&*seed, &derivation_path_obj)?;
            derived_xprv.private_key().to_bytes()
        };

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&private_key_bytes);

        // Calculate Ethereum address
        let secure_key = SecureKey::new(key_array);
        let address = secure_key.ethereum_address();

        // Store the mnemonic phrase (not the derived key)
        let mut mnemonic_bytes = mnemonic.to_string().as_bytes().to_vec();

        // Create encrypted entry
        let id = Uuid::new_v4();
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

        // Zeroize sensitive intermediate data
        seed.zeroize();
        // Note: XPrv doesn't implement Zeroize directly, but the private_key_bytes extracted from it are zeroized
        private_key_bytes.zeroize();
        key_array.zeroize();
        mnemonic_bytes.zeroize();

        Ok(id)
    }

    /// Get private key for signing
    pub fn get_private_key(&mut self, id: Uuid) -> Result<SecureKey, KeystoreError> {
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

        let entry = self
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or(KeystoreError::KeyNotFound(id))?;

        let decrypted_data = self.decrypt_data(
            master_key,
            &entry.nonce,
            &entry.encrypted_data,
            entry.id.as_bytes(),
        )?;

        match &entry.key_type {
            KeyType::PrivateKey => {
                if decrypted_data.len() != 32 {
                    return Err(KeystoreError::InvalidPrivateKey);
                }
                let mut key_bytes = [0u8; 32];
                key_bytes.copy_from_slice(&decrypted_data);
                Ok(SecureKey::new(key_bytes))
            }
            KeyType::Mnemonic { derivation_path } => {
                // Decrypt mnemonic and derive key
                let mnemonic_str = core::str::from_utf8(decrypted_data.as_ref())
                    .map_err(|_| KeystoreError::InvalidMnemonic("Invalid UTF-8".to_string()))?;
                let mnemonic = Mnemonic::from_str(&mnemonic_str)?;
                let derivation_path_obj = DerivationPath::from_str(derivation_path)?;

                // Derive private key from mnemonic - wrap seed in Zeroizing for automatic cleanup
                let seed = Zeroizing::new(mnemonic.to_seed(""));

                // Limit XPrv scope to ensure it's dropped quickly
                let secure_key = {
                    let derived_xprv = XPrv::derive_from_path(&*seed, &derivation_path_obj)?;
                    SecureKey::new(derived_xprv.private_key().to_bytes().into())
                };

                Ok(secure_key)
            }
        }
    }

    /// List stored keys (metadata only)
    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        Ok(self.entries.iter().map(KeyInfo::from).collect())
    }

    /// Remove key from keystore
    pub fn delete_key(&mut self, id: Uuid) -> Result<(), KeystoreError> {
        self.master_key.as_ref().ok_or(KeystoreError::Locked)?;

        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.id != id);

        if self.entries.len() == initial_len {
            return Err(KeystoreError::KeyNotFound(id));
        }

        self.save_to_disk()?;
        Ok(())
    }

    // Private helper methods

    fn initialize_new(&mut self, password: &str) -> Result<(), KeystoreError> {
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
        self.kdf_params = Some(kdf_params);
        self.master_key_verification = Some(master_key_verification);

        // Save empty keystore to disk (this will compute and include file_integrity_mac)
        self.save_to_disk()?;

        // Reload from disk to ensure file_integrity_mac is properly loaded
        self.master_key = None; // Clear temporarily to reload
        self.load_from_disk()?;

        // Rederive master key to restore unlocked state
        let kdf_params = self.kdf_params.as_ref().unwrap();
        let master_key = self.derive_master_key(password, kdf_params)?;
        self.master_key = Some(master_key);

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
            // Verify file integrity after password verification
            self.verify_file_integrity(&master_key)?;

            self.master_key = Some(master_key);
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
        let tag = hmac::sign(&key, file_data);
        let mut mac = [0u8; 32];
        mac.copy_from_slice(tag.as_ref());
        Ok(mac)
    }

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

    fn save_to_disk(&self) -> Result<(), KeystoreError> {
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
            entries: self.entries.clone(),
            file_integrity_mac,
        };

        let json_data = serde_json::to_string_pretty(&keystore_file)?;
        std::fs::write(&self.path, json_data)?;

        Ok(())
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        let data = std::fs::read(&self.path)?;
        let keystore_file: KeystoreFile = serde_json::from_slice(&data)?;

        if keystore_file.version != KEYSTORE_FILE_VERSION {
            return Err(KeystoreError::InvalidInput(format!(
                "Unsupported keystore version: {}",
                keystore_file.version
            )));
        }

        self.kdf_params = Some(keystore_file.kdf_params);
        self.master_key_verification = Some(keystore_file.master_key_verification);
        self.file_integrity_mac = Some(keystore_file.file_integrity_mac);
        self.entries = keystore_file.entries;

        Ok(())
    }

    fn early_file_validation(&self) -> Result<(), KeystoreError> {
        // Early validation with dummy key to detect obviously malformed files
        // before expensive password derivation
        if !self.path.exists() {
            return Ok(()); // New keystore, nothing to validate
        }

        let data = std::fs::read(&self.path)
            .map_err(|_| KeystoreError::InvalidInput("Cannot read keystore file".to_string()))?;

        // Basic size check - keystore files should be reasonable size
        const MAX_KEYSTORE_SIZE: usize = 10 * 1024 * 1024; // 10MB
        const MIN_KEYSTORE_SIZE: usize = 100; // Minimum JSON structure

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

        // Check for reasonable entry count
        const MAX_ENTRIES: usize = 10000; // Reasonable limit
        if keystore_file.entries.len() > MAX_ENTRIES {
            return Err(KeystoreError::InvalidInput(
                "Too many entries - possible DoS attempt".to_string(),
            ));
        }

        // Per-entry ciphertext size (optional limit ≈ 1 MiB)
        const MAX_ENTRY_DATA: usize = 1 * 1024 * 1024;
        if keystore_file
            .entries
            .iter()
            .any(|e| e.encrypted_data.len() > MAX_ENTRY_DATA)
        {
            return Err(KeystoreError::InvalidInput(
                "Entry too large – likely corrupted".into(),
            ));
        }

        Ok(())
    }

    fn verify_file_integrity(&self, master_key: &[u8; 32]) -> Result<(), KeystoreError> {
        let stored_mac = self.file_integrity_mac.ok_or(KeystoreError::InvalidInput(
            "No file integrity MAC found".to_string(),
        ))?;

        // Read the file again for verification
        let data = std::fs::read(&self.path)?;

        // Parse to extract the data without the MAC for verification
        let keystore_file: KeystoreFile = serde_json::from_slice(&data)?;

        // Create the same structure used during save (with placeholder MAC)
        let keystore_file_without_mac = KeystoreFile {
            version: keystore_file.version,
            kdf_params: keystore_file.kdf_params,
            master_key_verification: keystore_file.master_key_verification,
            entries: keystore_file.entries,
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
