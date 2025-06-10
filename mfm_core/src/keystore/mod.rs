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
use hex; // For encoding salt
use k256::{ecdsa::SigningKey, SecretKey};
use rand::TryRngCore;
// Removed PublicKey
// Removed incorrect imports for ScalarCore and ZeroizePrimitive
use ring::hkdf; // For HKDF key derivation
use serde::{Deserialize, Serialize};
use std::fs::{self}; // Removed unused File import
use std::io::Write as IoWrite; // Removed unused Read import, kept Write alias
use std::path::{Path, PathBuf};
use std::str::FromStr; // For DerivationPath::from_str
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq; // ID 12: For constant-time comparison
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
    #[serde(default)] // Ensures old files lacking this field deserialize with version 0
    pub kdf_version: u32,
}

// Uuid and DateTime<Utc> do not implement Zeroize/ZeroizeOnDrop by default.
// String fields (encrypted_pk, nonce) do. Alias is Option<String>.
// Address is a fixed-size array, which should be fine.
// Removing ZeroizeOnDrop from EncryptedKeyEntry derive.
// String fields within will handle their own zeroization as String implements ZeroizeOnDrop.
#[derive(Serialize, Deserialize, Debug, Clone, ZeroizeOnDrop)]
pub struct EncryptedKeyEntry {
    #[zeroize(skip)]
    pub id: Uuid, // Not secret
    pub alias: Option<String>, // String part will be zeroized on drop if Some
    pub address: Address,      // Not secret
    pub encrypted_pk: String, // base64 encoded encrypted private key - String implements ZeroizeOnDrop
    pub nonce: String,        // base64 encoded nonce for AES-GCM - String implements ZeroizeOnDrop
    pub hkdf_salt: String,    // New field for per-entry HKDF salt (base64 encoded)
    #[zeroize(skip)]
    pub created_at: DateTime<Utc>, // Not secret
    #[zeroize(skip)]
    pub updated_at: DateTime<Utc>, // Not secret
                              // Potentially other metadata like derivation path if applicable, key type, etc.
}

// KeystoreFile's ZeroizeOnDrop will apply to its String fields.
// MasterKdfParams also derives ZeroizeOnDrop for its salt.
// Vec<EncryptedKeyEntry> elements (Strings) will be zeroized when they are dropped.
#[derive(Serialize, Deserialize, Debug, Clone, ZeroizeOnDrop)]
struct KeystoreFile {
    version: String,    // String implements ZeroizeOnDrop
    master_kdf: String, // String implements ZeroizeOnDrop
    master_kdf_params: MasterKdfParams,
    // Password verification tag (F-1)
    verification_tag: Option<String>, // base64 encoded encrypted verification tag
    verification_nonce: Option<String>, // base64 encoded nonce for verification tag
    // Rate limiting data (KM-H-01) - REMOVED
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
    // Rate limiting - REMOVED
}

// Argon2 min number of memory blocks required
// OWASP: >= 128MiB (131072 KiB)
const MIN_M_COST: u32 = 131072;
// Argon2 min number of iterations required
// OWASP: >= 4
const MIN_T_COST: u32 = 4;
// KDF current version
const KDF_VERSION: u32 = 1;

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            // Default KDF parameters
            m_cost: MIN_M_COST,
            t_cost: MIN_T_COST,
            p_cost: 1,      // Default parallelism
            output_len: 32, // Minimum required output length
            // Default session management
            auto_lock_timeout: Duration::from_secs(300), // 5 minutes
                                                         // Default rate limiting - REMOVED
        }
    }
}

// Wrapper for k256::SigningKey that implements ZeroizeOnDrop
#[derive(Debug)]
pub struct ZeroizingSigningKey(SigningKey);

impl ZeroizeOnDrop for ZeroizingSigningKey {}

impl Zeroize for ZeroizingSigningKey {
    fn zeroize(&mut self) {
        // To explicitly zeroize the sensitive material within self.0 (the k256::ecdsa::SigningKey),
        // we replace it with a new, dummy key. This action causes the old self.0
        // to be dropped, and its Drop implementation will zeroize its internal scalar.
        // This is a safe way to ensure the original key material is cleared without unsafe code.

        // Create a dummy secret. Use a valid, non-zero byte array.
        // [1; 32] is a simple choice for a non-problematic dummy key.
        // Need k256::SecretKey for this.
        // Ensure k256::SecretKey is in scope (it should be via use k256::SecretKey).
        match k256::SecretKey::from_slice(&[1u8; 32]) {
            Ok(dummy_secret_key) => {
                let dummy_signing_key = k256::ecdsa::SigningKey::from(&dummy_secret_key);
                self.0 = dummy_signing_key; // Old self.0 is dropped here, its secret zeroized.
            }
            Err(_) => {}
        }
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

// --- MasterKey Struct ---
// Wrapper for the master key to ensure it's zeroized on drop.
#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey(Zeroizing<Vec<u8>>);

impl MasterKey {
    // Constructor that takes ownership of Zeroizing<Vec<u8>>.
    // This is the primary way MasterKey instances will be created from derived key material.
    fn from_zeroizing(key: Zeroizing<Vec<u8>>) -> Self {
        MasterKey(key)
    }
}

// Allows MasterKey to be used where a slice &[u8] is expected (e.g., for cryptographic operations).
impl std::ops::Deref for MasterKey {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0 // Dereferences to the inner Zeroizing<Vec<u8>>, which then Derefs to Vec<u8>, then to [u8]
    }
}

// Provides an explicit way to get a reference to the underlying byte slice.
impl AsRef<[u8]> for MasterKey {
    fn as_ref(&self) -> &[u8] {
        &self.0 // Similar to Deref, gets the &[u8] from Zeroizing<Vec<u8>>
    }
}

// --- Keystore Struct ---

#[derive(Debug)] // Removed ZeroizeOnDrop from Keystore struct itself
pub struct Keystore {
    file_path: PathBuf,            // PathBuf does not need to be zeroized
    master_key: Option<MasterKey>, // Changed type here. MasterKey handles its own zeroization.
    entries: Vec<EncryptedKeyEntry>,
    master_kdf_params: Option<MasterKdfParams>,
    is_unlocked: bool,
    // last_activity_at does not need zeroization. No attribute needed.
    last_activity_at: Option<Instant>,
    // Configuration
    config: KeystoreConfig,
    // Password verification fields (F-1)
    verification_tag: Option<String>,
    verification_nonce: Option<String>,
}

// Helper struct for change_password to temporarily hold decrypted key data
struct DecryptedData {
    pk_material: Zeroizing<Vec<u8>>,
    id: Uuid,
    address: Address,
    alias: Option<String>,
    created_at: DateTime<Utc>,
}

impl Keystore {
    const DEFAULT_KEYSTORE_FILENAME: &'static str = "keystore_v1.json";
    const APP_DIR_NAME: &'static str = "mfm";

    /// Securely create a directory with 0o700 permissions.
    #[allow(dead_code)]
    fn ensure_secure_dir(dir: &Path) -> Result<(), KeystoreError> {
        if !dir.exists() {
            fs::create_dir_all(dir).map_err(|e| {
                KeystoreError::FsError(format!("Failed to create directory: {}", e))
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(dir)?.permissions();
                perms.set_mode(0o700);
                fs::set_permissions(dir, perms).map_err(|e| {
                    KeystoreError::FsError(format!("Failed to set directory permissions: {}", e))
                })?;
            }
        }
        Ok(())
    }

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
        // ID 5: Enforce minimum m_cost and t_cost
        if config.m_cost < MIN_M_COST {
            return Err(KeystoreError::FsError(format!(
                "KDF m_cost must be at least {} KiB",
                MIN_M_COST
            )));
        }
        if config.t_cost < MIN_T_COST {
            return Err(KeystoreError::FsError(format!(
                "KDF t_cost must be at least {}",
                MIN_T_COST
            )));
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
            master_key: None,
            entries: Vec::new(),
            master_kdf_params: None,
            is_unlocked: false,
            last_activity_at: None,
            config,
            verification_tag: None,
            verification_nonce: None,
        })
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
                let kdf_params = Self::generate_kdf_params_with_config(&self.config)?;
                // Derive the master key using 'p' and the new kdf_params
                let master_key_val = self.derive_master_key(p, &kdf_params)?;
                self.master_key = Some(master_key_val); // Set the master key
                self.master_kdf_params = Some(kdf_params); // Store kdf_params

                // Fix for F-1: Create a verification tag for the new keystore
                self.create_verification_tag()?;

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
        if self.is_unlocked {
            // Optionally, update last_activity_at or just return Ok
            self.last_activity_at = Some(Instant::now());
            return Ok(());
        }

        if password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        let kdf_params = match self.master_kdf_params.as_ref() {
            Some(params) => params,
            None => {
                return Err(KeystoreError::FsError(
                    "Keystore is not initialized with KDF parameters.".to_string(),
                ));
            }
        };

        let derived_key = match self.derive_master_key(password, kdf_params) {
            Ok(key) => key,
            Err(e) => {
                return Err(e);
            }
        };

        // F-1: Verify password using the tag
        if let (Some(tag), Some(nonce)) = (
            self.verification_tag.as_deref(),
            self.verification_nonce.as_deref(),
        ) {
            // derived_key is MasterKey, verify_password expects &MasterKey
            if !self.verify_password(&derived_key, tag, nonce)? {
                return Err(KeystoreError::InvalidPassword);
            }
        } else {
            // This case should ideally not be reached if keystore was initialized properly
            // and has a verification tag. If not, it's a state inconsistency.
            return Err(KeystoreError::MissingVerificationTag);
        }

        self.master_key = Some(derived_key); // Store MasterKey directly_zeroizing);
        self.is_unlocked = true;
        self.last_activity_at = Some(Instant::now());

        Ok(())
    }

    // Fix for F-1: Create a verification tag for password verification
    fn create_verification_tag(&mut self) -> Result<(), KeystoreError> {
        // Ensure parent directory exists (best effort)
        if let Some(parent_dir) = self.file_path.parent() {
            if !parent_dir.exists() {
                let _ = fs::create_dir_all(parent_dir); // Ignore error if it fails, read/write will fail later
            }
        }

        // The master_key must be set by the caller (e.g. initialize_or_load or change_password)
        // before this function is invoked. This function uses self.master_key directly.

        // Generate a random nonce for the verification tag
        let mut nonce_bytes = [0u8; 12];
        rand::rng()
            .try_fill_bytes(&mut nonce_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate nonce: {}", e)))?;

        // Create a verification tag by encrypting a known plaintext
        let plaintext = b"ok";
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        let key = Key::<Aes256Gcm>::from_slice(master_key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let encrypted_tag = cipher
            .encrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: plaintext,
                    aad: b"verify",
                },
            )
            .map_err(|e| KeystoreError::AesGcm(format!("Encryption failed: {}", e)))?;

        // Store the verification tag and nonce
        self.verification_tag = Some(BASE64_STANDARD.encode(&encrypted_tag));
        self.verification_nonce = Some(BASE64_STANDARD.encode(nonce_bytes));

        Ok(())
    }

    // Fix for F-1: Verify the password using the verification tag
    fn verify_password(
        &self,
        master_key: &MasterKey, // Changed type
        tag: &str,
        nonce: &str,
    ) -> Result<bool, KeystoreError> {
        let encrypted_tag = BASE64_STANDARD.decode(tag).map_err(|_| {
            KeystoreError::InvalidFormat("Failed to decode verification tag".to_string())
        })?;

        let nonce_vec = BASE64_STANDARD.decode(nonce).map_err(|_| {
            KeystoreError::InvalidFormat("Failed to decode verification nonce".to_string())
        })?;

        let nonce_bytes: [u8; 12] = nonce_vec.try_into().map_err(|_| {
            KeystoreError::InvalidFormat("Invalid verification nonce length".to_string())
        })?;

        let key = Key::<Aes256Gcm>::from_slice(master_key.as_ref());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Try to decrypt the verification tag
        match cipher.decrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: &encrypted_tag,
                aad: b"verify",
            },
        ) {
            Ok(decrypted) => {
                // ID 12: Use constant-time comparison for the verification tag
                Ok(decrypted.ct_eq(b"ok").into())
            }
            Err(_) => {
                // Mitigate timing oracle (C-3):
                // Always perform a constant-time comparison even if decryption fails.
                // The expected plaintext is b"ok".
                let expected_plaintext = b"ok";
                // Create dummy data of the same length as the expected plaintext.
                // The content of dummy_data doesn't matter, only its length and the ct_eq call.
                let dummy_data = vec![0u8; expected_plaintext.len()];
                // Perform a constant-time comparison. The result is irrelevant here
                // as we already know verification failed, but the operation's timing is what matters.
                let _ = dummy_data.ct_eq(expected_plaintext);
                Ok(false) // Verification failed
            }
        }
    }

    // Helper methods to get verification tag and nonce
    fn get_verification_tag(&self) -> Option<&str> {
        self.verification_tag.as_deref()
    }

    fn get_verification_nonce(&self) -> Option<&str> {
        self.verification_nonce.as_deref()
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

        let pk_bytes = hex::decode(pk_hex)?;
        if pk_bytes.len() != 32 {
            return Err(KeystoreError::InvalidPrivateKey);
        }

        // Create a zeroizing buffer for the private key bytes
        let pk_bytes = Zeroizing::new(pk_bytes);

        // Create a k256::SecretKey from the private key bytes
        let secret_key = SecretKey::from_slice(pk_bytes.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;

        // Create a k256::SigningKey from the SecretKey
        let signing_key = SigningKey::from(&secret_key);

        // Get the public key from the signing key
        let public_key = signing_key.verifying_key();

        // Compute the Ethereum address from the public key
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
        let mut aes_nonce_bytes = [0u8; 12];
        rand::rng()
            .try_fill_bytes(&mut aes_nonce_bytes)
            .map_err(|e| {
                KeystoreError::FsError(format!(
                    "import_private_key_hex: Failed to generate AES nonce: {}",
                    e
                ))
            })?;

        let (encrypted_pk_data, new_hkdf_salt_bytes) =
            self.encrypt_pk(pk_bytes.as_slice(), &aes_nonce_bytes, &id, &address)?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_data),
            nonce: BASE64_STANDARD.encode(aes_nonce_bytes),
            hkdf_salt: BASE64_STANDARD.encode(new_hkdf_salt_bytes),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.entries.push(entry);
        self.save_to_disk()?;
        self.update_activity_timestamp();

        Ok((id, address))
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
        let mut aes_nonce_bytes = [0u8; 12];
        rand::rng()
            .try_fill_bytes(&mut aes_nonce_bytes)
            .map_err(|e| {
                KeystoreError::FsError(format!(
                    "import_mnemonic: Failed to generate AES nonce: {}",
                    e
                ))
            })?;

        let (encrypted_pk_data, new_hkdf_salt_bytes) = self.encrypt_pk(
            pk_bytes_for_encryption.as_slice(),
            &aes_nonce_bytes,
            &id,
            &address,
        )?;

        let entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_data),
            nonce: BASE64_STANDARD.encode(aes_nonce_bytes),
            hkdf_salt: BASE64_STANDARD.encode(new_hkdf_salt_bytes),
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
    pub fn get_signer(&mut self, uuid: Uuid) -> Result<ZeroizingSigningKey, KeystoreError> {
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

        let encrypted_pk_bytes = BASE64_STANDARD.decode(&entry.encrypted_pk).map_err(|_e| {
            KeystoreError::InvalidFormat(
                "get_signer: Failed to decode entry.encrypted_pk".to_string(),
            )
        })?;

        let nonce_vec = BASE64_STANDARD.decode(&entry.nonce).map_err(|_e| {
            KeystoreError::InvalidFormat("get_signer: Failed to decode entry.nonce".to_string())
        })?;

        let aes_nonce_bytes: [u8; 12] = nonce_vec.try_into().map_err(|_| {
            KeystoreError::InvalidFormat(
                "get_signer: Invalid AES nonce length for entry.nonce".to_string(),
            )
        })?;

        let decrypted_pk_zeroizing_vec = self.decrypt_pk(
            &encrypted_pk_bytes,
            &aes_nonce_bytes,
            &entry.id,
            &entry.address,
            &entry.hkdf_salt,
        )?;

        let secret_key = SecretKey::from_slice(decrypted_pk_zeroizing_vec.as_slice())
            .map_err(|_| KeystoreError::InvalidPrivateKey)?; // Should be valid if encryption/decryption worked

        let signing_key = SigningKey::from(secret_key);
        // Fix for F-3: Return ZeroizingSigningKey instead of raw SigningKey
        // This ensures the key will be properly zeroized when dropped

        self.update_activity_timestamp();
        Ok(ZeroizingSigningKey::from(signing_key))
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
        let zeroizing_signing_key = self.get_signer(uuid)?;

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

        if new_password.is_empty() {
            return Err(KeystoreError::InvalidPassword);
        }

        // 1. Verify the old password explicitly, regardless of current unlocked state.
        // This ensures the user knows the old password before changing it.
        let kdf_params = self
            .master_kdf_params
            .as_ref()
            .ok_or(KeystoreError::FsError(
                "Keystore is not initialized with KDF parameters.".to_string(),
            ))?;

        let old_master_key = self.derive_master_key(old_password, kdf_params)?;
        if let (Some(tag), Some(nonce)) =
            (self.get_verification_tag(), self.get_verification_nonce())
        {
            if !self.verify_password(&old_master_key, tag, nonce)? {
                // If old password verification fails, return InvalidPassword
                return Err(KeystoreError::InvalidPassword);
            }
        } else {
            // This should not happen if the keystore is initialized, but handle defensively.
            return Err(KeystoreError::MissingVerificationTag);
        }

        // At this point, old_password is confirmed correct.
        // Now proceed with generating new KDF params and re-encrypting.

        // 2. Generate new KDF parameters and derive new master key
        let new_kdf_params = Self::generate_kdf_params_with_config(&self.config)?;
        let new_master_key = self.derive_master_key(new_password, &new_kdf_params)?;

        // 3. The old master key is currently in `old_derived_key_zeroizing`.
        // We don't need to `take` from `self.master_key` here, as we derived it fresh.
        // We will use `old_derived_key_zeroizing` for decryption.

        // 4. Decrypt all private keys using the old_derived_key_zeroizing and store them
        // Temporarily set self.master_key to old_derived_key_zeroizing for decrypt_pk calls
        let original_master_key_state = self.master_key.take(); // Save current state
        self.master_key = Some(old_master_key.clone()); // Temporarily use old key (MasterKey is Clone)ived from old_password

        let mut temp_decrypted_data: Vec<DecryptedData> = Vec::new();
        for entry_to_decrypt in self.entries.iter() {
            let encrypted_pk_bytes = BASE64_STANDARD
                .decode(&entry_to_decrypt.encrypted_pk)
                .map_err(|e| {
                    KeystoreError::InvalidFormat(format!(
                        "change_password: Corrupted encrypted_pk for entry {}: {}",
                        entry_to_decrypt.id, e
                    ))
                })?;

            let aes_nonce_bytes: [u8; 12] = BASE64_STANDARD
                .decode(&entry_to_decrypt.nonce)
                .map_err(|e| {
                    KeystoreError::InvalidFormat(format!(
                        "change_password: Corrupted nonce for entry {}: {}",
                        entry_to_decrypt.id, e
                    ))
                })?
                .try_into()
                .map_err(|_| {
                    KeystoreError::InvalidFormat(format!(
                        "change_password: Invalid nonce length for entry {}",
                        entry_to_decrypt.id
                    ))
                })?;

            let decrypted_pk_material = self.decrypt_pk(
                &encrypted_pk_bytes,
                &aes_nonce_bytes,
                &entry_to_decrypt.id,
                &entry_to_decrypt.address,
                &entry_to_decrypt.hkdf_salt, // Corrected from `entry.hkdf_salt` to `entry_to_decrypt.hkdf_salt`
            )?;
            temp_decrypted_data.push(DecryptedData {
                pk_material: decrypted_pk_material,
                id: entry_to_decrypt.id,
                address: entry_to_decrypt.address,
                alias: entry_to_decrypt.alias.clone(),
                created_at: entry_to_decrypt.created_at,
            });
        }

        // Restore original master key state (if it was unlocked before)
        self.master_key = original_master_key_state;

        // 5. Clear existing entries (they are still the old encrypted ones)
        self.entries.clear();

        // 6. Set self.master_key to the new_master_key for the re-encryption phase
        self.master_key = Some(new_master_key);

        // 7. Re-encrypt all data with the new master key
        for data in temp_decrypted_data {
            let mut new_aes_nonce_bytes = [0u8; 12];
            rand::rng()
                .try_fill_bytes(&mut new_aes_nonce_bytes)
                .map_err(|e| {
                    KeystoreError::FsError(format!(
                        "change_password: Failed to generate AES nonce for re-encryption: {}",
                        e
                    ))
                })?;

            let (new_encrypted_pk_vec, new_hkdf_salt_bytes) = self.encrypt_pk(
                data.pk_material.as_slice(),
                &new_aes_nonce_bytes,
                &data.id,
                &data.address,
            )?;

            let new_entry = EncryptedKeyEntry {
                id: data.id,
                alias: data.alias,
                address: data.address,
                encrypted_pk: BASE64_STANDARD.encode(&new_encrypted_pk_vec),
                nonce: BASE64_STANDARD.encode(new_aes_nonce_bytes),
                hkdf_salt: BASE64_STANDARD.encode(new_hkdf_salt_bytes),
                created_at: data.created_at,
                updated_at: Utc::now(),
            };
            self.entries.push(new_entry);
        }
        // temp_decrypted_data and its Zeroizing<Vec<u8>> elements will be dropped here.

        // 8. Update KDF parameters, create verification tag, save
        self.master_kdf_params = Some(new_kdf_params);

        // Fix for F-1: Create a new verification tag with the new password
        self.create_verification_tag()?;

        self.save_to_disk()?;
        self.lock(); // Lock the keystore after password change
        self.update_activity_timestamp();

        Ok(())
    }

    fn save_to_disk(&mut self) -> Result<(), KeystoreError> {
        if let Some(parent) = self.file_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(KeystoreError::Io)?;
            }
        }

        let lock_file_path = self.file_path.with_extension("lock");
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&lock_file_path)
            .map_err(KeystoreError::Io)?;

        fs2::FileExt::lock_exclusive(&lock_file)
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
            verification_tag: self.verification_tag.clone(),
            verification_nonce: self.verification_nonce.clone(),
            entries: self.entries.clone(),
        };

        let json_data = serde_json::to_string_pretty(&keystore_data)?;

        let atomic_file = AtomicFile::new(&self.file_path, OverwriteBehavior::AllowOverwrite);
        atomic_file
            .write(|f| f.write_all(json_data.as_bytes()).map_err(KeystoreError::Io))
            .map_err(|e| KeystoreError::FsError(format!("Failed to write keystore file: {}", e)))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&self.file_path)
                .map_err(KeystoreError::Io)?
                .permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&self.file_path, perms).map_err(KeystoreError::Io)?;
        }

        fs2::FileExt::unlock(&lock_file)
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
        } // Closes if !self.file_path.exists()
          // Fix for F-7: Use the lock file for locking
        let lock_file_path = self.file_path.with_extension("lock");
        let lock_file = fs::OpenOptions::new()
            .read(true)
            .create(true)
            .append(true) // Use append for lock file to satisfy clippy, content is not relevant
            .open(&lock_file_path)
            .map_err(KeystoreError::Io)?;

        // Acquire a shared lock on the lock file
        fs2::FileExt::lock_shared(&lock_file)
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
                return Err(KeystoreError::InvalidFormat(format!(
                    "Unsupported keystore version: {}",
                    keystore_data.version
                )));
            }
        }

        if keystore_data.master_kdf != "argon2id" {
            // Clone master_kdf because KeystoreFile implements Drop
            return Err(KeystoreError::UnsupportedKdf(
                keystore_data.master_kdf.clone(),
            ));
        }

        let loaded_kdf_params = keystore_data.master_kdf_params.clone();

        // Validate loaded KDF parameters against minimums
        if loaded_kdf_params.m_cost < MIN_M_COST {
            return Err(KeystoreError::Argon2Error(format!(
                "Loaded KDF m_cost ({}) is below minimum required ({} KiB)",
                loaded_kdf_params.m_cost, MIN_M_COST
            )));
        }
        if loaded_kdf_params.t_cost < MIN_T_COST {
            return Err(KeystoreError::Argon2Error(format!(
                "Loaded KDF t_cost ({}) is below minimum required ({})",
                loaded_kdf_params.t_cost, MIN_T_COST
            )));
        }
        // Enforce minimum output length of 32 bytes for loaded params
        if loaded_kdf_params.output_len < 32 {
            return Err(KeystoreError::Argon2Error(format!(
                "Loaded KDF output length ({}) is below minimum required (32 bytes)",
                loaded_kdf_params.output_len
            )));
        }

        self.master_kdf_params = Some(loaded_kdf_params);
        self.verification_tag = keystore_data.verification_tag.clone();
        self.verification_nonce = keystore_data.verification_nonce.clone();
        self.entries = keystore_data.entries.clone(); // Ensure this is assigned

        // Keystore remains locked after loading. Unlock is a separate step.
        self.is_unlocked = false;
        self.master_key = None;

        // Unlock the lock file
        fs2::FileExt::unlock(&lock_file)
            .map_err(|e| KeystoreError::FsError(format!("Failed to unlock file: {}", e)))?;

        Ok(())
    } // End of load_from_disk

    fn encrypt_pk(
        &self,
        pk_bytes: &[u8],
        aes_nonce_bytes: &[u8; 12], // Renamed for clarity
        id: &Uuid,
        address: &Address,
    ) -> Result<(Vec<u8>, [u8; 32]), KeystoreError> {
        // Returns (encrypted_pk_vec, hkdf_salt_bytes)
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        // Generate a new random HKDF salt (ID 7)
        let mut hkdf_salt_bytes = [0u8; 32];
        rand::rng()
            .try_fill_bytes(&mut hkdf_salt_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate HKDF salt: {}", e)))?;

        let entry_key = self.derive_entry_key(master_key_bytes, &hkdf_salt_bytes, id, address)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let aes_nonce = Nonce::from_slice(aes_nonce_bytes); // Use renamed variable

        // Create AAD from entry metadata
        let aad = self.create_aad(id, address);

        let encrypted_data = cipher
            .encrypt(
                aes_nonce, // Use aes_nonce
                aes_gcm::aead::Payload {
                    msg: pk_bytes,
                    aad: &aad,
                },
            )
            .map_err(|e| KeystoreError::AesGcm(format!("Encryption failed: {}", e)))?;

        Ok((encrypted_data, hkdf_salt_bytes)) // Return both
    }

    fn decrypt_pk(
        &self,
        encrypted_pk_bytes: &[u8],
        aes_nonce_bytes: &[u8; 12], // Renamed for clarity
        id: &Uuid,
        address: &Address,
        hkdf_salt_b64: &str, // Added hkdf_salt_b64
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        // Decode HKDF salt (ID 7)
        let hkdf_salt_bytes_vec = BASE64_STANDARD.decode(hkdf_salt_b64).map_err(|_| {
            KeystoreError::InvalidFormat("decrypt_pk: Failed to decode HKDF salt".to_string())
        })?;
        let hkdf_salt_bytes: [u8; 32] = hkdf_salt_bytes_vec.try_into().map_err(|_| {
            KeystoreError::InvalidFormat("decrypt_pk: Invalid HKDF salt length".to_string())
        })?;

        let entry_key = self.derive_entry_key(master_key_bytes, &hkdf_salt_bytes, id, address)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_slice());
        let cipher = Aes256Gcm::new(key);
        let aes_nonce = Nonce::from_slice(aes_nonce_bytes); // Use renamed variable

        // Create AAD from entry metadata
        let aad = self.create_aad(id, address);

        let decrypted_bytes = cipher
            .decrypt(
                aes_nonce, // Corrected: Use aes_nonce here
                aes_gcm::aead::Payload {
                    msg: encrypted_pk_bytes,
                    aad: &aad,
                },
            )
            .map_err(|e| KeystoreError::AesGcm(format!("Decryption failed: {}", e)))?;

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
        hkdf_salt_bytes: &[u8],
        id: &Uuid,         // Added UUID parameter for domain separation
        address: &Address, // Added Address parameter for domain separation
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        // Use the provided salt for HKDF
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, hkdf_salt_bytes);
        let prk = salt.extract(master_key);

        // Create a single info buffer that includes:
        // 1. A fixed version string prefix
        // 2. The UUID of the key
        // 3. The ethereum address
        // This ensures domain separation even if salts collide
        let prefix = b"mfm-keystore-entry-key-v1";
        let mut info_buf = Vec::with_capacity(prefix.len() + 16 + 20); // version + UUID + Address
        info_buf.extend_from_slice(prefix);
        info_buf.extend_from_slice(id.as_bytes());
        info_buf.extend_from_slice(address.as_slice());

        // Use the combined info buffer
        let info_vec: &[&[u8]] = &[&info_buf];

        let mut okm = vec![0u8; 32]; // 32 bytes for AES-256
        prk.expand(info_vec, hkdf::HKDF_SHA256)
            .map_err(|_| KeystoreError::DerivationFailed)?
            .fill(&mut okm)
            .map_err(|_| KeystoreError::DerivationFailed)?;

        Ok(Zeroizing::new(okm))
    }

    fn derive_master_key(
        &self,
        password: &str,
        kdf_params: &MasterKdfParams,
    ) -> Result<MasterKey, KeystoreError> {
        // Changed return type
        let salt = hex::decode(&kdf_params.salt)
            .map_err(|e| KeystoreError::Argon2Error(format!("Failed to decode salt: {}", e)))?;

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
                password.as_bytes(), // Use password directly
                &salt,
                output_key_material.as_mut_slice(),
            )
            .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?;

        Ok(MasterKey::from_zeroizing(output_key_material)) // Changed return value
    }

    fn generate_kdf_params_with_config(
        config: &KeystoreConfig,
    ) -> Result<MasterKdfParams, KeystoreError> {
        // Fix for F-6: Enforce minimum output length of 32 bytes
        if config.output_len < 32 {
            return Err(KeystoreError::FsError(
                "KDF output length must be at least 32 bytes".to_string(),
            ));
        }
        // ID 5: Enforce minimum m_cost and t_cost
        if config.m_cost < MIN_M_COST {
            return Err(KeystoreError::FsError(format!(
                "KDF m_cost must be at least {} KiB",
                MIN_M_COST
            )));
        }
        if config.t_cost < MIN_T_COST {
            return Err(KeystoreError::FsError(format!(
                "KDF t_cost must be at least {}",
                MIN_T_COST
            )));
        }

        let mut salt_bytes = [0u8; 16]; // 16-byte salt
        rand::rng()
            .try_fill_bytes(&mut salt_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate salt: {}", e)))?;

        Ok(MasterKdfParams {
            salt: hex::encode(salt_bytes),
            m_cost: config.m_cost,
            t_cost: config.t_cost,
            p_cost: config.p_cost,
            output_len: config.output_len,
            kdf_version: KDF_VERSION,
        })
    }
} // Closing brace for impl Keystore
  // Ensure the module is declared in mfm_core/src/lib.rs or mfm_core/src/keystore/mod.rs
