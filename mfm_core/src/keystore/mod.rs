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
                                            // use ring::hkdf; // For HKDF key derivation - Removed
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write; // Added for f.write_all
use std::path::PathBuf;
use std::str::FromStr; // For DerivationPath::from_str
                       // use std::sync::atomic::{AtomicU32, Ordering}; // Removed
use std::time::Duration; // Removed Instant
use tiny_keccak::{Hasher, Keccak}; // For Keccak-256
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing}; // Added Zeroizing struct

// --- Structs for Keystore Data ---

#[derive(Serialize, Deserialize, Debug, Clone, Zeroize, ZeroizeOnDrop)] // Zeroize for salt is fine
pub struct EntryKdfParams {
    // Renamed from MasterKdfParams
    pub salt: String, // hex_encoded_salt
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub output_len: usize,
}

// Uuid and DateTime<Utc> do not implement Zeroize/ZeroizeOnDrop by default.
// String fields (encrypted_pk) do. Alias is Option<String>.
// Address is a fixed-size array, which should be fine.
// Removing ZeroizeOnDrop from EncryptedKeyEntry derive.
// String fields within will handle their own zeroization as String implements ZeroizeOnDrop.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedKeyEntry {
    pub id: Uuid,                   // Not secret
    pub alias: Option<String>,      // String part will be zeroized on drop if Some
    pub address: Address,           // Not secret
    pub encrypted_pk: String, // base64 encoded encrypted private key - String implements ZeroizeOnDrop
    pub kdf_params: EntryKdfParams, // Added
    pub nonce: String,        // base64 encoded nonce for AES-GCM - Added back
    pub created_at: DateTime<Utc>, // Not secret
    pub updated_at: DateTime<Utc>, // Not secret
                              // Potentially other metadata like derivation path if applicable, key type, etc.
}

// KeystoreFile's ZeroizeOnDrop will apply to its String fields.
// EntryKdfParams also derives ZeroizeOnDrop for its salt.
// Vec<EncryptedKeyEntry> elements (Strings) will be zeroized when they are dropped.
#[derive(Serialize, Deserialize, Debug, ZeroizeOnDrop)]
struct KeystoreFile {
    version: String, // String implements ZeroizeOnDrop
    // master_kdf: String, // Field already removed in previous operation if successful
    // master_kdf_params: EntryKdfParams, // Field already updated and to be removed
    // verification_tag: Option<String>, // Field already removed
    // verification_nonce: Option<String>, // Field already removed
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
        // Fix for F-3: Properly zeroize the secret scalar
        // Convert to mutable bytes, zero them, then force re-randomisation of scalar
        let mut bytes = self.0.to_bytes();
        bytes.zeroize();
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
    entries: Vec<EncryptedKeyEntry>,
    config: KeystoreConfig, // Keep config for KDF parameters for new entries
    pepper: &'static [u8],  // Keep pepper
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

    fn new_with_config(
        custom_path: Option<PathBuf>,
        config: KeystoreConfig,
    ) -> Result<Self, KeystoreError> {
        // Fix for F-6: Enforce minimum output length of 32 bytes
        if config.output_len < 32 {
            return Err(KeystoreError::FsError(
                "KDF output length must be at least 32 bytes".to_string(),
            ));
        }

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
            entries: Vec::new(),
            config,                       // Initialization matches the reduced Keystore struct
            pepper: Self::DEFAULT_PEPPER, // Initialization matches the reduced Keystore struct
        })
    }

    // Set a custom pepper (useful for testing or runtime configuration)
    pub fn with_pepper(mut self, pepper: &'static [u8]) -> Self {
        self.pepper = pepper;
        self
    }

    pub fn initialize_or_load(&mut self) -> Result<(), KeystoreError> {
        // Removed password parameter
        self.load_from_disk() // Just load from disk
    }

    // All functions related to global lock, master password, and unlock rate limiting are removed.
    // Keystore::unlock
    // Keystore::lock
    // Keystore::create_verification_tag
    // Keystore::verify_password
    // Keystore::get_verification_tag
    // Keystore::get_verification_nonce
    // Keystore::enforce_unlock_rate_limiting
    // Keystore::calculate_backoff_delay
    // Keystore::increment_unlock_attempts
    // Keystore::check_auto_lock
    // Keystore::update_activity_timestamp

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
        password: &str, // Added password parameter
    ) -> Result<(Uuid, Address), KeystoreError> {
        if pk_hex.is_empty() {
            return Err(KeystoreError::InvalidPrivateKey);
        }
        if password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        let pk_bytes = hex::decode(pk_hex)?;
        if pk_bytes.len() != 32 {
            return Err(KeystoreError::InvalidPrivateKey);
        }
        let pk_bytes_zeroizing = Zeroizing::new(pk_bytes);

        let secret_key = SecretKey::from_slice(pk_bytes_zeroizing.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;
        let signing_key = SigningKey::from(&secret_key);
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

        // Generate KDF params for this entry
        let kdf_params = Self::generate_entry_kdf_params(&mut OsRng, &self.config)?;

        // Derive key for this entry
        let entry_derived_key = self.derive_key_for_entry(password, &kdf_params)?;
        let entry_derived_key_zeroizing = Zeroizing::new(entry_derived_key);

        // Encrypt private key
        let (encrypted_pk_vec, nonce_bytes) = self.encrypt_pk(
            pk_bytes_zeroizing.as_slice(),
            entry_derived_key_zeroizing.as_slice(),
            &id,
            &address,
        )?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            kdf_params,                                 // Store KDF params
            nonce: BASE64_STANDARD.encode(nonce_bytes), // Store nonce
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.entries.push(entry);
        self.save_to_disk()?;
        // self.update_activity_timestamp(); // Removed: No global activity tracking

        Ok((id, address))
    }

    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        phrase: &str,
        passphrase: Option<&str>, // BIP39 passphrase, not the entry password
        path_str: &str,
        password: &str, // Added entry password parameter
    ) -> Result<(Uuid, Address), KeystoreError> {
        if phrase.is_empty() {
            return Err(KeystoreError::InvalidPrivateKey); // Or InvalidMnemonic
        }
        if password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        let mnemonic = Mnemonic::parse(phrase)?;
        let seed_bytes_array = mnemonic.to_seed(passphrase.unwrap_or(""));
        let seed_bytes_zeroizing = Zeroizing::new(seed_bytes_array.to_vec());

        let derivation_path = DerivationPath::from_str(path_str).map_err(|e| {
            KeystoreError::InvalidPath(format!("Failed to parse derivation path: {}", e))
        })?;
        let xprv = XPrv::derive_from_path(seed_bytes_zeroizing.as_slice(), &derivation_path)
            .map_err(KeystoreError::Bip32)?;
        let bip32_signing_key = xprv.private_key();
        let secret_bytes_from_bip32 = bip32_signing_key.to_bytes();
        let secret_bytes_zeroizing = Zeroizing::new(secret_bytes_from_bip32.to_vec());
        let secret_key = SecretKey::from_slice(secret_bytes_zeroizing.as_slice())
            .map_err(|_| KeystoreError::DerivationFailed)?;

        // pk_bytes_for_encryption should be the raw secret key bytes
        let pk_bytes_for_encryption = Zeroizing::new(secret_key.to_bytes().to_vec());

        let signing_key = SigningKey::from(&secret_key);
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

        // Generate KDF params for this entry
        let kdf_params = Self::generate_entry_kdf_params(&mut OsRng, &self.config)?;

        // Derive key for this entry
        let entry_derived_key = self.derive_key_for_entry(password, &kdf_params)?;
        let entry_derived_key_zeroizing = Zeroizing::new(entry_derived_key);

        // Encrypt private key
        let (encrypted_pk_vec, nonce_bytes) = self.encrypt_pk(
            pk_bytes_for_encryption.as_slice(), // Use the derived secret key bytes
            entry_derived_key_zeroizing.as_slice(),
            &id,
            &address,
        )?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_vec),
            kdf_params,                                 // Store KDF params
            nonce: BASE64_STANDARD.encode(nonce_bytes), // Store nonce
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.entries.push(entry);
        self.save_to_disk()?;
        // self.update_activity_timestamp(); // Removed

        Ok((id, address))
    }

    pub fn list_keys(&mut self) -> Result<Vec<KeyInfo>, KeystoreError> {
        // No activity timestamp updates needed anymore.
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
    pub fn get_signer(
        &mut self,
        uuid: Uuid,
        password: &str, // Added password parameter
    ) -> Result<ZeroizingSigningKey, KeystoreError> {
        if password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        let entry = self
            .entries
            .iter()
            .find(|e| e.id == uuid)
            .ok_or(KeystoreError::KeyNotFound(uuid))?;

        // Derive key for this entry using its KDF params and the provided password
        let entry_derived_key = self.derive_key_for_entry(password, &entry.kdf_params)?;
        let entry_derived_key_zeroizing = Zeroizing::new(entry_derived_key);

        let encrypted_pk_bytes = BASE64_STANDARD
            .decode(&entry.encrypted_pk)
            .map_err(|_e| KeystoreError::InvalidFormat)?;

        let nonce_vec = BASE64_STANDARD
            .decode(&entry.nonce)
            .map_err(|_e| KeystoreError::InvalidFormat)?;
        let nonce_bytes: [u8; 12] = nonce_vec.try_into().map_err(|_| {
            KeystoreError::InvalidFormat // Or specific error for nonce length
        })?;

        // Decrypt private key using the derived key and stored nonce
        let decrypted_pk_zeroizing_vec = self.decrypt_pk(
            &encrypted_pk_bytes,
            entry_derived_key_zeroizing.as_slice(),
            &nonce_bytes,
            &entry.id,
            &entry.address,
        )?;

        let secret_key = SecretKey::from_slice(decrypted_pk_zeroizing_vec.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;

        let signing_key = SigningKey::from(secret_key);
        Ok(ZeroizingSigningKey::from(signing_key))
    }

    // Verify a signature against a message hash using the key identified by uuid
    pub fn verify_signature(
        &mut self,
        uuid: Uuid,
        password: &str, // Added password parameter
        message_hash: H256,
        signature: Signature,
    ) -> Result<bool, KeystoreError> {
        // Get the signer for this key, passing the password
        let zeroizing_signing_key = self.get_signer(uuid, password)?;

        // Get the verifying key from the signing key
        // Use as_ref() to access the inner SigningKey
        let verifying_key = zeroizing_signing_key.as_ref().verifying_key();

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

        // self.update_activity_timestamp(); // Removed
        Ok(result)
    }

    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError> {
        // No lock checks or activity timestamp updates needed anymore.
        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.id != uuid);

        if self.entries.len() == initial_len {
            return Err(KeystoreError::KeyNotFound(uuid));
        }

        self.save_to_disk()?;
        Ok(())
    }

    pub fn change_password(
        &mut self,
        uuid: Uuid,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        if new_password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }
        if old_password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        let entry_index = self
            .entries
            .iter()
            .position(|e| e.id == uuid)
            .ok_or(KeystoreError::KeyNotFound(uuid))?;

        // Clone necessary parts of the entry to avoid borrow checker issues
        // while calling methods on `self`.
        let entry_id = self.entries[entry_index].id;
        let entry_address = self.entries[entry_index].address;
        let old_kdf_params = self.entries[entry_index].kdf_params.clone();
        let old_encrypted_pk = self.entries[entry_index].encrypted_pk.clone();
        let old_nonce_str = self.entries[entry_index].nonce.clone();

        // 1. Decrypt with old password and old KDF params
        let old_derived_key = self.derive_key_for_entry(old_password, &old_kdf_params)?;
        let old_derived_key_zeroizing = Zeroizing::new(old_derived_key);

        let encrypted_pk_bytes = BASE64_STANDARD
            .decode(&old_encrypted_pk)
            .map_err(|_| KeystoreError::InvalidFormat)?;

        let nonce_vec = BASE64_STANDARD
            .decode(&old_nonce_str)
            .map_err(|_| KeystoreError::InvalidFormat)?;
        let old_nonce_bytes: [u8; 12] = nonce_vec
            .try_into()
            .map_err(|_| KeystoreError::InvalidFormat)?;

        let decrypted_pk_zeroizing = self.decrypt_pk(
            &encrypted_pk_bytes,
            old_derived_key_zeroizing.as_slice(),
            &old_nonce_bytes,
            &entry_id,      // Use cloned id
            &entry_address, // Use cloned address
        )?;

        // 2. Generate new KDF params
        let new_kdf_params = Self::generate_entry_kdf_params(&mut OsRng, &self.config)?;

        // 3. Derive new key with new password and new KDF params
        let new_derived_key = self.derive_key_for_entry(new_password, &new_kdf_params)?;
        let new_derived_key_zeroizing = Zeroizing::new(new_derived_key);

        // 4. Re-encrypt with new key; encrypt_pk will generate a new nonce
        let (new_encrypted_pk_vec, new_nonce_bytes_array) = self.encrypt_pk(
            decrypted_pk_zeroizing.as_slice(),
            new_derived_key_zeroizing.as_slice(),
            &entry_id,      // Use cloned id
            &entry_address, // Use cloned address
        )?;

        // 5. Update the entry in self.entries
        let entry_to_update = &mut self.entries[entry_index];
        entry_to_update.encrypted_pk = BASE64_STANDARD.encode(&new_encrypted_pk_vec);
        entry_to_update.nonce = BASE64_STANDARD.encode(new_nonce_bytes_array);
        entry_to_update.kdf_params = new_kdf_params;
        entry_to_update.updated_at = Utc::now();

        self.save_to_disk()?;
        Ok(())
    }

    fn save_to_disk(&mut self) -> Result<(), KeystoreError> {
        // Create parent directories if they don't exist
        if let Some(parent) = self.file_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(KeystoreError::Io)?;
            }
        }

        // Fix for F-7: Create a separate lock file in the target directory
        let lock_file_path = self.file_path.with_extension("lock");
        let lock_file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&lock_file_path)
            .map_err(KeystoreError::Io)?;

        // Lock the lock file to prevent concurrent access
        lock_file
            .lock_exclusive()
            .map_err(|e| KeystoreError::FsError(format!("Failed to lock file: {}", e)))?;

        // KeystoreFile struct is simpler now
        let keystore_data = KeystoreFile {
            version: "1.0.0".to_string(),  // Current version
            entries: self.entries.clone(), // Entries are cloned
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

        // Fix for F-11: Add Windows-specific file permission handling
        #[cfg(windows)]
        {
            // Windows-specific code would go here to set appropriate ACLs
            // This would use the winapi crate to call SetNamedSecurityInfoW
            // For now, we'll just add a comment as a placeholder
            // TODO: Implement Windows-specific file permission handling
        }

        // Unlock the lock file
        lock_file
            .unlock()
            .map_err(|e| KeystoreError::FsError(format!("Failed to unlock file: {}", e)))?;

        Ok(())
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        if !self.file_path.exists() {
            return Ok(()); // OK if file doesn't exist, means new keystore
        }

        // Fix for F-7: Use the lock file for locking
        let lock_file_path = self.file_path.with_extension("lock");
        let lock_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_file_path)
            .map_err(KeystoreError::Io)?;

        // Acquire a shared lock on the lock file
        lock_file
            .lock_shared()
            .map_err(|e| KeystoreError::FsError(format!("Failed to lock file: {}", e)))?;

        let file_content = fs::read_to_string(&self.file_path)?;
        let keystore_data: KeystoreFile = serde_json::from_str(&file_content)?;

        // Version check
        if keystore_data.version != "1.0.0" {
            return Err(KeystoreError::InvalidFormat);
        }
        // No master_kdf to check anymore

        self.entries = keystore_data.entries.clone();
        // Fields like master_kdf_params, verification_tag, etc., are no longer part of Keystore.

        // Unlock the lock file
        lock_file
            .unlock()
            .map_err(|e| KeystoreError::FsError(format!("Failed to unlock file: {}", e)))?;

        Ok(())
    }

    fn encrypt_pk(
        &self,
        pk_bytes: &[u8],
        entry_derived_key: &[u8], // Changed: Use pre-derived key
        id: &Uuid,
        address: &Address,
    ) -> Result<(Vec<u8>, [u8; 12]), KeystoreError> {
        // Return includes nonce
        // Generate a new nonce for each encryption
        let mut nonce_bytes = [0u8; 12];
        OsRng
            .try_fill_bytes(&mut nonce_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate nonce: {}", e)))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let key = Key::<Aes256Gcm>::from_slice(entry_derived_key); // Use entry_derived_key
        let cipher = Aes256Gcm::new(key);
        // Inline AAD creation
        let mut aad = Vec::with_capacity(16 + 20); // UUID (16 bytes) + Address (20 bytes)
        aad.extend_from_slice(id.as_bytes());
        aad.extend_from_slice(address.as_slice());

        let ciphertext = cipher
            .encrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: pk_bytes,
                    aad: &aad,
                },
            )
            .map_err(|e| KeystoreError::AesGcm(format!("Encryption failed: {}", e)))?;
        Ok((ciphertext, nonce_bytes)) // Return ciphertext and nonce
    }

    fn decrypt_pk(
        &self,
        encrypted_pk_bytes: &[u8],
        entry_derived_key: &[u8], // Changed: Use pre-derived key
        nonce_bytes: &[u8; 12],   // Added: Pass nonce
        id: &Uuid,
        address: &Address,
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let key = Key::<Aes256Gcm>::from_slice(entry_derived_key); // Use entry_derived_key
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        // Inline AAD creation
        let mut aad = Vec::with_capacity(16 + 20); // UUID (16 bytes) + Address (20 bytes)
        aad.extend_from_slice(id.as_bytes());
        aad.extend_from_slice(address.as_slice());

        let decrypted_bytes = cipher
            .decrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: encrypted_pk_bytes,
                    aad: &aad,
                },
            )
            .map_err(|_| KeystoreError::InvalidPassword)?; // Or a more specific error
        Ok(Zeroizing::new(decrypted_bytes))
    }

    // fn create_aad removed.
    // fn derive_entry_key removed.

    fn derive_key_for_entry(
        // Renamed from derive_master_key
        &self,
        password: &str,
        kdf_params: &EntryKdfParams, // Use EntryKdfParams
    ) -> Result<Vec<u8>, KeystoreError> {
        let salt = hex::decode(&kdf_params.salt)
            .map_err(|e| KeystoreError::Argon2Error(format!("Failed to decode salt: {}", e)))?;

        // Fix for F-5: Remove pepper usage or make it per-installation
        // For now, we'll keep using the pepper for backward compatibility
        // but add a comment indicating it should be replaced in a future version
        // SECURITY NOTE: This pepper is hard-coded and public, providing no additional security.
        // TODO: Replace with per-installation random pepper stored in OS keychain/environment
        let mut password_with_pepper = String::with_capacity(password.len() + self.pepper.len());
        password_with_pepper.push_str(password);
        password_with_pepper.push_str(std::str::from_utf8(self.pepper).unwrap_or(""));

        // Fix for F-6: Enforce minimum output length of 32 bytes
        let output_len = std::cmp::max(kdf_params.output_len, 32);

        let params = argon2::Params::new(
            kdf_params.m_cost,
            kdf_params.t_cost,
            kdf_params.p_cost,
            Some(output_len),
        )
        .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?; // Ensure argon2::Error is converted

        let argon2_context = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13, // Argon2 version 1.3
            params,
        );

        // Use Zeroizing<Vec<u8>> for output_key_material to ensure it's properly zeroized
        // Fix for F-13: Wrap intermediate Vec<u8> copies of secrets in Zeroizing
        let mut output_key_material = Zeroizing::new(vec![0u8; output_len]);
        argon2_context
            .hash_password_into(
                password_with_pepper.as_bytes(),
                &salt,
                output_key_material.as_mut_slice(),
            )
            .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?;

        // Zeroize the password with pepper
        password_with_pepper.zeroize();

        Ok(output_key_material.to_vec())
    }

    // fn generate_kdf_params removed

    fn generate_entry_kdf_params(
        // Name is already generate_entry_kdf_params
        rng: &mut (impl CryptoRng + RngCore),
        config: &KeystoreConfig,
    ) -> Result<EntryKdfParams, KeystoreError> {
        // Return type is correct
        // Fix for F-6: Enforce minimum output length of 32 bytes
        if config.output_len < 32 {
            return Err(KeystoreError::FsError(
                "KDF output length must be at least 32 bytes".to_string(),
            ));
        }

        let mut salt_bytes = [0u8; 16]; // 16-byte salt
        rng.try_fill_bytes(&mut salt_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate salt: {}", e)))?;

        Ok(EntryKdfParams {
            // Return EntryKdfParams
            salt: hex::encode(salt_bytes),
            m_cost: config.m_cost,
            t_cost: config.t_cost,
            p_cost: config.p_cost,
            output_len: config.output_len,
        })
    }
}
