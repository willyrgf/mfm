# Plan: In-Process Keystore for mfm_core (Revised with Community Feedback)

This document outlines the revised plan for implementing a secure in-process keystore within the `mfm_core` library to manage wallet private keys for EVM transaction signing, incorporating user feedback for enhanced security, usability, and robustness.

## Core Goals for the Keystore:

1.  **Secure Storage:** Private keys must be encrypted at rest using strong, high-standard cryptographic methods (e.g., Argon2 for KDF, AES-256-GCM for encryption).
2.  **Key Management:** Support for importing (mnemonics, raw private keys), listing (UUID, alias, address), and deleting keys.
3.  **Usability:** Easy integration with the `mfm_cli` for operations like unlocking the keystore and using keys for signing.
4.  **Key Derivation:** Proper handling of BIP-39 mnemonics (including optional passphrase) to derive EVM-compatible private keys using standard derivation paths (e.g., BIP-44).
5.  **Resilience:** Protection against common attacks on stored secrets, particularly brute-forcing the master password, and data integrity against corruption.
6.  **Auto-Lock:** Implement an idle timeout for automatic keystore locking to enhance security if the CLI is left unattended.

## Decisions Summary (Based on User Feedback):

*   **Keystore File Location:** Default to `~/.local/.mfm/keystore_v1.json` (or similar, using `dirs_next` for platform-appropriate path, versioned filename for future upgrades).
*   **Mnemonic Storage Strategy:** Option A - Derive the private key from the mnemonic and specified path, then encrypt and store only this derived private key. The original mnemonic phrase is not stored.
*   **Key Identifiers (IDs):** Use UUIDs as primary identifiers (auto-generated). Allow an optional user-defined `alias` for each key.
*   **Password for Mnemonic Seed (BIP-39):** Support the optional password for BIP-39 mnemonic-to-seed derivation.
*   **KDF Choice (Master Password):** Use Argon2 (specifically Argon2id) for deriving the master encryption key from the user's password.
*   **Automatic Save:** Changes (import/delete) should be saved to disk immediately to prevent data loss, using atomic write operations.
*   **Dependencies:** Prioritize high-standard, well-maintained, and secure dependencies for all cryptographic operations.
*   **Concurrency:** Ensure thread-safe access to keystore internals if it's intended to be used across multiple threads or async tasks.
*   **Backup/Recovery:** Facilitate user-managed backup of the keystore file and provide clear recovery instructions.
*   **Atomic Operations:** Employ atomic file writes and safe password change mechanisms to prevent data corruption.
*   **Extensibility:** Design with future support for external signers (e.g., hardware wallets) in mind, even if not implemented initially.

## Recommended Approach: Password-Protected Keystore with Per-Entry Encryption

A dedicated module `mfm_core::keystore` will manage a single encrypted file.

### 1. Keystore Structure and Data:

*   **Keystore File:** A single JSON file (e.g., `keystore_v1.json`) located by default in `~/.local/.mfm/`. The exact path will be determined using a crate like `dirs_next::data_local_dir()`. The file will have a top-level structure containing metadata for master key derivation and a list of encrypted key entries.

    ```json
    // Example keystore_v1.json structure
    {
      "version": "1.0.0", // Keystore format version
      "master_kdf": "argon2id",
      "master_kdf_params": { // Parameters for deriving the master encryption key
        "salt": "hex_encoded_salt_for_master_password_argon2_derivation",
        "m_cost": 65536,  // Memory cost (KiB)
        "t_cost": 3,      // Time cost (iterations)
        "p_cost": 1,      // Parallelism factor
        "output_len": 32  // Desired key length in bytes (e.g., for AES-256)
      },
      "entries": [
        // Array of EncryptedKeyEntry objects
      ]
    }
    ```

*   **`Keystore` struct (in `mfm_core::keystore::mod.rs`):**
    *   Manages loading/saving the keystore file.
    *   Handles the master encryption key (derived via Argon2 from user password, held in memory only when unlocked, and zeroized).
    *   Performs CRUD operations on keys.
    *   Internally, may use appropriate synchronization primitives (e.g., `std::sync::Mutex`) to ensure thread-safe access to `master_key` and `entries` if concurrent access is anticipated.
    *   May include `last_activity_timestamp: Option<std::time::Instant>` to support auto-lock functionality.

*   **`EncryptedKeyEntry` struct (Stored within the `entries` array in the JSON file):**
    *   `uuid: String` (UUID v4 string, primary identifier).
    *   `alias: Option<String>` (User-defined friendly name).
    *   `address: String` (Public Ethereum address, hex-encoded).
    *   `key_type: KeyType` (Enum: `MnemonicDerived` or `PrivateKeyImported`. Future: Consider `ExternalSigner`).
    *   `encrypted_material: String` (Base64 encoded encrypted private key bytes).
    *   `encryption_details`:
        *   `cipher: String` (e.g., "aes-256-gcm").
        *   `cipher_params`: (JSON object containing nonce/IV (hex) used for this specific entry's AES-GCM encryption).
    *   `derivation_path: Option<String>` (If `key_type` is `MnemonicDerived`, e.g., "m/44'/60'/0'/0/0").
    *   `created_at: String` (ISO 8601 timestamp).
    *   `updated_at: String` (ISO 8601 timestamp).

### 2. Encryption and Decryption Workflow:

*   **Master Password & Key Derivation:**
    1.  User provides a master password.
    2.  On keystore creation or password change:
        *   A strong, unique salt for Argon2 is generated.
        *   The master encryption key is derived using Argon2id with the password, this new salt, and strong parameters (e.g., OWASP recommendations).
        *   The salt and Argon2 parameters are stored in the `master_kdf_params` section of the keystore file.
    3.  When unlocking, the salt and KDF parameters are read from the file, and the master key is re-derived.
    4.  This master key is kept **only in memory** (wrapped in `zeroize::Zeroizing`) and zeroized when the keystore is locked or the application closes.

*   **Key Import and Encryption (New Entry):**
    1.  **Mnemonic:**
        *   Parse the mnemonic phrase (using `bip39` crate).
        *   Derive the seed using the mnemonic and optional BIP-39 passphrase (uses PBKDF2-HMAC-SHA512 internally by `bip39`).
        *   Derive the raw private key bytes using a BIP-32/BIP-44 library (e.g., `hdwallet` or `k256`/`coins_bip32`) and the specified derivation path.
    2.  **Private Key Hex:** Decode the hex string into raw private key bytes.
    3.  Generate a unique nonce (IV) for AES-GCM for this specific key entry.
    4.  Encrypt the raw private key bytes using AES-256-GCM with the **master encryption key** and the newly generated nonce.
    5.  Construct the `EncryptedKeyEntry` with all metadata (UUID, alias, encrypted data, cipher, nonce, etc.) and add it to the in-memory list of entries.
    6.  Save the entire keystore (including the updated `entries` array) to disk using an atomic write operation.

*   **Key Usage and Decryption:**
    1.  User unlocks keystore with master password, re-deriving the master encryption key as described above.
    2.  To use a key (identified by UUID), retrieve its `EncryptedKeyEntry`.
    3.  Decrypt `encrypted_material` using the master encryption key and the stored `cipher_params` (nonce) from that entry.
    4.  The decrypted private key bytes are used to create an `alloy_signer::LocalWallet`.
    5.  Decrypted private key bytes **must be zeroized** from memory immediately after use or when the `LocalWallet` is dropped if it doesn't handle zeroization internally (ensure `alloy_signer::LocalWallet` with `k256::SecretKey` does this via `ZeroizeOnDrop`).

### 3. Mnemonic Handling:

*   **Import:** Accept 12/20/24 word phrases. Support optional BIP-39 passphrase.
*   **Derivation:** Use standard Ethereum derivation paths (e.g., `m/44'/60'/0'/0/X`).
*   **Storage:** Only the derived private key is encrypted and stored. The mnemonic phrase itself is discarded after derivation for storage purposes. Users are responsible for backing up their mnemonics separately.

### 4. Storage:

*   Keystore file: `~/.local/.mfm/keystore_v1.json` (platform-aware path via `dirs_next`).
*   Format: JSON, for ease of use with `serde`.
*   **Atomic Writes:** All modifications to the keystore file (e.g., saving after import, delete, password change) must use an atomic write strategy (e.g., write to a temporary file in the same directory, `fsync`, then `rename` over the original file) to prevent data corruption if the process is killed mid-write. Crates like `atomicwrites` or manual implementation can be used.
*   **File Permissions:** Attempt to set restrictive file permissions if feasible on the OS (e.g., 0600 on Unix-like systems) when the file is created.
*   **File Locking:** Optionally, consider using OS-level file locks (e.g., via the `fs2` crate) when writing to the keystore file to prevent concurrent modification issues if multiple processes might attempt to access it simultaneously (though typical CLI usage might make this less critical than for a daemon).

### 5. Proposed Module and API (High-Level):

Module: `mfm_core::keystore`
```rust
// mfm_core/src/keystore/mod.rs
pub mod error;
mod types; // Defines KeyType, EncryptedKeyEntry (for serde), KeyInfo (for listing), KeystoreFile (top-level JSON structure)

use alloy_primitives::{Address, PrivateKey};
use alloy_signer::LocalWallet;
use error::KeystoreError;
use std::path::PathBuf;
use std::time::Instant; // For auto-lock
use uuid::Uuid;
use zeroize::Zeroizing;

// Information returned when listing keys (non-sensitive)
pub struct KeyInfo {
    pub uuid: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub key_type: types::KeyType,
    pub created_at: String, // Consider chrono::DateTime<Utc>
    pub updated_at: String, // Consider chrono::DateTime<Utc>
}

pub struct Keystore {
    file_path: PathBuf,
    master_key: Option<Zeroizing<Vec<u8>>>, // Derived Argon2 key
    // In-memory representation of key entries, loaded from file.
    // This would be Vec<types::EncryptedKeyEntry> from the deserialized KeystoreFile struct.
    entries: Vec<types::EncryptedKeyEntry>,
    // Master KDF parameters loaded from file, needed for unlock and password change.
    master_kdf_params: Option<types::MasterKdfParams>, // Contains salt, Argon2 params
    is_unlocked: bool,
    last_activity_at: Option<Instant>, // For auto-lock timeout
                                       // Potentially a Mutex if concurrent access is a design goal
                                       // key_data_lock: std::sync::Mutex<()>,
}

impl Keystore {
    // Initializes a Keystore instance, pointing to a keystore file.
    // Does not create the file or load it yet.
    pub fn new(custom_path: Option<PathBuf>) -> Result<Self, KeystoreError>;

    // Creates a new keystore file if it doesn't exist, or loads an existing one.
    // If new, it will require a password to initialize and encrypt the keystore structure.
    // If existing, it loads master KDF params and encrypted entries but remains locked.
    pub fn initialize_or_load(&mut self, password_for_creation: Option<&str>) -> Result<(), KeystoreError>;

    // Attempts to unlock the keystore with the given password.
    // Derives master_key using Argon2 and stored master_kdf_params.
    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError>;

    pub fn lock(&mut self);
    pub fn is_locked(&self) -> bool;

    // Records activity, resetting the auto-lock timer.
    pub fn record_activity(&mut self);
    // Checks if the keystore should be auto-locked based on idle_duration.
    pub fn should_auto_lock(&self, idle_duration: std::time::Duration) -> bool;

    // Changes the master password for the keystore.
    // Requires the old password to unlock.
    // Generates a new salt, derives a new master key, re-encrypts all entries,
    // and saves atomically.
    pub fn change_password(&mut self, old_password: &str, new_password: &str) -> Result<(), KeystoreError>;

    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        phrase: &str,
        mnemonic_password: Option<&str>, // BIP-39 passphrase
        derivation_path: &str,          // e.g., "m/44'/60'/0'/0/0"
    ) -> Result<(Uuid, Address), KeystoreError>; // Keystore must be unlocked

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError>; // Keystore must be unlocked

    // Lists non-sensitive info about all keys. Does not require keystore to be unlocked
    // if key metadata (address, alias) can be read without decryption.
    // The current EncryptedKeyEntry structure has address in plaintext, so this is possible.
    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError>;

    // Retrieves a signer for the given UUID. Keystore must be unlocked.
    pub fn get_signer(&mut self, uuid: Uuid) -> Result<LocalWallet, KeystoreError>; // record_activity inside

    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError>; // Keystore must be unlocked

    pub fn update_alias(&mut self, uuid: Uuid, new_alias: Option<String>) -> Result<(), KeystoreError>; // Keystore must be unlocked

    // Saves the current state of entries and master KDF params to disk (encrypted where appropriate).
    // Called internally after modifications. Uses atomic write.
    fn save_to_disk(&self) -> Result<(), KeystoreError>;
}
```

### 6. Security Considerations:

*   **KDF:** Use Argon2id with strong, recommended parameters (salt, memory cost, time cost, parallelism) stored alongside the encrypted data. The salt must be unique per keystore.
*   **Encryption:** AES-256-GCM for authenticated encryption of individual private keys. Each key encryption must use a unique, randomly generated nonce/IV.
*   **`zeroize`:** Diligently use `zeroize::Zeroizing` for the master password in memory, derived master key, and any decrypted private key material. Ensure that `alloy_signer::LocalWallet` and its underlying secret key type (e.g., `k256::SecretKey`) correctly implement `ZeroizeOnDrop`.
*   **Dependencies:** Carefully vet all cryptographic dependencies for security, maintenance, and correctness (e.g., `argon2`, `aes-gcm`, `bip39`, `hdwallet`/`k256`, `uuid`, `hex`, `base64`).
*   **Error Handling:** Robust error types to prevent information leakage.
*   **No Plaintext Storage:** Absolutely no private key material written to disk unencrypted.
*   **File Integrity and Permissions:**
    *   Employ atomic writes (write-to-temp then rename) for saving the keystore to prevent corruption.
    *   Attempt to set restrictive permissions (e.g., 0600 on Unix) on the keystore file during creation.
    *   Consider OS-level file locking if concurrent process access is a significant risk.
*   **Auto-Lock Mechanism:** Implement an idle timeout (e.g., configurable, defaulting to 5-15 minutes) in the consuming application (e.g., `mfm_cli`) using `Keystore::record_activity()` and `Keystore::should_auto_lock() / lock()`. The keystore itself should zeroize the master key when locked.
*   **Concurrency Safety:** If the `Keystore` instance is shared across threads (e.g., wrapped in an `Arc<Mutex<Keystore>>`), ensure all internal operations that modify state or access the `master_key` are properly synchronized. `zeroize` on drop of the `master_key` remains crucial.
*   **Master Password Rotation:** The `change_password` operation must be atomic. It involves decrypting all entries with the old key, deriving a new master key (with a new salt), re-encrypting all entries (each with a new nonce), and then atomically replacing the old keystore file content with the new. If it fails, the keystore must remain in its previous valid state.

### 7. New/Key Dependencies for `mfm_core/Cargo.toml`:

*   `argon2`: For KDF of the master password.
*   `aes-gcm` (or a suitable crypto suite like `ring::aead` if its API is preferred): For symmetric encryption.
*   `bip39`: For mnemonic phrase parsing and seed generation.
*   `hdwallet` or (`k256` + `tiny-keccak` + `slip10` or similar for manual BIP32/SLIP-10 derivation): For BIP-32/BIP-44 hierarchical key derivation.
*   `uuid`: With features `v4` and `serde`.
*   `dirs_next`: For determining the default keystore file location.
*   `serde`, `serde_json`: For serialization/deserialization of the keystore file.
*   `zeroize`: For clearing sensitive data from memory.
*   `alloy-primitives`, `alloy-signer`: Already present, for Address/PrivateKey types and LocalWallet.
*   `hex`, `base64`: For encoding/decoding binary data for storage in JSON.
*   `chrono` (optional, with `serde` feature): For handling timestamps robustly.
*   `rand_core` (for `CryptoRng` and `RngCore`): For generating salts and nonces securely.
*   `atomicwrites` (optional, or implement manually): For atomic file saving.
*   `fs2` (optional): For OS-level file locking.

### 8. Backup and Recovery:

*   **Keystore Backup:**
    *   The primary method of backup is for the user to securely copy the `keystore_v1.json` file itself to a safe, offline location. Documentation must strongly emphasize the sensitivity of this file and the importance of the master password.
    *   Losing the file or the master password will result in loss of access to the keys managed by this keystore.
*   **Keystore Recovery/Migration:**
    *   Recovery involves placing the backed-up `keystore_v1.json` file into the expected location (`~/.local/.mfm/` or user-configured path) on a new or restored system.
    *   The user will then need the original master password to unlock and use the keystore.
*   **Mnemonic Phrases:**
    *   Crucially, since mnemonic phrases are **not stored** (as per "Mnemonic Storage Strategy: Option A"), users are solely responsible for backing up their original mnemonic phrases (and any associated BIP-39 passphrases) securely and independently of this keystore.
    *   This separate backup of mnemonics allows for recovery of assets on other wallet software or re-importing into `mfm_core` if the keystore file is lost/corrupted and the master password is forgotten.

### 9. Future Considerations:

*   **Hardware/External Signer Support:**
    *   The `KeyType` enum within `EncryptedKeyEntry` (or a similar top-level structure for managing signers) could be extended in the future. For example:
        ```rust
        enum KeySource {
            EncryptedLocal(EncryptedKeyEntry), // Current structure
            Hardware {
                fingerprint: String, // e.g., Ledger/Trezor fingerprint
                device_type: String, // "ledger", "trezor"
                derivation_path: String,
                address: Address,
            },
            // Other external signers like cloud KMS, MPC, etc.
        }
        ```
    *   This would allow the `Keystore` to list these "keys" and its `get_signer` method to return a signer implementation that delegates signing requests to external libraries/drivers that interface with these devices/services. This plan focuses on in-process software keys first.
*   **Keystore Upgrades:** A version field in the keystore JSON allows for future format changes. Migration logic would be needed if the format evolves.
*   **Export/Import of Individual Keys:** While the entire keystore file can be backed up, functionality to securely export an individual key (encrypted with a temporary password, or in a standard format like EIP-2335 if desired) and re-import it could be added.

This revised plan incorporates the requested features and aims for a robust and secure keystore implementation.