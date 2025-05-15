# Updated Plan: In-Process Keystore for `mfm_core` (Revised with Latest Dependencies)

This document outlines the revised plan for implementing a secure in-process keystore within the `mfm_core` library to manage wallet private keys for EVM transaction signing. The plan has been updated to incorporate the latest dependency recommendations, ensuring security, usability, and maintainability as of May 15, 2025.

## Core Goals for the Keystore
1. **Secure Storage:** Encrypt private keys at rest using robust cryptographic methods (Argon2 for KDF, AES-256-GCM for encryption).
2. **Key Management:** Support importing keys (from mnemonics or raw private keys), listing keys (with UUID, alias, and address), and deleting keys.
3. **Usability:** Seamlessly integrate with `mfm_cli` for unlocking and signing operations.
4. **Key Derivation:** Handle BIP-39 mnemonic phrases (with optional passphrases) to derive EVM-compatible private keys using BIP-44 paths.
5. **Resilience:** Protect against brute-force attacks on the master password and ensure data integrity.
6. **Auto-Lock:** Include an idle timeout mechanism to lock the keystore automatically for enhanced security.

## Decisions Summary
- **Keystore File Location:** Default to `~/.local/.mfm/keystore_v1.json` (platform-aware via `dirs_next`).
- **Mnemonic Storage:** Derive and store only the private key from a mnemonic, not the mnemonic itself.
- **Key Identifiers:** Use UUIDs as primary identifiers, with optional user-defined aliases.
- **BIP-39 Password:** Support optional passphrases for mnemonic-to-seed derivation.
- **KDF Choice:** Use Argon2id for deriving the master encryption key.
- **Automatic Save:** Save changes (imports/deletes) to disk immediately using atomic writes.
- **Dependencies:** Use high-standard, well-maintained dependencies for all operations.
- **Concurrency:** Ensure thread-safe access if used across multiple threads.
- **Backup/Recovery:** Support user-managed backups with clear recovery instructions.

## Recommended Approach: Password-Protected Keystore with Per-Entry Encryption
A dedicated module `mfm_core::keystore` will manage a single encrypted JSON file.

### 1. Keystore Structure and Data
- **Keystore File:** Stored as `keystore_v1.json` in `~/.local/.mfm/` (path via `dirs_next`).
  ```json
  {
    "version": "1.0.0",
    "master_kdf": "argon2id",
    "master_kdf_params": {
      "salt": "hex_encoded_salt",
      "m_cost": 65536,
      "t_cost": 3,
      "p_cost": 1,
      "output_len": 32
    },
    "entries": []
  }
  ```

- **`Keystore` Struct:** Manages file operations, master key (in memory, zeroized when locked), and CRUD operations.
- **`EncryptedKeyEntry` Struct:** Contains key metadata and encrypted private key data.

### 2. Encryption and Decryption Workflow
- **Master Key Derivation:** Use Argon2id with a unique salt and strong parameters to derive the master encryption key from the password.
- **Key Import:** Encrypt derived or imported private keys with AES-256-GCM using the master key and a unique nonce.
- **Key Usage:** Decrypt keys on demand for signing, zeroizing them immediately after use.

### 3. Mnemonic Handling
- Parse mnemonics with `bip39`, derive keys with `bip32` and `k256`, and store only the encrypted private key.

### 4. Storage
- Use JSON format with `serde`, atomic writes via `atomicwrites`, and optional file locking with `fs2`.

### 5. Proposed Module and API
```rust
pub struct Keystore {
    file_path: PathBuf,
    master_key: Option<Zeroizing<Vec<u8>>>,
    entries: Vec<EncryptedKeyEntry>,
    master_kdf_params: Option<MasterKdfParams>,
    is_unlocked: bool,
    last_activity_at: Option<Instant>,
}

impl Keystore {
    pub fn new(custom_path: Option<PathBuf>) -> Result<Self, KeystoreError>;
    pub fn initialize_or_load(&mut self, password: Option<&str>) -> Result<(), KeystoreError>;
    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError>;
    pub fn lock(&mut self);
    pub fn import_mnemonic(&mut self, alias: Option<String>, phrase: &str, passphrase: Option<&str>, path: &str) -> Result<(Uuid, Address), KeystoreError>;
    pub fn import_private_key_hex(&mut self, alias: Option<String>, pk_hex: &str) -> Result<(Uuid, Address), KeystoreError>;
    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError>;
    pub fn get_signer(&mut self, uuid: Uuid) -> Result<PrivateKeySigner, KeystoreError>;
    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError>;
    pub fn verify_signature(&self, uuid: Uuid, message_hash: H256, signature: Signature) -> Result<bool, KeystoreError>;
}
```

### 6. Security Considerations
- Use Argon2id with strong parameters.
- Encrypt keys with AES-256-GCM and unique nonces.
- Zeroize sensitive data with `zeroize`.
- Employ atomic writes and restrictive file permissions.

### 7. Updated Dependencies
- **`argon2`**: For master password KDF.
- **`aes-gcm`**: For encryption (pure Rust, audited by NCC Group).
- **`bip39`**: For mnemonic parsing.
- **`bip32`**: For BIP-32/BIP-44 derivation
- **`k256`**: For secp256k1 operations.
- **`uuid`**: For unique identifiers.
- **`dirs_next`**: For file paths.
- **`serde`, `serde_json`**: For serialization.
- **`zeroize`**: For memory security.
- **`alloy-primitives`, `alloy-signer`**: For EVM integration.
- **`hex`, `base64`**: For encoding/decoding.
- **`chrono`**: For timestamps.
- **`rand_core`**: For secure random numbers.
- **`atomicwrites`**: For atomic writes.
- **`fs2`**: For file locking.

### 8. Backup and Recovery
- Users must back up `keystore_v1.json` and their mnemonics separately.

### 9. Future Considerations
- Support for hardware wallets via an extended `KeyType` enum.
- Keystore format versioning for future upgrades.
