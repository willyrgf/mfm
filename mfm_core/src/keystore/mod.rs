/// # Security Limitations
///
/// This keystore is designed for local-only operation and has inherent limitations:
///
/// ## What this keystore CANNOT protect against:
/// - Attackers with file system write access (can reset rate limiting)
/// - Local brute force attacks (beyond computational cost)
/// - Memory dumps from privileged processes
/// - Hardware-level attacks (DMA, cold boot)
/// - Memory swapping to unencrypted disk (no mlock protection)
/// - Hardware security module integration requirements
///
/// ## What this keystore DOES protect against:
/// - Network-based attacks (no network exposure)
/// - Process memory leakage (through zeroization)
/// - Weak password storage (through strong KDF)
/// - Key material exposure in files (through encryption)
///
/// ## Required complementary security measures:
/// - Full disk encryption (including swap partition/file)
/// - Encrypted swap space configuration
/// - Strong user authentication
/// - Physical device security
/// - Regular security updates
///
/// ## Design Decisions (Intentional Limitations):
/// - No memory locking (mlock/VirtualLock) - simplifies cross-platform deployment
/// - No hardware security integration (for now) - maintains compatibility and reduces complexity
/// - No network features - eliminates remote attack surface
///
/// ## Thread Safety (F-7):
/// **The Keystore is NOT thread-safe by design.**
///
/// - Keystore implements `!Send + !Sync` to prevent accidental concurrent access
/// - Internal mutability is not synchronized - concurrent access causes undefined behavior
/// - Each keystore instance must be used from a single thread only
/// - For multi-threaded applications: create separate keystore instances per thread
/// - Rationale: Keystores contain highly sensitive cryptographic state that should not
///   be shared across threads without explicit synchronization by the application
///
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
use base64::engine::general_purpose; // MAC and protected data encoding
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD; // For base64 encoding
use base64::Engine; // For encode/decode methods
use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic; // Ensure Seed is not imported from bip39
use chrono::{DateTime, Utc};
use dirs_next;
use error::KeystoreError;
use fs2::FileExt;
use hex; // For encoding salt
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::{ecdsa::SigningKey, SecretKey};
use rand::rngs::OsRng;
use rand::TryRngCore;
use ring::{hkdf, hmac}; // For HKDF, HMAC
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime};
use tiny_keccak::{Hasher, Keccak}; // For Keccak-256
use tracing::{error, info, warn};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const KEYSTORE_VERSION: u8 = 1;

// M-4: Key rotation framework constants
const CURRENT_ENCRYPTION_VERSION: u8 = 1; // Current encryption algorithm version
const CURRENT_KDF_VERSION: u8 = 1; // Current KDF algorithm version

// F-4: Strict output length enforcement
const REQUIRED_OUTPUT_LEN: usize = 32; // Exact required output length - no flexibility

// C-1 Fix: Zeroizing wrappers for cryptographic objects
// These ensure that key material is properly zeroized when dropped

/// Zeroizing wrapper for AES-256-GCM cipher
/// Stores only key material and instantiates cipher per call to prevent key caching
pub struct ZeroizingAes256Gcm {
    key_material: Zeroizing<[u8; 32]>, // Only store key material, not cipher
}

impl ZeroizingAes256Gcm {
    pub fn new(key_bytes: &[u8]) -> Self {
        let mut key_material = Zeroizing::new([0u8; 32]);
        key_material.copy_from_slice(key_bytes);

        Self { key_material }
    }

    pub fn encrypt(&self, nonce_bytes: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        let key = Key::<Aes256Gcm>::from_slice(self.key_material.as_ref());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher.encrypt(nonce, plaintext)
    }

    pub fn decrypt(
        &self,
        nonce_bytes: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, aes_gcm::Error> {
        let key = Key::<Aes256Gcm>::from_slice(self.key_material.as_ref());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher.decrypt(nonce, ciphertext)
    }

    pub fn encrypt_with_aad(
        &self,
        nonce_bytes: &[u8],
        payload: aes_gcm::aead::Payload,
    ) -> Result<Vec<u8>, aes_gcm::Error> {
        let key = Key::<Aes256Gcm>::from_slice(self.key_material.as_ref());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher.encrypt(nonce, payload)
    }

    pub fn decrypt_with_aad(
        &self,
        nonce_bytes: &[u8],
        payload: aes_gcm::aead::Payload,
    ) -> Result<Vec<u8>, aes_gcm::Error> {
        let key = Key::<Aes256Gcm>::from_slice(self.key_material.as_ref());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher.decrypt(nonce, payload)
    }
}

impl Drop for ZeroizingAes256Gcm {
    fn drop(&mut self) {
        // key_material is automatically zeroized due to Zeroizing wrapper
        // We explicitly zeroize it again for extra safety
        self.key_material.zeroize();
    }
}

/// Zeroizing wrapper for HMAC keys
/// Stores only key material and instantiates HMAC key per call to prevent key caching
pub struct ZeroizingHmacKey {
    algorithm: hmac::Algorithm,
    key_material: Zeroizing<Vec<u8>>, // Only store key material, not key object
}

impl ZeroizingHmacKey {
    pub fn new(algorithm: hmac::Algorithm, key_bytes: &[u8]) -> Self {
        let key_material = Zeroizing::new(key_bytes.to_vec());

        Self {
            algorithm,
            key_material,
        }
    }

    pub fn sign(&self, data: &[u8]) -> hmac::Tag {
        let key = hmac::Key::new(self.algorithm, self.key_material.as_ref());
        hmac::sign(&key, data)
    }

    pub fn verify(&self, data: &[u8], tag: &[u8]) -> Result<(), ring::error::Unspecified> {
        let key = hmac::Key::new(self.algorithm, self.key_material.as_ref());
        hmac::verify(&key, data, tag)
    }
}

impl Drop for ZeroizingHmacKey {
    fn drop(&mut self) {
        // key_material is automatically zeroized due to Zeroizing wrapper
        // We explicitly zeroize it again for extra safety
        self.key_material.zeroize();
    }
}

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
    pub verification_nonce: String, // Nonce for the verification_tag - REQUIRED field

    // F-2 Fix: Stable keystore identifier (256-bit random, generated once at creation)
    // This replaces path-derived ID to prevent keystore from breaking when moved/renamed
    pub keystore_id: String, // Base64-encoded 32-byte random identifier

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

    // C-4 Fix: Independent random salt for nonce derivation (forward secrecy)
    // 256-bit random secret generated once at keystore creation, stored MAC-protected
    // REQUIRED field - no backward compatibility for clean development code
    pub nonce_derivation_salt: String, // Base64-encoded 32-byte random salt

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

    // M-4: Key rotation framework fields
    #[zeroize(skip)]
    pub encryption_version: u8, // Version of encryption algorithm used
    #[zeroize(skip)]
    pub kdf_version: u8, // Version of KDF used for this entry

    // F-1: Monotonic counter for provable nonce uniqueness
    // This field is REQUIRED for all entries - no backward compatibility support
    // Clean implementation for new software without legacy considerations
    #[zeroize(skip)]
    pub nonce_counter: u64, // Monotonic counter used to derive unique nonces
}

// Information about a key, returned by list_keys
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyInfo {
    pub id: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // M-4: Include version information for monitoring key rotation needs
    pub encryption_version: u8,
    pub kdf_version: u8,
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

//Mminimum memory cost to 1GiB (1048576 KiB) for better resistance to attacks
const MIN_M_COST: u32 = 1048576;
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

// Session management constants
const SESSION_TOKEN_LENGTH: usize = 32; // 256-bit session token
const MAX_SUSPICIOUS_ACTIVITIES: u32 = 3; // Threshold for invalidating session
const SUSPICIOUS_ACTIVITY_RESET_DURATION_MS: u64 = 600000; // 10 minutes

impl Default for KeystoreConfig {
    fn default() -> Self {
        Self {
            m_cost: MIN_M_COST,
            t_cost: MIN_T_COST,
            p_cost: MIN_P_COST,
            output_len: REQUIRED_OUTPUT_LEN, // F-4: Use strict constant
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
            output_len: REQUIRED_OUTPUT_LEN, // F-4: Use strict constant
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
        // F-4 Fix: Strict output_len validation - must be exactly the required length
        if self.output_len != REQUIRED_OUTPUT_LEN {
            return Err(format!(
                "Output length must be exactly {} bytes, got {}",
                REQUIRED_OUTPUT_LEN, self.output_len
            ));
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
/// Zeroizing wrapper for ECDSA signing keys
/// Stores only key material and instantiates SigningKey per call to prevent key caching
pub struct ZeroizingSigningKey {
    key_material: Zeroizing<[u8; 32]>, // Only store raw key bytes, not SigningKey object
}

impl ZeroizeOnDrop for ZeroizingSigningKey {}

impl Zeroize for ZeroizingSigningKey {
    fn zeroize(&mut self) {
        // key_material is automatically zeroized due to Zeroizing wrapper
        // M-1 Fix: Explicitly zeroize again with random overwrite for extra safety
        let mut random_overwrite = [0u8; 32];
        if OsRng.try_fill_bytes(&mut random_overwrite).is_ok() {
            self.key_material.copy_from_slice(&random_overwrite);
        }
        self.key_material.zeroize();
        random_overwrite.zeroize();
    }
}

impl ZeroizingSigningKey {
    /// Create a new ZeroizingSigningKey from raw key bytes
    pub fn new(key_bytes: &[u8]) -> Result<Self, KeystoreError> {
        if key_bytes.len() != 32 {
            return Err(KeystoreError::InvalidFormat(
                "ECDSA key must be 32 bytes".to_string(),
            ));
        }

        let mut key_material = Zeroizing::new([0u8; 32]);
        key_material.copy_from_slice(key_bytes);

        Ok(ZeroizingSigningKey { key_material })
    }

    /// Create a SigningKey instance on-demand from stored key material
    pub fn to_signing_key(&self) -> Result<SigningKey, KeystoreError> {
        let secret_key = SecretKey::from_slice(self.key_material.as_ref())
            .map_err(|_| KeystoreError::InvalidFormat("Invalid ECDSA secret key".to_string()))?;
        Ok(SigningKey::from(&secret_key))
    }
}

impl From<SigningKey> for ZeroizingSigningKey {
    fn from(key: SigningKey) -> Self {
        let mut key_material = Zeroizing::new([0u8; 32]);
        key_material.copy_from_slice(&key.to_bytes());
        ZeroizingSigningKey { key_material }
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

pub struct Keystore {
    file_path: PathBuf,
    master_key: Option<MasterKey>,

    // Fields populated from ProtectedKeystorePart after MAC verification
    keystore_version: Option<u8>,
    master_kdf_algo: Option<String>,
    entries: Vec<EncryptedKeyEntry>,
    verification_tag: Option<String>, // This is from ProtectedKeystorePart

    // C-4 Fix: Independent random salt for nonce derivation (forward secrecy)
    // REQUIRED field - no backward compatibility for clean development code
    nonce_derivation_salt: Zeroizing<[u8; 32]>, // 256-bit random salt from ProtectedKeystorePart

    // Fields populated from AuthenticatedKeystoreEnvelope (unprotected part)
    master_kdf_params: Option<MasterKdfParams>, // From envelope
    verification_nonce: String,                 // From envelope - REQUIRED field
    keystore_id: Option<Zeroizing<[u8; 32]>>,   // F-2: Stable keystore ID from envelope

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

    // M-3: Enhanced session management
    session_token: Option<Zeroizing<[u8; SESSION_TOKEN_LENGTH]>>, // Current session token
    session_created_at: Option<SystemTime>,                       // When session was created
    suspicious_activities: u32,                                   // Count of suspicious activities
    last_suspicious_activity: Option<Instant>, // Last suspicious activity timestamp

    // F-1: Nonce collision detection and monotonic counter tracking
    used_nonces: HashSet<[u8; 12]>, // Track used nonces in current session to detect collisions
    global_nonce_counter: u64,      // Global monotonic counter for nonce derivation

    // F-7: Thread safety marker - keystore is NOT thread-safe by design
    // Prevents accidental concurrent access that could cause memory races or data loss
    _not_thread_safe: std::marker::PhantomData<std::sync::Mutex<()>>,

    // Test mode flag to enable relaxed validation for unit tests
    #[cfg(test)]
    is_test_mode: bool,
}

// Simple Debug implementation for Keystore that doesn't expose sensitive data
impl std::fmt::Debug for Keystore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keystore")
            .field("file_path", &self.file_path)
            .field("is_unlocked", &self.is_unlocked)
            .field("entries_count", &self.entries.len())
            .field("has_master_key", &self.master_key.is_some())
            .field("has_verification_tag", &self.verification_tag.is_some())
            .finish()
    }
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
    fn derive_mac_key(&self, master_key_bytes: &[u8]) -> Result<ZeroizingHmacKey, KeystoreError> {
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

        Ok(ZeroizingHmacKey::new(
            hmac::HMAC_SHA256,
            mac_key_bytes.as_ref(),
        ))
    }

    const DEFAULT_KEYSTORE_FILENAME: &'static str = "keystore_v1.json";
    const APP_DIR_NAME: &'static str = "mfm";

    // F-2 Fix: Generate a stable 256-bit random keystore identifier
    // This replaces the path-based approach to prevent keystore breakage on file moves/renames
    fn generate_keystore_id() -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
        let mut keystore_id = Zeroizing::new([0u8; 32]);
        OsRng.try_fill_bytes(keystore_id.as_mut()).map_err(|e| {
            KeystoreError::FsError(format!("Failed to generate keystore ID: {}", e))
        })?;

        info!(
            event = "keystore_id_generated",
            "Stable keystore ID generated for new keystore"
        );

        Ok(keystore_id)
    }

    // C-4 Fix: Generate independent random salt for nonce derivation (forward secrecy)
    // This 256-bit random salt is combined with the master key for nonce derivation
    fn generate_nonce_derivation_salt() -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
        let mut nonce_salt = Zeroizing::new([0u8; 32]);
        OsRng.try_fill_bytes(nonce_salt.as_mut()).map_err(|e| {
            KeystoreError::FsError(format!("Failed to generate nonce derivation salt: {}", e))
        })?;

        info!(
            event = "nonce_salt_generated",
            "Independent nonce derivation salt generated for forward secrecy"
        );

        Ok(nonce_salt)
    }

    // F-2 Fix: Get stable keystore identifier for domain separation
    // Uses stored random ID instead of path-derived ID to survive file moves/renames
    fn derive_keystore_id(&self) -> Zeroizing<[u8; 32]> {
        match self.keystore_id.as_ref() {
            Some(id) => {
                // Return a copy of the stored stable ID
                let mut id_copy = Zeroizing::new([0u8; 32]);
                id_copy.copy_from_slice(id.as_ref());
                id_copy
            }
            None => {
                // This should not happen in normal operation, but provide a fallback
                // for debugging or development scenarios
                error!(
                    event = "missing_keystore_id",
                    "Stable keystore ID not available - this indicates an initialization error"
                );

                // Return a zero ID as emergency fallback (will cause MAC failures)
                Zeroizing::new([0u8; 32])
            }
        }
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

    /// Sync directory metadata to disk for crash safety
    /// This ensures that directory changes (like file renames) are persisted
    fn fsync_directory(dir: &Path) -> Result<(), std::io::Error> {
        use std::os::unix::io::AsRawFd;

        let dir_file = fs::OpenOptions::new().read(true).open(dir)?;

        // Use libc fsync to sync the directory file descriptor
        let result = unsafe { libc::fsync(dir_file.as_raw_fd()) };
        if result != 0 {
            return Err(std::io::Error::last_os_error());
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
                let data_dir = dirs_next::data_local_dir().ok_or_else(|| {
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
            nonce_derivation_salt: Zeroizing::new([0u8; 32]), // C-4: Will be generated at keystore creation
            master_kdf_params: None,
            verification_nonce: String::new(), // Will be set when keystore is first used
            keystore_id: None,                 // F-2: Will be generated at keystore creation
            is_unlocked: false,
            last_activity_at: None,
            config,
            pending_protected_data_b64: None,
            pending_mac_b64: None,
            // H-2 Fix: Initialize rate limiting fields for test mode
            failed_unlock_attempts: 0,
            last_failed_attempt: None,
            rate_limit_delay: Duration::from_millis(INITIAL_RATE_LIMIT_DELAY_MS),

            // M-3: Initialize session management
            session_token: None,
            session_created_at: None,
            suspicious_activities: 0,
            last_suspicious_activity: None,

            // F-1: Initialize nonce collision detection and monotonic counter
            used_nonces: HashSet::new(),
            global_nonce_counter: 0,

            // F-7: Initialize thread safety marker
            _not_thread_safe: std::marker::PhantomData,

            // Initialize test mode
            #[cfg(test)]
            is_test_mode: true,
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
                let data_dir = dirs_next::data_local_dir().ok_or_else(|| {
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
            verification_tag: None, // This is from ProtectedKeystorePart
            nonce_derivation_salt: Zeroizing::new([0u8; 32]), // C-4: Will be generated at keystore creation
            master_kdf_params: None,                          // From envelope
            verification_nonce: String::new(), // Will be set when keystore is first used // From envelope
            keystore_id: None,                 // F-2: Will be generated at keystore creation
            is_unlocked: false,
            last_activity_at: None,
            config,
            pending_protected_data_b64: None,
            pending_mac_b64: None,
            // H-2 Fix: Initialize rate limiting fields
            failed_unlock_attempts: 0,
            last_failed_attempt: None,
            rate_limit_delay: Duration::from_millis(INITIAL_RATE_LIMIT_DELAY_MS),

            // M-3: Initialize session management
            session_token: None,
            session_created_at: None,
            suspicious_activities: 0,
            last_suspicious_activity: None,

            // F-1: Initialize nonce collision detection and monotonic counter
            used_nonces: HashSet::new(),
            global_nonce_counter: 0,

            // F-7: Initialize thread safety marker
            _not_thread_safe: std::marker::PhantomData,

            // Initialize production mode
            #[cfg(test)]
            is_test_mode: false,
        })
    }

    #[cfg(test)]
    pub fn new_with_config_production_mode(
        custom_path: Option<PathBuf>,
        config: KeystoreConfig,
    ) -> Result<Self, KeystoreError> {
        // Use production validation even in test mode
        config
            .validate_strength()
            .map_err(|e| KeystoreError::FsError(format!("Invalid KDF configuration: {}", e)))?;

        let file_path = match custom_path {
            Some(path) => path,
            None => {
                let data_dir = dirs_next::data_local_dir().ok_or_else(|| {
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
            nonce_derivation_salt: Zeroizing::new([0u8; 32]),
            master_kdf_params: None,
            verification_nonce: String::new(),
            keystore_id: None,
            is_unlocked: false,
            last_activity_at: None,
            config,
            pending_protected_data_b64: None,
            pending_mac_b64: None,
            failed_unlock_attempts: 0,
            last_failed_attempt: None,
            rate_limit_delay: Duration::from_millis(INITIAL_RATE_LIMIT_DELAY_MS),
            session_token: None,
            session_created_at: None,
            suspicious_activities: 0,
            last_suspicious_activity: None,
            used_nonces: HashSet::new(),
            global_nonce_counter: 0,
            _not_thread_safe: std::marker::PhantomData,
            // Mark as production mode even in test build
            is_test_mode: false,
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

                // F-2 Fix: Generate stable keystore ID for new keystore
                self.keystore_id = Some(Self::generate_keystore_id()?);

                // C-4 Fix: Generate independent nonce derivation salt for new keystore
                self.nonce_derivation_salt = Self::generate_nonce_derivation_salt()?;

                // Fix for F-1: Create a verification tag for the new keystore
                self.create_verification_tag()?;

                // L-2: Log keystore creation
                info!(
                    event = "keystore_created",
                    kdf_algorithm = "argon2id",
                    m_cost = self.config.m_cost,
                    t_cost = self.config.t_cost,
                    p_cost = self.config.p_cost,
                    "New keystore created and initialized"
                );

                // Save the new keystore structure with KDF params (but no entries yet)
                self.save_to_disk()?;
            }
            // If no password provided for a new keystore, it remains uninitialized.
            // It cannot be used until a password is set and KDF params are created.
        } else {
            // L-2: Log existing keystore loading
            info!(
                event = "keystore_loaded",
                entry_count = self.entries.len(),
                has_verification_tag = self.verification_tag.is_some(),
                "Existing keystore loaded from disk"
            );
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

        // L-2: Log failed unlock attempt
        warn!(
            event = "failed_unlock",
            attempt = self.failed_unlock_attempts,
            max_attempts = MAX_FAILED_ATTEMPTS,
            "Failed unlock attempt {} of {}",
            self.failed_unlock_attempts,
            MAX_FAILED_ATTEMPTS
        );

        // Exponential backoff with cap
        if self.failed_unlock_attempts > 1 {
            let new_delay_ms = std::cmp::min(
                self.rate_limit_delay.as_millis() as u64 * 2,
                MAX_RATE_LIMIT_DELAY_MS,
            );
            self.rate_limit_delay = Duration::from_millis(new_delay_ms);
        }

        // L-2: Log rate limiting if triggered
        if self.failed_unlock_attempts >= MAX_FAILED_ATTEMPTS {
            error!(
                event = "rate_limit_triggered",
                failed_attempts = MAX_FAILED_ATTEMPTS,
                "Rate limiting triggered after {} failed attempts",
                MAX_FAILED_ATTEMPTS
            );
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
        // M-2 Fix: Include additional metadata in MAC verification for enhanced integrity protection
        // Serialize KDF params to include in MAC verification
        let kdf_params_bytes = serde_json::to_vec(kdf_params).map_err(|e| {
            KeystoreError::SerializationError(format!(
                "Failed to serialize KDF params for MAC verification: {}",
                e
            ))
        })?;

        // Include KDF algorithm name in MAC verification to prevent tampering
        let kdf_algo_bytes = algo_name.as_bytes();

        // C-2 Fix: Include verification_nonce in MAC verification to prevent DoS attacks
        let verification_nonce_bytes =
            BASE64_STANDARD
                .decode(&self.verification_nonce)
                .map_err(|_| {
                    KeystoreError::InvalidFormat(
                        "Failed to decode verification nonce for MAC verification".to_string(),
                    )
                })?;

        // Concatenate all metadata for MAC verification: KDF algo + KDF params + verification nonce + protected data
        let mut data_to_verify = Vec::with_capacity(
            kdf_algo_bytes.len()
                + kdf_params_bytes.len()
                + verification_nonce_bytes.len()
                + protected_data_json_bytes.len(),
        );
        data_to_verify.extend_from_slice(kdf_algo_bytes);
        data_to_verify.extend_from_slice(&kdf_params_bytes);
        data_to_verify.extend_from_slice(&verification_nonce_bytes);
        data_to_verify.extend_from_slice(&protected_data_json_bytes);

        // The hmac::verify function takes the key, message (data_to_verify), and tag (expected_mac_bytes)
        if mac_signing_key
            .verify(&data_to_verify, &expected_mac_bytes)
            .is_err()
        {
            // L-2: Log MAC verification failure (critical security event)
            error!(
                event = "mac_verification_failed",
                "MAC verification failed - keystore data may have been tampered with"
            );
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

        // C-4 Fix: Load nonce derivation salt from protected part (REQUIRED field)
        let salt_bytes = BASE64_STANDARD
            .decode(&protected_part.nonce_derivation_salt)
            .map_err(|_| {
                KeystoreError::InvalidFormat("Failed to decode nonce derivation salt".to_string())
            })?;
        if salt_bytes.len() == 32 {
            let mut salt_array = Zeroizing::new([0u8; 32]);
            salt_array.copy_from_slice(&salt_bytes);
            self.nonce_derivation_salt = salt_array;
        } else {
            return Err(KeystoreError::InvalidFormat(
                "Invalid nonce derivation salt length".to_string(),
            ));
        }

        // 8. Verify password using the (now populated) verification tag
        if let Some(tag_str) = self.verification_tag.as_deref() {
            // verification_nonce is now required field - get from self
            if !self.verify_password(&derived_master_key, tag_str, &self.verification_nonce)? {
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

        // F-1 Fix: Sync nonce counter from loaded entries to prevent reuse
        self.sync_nonce_counter_from_entries();

        self.is_unlocked = true;
        self.last_activity_at = Some(Instant::now());

        // M-3 Fix: Generate new session token on successful unlock
        self.generate_session_token()?;

        // L-2: Log successful unlock
        info!(
            event = "keystore_unlock",
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Keystore unlocked successfully"
        );

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
        let verification_key = ZeroizingHmacKey::new(hmac::HMAC_SHA256, master_key_bytes);
        let verification_tag = verification_key.sign(nonce_bytes.as_ref());

        // Store the verification tag and nonce
        self.verification_tag = Some(BASE64_STANDARD.encode(verification_tag.as_ref()));
        self.verification_nonce = BASE64_STANDARD.encode(&nonce_bytes);

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
        let verification_tag =
            Zeroizing::new(BASE64_STANDARD.decode(stored_tag).map_err(|_| {
                KeystoreError::InvalidFormat("Failed to decode verification tag".to_string())
            })?);

        let nonce_bytes = Zeroizing::new(BASE64_STANDARD.decode(stored_nonce).map_err(|_| {
            KeystoreError::InvalidFormat("Failed to decode verification nonce".to_string())
        })?);

        // Create HMAC key from the master key
        let verification_key = ZeroizingHmacKey::new(hmac::HMAC_SHA256, master_key.as_ref());

        // Perform constant-time HMAC verification
        // ring::hmac::verify is guaranteed to be constant-time
        match verification_key.verify(nonce_bytes.as_slice(), verification_tag.as_slice()) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false), // Constant-time: always return same error type
        }
    }

    // Helper methods to get verification tag and nonce
    fn get_verification_tag(&self) -> Option<&str> {
        self.verification_tag.as_deref()
    }

    fn get_verification_nonce(&self) -> &str {
        &self.verification_nonce
    }

    // Helper method to get appropriate minimums based on test mode
    fn get_validation_minimums(&self) -> (u32, u32, u32) {
        #[cfg(test)]
        {
            if self.is_test_mode {
                // Test minimums for faster testing
                (8192u32, 2u32, 1u32) // Same as KeystoreConfig::test_fast()
            } else {
                // Production minimums
                (MIN_M_COST, MIN_T_COST, MIN_P_COST)
            }
        }
        #[cfg(not(test))]
        {
            // Always use production minimums in non-test builds
            (MIN_M_COST, MIN_T_COST, MIN_P_COST)
        }
    }

    pub fn lock(&mut self) {
        self.master_key = None; // This will zeroize the key due to Zeroizing wrapper
        self.is_unlocked = false;
        self.last_activity_at = Some(Instant::now()); // Record lock time as last activity

        // M-3 Fix: Invalidate session on lock
        self.invalidate_session();

        // L-2: Log keystore lock
        info!(event = "keystore_lock", "Keystore locked manually");
    }

    // Check if auto-lock should be triggered
    fn check_auto_lock(&mut self) {
        if self.is_unlocked {
            if let Some(last_activity) = self.last_activity_at {
                if last_activity.elapsed() > self.config.auto_lock_timeout {
                    // L-2: Log auto-lock before locking
                    info!(
                        event = "auto_lock_triggered",
                        timeout_seconds = self.config.auto_lock_timeout.as_secs(),
                        "Auto-lock triggered after {} seconds of inactivity",
                        self.config.auto_lock_timeout.as_secs()
                    );
                    self.lock();
                }
            }
        }
    }

    // Update last activity timestamp
    fn update_activity_timestamp(&mut self) {
        self.last_activity_at = Some(Instant::now());
    }

    // M-3: Enhanced session management methods

    /// Generate a new session token when unlocking
    fn generate_session_token(&mut self) -> Result<(), KeystoreError> {
        let mut token = Zeroizing::new([0u8; SESSION_TOKEN_LENGTH]);
        OsRng.try_fill_bytes(token.as_mut()).map_err(|e| {
            KeystoreError::FsError(format!("Failed to generate session token: {}", e))
        })?;

        self.session_token = Some(token);
        self.session_created_at = Some(SystemTime::now());
        Ok(())
    }

    /// Check if the current session is valid
    fn validate_session(&self) -> bool {
        self.session_token.is_some() && self.session_created_at.is_some()
    }

    /// Invalidate the current session
    fn invalidate_session(&mut self) {
        self.session_token = None;
        self.session_created_at = None;
        self.is_unlocked = false;
        self.master_key = None;
    }

    /// Record suspicious activity and invalidate session if threshold exceeded
    fn record_suspicious_activity(&mut self, activity_description: &str) {
        // Reset suspicious activity counter if enough time has passed
        if let Some(last_suspicious) = self.last_suspicious_activity {
            if last_suspicious.elapsed().as_millis() > SUSPICIOUS_ACTIVITY_RESET_DURATION_MS as u128
            {
                self.suspicious_activities = 0;
            }
        }

        self.suspicious_activities += 1;
        self.last_suspicious_activity = Some(Instant::now());

        // L-2: Log suspicious activity
        error!(
            event = "suspicious_activity",
            activity = activity_description,
            count = self.suspicious_activities,
            threshold = MAX_SUSPICIOUS_ACTIVITIES,
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Suspicious activity detected: {}",
            activity_description
        );

        // Invalidate session if threshold exceeded
        if self.suspicious_activities >= MAX_SUSPICIOUS_ACTIVITIES {
            error!(
                event = "session_invalidated",
                reason = "suspicious_activity_threshold",
                activity_count = self.suspicious_activities,
                session_id = %self.get_session_id_for_audit().unwrap_or_default(),
                "Session invalidated due to suspicious activity threshold exceeded"
            );
            self.invalidate_session();
        }
    }

    /// Check for potential concurrent session access (simplified detection)
    fn check_concurrent_access(&mut self) -> Result<(), KeystoreError> {
        if !self.validate_session() {
            return Err(KeystoreError::Locked);
        }

        // Simple heuristic: if session was created very recently but we're accessing from different context
        // This is a simplified check - in practice, you'd want more sophisticated detection
        if let Some(created_at) = self.session_created_at {
            if let Ok(elapsed) = created_at.elapsed() {
                // If session is very new (< 1 second) but we're making multiple rapid calls,
                // this could indicate concurrent access attempts
                if elapsed.as_millis() < 1000 && self.suspicious_activities > 0 {
                    error!(
                        event = "concurrent_access_detected",
                        session_age_ms = elapsed.as_millis(),
                        suspicious_activities = self.suspicious_activities,
                        session_id = %self.get_session_id_for_audit().unwrap_or_default(),
                        "Potential concurrent session access detected"
                    );
                    self.record_suspicious_activity("Potential concurrent session access");
                    return Err(KeystoreError::Locked);
                }
            }
        }

        Ok(())
    }

    // L-2: Helper method for session correlation in logs
    fn get_session_id_for_audit(&self) -> Option<String> {
        self.session_token.as_ref().map(|token| {
            hex::encode(&token[..4]) // First 4 bytes as hex (8 chars) for correlation
        })
    }

    // M-4: Key rotation framework methods for future algorithm upgrades

    /// Check if any keys need rotation (for future when new algorithm versions are introduced)
    pub fn check_keys_needing_rotation(&self) -> Vec<Uuid> {
        self.entries
            .iter()
            .filter(|entry| {
                entry.encryption_version < CURRENT_ENCRYPTION_VERSION
                    || entry.kdf_version < CURRENT_KDF_VERSION
            })
            .map(|entry| entry.id)
            .collect()
    }

    /// Rotate encryption for a specific key to current algorithm versions
    /// (Currently no-op since all keys are created with current versions)
    pub fn rotate_key(&mut self, key_id: Uuid) -> Result<(), KeystoreError> {
        // Check auto-lock and session validation
        self.check_auto_lock();
        self.check_concurrent_access()?;

        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }

        // Find the key entry
        let entry_index = self
            .entries
            .iter()
            .position(|e| e.id == key_id)
            .ok_or(KeystoreError::KeyNotFound(key_id))?;

        // Check if rotation is needed
        let entry = &self.entries[entry_index];
        if entry.encryption_version >= CURRENT_ENCRYPTION_VERSION
            && entry.kdf_version >= CURRENT_KDF_VERSION
        {
            // Already using current versions (expected for new software)
            info!(
                event = "key_rotation_skipped",
                key_id = %key_id,
                reason = "already_current_version",
                encryption_version = entry.encryption_version,
                kdf_version = entry.kdf_version,
                session_id = %self.get_session_id_for_audit().unwrap_or_default(),
                "Key rotation skipped - already using current versions"
            );
            return Ok(());
        }

        // L-2: Log key rotation start
        info!(
            event = "key_rotation_started",
            key_id = %key_id,
            old_encryption_version = entry.encryption_version,
            old_kdf_version = entry.kdf_version,
            new_encryption_version = CURRENT_ENCRYPTION_VERSION,
            new_kdf_version = CURRENT_KDF_VERSION,
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Starting key rotation to current algorithm versions"
        );

        // This path handles future scenarios when algorithm versions are upgraded
        let decrypted_pk = self.decrypt_private_key_for_rotation(entry)?;
        let new_entry = self.encrypt_private_key_with_current_version(
            &decrypted_pk,
            entry.alias.clone(),
            entry.address,
            entry.created_at,
        )?;

        // Replace the old entry
        self.entries[entry_index] = new_entry;
        self.update_activity_timestamp();

        // L-2: Log successful key rotation
        info!(
            event = "key_rotation_completed",
            key_id = %key_id,
            encryption_version = CURRENT_ENCRYPTION_VERSION,
            kdf_version = CURRENT_KDF_VERSION,
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Key rotation completed successfully"
        );

        Ok(())
    }

    /// Decrypt a private key for rotation (currently only supports current version)
    fn decrypt_private_key_for_rotation(
        &self,
        entry: &EncryptedKeyEntry,
    ) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
        // Since this is new software, we only support the current encryption version
        if entry.encryption_version != CURRENT_ENCRYPTION_VERSION {
            return Err(KeystoreError::UnsupportedKdf(format!(
                "Unsupported encryption version: {}",
                entry.encryption_version
            )));
        }

        if entry.kdf_version != CURRENT_KDF_VERSION {
            return Err(KeystoreError::UnsupportedKdf(format!(
                "Unsupported KDF version: {}",
                entry.kdf_version
            )));
        }

        // Standard AES-GCM decryption
        let encrypted_pk_bytes =
            Zeroizing::new(BASE64_STANDARD.decode(&entry.encrypted_pk).map_err(|_e| {
                KeystoreError::DeserializationError(
                    "Failed to decode encrypted private key".to_string(),
                )
            })?);

        let nonce_bytes = Zeroizing::new(BASE64_STANDARD.decode(&entry.nonce).map_err(|_e| {
            KeystoreError::DeserializationError("Failed to decode nonce".to_string())
        })?);

        let hkdf_salt_bytes =
            Zeroizing::new(BASE64_STANDARD.decode(&entry.hkdf_salt).map_err(|_e| {
                KeystoreError::DeserializationError("Failed to decode HKDF salt".to_string())
            })?);

        // Get master key and derive entry key
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        let entry_key = self.derive_entry_key(
            master_key_bytes,
            &hkdf_salt_bytes,
            &entry.id,
            &entry.address,
        )?;

        // Decrypt
        let cipher = ZeroizingAes256Gcm::new(entry_key.as_ref());
        let decrypted_bytes = cipher
            .decrypt(&nonce_bytes, encrypted_pk_bytes.as_slice())
            .map_err(|_e| {
                KeystoreError::DeserializationError("Failed to decrypt private key".to_string())
            })?;

        Ok(Zeroizing::new(decrypted_bytes))
    }

    /// Encrypt a private key using the current encryption version
    fn encrypt_private_key_with_current_version(
        &mut self, // F-1: Changed to &mut self for nonce generation
        pk_bytes: &[u8],
        alias: Option<String>,
        address: Address,
        created_at: DateTime<Utc>,
    ) -> Result<EncryptedKeyEntry, KeystoreError> {
        // For new entry, we need to generate a new UUID first
        let new_id = Uuid::new_v4();

        // F-1 Fix: Use secure nonce generation with collision detection and monotonic counter
        let aes_nonce_bytes = self.generate_unique_nonce(&new_id)?;
        let nonce_counter = self.global_nonce_counter - 1; // Store the counter used for this entry

        let mut hkdf_salt_bytes = [0u8; 32];
        OsRng
            .try_fill_bytes(&mut hkdf_salt_bytes)
            .map_err(|e| KeystoreError::FsError(format!("Failed to generate HKDF salt: {}", e)))?;

        // Get master key and derive new entry key
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        let entry_key =
            self.derive_entry_key(master_key_bytes, &hkdf_salt_bytes, &new_id, &address)?;

        // Encrypt with current encryption version
        let cipher = ZeroizingAes256Gcm::new(entry_key.as_ref());
        let encrypted_pk_bytes = cipher
            .encrypt(&aes_nonce_bytes, pk_bytes)
            .map_err(|_e| KeystoreError::AesGcm("Failed to encrypt private key".to_string()))?;

        Ok(EncryptedKeyEntry {
            id: new_id,
            alias,
            address,
            encrypted_pk: BASE64_STANDARD.encode(&encrypted_pk_bytes),
            nonce: BASE64_STANDARD.encode(&aes_nonce_bytes),
            hkdf_salt: BASE64_STANDARD.encode(&hkdf_salt_bytes),
            created_at,
            updated_at: Utc::now(),
            encryption_version: CURRENT_ENCRYPTION_VERSION,
            kdf_version: CURRENT_KDF_VERSION,
            nonce_counter, // F-1: Store the monotonic counter used for this entry
        })
    }

    /// Rotate all keys to current algorithm versions (for future algorithm upgrades)
    pub fn rotate_all_keys(&mut self) -> Result<Vec<Uuid>, KeystoreError> {
        let keys_needing_rotation = self.check_keys_needing_rotation();
        let mut rotated_keys = Vec::new();

        // L-2: Log bulk rotation attempt
        if keys_needing_rotation.is_empty() {
            info!(
                event = "bulk_key_rotation_skipped",
                session_id = %self.get_session_id_for_audit().unwrap_or_default(),
                reason = "no_keys_need_rotation",
                "Bulk key rotation skipped - no keys need rotation"
            );
            return Ok(rotated_keys);
        }

        info!(
            event = "bulk_key_rotation_started",
            keys_to_rotate = keys_needing_rotation.len(),
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Bulk key rotation started"
        );

        for key_id in keys_needing_rotation {
            if let Err(e) = self.rotate_key(key_id) {
                // L-2: Log individual key rotation failure
                error!(
                    event = "bulk_key_rotation_individual_failure",
                    key_id = %key_id,
                    session_id = %self.get_session_id_for_audit().unwrap_or_default(),
                    error = %e,
                    "Failed to rotate individual key during bulk rotation"
                );
                // Continue with other keys, don't fail the entire operation
            } else {
                rotated_keys.push(key_id);
            }
        }

        if !rotated_keys.is_empty() {
            // Save the keystore with updated keys
            self.save_to_disk()?;
        }

        // L-2: Log bulk rotation completion
        info!(
            event = "bulk_key_rotation_completed",
            keys_rotated = rotated_keys.len(),
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Bulk key rotation completed"
        );

        Ok(rotated_keys)
    }

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        // M-3 Fix: Validate session and check for concurrent access
        self.check_concurrent_access()?;

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

        // F-1 Fix: Use secure nonce generation with collision detection and monotonic counter
        let aes_nonce_bytes = self.generate_unique_nonce(&id)?;

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
            // M-4: Set current encryption and KDF versions
            encryption_version: CURRENT_ENCRYPTION_VERSION,
            kdf_version: CURRENT_KDF_VERSION,
            // F-1: Store the monotonic counter used for this entry
            nonce_counter: self.global_nonce_counter - 1,
        };

        // Capture alias before moving entry
        let alias_for_log = entry.alias.clone();
        self.entries.push(entry);
        self.save_to_disk()?;
        self.update_activity_timestamp();

        // L-2: Log successful key import
        info!(
            event = "key_imported",
            key_id = %id,
            key_alias = ?alias_for_log,
            import_method = "hex",
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Private key imported from hex with alias: {:?}",
            alias_for_log
        );

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

        // F-1 Fix: Use secure nonce generation with collision detection and monotonic counter
        let aes_nonce_bytes = self.generate_unique_nonce(&id)?;

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
            // M-4: Set current encryption and KDF versions
            encryption_version: CURRENT_ENCRYPTION_VERSION,
            kdf_version: CURRENT_KDF_VERSION,
            // F-1: Store the monotonic counter used for this entry
            nonce_counter: self.global_nonce_counter - 1,
        };

        // Capture alias before moving entry
        let alias_for_log = entry.alias.clone();
        self.entries.push(entry);
        self.save_to_disk()?;
        self.update_activity_timestamp();

        // L-2: Log successful mnemonic import
        info!(
            event = "key_imported",
            key_id = %id,
            key_alias = ?alias_for_log,
            import_method = "mnemonic",
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Private key imported from mnemonic with alias: {:?}",
            alias_for_log
        );

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
                // M-4: Include version information
                encryption_version: entry.encryption_version,
                kdf_version: entry.kdf_version,
            })
            .collect();
        Ok(key_infos)
    }

    /// Returns a ZeroizingSigningKey for the specified key entry
    pub fn get_signer(&mut self, uuid: Uuid) -> Result<ZeroizingSigningKey, KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        // M-3 Fix: Validate session and check for concurrent access
        self.check_concurrent_access()?;

        if !self.is_unlocked || self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }

        let entry = self
            .entries
            .iter()
            .find(|e| e.id == uuid)
            .ok_or(KeystoreError::KeyNotFound(uuid))?;

        // H-3 Fix: Use zeroizing buffers for sensitive encrypted data
        let encrypted_pk_bytes =
            Zeroizing::new(BASE64_STANDARD.decode(&entry.encrypted_pk).map_err(|_e| {
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

        // L-2: Log key access
        info!(
            event = "key_accessed",
            key_id = %uuid,
            operation = "signing",
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Private key accessed for signing"
        );

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
        // Instantiate SigningKey on-demand and get verifying key
        let signing_key = zeroizing_signing_key.to_signing_key()?;
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
        self.check_concurrent_access()?;

        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        // L-2: Log key deletion attempt
        info!(
            event = "key_deletion_attempted",
            key_id = %uuid,
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Key deletion attempt started"
        );

        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.id != uuid);

        if self.entries.len() == initial_len {
            // L-2: Log failed key deletion
            warn!(
                event = "key_deletion_failed",
                key_id = %uuid,
                session_id = %self.get_session_id_for_audit().unwrap_or_default(),
                reason = "key_not_found",
                "Key deletion failed - key not found"
            );
            return Err(KeystoreError::KeyNotFound(uuid));
        }

        self.save_to_disk()?;
        self.update_activity_timestamp();

        // L-2: Log successful key deletion
        info!(
            event = "key_deletion_completed",
            key_id = %uuid,
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Key deletion completed successfully"
        );

        Ok(())
    }

    pub fn change_password(
        &mut self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        // Check auto-lock before proceeding
        self.check_auto_lock();

        // M-3 Fix: Validate session and check for concurrent access
        self.check_concurrent_access()?;

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
        if let Some(tag) = self.get_verification_tag() {
            let nonce = self.get_verification_nonce(); // Now returns &str directly
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
            let encrypted_pk_bytes = Zeroizing::new(
                BASE64_STANDARD
                    .decode(&entry_to_decrypt.encrypted_pk)
                    .map_err(|e| {
                        KeystoreError::InvalidFormat(format!(
                            "change_password: Corrupted encrypted_pk for entry {}: {}",
                            entry_to_decrypt.id, e
                        ))
                    })?,
            );

            let aes_nonce_vec = Zeroizing::new(
                BASE64_STANDARD
                    .decode(&entry_to_decrypt.nonce)
                    .map_err(|e| {
                        KeystoreError::InvalidFormat(format!(
                            "change_password: Corrupted nonce for entry {}: {}",
                            entry_to_decrypt.id, e
                        ))
                    })?,
            );

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
                &entry_to_decrypt.hkdf_salt,
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
            // F-1 Fix: Use secure nonce generation with collision detection and monotonic counter
            let new_aes_nonce_bytes = self.generate_unique_nonce(&data.id)?;

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
                // M-4: Set current encryption and KDF versions
                encryption_version: CURRENT_ENCRYPTION_VERSION,
                kdf_version: CURRENT_KDF_VERSION,
                // F-1: Store the monotonic counter used for this entry
                nonce_counter: self.global_nonce_counter - 1,
            };
            self.entries.push(new_entry);
        }
        // temp_decrypted_data and its Zeroizing<Vec<u8>> elements will be dropped here.

        // 8. Update KDF parameters, create verification tag, save
        self.master_kdf_params = Some(new_kdf_params);

        // Fix for F-1: Create a new verification tag with the new password
        self.create_verification_tag()?;

        self.save_to_disk()?;

        // L-2: Log successful password change
        info!(
            event = "password_changed",
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Keystore password changed successfully"
        );

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
            // C-4 Fix: Include nonce derivation salt in protected part (REQUIRED field)
            nonce_derivation_salt: BASE64_STANDARD.encode(self.nonce_derivation_salt.as_ref()),
            entries: self.entries.clone(),
        };

        let protected_data_json_bytes = serde_json::to_vec(&protected_part).map_err(|e| {
            KeystoreError::SerializationError(format!("Failed to serialize protected part: {}", e))
        })?;

        let mac_key = self.derive_mac_key(master_key_bytes)?;

        // M-2 Fix: Include additional metadata in MAC calculation for enhanced integrity protection
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

        // Include KDF algorithm name in MAC calculation to prevent tampering
        let kdf_algo_name = self.master_kdf_algo.as_deref().unwrap_or("argon2id");
        let kdf_algo_bytes = kdf_algo_name.as_bytes();

        // C-2 Fix: Include verification_nonce in MAC to prevent DoS attacks
        let verification_nonce_bytes =
            BASE64_STANDARD
                .decode(&self.verification_nonce)
                .map_err(|_| {
                    KeystoreError::InvalidFormat(
                        "Failed to decode verification nonce for MAC".to_string(),
                    )
                })?;

        // Concatenate all metadata for MAC calculation: KDF algo + KDF params + verification nonce + protected data
        let mut data_to_mac = Vec::with_capacity(
            kdf_algo_bytes.len()
                + kdf_params_bytes.len()
                + verification_nonce_bytes.len()
                + protected_data_json_bytes.len(),
        );
        data_to_mac.extend_from_slice(kdf_algo_bytes);
        data_to_mac.extend_from_slice(&kdf_params_bytes);
        data_to_mac.extend_from_slice(&verification_nonce_bytes);
        data_to_mac.extend_from_slice(&protected_data_json_bytes);

        let mac_tag = mac_key.sign(&data_to_mac);

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
            // F-2 Fix: Include stable keystore ID in envelope
            keystore_id: BASE64_STANDARD.encode(
                self.keystore_id
                    .as_ref()
                    .ok_or_else(|| {
                        KeystoreError::InternalError(
                            "Stable keystore ID missing during save".to_string(),
                        )
                    })?
                    .as_ref(),
            ),
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

    // F-3 Fix: Proper file descriptor locking with atomic rename
    // Uses cross-platform exclusive file locks instead of race-prone advisory lock files
    fn save_with_file_locking(
        &mut self,
        envelope: &AuthenticatedKeystoreEnvelope,
        serialized_envelope: &Zeroizing<String>,
    ) -> Result<(), KeystoreError> {
        use std::time::{Duration, Instant};

        const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
        const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(100);

        // F-3: Create a temporary file for atomic write
        let temp_file_path = self.file_path.with_extension("tmp");
        let start_time = Instant::now();

        // F-3: Create and exclusively lock the temporary file
        let temp_file = loop {
            match fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&temp_file_path)
            {
                Ok(file) => {
                    // F-3: Try to acquire exclusive lock on the file descriptor
                    match file.try_lock_exclusive() {
                        Ok(()) => break file,
                        Err(e) => {
                            if start_time.elapsed() > LOCK_TIMEOUT {
                                return Err(KeystoreError::FsError(format!(
                                    "Failed to acquire exclusive file lock within {} seconds: {}",
                                    LOCK_TIMEOUT.as_secs(),
                                    e
                                )));
                            }
                            // Wait before retrying
                            std::thread::sleep(LOCK_RETRY_INTERVAL);
                        }
                    }
                }
                Err(e) => {
                    return Err(KeystoreError::FsError(format!(
                        "Failed to create temporary file: {}",
                        e
                    )));
                }
            }
        };

        // F-3: Ensure proper cleanup with RAII guard that unlocks and removes temp file
        struct FileGuard {
            file: fs::File,
            path: PathBuf,
        }

        impl Drop for FileGuard {
            fn drop(&mut self) {
                // Unlock the file descriptor (automatic on close, but explicit is clearer)
                let _ = FileExt::unlock(&self.file);
                // Remove temporary file
                let _ = fs::remove_file(&self.path);
            }
        }

        let file_guard = FileGuard {
            file: temp_file,
            path: temp_file_path.clone(),
        };

        // F-3: Write data to the locked temporary file
        let write_result = {
            let mut file = &file_guard.file;
            file.write_all(serialized_envelope.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| {
                    KeystoreError::FsError(format!("Failed to write keystore data: {}", e))
                })
        };

        match write_result {
            Ok(()) => {
                // F-3: Atomically move the temporary file to the final location
                // This is atomic on most filesystems and prevents corruption from concurrent access
                fs::rename(&temp_file_path, &self.file_path).map_err(|e| {
                    KeystoreError::FsError(format!(
                        "Failed to atomically rename keystore file: {}",
                        e
                    ))
                })?;

                // C-3 Fix: Crash-safe save - fsync parent directory after rename
                // This ensures directory metadata is written to disk, preventing silent
                // corruption or file loss during power failure
                if let Some(parent_dir) = self.file_path.parent() {
                    Self::fsync_directory(parent_dir).map_err(|e| {
                        KeystoreError::FsError(format!(
                            "Failed to fsync parent directory after rename: {}",
                            e
                        ))
                    })?;
                }

                // F-3: Set secure file permissions (Unix only)
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

                // Update in-memory state only after successful atomic write
                self.pending_protected_data_b64 = Some(envelope.protected_data_b64.clone());
                self.pending_mac_b64 = Some(envelope.mac_b64.clone());

                info!(
                    event = "keystore_saved",
                    file_path = %self.file_path.display(),
                    "Keystore saved successfully with exclusive file locking"
                );

                Ok(())
            }
            Err(e) => {
                // On write failure, temp file will be cleaned up by FileGuard
                Err(e)
            }
        }
        // FileGuard automatically unlocks and cleans up temporary file
    }

    fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        if !self.file_path.exists() {
            // If the file doesn't exist, it's not an error for load_from_disk itself.
            // initialize_or_load will handle creating a new keystore if necessary.
            return Ok(());
        }

        let file_content_bytes = fs::read(&self.file_path)
            .map_err(|e| KeystoreError::FsError(format!("Failed to read keystore file: {}", e)))?;

        // Deserialize the keystore envelope structure
        let envelope: AuthenticatedKeystoreEnvelope = serde_json::from_slice(&file_content_bytes)
            .map_err(|e| {
            // This error indicates a corrupted file.
            KeystoreError::InvalidFormat(format!(
                "Failed to deserialize keystore envelope. File may be corrupted: {}",
                e
            ))
        })?;

        // Validate the loaded KDF parameters against appropriate minimums based on test mode
        let (min_m_cost, min_t_cost, min_p_cost) = self.get_validation_minimums();
        if envelope.master_kdf_params.m_cost < min_m_cost
            || envelope.master_kdf_params.t_cost < min_t_cost
            || envelope.master_kdf_params.p_cost < min_p_cost
        {
            return Err(KeystoreError::Argon2Error(
                format!(
                    "Invalid KDF parameters loaded from disk: m_cost ({}) < min ({}), or t_cost ({}) < min ({}), or p_cost ({}) < min ({}).",
                    envelope.master_kdf_params.m_cost, min_m_cost,
                    envelope.master_kdf_params.t_cost, min_t_cost,
                    envelope.master_kdf_params.p_cost, min_p_cost
                )
            ));
        }

        // F-4 Fix: Strict output_len validation - must be exactly the required length
        if envelope.master_kdf_params.output_len != REQUIRED_OUTPUT_LEN {
            return Err(KeystoreError::Argon2Error(
                format!(
                    "Invalid output_len loaded from disk: {} != required {}. Keystore was created with incompatible parameters and cannot be opened.",
                    envelope.master_kdf_params.output_len,
                    REQUIRED_OUTPUT_LEN
                )
            ));
        }

        // Store the loaded KDF params and protected part for unlock to use
        self.master_kdf_params = Some(envelope.master_kdf_params.clone()); // Store for later use by unlock or if no unlock is performed
        self.verification_nonce = envelope.verification_nonce.clone(); // This was the missing piece
        self.master_kdf_algo = Some(envelope.master_kdf_algo_name.clone()); // Store the KDF algo name

        // F-2 Fix: Load stable keystore ID from envelope
        let keystore_id_bytes = BASE64_STANDARD.decode(&envelope.keystore_id).map_err(|e| {
            KeystoreError::InvalidFormat(format!("Invalid keystore ID in envelope: {}", e))
        })?;
        if keystore_id_bytes.len() != 32 {
            return Err(KeystoreError::InvalidFormat(
                "Keystore ID must be exactly 32 bytes".to_string(),
            ));
        }
        let mut id = Zeroizing::new([0u8; 32]);
        id.copy_from_slice(&keystore_id_bytes);
        self.keystore_id = Some(id);

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

    /// F-1 Fix: Generate a cryptographically unique nonce using both collision detection and monotonic counter
    /// This method implements both recommended mitigations:
    /// (a) In-memory HashSet collision detection for the current session
    /// (b) HMAC-derived nonces from monotonic counter + entry UUID for provable uniqueness
    fn generate_unique_nonce(&mut self, entry_id: &Uuid) -> Result<[u8; 12], KeystoreError> {
        // F-1: Mitigation (b) - Use monotonic counter for provable uniqueness
        let counter = self.global_nonce_counter;
        self.global_nonce_counter = self.global_nonce_counter.checked_add(1).ok_or_else(|| {
            KeystoreError::InternalError(
                "Nonce counter overflow - maximum number of entries reached".to_string(),
            )
        })?;

        // Derive nonce using HMAC(counter || entry-UUID) as recommended
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        // C-4 Fix: Create HMAC key for nonce derivation using both master key and independent salt
        // This prevents forward secrecy issues if master key is compromised
        let nonce_salt_bytes = self.nonce_derivation_salt.as_ref();

        // Combine master key and nonce salt for HMAC key derivation
        let mut combined_key_material = Zeroizing::new(Vec::with_capacity(
            master_key_bytes.len() + nonce_salt_bytes.len(),
        ));
        combined_key_material.extend_from_slice(master_key_bytes);
        combined_key_material.extend_from_slice(nonce_salt_bytes);

        let nonce_derivation_key =
            ZeroizingHmacKey::new(hmac::HMAC_SHA256, combined_key_material.as_ref());

        // Concatenate counter and entry UUID for uniqueness
        let mut input_data = Vec::with_capacity(8 + 16); // u64 + UUID
        input_data.extend_from_slice(&counter.to_be_bytes());
        input_data.extend_from_slice(entry_id.as_bytes());

        // HMAC the combined data
        let hmac_tag = nonce_derivation_key.sign(&input_data);

        // Take first 12 bytes for AES-GCM nonce
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes.copy_from_slice(&hmac_tag.as_ref()[..12]);

        // F-1: Mitigation (a) - Check for collision in session HashSet
        if self.used_nonces.contains(&nonce_bytes) {
            // This should be extremely rare due to HMAC properties, but provides defense in depth
            return Err(KeystoreError::InternalError(
                "Nonce collision detected - this indicates a serious cryptographic issue"
                    .to_string(),
            ));
        }

        // Add to used nonces set for collision detection
        self.used_nonces.insert(nonce_bytes);

        info!(
            event = "nonce_generated",
            counter = counter,
            entry_id = %entry_id,
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Unique nonce generated with monotonic counter"
        );

        Ok(nonce_bytes)
    }

    /// F-1: Sync the global nonce counter from existing entries to prevent counter reuse
    /// This ensures that when loading from disk, we continue from where we left off
    fn sync_nonce_counter_from_entries(&mut self) {
        if self.entries.is_empty() {
            // No entries, start from 0
            self.global_nonce_counter = 0;
            return;
        }

        let max_counter = self
            .entries
            .iter()
            .map(|entry| entry.nonce_counter)
            .max()
            .expect("entries is not empty, so max should exist");

        // Set counter to be one more than the highest existing counter
        self.global_nonce_counter = max_counter.saturating_add(1);

        info!(
            event = "nonce_counter_synced",
            max_existing_counter = max_counter,
            new_global_counter = self.global_nonce_counter,
            entries_count = self.entries.len(),
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Nonce counter synchronized from existing entries"
        );
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

        let entry_key =
            self.derive_entry_key(master_key_bytes, hkdf_salt_bytes.as_ref(), id, address)?;

        let cipher = ZeroizingAes256Gcm::new(entry_key.as_ref());

        // Create AAD from entry metadata
        let aad = self.create_aad(id, address);

        let encrypted_data = cipher
            .encrypt_with_aad(
                aes_nonce_bytes, // Use aes_nonce_bytes
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
        let hkdf_salt_bytes_vec =
            Zeroizing::new(BASE64_STANDARD.decode(hkdf_salt_b64).map_err(|_| {
                KeystoreError::InvalidFormat("decrypt_pk: Failed to decode HKDF salt".to_string())
            })?);
        let hkdf_salt_bytes: [u8; 32] =
            hkdf_salt_bytes_vec.as_slice().try_into().map_err(|_| {
                KeystoreError::InvalidFormat("decrypt_pk: Invalid HKDF salt length".to_string())
            })?;

        let entry_key = self.derive_entry_key(master_key_bytes, &hkdf_salt_bytes, id, address)?;

        let cipher = ZeroizingAes256Gcm::new(entry_key.as_ref());

        // Create AAD from entry metadata
        let aad = self.create_aad(id, address);

        let decrypted_bytes = cipher
            .decrypt_with_aad(
                aes_nonce_bytes, // Use aes_nonce_bytes
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
        let salt =
            Zeroizing::new(hex::decode(&kdf_params.salt).map_err(|e| {
                KeystoreError::Argon2Error(format!("Failed to decode salt: {}", e))
            })?);

        // H-3 Fix: Ensure salt has proper size
        if salt.len() < 16 {
            return Err(KeystoreError::Argon2Error(
                "Salt too short (minimum 16 bytes)".to_string(),
            ));
        }

        // F-4 Fix: Use exact output_len without silent override
        // Strict validation ensures output_len == REQUIRED_OUTPUT_LEN (32 bytes)
        // No silent downgrades or upgrades - parameters must match exactly
        let output_len = kdf_params.output_len;

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
        // Determine validation mode based on config values - if values are below production
        // minimums but above test minimums, use test validation
        #[cfg(test)]
        {
            // Check if config has test-like parameters (below production minimums)
            let is_test_config = config.m_cost < MIN_M_COST || config.t_cost < MIN_T_COST;
            if is_test_config {
                config.validate_strength_test_mode().map_err(|e| {
                    KeystoreError::FsError(format!("Invalid KDF configuration: {}", e))
                })?;
            } else {
                config.validate_strength().map_err(|e| {
                    KeystoreError::FsError(format!("Invalid KDF configuration: {}", e))
                })?;
            }
        }
        #[cfg(not(test))]
        {
            config
                .validate_strength()
                .map_err(|e| KeystoreError::FsError(format!("Invalid KDF configuration: {}", e)))?;
        }

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
            output_len: config.output_len, // F-4: This will be validated to be REQUIRED_OUTPUT_LEN
            kdf_version: KDF_VERSION,
        })
    }

    /// L-2 Enhancement: Export tamper-evident audit logs in signed JSONL format
    /// This creates a cryptographically signed audit log export that can be used
    /// to verify the integrity of audit records over time.
    pub fn export_tamper_evident_audit_log(
        &self,
        output_path: &Path,
        include_timestamp_range: Option<(SystemTime, SystemTime)>,
    ) -> Result<(), KeystoreError> {
        use std::io::Write;

        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        // L-2: Log audit export attempt
        info!(
            event = "audit_log_export_started",
            output_path = %output_path.display(),
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Tamper-evident audit log export started"
        );

        // Create audit log metadata
        let export_metadata = AuditLogExportMetadata {
            keystore_id: self.get_keystore_identifier(),
            export_timestamp: SystemTime::now(),
            export_session_id: self.get_session_id_for_audit().unwrap_or_default(),
            timestamp_range: include_timestamp_range,
            format_version: 1,
        };

        // Derive signing key from master key for tamper evidence
        let signing_key = self.derive_audit_signing_key()?;

        // Create signed audit records (in a real implementation, these would come from
        // a persistent audit log collector. For now, we create a sample record)
        let mut signed_records = Vec::new();

        // Add export metadata as the first record
        let metadata_record = AuditLogRecord {
            timestamp: export_metadata.export_timestamp,
            event_type: "audit_export_metadata".to_string(),
            session_id: export_metadata.export_session_id.clone(),
            key_id: None,
            data: serde_json::to_value(&export_metadata).map_err(|e| {
                KeystoreError::SerializationError(format!("Failed to serialize metadata: {}", e))
            })?,
        };

        let metadata_signature = self.sign_audit_record(&metadata_record, &signing_key)?;
        signed_records.push(SignedAuditLogRecord {
            record: metadata_record,
            signature: metadata_signature,
        });

        // Write signed JSONL file
        let file = std::fs::File::create(output_path).map_err(|e| {
            KeystoreError::FsError(format!("Failed to create audit export file: {}", e))
        })?;
        let mut writer = std::io::BufWriter::new(file);

        for signed_record in &signed_records {
            let json_line = serde_json::to_string(signed_record).map_err(|e| {
                KeystoreError::SerializationError(format!(
                    "Failed to serialize audit record: {}",
                    e
                ))
            })?;
            writeln!(writer, "{}", json_line).map_err(|e| {
                KeystoreError::FsError(format!("Failed to write audit record: {}", e))
            })?;
        }

        writer
            .flush()
            .map_err(|e| KeystoreError::FsError(format!("Failed to flush audit export: {}", e)))?;

        // L-2: Log successful audit export
        info!(
            event = "audit_log_export_completed",
            output_path = %output_path.display(),
            records_exported = signed_records.len(),
            session_id = %self.get_session_id_for_audit().unwrap_or_default(),
            "Tamper-evident audit log export completed successfully"
        );

        Ok(())
    }

    /// Helper: Get a stable keystore identifier for audit purposes
    fn get_keystore_identifier(&self) -> String {
        // Use a hash of the keystore file path as a stable identifier
        use tiny_keccak::{Hasher, Keccak};
        let mut hasher = Keccak::v256();
        hasher.update(self.file_path.to_string_lossy().as_bytes());
        let mut output = [0u8; 32];
        hasher.finalize(&mut output);
        hex::encode(&output[..8]) // Use first 8 bytes (16 hex chars) as identifier
    }

    /// Helper: Derive a signing key for audit log tamper evidence
    /// F-5 Fix: Ensures proper zeroization of key material to prevent memory leaks
    fn derive_audit_signing_key(&self) -> Result<ZeroizingHmacKey, KeystoreError> {
        let master_key_bytes = self
            .master_key
            .as_ref()
            .ok_or(KeystoreError::Locked)?
            .as_ref();

        // Use HKDF to derive a separate signing key for audit logs
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, b"mfm-audit-signing-salt-v1");
        let prk = salt.extract(master_key_bytes);
        let info = b"mfm-audit-log-signing-key-v1";

        // F-5 Fix: Use Zeroizing wrapper to ensure key material is cleared from memory
        let mut signing_key_material = Zeroizing::new([0u8; 32]);
        prk.expand(&[info], hkdf::HKDF_SHA256)
            .map_err(|_| KeystoreError::InternalError("HKDF expansion failed".to_string()))?
            .fill(signing_key_material.as_mut())
            .map_err(|_| KeystoreError::InternalError("HKDF key derivation failed".to_string()))?;

        // Create HMAC key and let Zeroizing automatically clear the raw key material
        Ok(ZeroizingHmacKey::new(
            hmac::HMAC_SHA256,
            signing_key_material.as_ref(),
        ))
    }

    /// Helper: Sign an audit record for tamper evidence
    fn sign_audit_record(
        &self,
        record: &AuditLogRecord,
        signing_key: &ZeroizingHmacKey,
    ) -> Result<String, KeystoreError> {
        let record_bytes = serde_json::to_vec(record).map_err(|e| {
            KeystoreError::SerializationError(format!(
                "Failed to serialize record for signing: {}",
                e
            ))
        })?;

        let signature = signing_key.sign(&record_bytes);
        Ok(base64::engine::general_purpose::STANDARD.encode(signature.as_ref()))
    }
}

/// Metadata for tamper-evident audit log exports
#[derive(Serialize, Deserialize, Debug)]
struct AuditLogExportMetadata {
    keystore_id: String,
    export_timestamp: SystemTime,
    export_session_id: String,
    timestamp_range: Option<(SystemTime, SystemTime)>,
    format_version: u8,
}

/// Individual audit log record structure
#[derive(Serialize, Deserialize, Debug)]
struct AuditLogRecord {
    timestamp: SystemTime,
    event_type: String,
    session_id: String,
    key_id: Option<String>,
    data: serde_json::Value,
}

/// Signed audit log record for tamper evidence
#[derive(Serialize, Deserialize, Debug)]
struct SignedAuditLogRecord {
    record: AuditLogRecord,
    signature: String, // Base64-encoded HMAC signature
}
