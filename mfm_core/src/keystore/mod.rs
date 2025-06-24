// modules
pub mod error;
#[cfg(test)]
mod tests;

// keystore impl
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy_primitives::{Address, B256 as H256};
use alloy_signer::Signature;
use argon2::{self, Argon2}; // Import argon2 module for Params
use atomicwrites::{AtomicFile, OverwriteBehavior};
use base64::engine::general_purpose; // MAC and protected data encoding
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD; // For base64 encoding
use base64::Engine; // For encode/decode methods
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic; // Ensure Seed is not imported from bip39
use chrono::{DateTime, Utc};
use dirs_next;
use error::KeystoreError;
use hex; // For encoding salt
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::{ecdsa::SigningKey, SecretKey};
use rand::rngs::OsRng;
use rand::TryRngCore;
use ring::{hkdf, hmac}; // For HKDF, HMAC
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};
use tiny_keccak::{Hasher, Keccak}; // For Keccak-256
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const KEYSTORE_VERSION: u8 = 1;

// --- Structs for Keystore Data ---

// Constants for MAC key derivation
const HKDF_SALT_FOR_MAC_KEY_DERIVATION: &[u8] = b"mfm-mac-key-derivation-salt-v1";
const HKDF_INFO_MAC_KEY: &[u8] = b"mfm-keystore-mac-key-v1";

/// Top-level structure for the persisted keystore file, including a MAC.
#[derive(Serialize, Deserialize, Debug)]
pub struct AuthenticatedKeystoreEnvelope {
    // These fields are NOT MAC-protected but are necessary to derive the master_key,
    // which is then used to derive the MAC key for verifying `mac_b64`.
    pub master_kdf_algo_name: String,
    pub master_kdf_params: MasterKdfParams,
    pub verification_nonce: Option<String>, // Nonce for the verification_tag

    // This data IS MAC-protected.
    // It's the Base64 encoded JSON string of `ProtectedKeystorePart`.
    pub protected_data_b64: String,
    // Base64 encoded HMAC-SHA256 of the raw bytes of `protected_data_b64` (before it was base64 encoded itself).
    // Correction: MAC is over the *raw JSON bytes* of ProtectedKeystorePart.
    pub mac_b64: String,
}

/// Contains the part of the keystore that is integrity-protected by a MAC.
#[derive(Serialize, Deserialize, Debug, Clone, ZeroizeOnDrop)]
pub struct ProtectedKeystorePart {
    pub version: u8,
    pub verification_tag: Option<String>, // The verification_tag itself must be MAC-protected.
    #[zeroize(skip)] // Skip entries vec, as EncryptedKeyEntry doesn't derive ZeroizeOnDrop itself.
    // Sensitive String fields within EncryptedKeyEntry will self-zeroize.
    pub entries: Vec<EncryptedKeyEntry>,
    // Potentially other metadata like auto_lock_timeout if it needs to be protected.
    // For now, auto_lock_timeout is part of KeystoreConfig, not persisted directly in this struct.
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Zeroize, ZeroizeOnDrop)] // Zeroize for salt is fine
pub struct MasterKdfParams {
    pub salt: String, // hex_encoded_salt
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub output_len: usize,
    #[serde(default)] // Ensures old files lacking this field deserialize with version 0
    pub kdf_version: u8,
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

//Mminimum memory cost to 256MiB (262144 KiB) for better resistance to attacks
const MIN_M_COST: u32 = 262144;
// Minimum iterations to 8 for enhanced time-based protection
const MIN_T_COST: u32 = 8;
const MIN_P_COST: u32 = 1;
// Minimum parameter strength (time × memory product for validation)
const MIN_PARAM_STRENGTH: u64 = MIN_T_COST as u64 * MIN_M_COST as u64;
// KDF current version
const KDF_VERSION: u8 = 1;

// Rate limiting constants for memory-only protection
const MAX_FAILED_ATTEMPTS: u32 = 5;
const INITIAL_RATE_LIMIT_DELAY_MS: u64 = 1000; // 1 second
const MAX_RATE_LIMIT_DELAY_MS: u64 = 30000; // 30 seconds
const RATE_LIMIT_RESET_DURATION_MS: u64 = 300000; // 5 minutes

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            m_cost: MIN_M_COST,
            t_cost: MIN_T_COST,
            p_cost: 1,      // Default parallelism
            output_len: 32, // Minimum required output length
            // Default session management
            auto_lock_timeout: Duration::from_secs(300), // 5 minutes
        }
    }
}

// Production-grade parameter presets for different security levels
impl KeystoreConfig {
    /// Test configuration with very fast parameters for unit tests
    #[cfg(test)]
    pub fn test_fast() -> Self {
        Self {
            m_cost: 8192, // 8 MB - fast for tests
            t_cost: 2,    // 2 iterations - fast for tests
            p_cost: 1,
            output_len: 32,
            auto_lock_timeout: Duration::from_secs(300),
        }
    }

    /// Validate parameter strength to ensure adequate security
    /// Returns Ok if parameters meet minimum security requirements
    pub fn validate_strength(&self) -> Result<(), String> {
        self.validate_strength_with_mode(false)
    }

    /// Validate parameter strength with optional test mode
    /// test_mode: if true, allows lower parameters for testing
    #[cfg(test)]
    pub fn validate_strength_test_mode(&self) -> Result<(), String> {
        self.validate_strength_with_mode(true)
    }

    fn validate_strength_with_mode(&self, test_mode: bool) -> Result<(), String> {
        let (min_m, min_t, min_p, min_strength) = if test_mode {
            (8192u32, 2u32, 1u32, 8192u64 * 2u64) // Test minimums
        } else {
            (MIN_M_COST, MIN_T_COST, MIN_P_COST, MIN_PARAM_STRENGTH) // Production minimums
        };
        // Check individual minimums
        if self.m_cost < min_m {
            return Err(format!(
                "Memory cost {} is below minimum {} KiB",
                self.m_cost, min_m
            ));
        }
        if self.t_cost < min_t {
            return Err(format!(
                "Time cost {} is below minimum {}",
                self.t_cost, min_t
            ));
        }
        if self.p_cost < min_p {
            return Err(format!(
                "Parallelism {} is below minimum {}",
                self.p_cost, min_p
            ));
        }
        if self.output_len < 32 {
            return Err("Output length must be at least 32 bytes".to_string());
        }

        // Check parameter strength (time × memory product)
        let param_strength = self.t_cost as u64 * self.m_cost as u64;
        if param_strength < min_strength {
            return Err(format!(
                "Parameter strength {} is below minimum {} (time × memory product)",
                param_strength, min_strength
            ));
        }

        Ok(())
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
        if let Ok(dummy_secret_key) = k256::SecretKey::from_slice(&[1u8; 32]) {
            let dummy_signing_key = k256::ecdsa::SigningKey::from(&dummy_secret_key);
            self.0 = dummy_signing_key; // Old self.0 is dropped here, its secret zeroized.
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
        //TODO: doc warning non mlock for sensitive data on this module
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

#[derive(Debug)]
pub struct Keystore {
    file_path: PathBuf,
    master_key: Option<MasterKey>,

    // Fields populated from ProtectedKeystorePart after MAC verification
    keystore_version: Option<u8>,
    master_kdf_algo: Option<String>,
    entries: Vec<EncryptedKeyEntry>,
    verification_tag: Option<String>, // This is from ProtectedKeystorePart

    // Fields populated from AuthenticatedKeystoreEnvelope (unprotected part)
    master_kdf_params: Option<MasterKdfParams>, // From envelope
    verification_nonce: Option<String>,         // From envelope

    is_unlocked: bool,
    last_activity_at: Option<Instant>,
    config: KeystoreConfig,

    // Temporary fields for data loaded from disk, pending MAC verification in unlock()
    pending_protected_data_b64: Option<String>,
    pending_mac_b64: Option<String>,

    // Memory-only rate limiting for unlock attempts
    // These fields track failed attempts during the current process lifetime only
    failed_unlock_attempts: u32,
    last_failed_attempt: Option<Instant>,
    rate_limit_delay: Duration,
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
    fn derive_mac_key(&self, master_key_bytes: &[u8]) -> Result<hmac::Key, KeystoreError> {
        let keystore_id = self.derive_keystore_id();

        // Use keystore ID as part of the salt for proper domain separation
        let mut combined_salt = Vec::with_capacity(HKDF_SALT_FOR_MAC_KEY_DERIVATION.len() + 32);
        combined_salt.extend_from_slice(HKDF_SALT_FOR_MAC_KEY_DERIVATION);
        combined_salt.extend_from_slice(keystore_id.as_ref());

        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &combined_salt);
        let prk = salt.extract(master_key_bytes);
        let mut mac_key_bytes = Zeroizing::new([0u8; 32]); // 256-bit key for HMAC-SHA256

        // Include keystore ID in info parameter for additional domain separation
        let mut info_with_keystore = Vec::with_capacity(HKDF_INFO_MAC_KEY.len() + 32);
        info_with_keystore.extend_from_slice(HKDF_INFO_MAC_KEY);
        info_with_keystore.extend_from_slice(keystore_id.as_ref());

        prk.expand(&[&info_with_keystore], hkdf::HKDF_SHA256)
            .map_err(|_| {
                KeystoreError::InternalError("Failed to expand MAC key from PRK".to_string())
            })?
            .fill(mac_key_bytes.as_mut())
            .map_err(|_| {
                KeystoreError::InternalError("Failed to fill MAC key bytes".to_string())
            })?;

        Ok(hmac::Key::new(hmac::HMAC_SHA256, mac_key_bytes.as_ref()))
    }

    const DEFAULT_KEYSTORE_FILENAME: &'static str = "keystore_v1.json";
    const APP_DIR_NAME: &'static str = "mfm";

    // C-2 Fix: Generate keystore-specific identifier for domain separation
    fn derive_keystore_id(&self) -> Zeroizing<[u8; 32]> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Create a stable keystore identifier based on file path
        // This ensures different keystores have different domain separation
        let mut hasher = DefaultHasher::new();
        self.file_path.hash(&mut hasher);
        let path_hash = hasher.finish();

        // Use HKDF to derive a proper keystore ID from the path hash
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, b"mfm-keystore-id-derivation-v1");
        let prk = salt.extract(&path_hash.to_be_bytes());

        let mut keystore_id = Zeroizing::new([0u8; 32]);
        prk.expand(&[b"mfm-keystore-domain-separator-v1"], hkdf::HKDF_SHA256)
            .expect("HKDF expand should not fail with valid inputs")
            .fill(keystore_id.as_mut())
            .expect("HKDF fill should not fail with valid inputs");

        keystore_id
    }

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

    #[cfg(test)]
    pub fn new_with_config_test_mode(
        custom_path: Option<PathBuf>,
        config: KeystoreConfig,
    ) -> Result<Self, KeystoreError> {
        // H-1 Fix: Use test mode validation for unit tests
        config
            .validate_strength_test_mode()
            .map_err(|e| KeystoreError::FsError(format!("Invalid KDF configuration: {}", e)))?;

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

        Ok(Keystore {
            file_path,
            master_key: None,
            keystore_version: None,
            master_kdf_algo: None,
            entries: Vec::new(),
            verification_tag: None,
            master_kdf_params: None,
            verification_nonce: None,
            is_unlocked: false,
            last_activity_at: None,
            config,
            pending_protected_data_b64: None,
            pending_mac_b64: None,
            // H-2 Fix: Initialize rate limiting fields for test mode
            failed_unlock_attempts: 0,
            last_failed_attempt: None,
            rate_limit_delay: Duration::from_millis(INITIAL_RATE_LIMIT_DELAY_MS),
        })
    }

    fn new_with_config(
        custom_path: Option<PathBuf>,
        config: KeystoreConfig,
    ) -> Result<Self, KeystoreError> {
        // H-1 Fix: Use comprehensive parameter strength validation
        config
            .validate_strength()
            .map_err(|e| KeystoreError::FsError(format!("Invalid KDF configuration: {}", e)))?;

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

        Ok(Keystore {
            file_path,
            master_key: None,
            keystore_version: None,
            master_kdf_algo: None,
            entries: Vec::new(),
            verification_tag: None,   // This is from ProtectedKeystorePart
            master_kdf_params: None,  // From envelope
            verification_nonce: None, // From envelope
            is_unlocked: false,
            last_activity_at: None,
            config,
            pending_protected_data_b64: None,
            pending_mac_b64: None,
            // H-2 Fix: Initialize rate limiting fields
            failed_unlock_attempts: 0,
            last_failed_attempt: None,
            rate_limit_delay: Duration::from_millis(INITIAL_RATE_LIMIT_DELAY_MS),
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
                let master_key_val = self.derive_master_key(p, &kdf_params, "argon2id")?;
                self.master_key = Some(master_key_val); // Set the master key
                self.master_kdf_params = Some(kdf_params); // Store kdf_params
                self.master_kdf_algo = Some("argon2id".to_string());

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

    // H-2 Fix: Rate limiting methods for memory-only protection
    fn check_rate_limit(&mut self) -> Result<(), KeystoreError> {
        // Check if we need to reset the rate limiting due to timeout
        if let Some(last_failed) = self.last_failed_attempt {
            let elapsed = last_failed.elapsed();
            if elapsed.as_millis() >= RATE_LIMIT_RESET_DURATION_MS as u128 {
                // Reset period has elapsed, reset state and allow attempt
                self.reset_rate_limiting();
                return Ok(());
            }
        }

        // Check if we've exceeded the maximum attempts
        if self.failed_unlock_attempts >= MAX_FAILED_ATTEMPTS {
            return Err(KeystoreError::RateLimited {
                retry_after_ms: self.rate_limit_delay.as_millis() as u64,
                attempts_remaining: 0,
            });
        }

        // Check if we need to apply rate limiting delay
        if self.failed_unlock_attempts > 0 {
            if let Some(last_failed) = self.last_failed_attempt {
                let elapsed = last_failed.elapsed();
                if elapsed < self.rate_limit_delay {
                    let remaining_delay = self.rate_limit_delay - elapsed;
                    return Err(KeystoreError::RateLimited {
                        retry_after_ms: remaining_delay.as_millis() as u64,
                        attempts_remaining: MAX_FAILED_ATTEMPTS - self.failed_unlock_attempts,
                    });
                }
            }
        }

        Ok(())
    }

    fn record_failed_attempt_internal(&mut self) {
        self.failed_unlock_attempts += 1;
        self.last_failed_attempt = Some(Instant::now());

        // Exponential backoff with cap
        if self.failed_unlock_attempts > 1 {
            let new_delay_ms = std::cmp::min(
                self.rate_limit_delay.as_millis() as u64 * 2,
                MAX_RATE_LIMIT_DELAY_MS,
            );
            self.rate_limit_delay = Duration::from_millis(new_delay_ms);
        }
    }

    #[cfg(test)]
    pub fn record_failed_attempt(&mut self) {
        self.record_failed_attempt_internal();
    }

    fn reset_rate_limiting(&mut self) {
        self.failed_unlock_attempts = 0;
        self.last_failed_attempt = None;
        self.rate_limit_delay = Duration::from_millis(INITIAL_RATE_LIMIT_DELAY_MS);
    }

    // H-2 Fix: Test-only method to reset rate limiting for test scenarios
    #[cfg(test)]
    pub fn reset_rate_limiting_for_test(&mut self) {
        self.reset_rate_limiting();
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        // H-2 Fix: Check rate limiting before attempting unlock
        self.check_rate_limit()?;

        if self.is_unlocked {
            self.last_activity_at = Some(Instant::now());
            return Ok(());
        }

        if password.is_empty() {
            self.record_failed_attempt_internal();
            return Err(KeystoreError::InvalidPassword);
        }

        let kdf_params = self.master_kdf_params.as_ref().ok_or_else(|| {
            KeystoreError::InternalError("KDF parameters missing before unlock".to_string())
        })?;

        let algo_name = self.master_kdf_algo.as_deref().ok_or_else(|| {
            KeystoreError::InternalError("KDF algorithm name missing before unlock".to_string())
        })?;

        // 1. Derive Master Key from password
        let derived_master_key = self.derive_master_key(password, kdf_params, algo_name)?;

        // 2. Retrieve pending protected data and MAC
        let protected_data_b64 = self.pending_protected_data_b64.as_ref().ok_or_else(|| {
            KeystoreError::InternalError("Pending protected data missing before unlock".to_string())
        })?;
        let expected_mac_b64 = self.pending_mac_b64.as_ref().ok_or_else(|| {
            KeystoreError::InternalError("Pending MAC missing before unlock".to_string())
        })?;

        // 3. Decode Base64 data
        let protected_data_json_bytes = general_purpose::STANDARD
            .decode(protected_data_b64)
            .map_err(|e| {
                KeystoreError::DeserializationError(format!(
                    "Failed to decode protected data: {}",
                    e
                ))
            })?;
        let expected_mac_bytes =
            general_purpose::STANDARD
                .decode(expected_mac_b64)
                .map_err(|e| {
                    KeystoreError::DeserializationError(format!("Failed to decode MAC: {}", e))
                })?;

        // 4. Derive MAC Key from Master Key
        let mac_signing_key = self.derive_mac_key(derived_master_key.as_ref())?;

        // 5. Verify MAC
        // Serialize KDF params to include in MAC verification
        // kdf_params is already retrieved and unwrapped earlier in the function
        let kdf_params_bytes = serde_json::to_vec(kdf_params).map_err(|e| {
            KeystoreError::SerializationError(format!(
                "Failed to serialize KDF params for MAC verification: {}",
                e
            ))
        })?;

        // Concatenate KDF params bytes and protected data bytes
        let mut data_to_verify = kdf_params_bytes;
        data_to_verify.extend_from_slice(&protected_data_json_bytes);

        // The hmac::verify function takes the key, message (data_to_verify), and tag (expected_mac_bytes)
        if hmac::verify(&mac_signing_key, &data_to_verify, &expected_mac_bytes).is_err() {
            // H-2 Fix: Record failed attempt for rate limiting on MAC failure
            self.record_failed_attempt_internal();
            return Err(KeystoreError::MacVerificationFailure);
        }

        // 6. MAC verified, now deserialize ProtectedKeystorePart
        let protected_part: ProtectedKeystorePart =
            serde_json::from_slice(&protected_data_json_bytes).map_err(|e| {
                KeystoreError::DeserializationError(format!(
                    "Failed to deserialize protected part after MAC verification: {}",
                    e
                ))
            })?;

        // 7. Populate keystore fields from the verified and deserialized protected part
        self.keystore_version = Some(protected_part.version);
        self.verification_tag = protected_part.verification_tag.clone();
        self.entries = protected_part.entries.clone();

        // 8. Verify password using the (now populated) verification tag
        if let (Some(tag_str), Some(nonce_str)) = (
            self.verification_tag.as_deref(),   // Now correctly populated
            self.verification_nonce.as_deref(), // Was populated by load_from_disk
        ) {
            if !self.verify_password(&derived_master_key, tag_str, nonce_str)? {
                // H-2 Fix: Record failed attempt for rate limiting
                self.record_failed_attempt_internal();
                // If the password-derived key fails to verify the tag, it's a MAC failure.
                return Err(KeystoreError::MacVerificationFailure);
            }
        } else {
            // If verification_tag is None after loading a supposedly valid keystore, it's an issue.
            return Err(KeystoreError::MissingVerificationTag);
        }

        // 9. All checks passed, finalize unlock
        // H-2 Fix: Reset rate limiting on successful unlock
        self.reset_rate_limiting();
        self.master_key = Some(derived_master_key);
        self.is_unlocked = true;
        self.last_activity_at = Some(Instant::now());

        // 10. Clear pending data
        self.pending_protected_data_b64 = None;
        self.pending_mac_b64 = None;

        Ok(())
    }

    // C-1 Fix: Create HMAC-based password verification tag
    fn create_verification_tag(&mut self) -> Result<(), KeystoreError> {
        // Ensure parent directory exists (best effort)
        if let Some(parent_dir) = self.file_path.parent() {
            if !parent_dir.exists() {
                let _ = fs::create_dir_all(parent_dir); // Ignore error if it fails, read/write will fail later
            }
        }

        // The master_key must be set by the caller (e.g. initialize_or_load or change_password)
        // before this function is invoked. This function uses self.master_key directly.

        // Generate a cryptographically secure random nonce for HMAC verification
        let mut nonce_bytes = Zeroizing::new([0u8; 32]); // Use 32 bytes for better security
        OsRng.try_fill_bytes(nonce_bytes.as_mut()).map_err(|e| {
            KeystoreError::FsError(format!("Failed to generate verification nonce: {}", e))
        })?;

        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        // Create HMAC verification tag using the master key and random nonce
        let verification_key = hmac::Key::new(hmac::HMAC_SHA256, master_key_bytes);
        let verification_tag = hmac::sign(&verification_key, nonce_bytes.as_ref());

        // Store the verification tag and nonce
        self.verification_tag = Some(BASE64_STANDARD.encode(verification_tag.as_ref()));
        self.verification_nonce = Some(BASE64_STANDARD.encode(&nonce_bytes));

        Ok(())
    }

    // C-1 Fix: HMAC-based password verification for constant-time security
    fn verify_password(
        &self,
        master_key: &MasterKey,
        stored_tag: &str,
        stored_nonce: &str,
    ) -> Result<bool, KeystoreError> {
        // Decode the stored verification tag and nonce
        // H-3 Fix: Use zeroizing buffers for sensitive verification data
        let verification_tag = Zeroizing::new(BASE64_STANDARD.decode(stored_tag).map_err(|_| {
            KeystoreError::InvalidFormat("Failed to decode verification tag".to_string())
        })?);

        let nonce_bytes = Zeroizing::new(BASE64_STANDARD.decode(stored_nonce).map_err(|_| {
            KeystoreError::InvalidFormat("Failed to decode verification nonce".to_string())
        })?);

        // Create HMAC key from the master key
        let verification_key = hmac::Key::new(hmac::HMAC_SHA256, master_key.as_ref());

        // Perform constant-time HMAC verification
        // ring::hmac::verify is guaranteed to be constant-time
        match hmac::verify(&verification_key, nonce_bytes.as_slice(), verification_tag.as_slice()) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false), // Constant-time: always return same error type
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
        // M-1 fix: Wrap temporary buffers with Zeroizing to prevent memory leakage
        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hashed_pk = Zeroizing::new([0u8; 32]);
        keccak.finalize(hashed_pk.as_mut());
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
        let new_uuid = Uuid::new_v4();
        // A UUID is 16 bytes (128 bits). AES-GCM typically uses a 12-byte (96-bit) nonce.
        // We'll take the first 12 bytes of the UUID.
        let aes_nonce_bytes: [u8; 12] = new_uuid.as_bytes()[..12]
            .try_into()
            .expect("UUID to 12-byte nonce conversion failed, this should not happen");

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

        // M-1 fix: Wrap temporary buffers with Zeroizing to prevent memory leakage
        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hashed_pk = Zeroizing::new([0u8; 32]);
        keccak.finalize(hashed_pk.as_mut());
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
        let new_uuid = Uuid::new_v4();
        // A UUID is 16 bytes (128 bits). AES-GCM typically uses a 12-byte (96-bit) nonce.
        // We'll take the first 12 bytes of the UUID.
        let aes_nonce_bytes: [u8; 12] = new_uuid.as_bytes()[..12]
            .try_into()
            .expect("UUID to 12-byte nonce conversion failed, this should not happen");

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

        // H-3 Fix: Use zeroizing buffers for sensitive encrypted data
        let encrypted_pk_bytes = Zeroizing::new(BASE64_STANDARD.decode(&entry.encrypted_pk).map_err(|_e| {
            KeystoreError::InvalidFormat(
                "get_signer: Failed to decode entry.encrypted_pk".to_string(),
            )
        })?);

        let nonce_vec = Zeroizing::new(BASE64_STANDARD.decode(&entry.nonce).map_err(|_e| {
            KeystoreError::InvalidFormat("get_signer: Failed to decode entry.nonce".to_string())
        })?);

        let aes_nonce_bytes: [u8; 12] = nonce_vec.as_slice().try_into().map_err(|_| {
            KeystoreError::InvalidFormat(
                "get_signer: Invalid AES nonce length for entry.nonce".to_string(),
            )
        })?;

        let decrypted_pk_zeroizing_vec = self.decrypt_pk(
            encrypted_pk_bytes.as_slice(),
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

        let old_master_key = self.derive_master_key(
            old_password,
            kdf_params,
            self.master_kdf_algo.as_deref().ok_or_else(|| {
                KeystoreError::InternalError(
                    "KDF algorithm missing during password change".to_string(),
                )
            })?,
        )?;
        if let (Some(tag), Some(nonce)) =
            (self.get_verification_tag(), self.get_verification_nonce())
        {
            if !self.verify_password(&old_master_key, tag, nonce)? {
                // If old password verification fails, return MacVerificationFailure consistent with unlock()
                return Err(KeystoreError::MacVerificationFailure);
            }
        } else {
            // This should not happen if the keystore is initialized, but handle defensively.
            return Err(KeystoreError::MissingVerificationTag);
        }

        // ... (rest of the code remains the same)
        // At this point, old_password is confirmed correct.
        // Now proceed with generating new KDF params and re-encrypting.

        // 2. Generate new KDF parameters and derive new master key
        let new_kdf_params = Self::generate_kdf_params_with_config(&self.config)?;
        let new_master_key = self.derive_master_key(
            new_password,
            &new_kdf_params,
            self.master_kdf_algo.as_deref().ok_or_else(|| {
                KeystoreError::InternalError(
                    "KDF algorithm missing during password change".to_string(),
                )
            })?,
        )?;

        // 3. The old master key is currently in `old_derived_key_zeroizing`.
        // We don't need to `take` from `self.master_key` here, as we derived it fresh.
        // We will use `old_derived_key_zeroizing` for decryption.

        // 4. Decrypt all private keys using the old_derived_key_zeroizing and store them
        // Temporarily set self.master_key to old_derived_key_zeroizing for decrypt_pk calls
        let original_master_key_state = self.master_key.take(); // Save current state
        self.master_key = Some(old_master_key.clone()); // Temporarily use old key (MasterKey is Clone)ived from old_password

        let mut temp_decrypted_data: Vec<DecryptedData> = Vec::new();
        for entry_to_decrypt in self.entries.iter() {
            // H-3 Fix: Use zeroizing buffer for encrypted private key data
            let encrypted_pk_bytes = Zeroizing::new(BASE64_STANDARD
                .decode(&entry_to_decrypt.encrypted_pk)
                .map_err(|e| {
                    KeystoreError::InvalidFormat(format!(
                        "change_password: Corrupted encrypted_pk for entry {}: {}",
                        entry_to_decrypt.id, e
                    ))
                })?);

            let aes_nonce_vec = Zeroizing::new(BASE64_STANDARD
                .decode(&entry_to_decrypt.nonce)
                .map_err(|e| {
                    KeystoreError::InvalidFormat(format!(
                        "change_password: Corrupted nonce for entry {}: {}",
                        entry_to_decrypt.id, e
                    ))
                })?);

            let aes_nonce_bytes: [u8; 12] = aes_nonce_vec.as_slice().try_into().map_err(|_| {
                KeystoreError::InvalidFormat(format!(
                    "change_password: Invalid nonce length for entry {}",
                    entry_to_decrypt.id
                ))
            })?;

            let decrypted_pk_material = self.decrypt_pk(
                encrypted_pk_bytes.as_slice(),
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
            let new_uuid = Uuid::new_v4();
            // A UUID is 16 bytes (128 bits). AES-GCM typically uses a 12-byte (96-bit) nonce.
            // We'll take the first 12 bytes of the UUID.
            let new_aes_nonce_bytes: [u8; 12] = new_uuid.as_bytes()[..12]
                .try_into()
                .expect("UUID to 12-byte nonce conversion failed, this should not happen");

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
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        // When saving, master_kdf_params MUST exist.
        let _kdf_params = self.master_kdf_params.as_ref().ok_or_else(|| {
            KeystoreError::InternalError("Master KDF params missing during save".to_string())
        })?;

        let protected_part = ProtectedKeystorePart {
            version: self.keystore_version.unwrap_or(KEYSTORE_VERSION),
            verification_tag: self.verification_tag.clone(),
            entries: self.entries.clone(),
        };

        let protected_data_json_bytes = serde_json::to_vec(&protected_part).map_err(|e| {
            KeystoreError::SerializationError(format!("Failed to serialize protected part: {}", e))
        })?;

        let mac_key = self.derive_mac_key(master_key_bytes)?;

        // Serialize KDF params to include in MAC
        let kdf_params_for_mac = self.master_kdf_params.as_ref().ok_or_else(|| {
            KeystoreError::InternalError(
                "Master KDF params missing during MAC calculation in save".to_string(),
            )
        })?;
        let kdf_params_bytes = serde_json::to_vec(kdf_params_for_mac).map_err(|e| {
            KeystoreError::SerializationError(format!(
                "Failed to serialize KDF params for MAC: {}",
                e
            ))
        })?;

        // Concatenate KDF params bytes and protected data bytes
        let mut data_to_mac = kdf_params_bytes;
        data_to_mac.extend_from_slice(&protected_data_json_bytes);

        let mac_tag = hmac::sign(&mac_key, &data_to_mac);

        let protected_data_b64 = general_purpose::STANDARD.encode(&protected_data_json_bytes);
        let mac_b64 = general_purpose::STANDARD.encode(mac_tag.as_ref());

        let envelope = AuthenticatedKeystoreEnvelope {
            master_kdf_algo_name: self.master_kdf_algo.clone().ok_or_else(|| {
                KeystoreError::InternalError("Master KDF algo name missing during save".to_string())
            })?,
            master_kdf_params: self.master_kdf_params.clone().ok_or_else(|| {
                KeystoreError::InternalError("Master KDF params missing during save".to_string())
            })?,
            verification_nonce: self.verification_nonce.clone(),
            protected_data_b64,
            mac_b64,
        };

        // M-1 fix: Wrap serialized data with Zeroizing to prevent memory leakage
        let serialized_envelope =
            Zeroizing::new(serde_json::to_string_pretty(&envelope).map_err(|e| {
                KeystoreError::SerializationError(format!("Failed to serialize envelope: {}", e))
            })?);

        // Ensure parent directory exists
        if let Some(parent_dir) = self.file_path.parent() {
            Self::ensure_secure_dir(parent_dir)?;
        }

        // C-3 Fix: Robust file locking with timeout and recovery mechanisms
        self.save_with_file_locking(&envelope, &serialized_envelope)?;

        Ok(())
    }

    // C-3 Fix: Robust file locking implementation with timeout and recovery
    fn save_with_file_locking(
        &mut self,
        envelope: &AuthenticatedKeystoreEnvelope,
        serialized_envelope: &Zeroizing<String>,
    ) -> Result<(), KeystoreError> {
        use std::time::{Duration, Instant};

        const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
        const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(100);

        let lock_file_path = self.file_path.with_extension("lock");
        let start_time = Instant::now();

        // Retry loop for lock acquisition with timeout
        let _lock_file = loop {
            match fs::OpenOptions::new()
                .create_new(true) // Fail if lock file already exists (prevents races)
                .write(true)
                .open(&lock_file_path)
            {
                Ok(file) => break file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lock file exists, check if we've timed out
                    if start_time.elapsed() > LOCK_TIMEOUT {
                        return Err(KeystoreError::FsError(format!(
                            "Failed to acquire file lock within {} seconds",
                            LOCK_TIMEOUT.as_secs()
                        )));
                    }

                    // Check if lock file is stale (older than timeout)
                    if let Ok(metadata) = fs::metadata(&lock_file_path) {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(elapsed) = modified.elapsed() {
                                if elapsed > LOCK_TIMEOUT {
                                    // Remove stale lock file and retry
                                    let _ = fs::remove_file(&lock_file_path);
                                    continue;
                                }
                            }
                        }
                    }

                    // Wait before retrying
                    std::thread::sleep(LOCK_RETRY_INTERVAL);
                }
                Err(e) => {
                    return Err(KeystoreError::FsError(format!(
                        "Failed to create lock file: {}",
                        e
                    )))
                }
            }
        };

        // Ensure lock file is removed on drop (RAII)
        struct LockFileGuard(PathBuf);
        impl Drop for LockFileGuard {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let _lock_guard = LockFileGuard(lock_file_path.clone());

        // Perform atomic write with integrity verification
        let write_result = self.atomic_write_with_verification(serialized_envelope);

        // Handle write result and update state
        match write_result {
            Ok(()) => {
                // Update in-memory state only after successful write
                self.pending_protected_data_b64 = Some(envelope.protected_data_b64.clone());
                self.pending_mac_b64 = Some(envelope.mac_b64.clone());
                Ok(())
            }
            Err(e) => {
                // On failure, attempt to verify if file was actually written correctly
                if self.verify_file_integrity(envelope).unwrap_or(false) {
                    // File was written correctly despite error - update state
                    self.pending_protected_data_b64 = Some(envelope.protected_data_b64.clone());
                    self.pending_mac_b64 = Some(envelope.mac_b64.clone());
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
        // Lock file is automatically removed by LockFileGuard drop
    }

    // C-3 Fix: Atomic write with verification
    fn atomic_write_with_verification(
        &self,
        serialized_envelope: &Zeroizing<String>,
    ) -> Result<(), KeystoreError> {
        let atomic_file = AtomicFile::new(&self.file_path, OverwriteBehavior::AllowOverwrite);
        atomic_file
            .write(|f| f.write_all(serialized_envelope.as_bytes()))
            .map_err(|e| KeystoreError::FsError(format!("Atomic write failed: {:?}", e)))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.file_path, perms).map_err(|e| {
                KeystoreError::FsError(format!(
                    "Failed to set permissions on {}: {}",
                    self.file_path.display(),
                    e
                ))
            })?;
        }

        Ok(())
    }

    // C-3 Fix: Verify file integrity after write
    fn verify_file_integrity(
        &self,
        expected_envelope: &AuthenticatedKeystoreEnvelope,
    ) -> Result<bool, KeystoreError> {
        if !self.file_path.exists() {
            return Ok(false);
        }

        let file_content = fs::read(&self.file_path).map_err(|_| {
            KeystoreError::FsError("Failed to read file for verification".to_string())
        })?;

        let loaded_envelope: AuthenticatedKeystoreEnvelope = serde_json::from_slice(&file_content)
            .map_err(|_| KeystoreError::InvalidFormat("File verification failed".to_string()))?;

        // Compare critical fields
        Ok(
            loaded_envelope.protected_data_b64 == expected_envelope.protected_data_b64
                && loaded_envelope.mac_b64 == expected_envelope.mac_b64
                && loaded_envelope.master_kdf_params == expected_envelope.master_kdf_params,
        )
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        if !self.file_path.exists() {
            // If the file doesn't exist, it's not an error for load_from_disk itself.
            // initialize_or_load will handle creating a new keystore if necessary.
            return Ok(());
        }

        let file_content_bytes = fs::read(&self.file_path)
            .map_err(|e| KeystoreError::FsError(format!("Failed to read keystore file: {}", e)))?;

        // Attempt to deserialize into the new AuthenticatedKeystoreEnvelope structure
        let envelope: AuthenticatedKeystoreEnvelope = serde_json::from_slice(&file_content_bytes)
            .map_err(|e| {
                // This error indicates a corrupted file or a format incompatible with H-1.
                KeystoreError::InvalidFormat(format!(
                    "Failed to deserialize H-1 keystore envelope. File may be corrupted or in an unsupported format: {}",
                    e
                ))
            })?;

        // Validate the loaded KDF parameters against minimums.
        if envelope.master_kdf_params.m_cost < MIN_M_COST
            || envelope.master_kdf_params.t_cost < MIN_T_COST
            || envelope.master_kdf_params.p_cost < MIN_P_COST
        {
            return Err(KeystoreError::Argon2Error(
                format!(
                    "Invalid KDF parameters loaded from disk: m_cost ({}) < min ({}), or t_cost ({}) < min ({}), or p_cost ({}) < min ({}).",
                    envelope.master_kdf_params.m_cost, MIN_M_COST,
                    envelope.master_kdf_params.t_cost, MIN_T_COST,
                    envelope.master_kdf_params.p_cost, MIN_P_COST
                )
            ));
        }

        // Store the loaded KDF params and protected part for unlock to use
        self.master_kdf_params = Some(envelope.master_kdf_params.clone()); // Store for later use by unlock or if no unlock is performed
        self.verification_nonce = envelope.verification_nonce.clone(); // This was the missing piece
        self.master_kdf_algo = Some(envelope.master_kdf_algo_name.clone()); // Store the KDF algo name
        self.pending_protected_data_b64 = Some(envelope.protected_data_b64);
        self.pending_mac_b64 = Some(envelope.mac_b64);

        // IMPORTANT: Do NOT populate self.entries, self.keystore_version, etc. yet.
        // These will be populated in unlock() after MAC verification.
        // Clear any potentially stale entries from a previous state.
        self.entries.clear();
        self.keystore_version = None;
        self.verification_tag = None;

        Ok(())
    }

    fn encrypt_pk(
        &self,
        pk_bytes: &[u8],
        aes_nonce_bytes: &[u8; 12], // Renamed for clarity
        id: &Uuid,
        address: &Address,
        // Removed alias parameter as it's not used for AAD here
    ) -> Result<(Vec<u8>, [u8; 32]), KeystoreError> {
        // Returns (encrypted_pk_vec, hkdf_salt_bytes)
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        // H-3 Fix: Generate a new random HKDF salt with zeroizing protection
        let mut hkdf_salt_bytes = Zeroizing::new([0u8; 32]);
        OsRng
            .try_fill_bytes(hkdf_salt_bytes.as_mut())
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate HKDF salt: {}", e)))?;

        let entry_key = self.derive_entry_key(master_key_bytes, hkdf_salt_bytes.as_ref(), id, address)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_ref());
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

        Ok((encrypted_data, *hkdf_salt_bytes)) // H-3 Fix: Dereference zeroizing salt
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

        // H-3 Fix: Decode HKDF salt with zeroizing protection
        let hkdf_salt_bytes_vec = Zeroizing::new(BASE64_STANDARD.decode(hkdf_salt_b64).map_err(|_| {
            KeystoreError::InvalidFormat("decrypt_pk: Failed to decode HKDF salt".to_string())
        })?);
        let hkdf_salt_bytes: [u8; 32] = hkdf_salt_bytes_vec.as_slice().try_into().map_err(|_| {
            KeystoreError::InvalidFormat("decrypt_pk: Invalid HKDF salt length".to_string())
        })?;

        let entry_key = self.derive_entry_key(master_key_bytes, &hkdf_salt_bytes, id, address)?;

        let key = Key::<Aes256Gcm>::from_slice(entry_key.as_ref());
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
    // M-1 fix: Wrap AAD buffer with Zeroizing to prevent memory leakage
    fn create_aad(&self, id: &Uuid, address: &Address) -> Zeroizing<Vec<u8>> {
        let mut aad = Zeroizing::new(Vec::with_capacity(16 + 20)); // UUID (16 bytes) + Address (20 bytes)
        aad.extend_from_slice(id.as_bytes());
        aad.extend_from_slice(address.as_slice());
        aad
    }

    // C-2 Fix: Derive per-entry encryption key using HKDF with keystore domain separation
    fn derive_entry_key(
        &self,
        master_key: &[u8],
        hkdf_salt_bytes: &[u8],
        id: &Uuid,         // Added UUID parameter for domain separation
        address: &Address, // Added Address parameter for domain separation
    ) -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
        let keystore_id = self.derive_keystore_id();

        // Combine the provided salt with keystore ID for proper domain separation
        let mut combined_salt = Vec::with_capacity(hkdf_salt_bytes.len() + 32);
        combined_salt.extend_from_slice(hkdf_salt_bytes);
        combined_salt.extend_from_slice(keystore_id.as_ref());

        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &combined_salt);
        let prk = salt.extract(master_key);

        // Create a single info buffer that includes:
        // 1. A fixed version string prefix
        // 2. The keystore ID for domain separation
        // 3. The UUID of the key
        // 4. The ethereum address
        // This ensures domain separation even if salts collide between keystores
        let prefix = b"mfm-keystore-entry-key-v1";
        let mut info_buf = Zeroizing::new(Vec::with_capacity(prefix.len() + 32 + 16 + 20)); // version + keystore_id + UUID + Address
        info_buf.extend_from_slice(prefix);
        info_buf.extend_from_slice(keystore_id.as_ref());
        info_buf.extend_from_slice(id.as_bytes());
        info_buf.extend_from_slice(address.as_slice());

        // Use the combined info buffer
        let info_vec: &[&[u8]] = &[&info_buf];

        // M-3 fix: Use [u8;32] on stack instead of Vec<u8> to prevent stack copy leakage
        let mut okm = Zeroizing::new([0u8; 32]); // 32 bytes for AES-256 on stack
        prk.expand(info_vec, hkdf::HKDF_SHA256)
            .map_err(|_| KeystoreError::DerivationFailed)?
            .fill(okm.as_mut())
            .map_err(|_| KeystoreError::DerivationFailed)?;

        Ok(okm)
    }

    fn derive_master_key(
        &self,
        password: &str,
        kdf_params: &MasterKdfParams,
        algo_name: &str,
    ) -> Result<MasterKey, KeystoreError> {
        // Changed return type
        if algo_name != "argon2id" {
            return Err(KeystoreError::UnsupportedKdf(algo_name.to_string()));
        }

        // H-3 Fix: Use zeroizing buffer for password bytes
        let password_bytes = Zeroizing::new(password.as_bytes().to_vec());
        
        // H-3 Fix: Use zeroizing buffer for decoded salt
        let salt = Zeroizing::new(hex::decode(&kdf_params.salt)
            .map_err(|e| KeystoreError::Argon2Error(format!("Failed to decode salt: {}", e)))?);

        // H-3 Fix: Ensure salt has proper size
        if salt.len() < 16 {
            return Err(KeystoreError::Argon2Error("Salt too short (minimum 16 bytes)".to_string()));
        }

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
                password_bytes.as_slice(), // H-3 Fix: Use zeroizing password bytes
                salt.as_slice(),
                output_key_material.as_mut_slice(),
            )
            .map_err(|e: argon2::Error| KeystoreError::Argon2Error(e.to_string()))?;

        Ok(MasterKey::from_zeroizing(output_key_material)) // Changed return value
    }

    fn generate_kdf_params_with_config(
        config: &KeystoreConfig,
    ) -> Result<MasterKdfParams, KeystoreError> {
        // H-1 Fix: Use comprehensive parameter strength validation
        config
            .validate_strength()
            .map_err(|e| KeystoreError::FsError(format!("Invalid KDF configuration: {}", e)))?;

        // H-3 Fix: Use zeroizing buffer for salt generation
        let mut salt_bytes = Zeroizing::new([0u8; 16]); // 16-byte salt
        OsRng
            .try_fill_bytes(salt_bytes.as_mut())
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate salt: {}", e)))?;

        Ok(MasterKdfParams {
            salt: hex::encode(salt_bytes.as_ref()),
            m_cost: config.m_cost,
            t_cost: config.t_cost,
            p_cost: config.p_cost,
            output_len: config.output_len,
            kdf_version: KDF_VERSION,
        })
    }
}
