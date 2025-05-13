// mfm_core/src/keystore/mod.rs

pub mod error;
pub mod types;

use crate::blockchain::LocalWallet; // Corrected import
use alloy_primitives::Address;
use error::KeystoreError;
use std::path::PathBuf;
use types::{EncryptedKeyEntry, KeyInfo, KeyType}; // Assuming KeyType will be in types.rs
use uuid::Uuid;
use zeroize::Zeroizing;

// Default keystore directory relative to local data dir
const DEFAULT_KEYSTORE_DIR_NAME: &str = ".mfm";
// Default keystore file name
const DEFAULT_KEYSTORE_FILE_NAME: &str = "keystore.json";

pub struct Keystore {
    file_path: PathBuf,
    master_key: Option<Zeroizing<Vec<u8>>>, // Derived Argon2 key
    entries: Vec<EncryptedKeyEntry>,        // Deserialized structs from JSON
    is_unlocked: bool,
}

impl Keystore {
    /// Initializes a Keystore instance, pointing to a keystore file.
    ///
    /// If `custom_path` is None, it defaults to a platform-specific local data directory:
    /// - Linux: `$XDG_DATA_HOME/.local/.mfm/keystore.json` or `$HOME/.local/share/.local/.mfm/keystore.json`
    /// - macOS: `$HOME/Library/Application Support/.local/.mfm/keystore.json`
    /// - Windows: `{FOLDERID_LocalAppData}/.local/.mfm/keystore.json`
    ///
    /// # Errors
    ///
    /// Returns `KeystoreError::PathError` if the path cannot be determined or created.
    pub fn new(custom_path: Option<PathBuf>) -> Result<Self, KeystoreError> {
        let path = match custom_path {
            Some(p) => p,
            None => {
                let mut base_path = dirs_next::data_local_dir().ok_or_else(|| {
                    KeystoreError::PathError("Could not determine local data directory".to_string())
                })?;
                base_path.push(DEFAULT_KEYSTORE_DIR_NAME);
                std::fs::create_dir_all(&base_path).map_err(|e| KeystoreError::IoError(e))?; // ensure .mfm dir exists
                base_path.push(DEFAULT_KEYSTORE_FILE_NAME);
                base_path
            }
        };

        Ok(Self {
            file_path: path,
            master_key: None,
            entries: Vec::new(),
            is_unlocked: false,
        })
    }

    /// Creates a new keystore file if it doesn't exist, or loads an existing one.
    /// If new, it will require a password to encrypt an empty list of entries.
    /// If existing, it loads encrypted entries but remains locked.
    ///
    /// # Errors
    ///
    /// Returns `KeystoreError` on I/O issues, serialization/deserialization problems,
    /// or if a password is required for creation but not provided.
    pub fn initialize_or_load(
        &mut self,
        password_for_creation: Option<&str>,
    ) -> Result<(), KeystoreError> {
        // todo: ai: implement file loading/creation logic
        // if file exists, load and deserialize (entries remain encrypted until unlock)
        // if file does not exist and password_for_creation is Some, create new with empty entries, encrypt and save
        // if file does not exist and password_for_creation is None, error
        Err(KeystoreError::NotImplemented(
            "initialize_or_load".to_string(),
        ))
    }

    /// Attempts to unlock the keystore with the given password.
    /// Derives master_key using Argon2 and decrypts entries for use.
    ///
    /// # Errors
    ///
    /// Returns `KeystoreError` if the password is incorrect, decryption fails, or other issues occur.
    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        // todo: ai: implement unlock logic
        // 1. check if already unlocked
        // 2. derive master key using argon2 from password and stored salt (from first entry or keystore metadata)
        // 3. attempt to decrypt a piece of data (e.g., first entry's material or a dedicated check field)
        // 4. if successful, store master_key, set is_unlocked = true
        // 5. (optional) decrypt all entries' materials and store them in a temporary in-memory structure if needed, or decrypt on demand
        Err(KeystoreError::NotImplemented("unlock".to_string()))
    }

    pub fn lock(&mut self) {
        self.master_key = None;
        self.is_unlocked = false;
        // ensure any temporarily decrypted private keys are zeroized
    }

    pub fn is_locked(&self) -> bool {
        !self.is_unlocked
    }

    /// Changes the master password for the keystore.
    /// Requires the old password to unlock, then re-encrypts all entries with new master key derived from new password.
    ///
    /// # Errors
    ///
    /// Returns `KeystoreError` if the old password is incorrect or re-encryption fails.
    pub fn change_password(
        &mut self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        // todo: ai: implement change_password logic
        // 1. unlock with old_password
        // 2. derive new master_key with new_password (generate new salt for kdf)
        // 3. re-encrypt all entries' materials with the new master_key and new cipher_params (new nonces)
        // 4. update kdf_params for all entries (or keystore metadata)
        // 5. save to disk
        // 6. update self.master_key to the new one
        Err(KeystoreError::NotImplemented("change_password".to_string()))
    }

    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        phrase: &str,
        mnemonic_password: Option<&str>, // BIP-39 passphrase
        derivation_path: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        // todo: ai: implement import_mnemonic
        // 1. ensure keystore is unlocked
        // 2. parse mnemonic, derive seed (with optional bip39 password)
        // 3. derive private key using derivation_path
        // 4. encrypt private key with master_key
        // 5. create EncryptedKeyEntry, add to self.entries
        // 6. save_to_disk
        Err(KeystoreError::NotImplemented("import_mnemonic".to_string()))
    }

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        // todo: ai: implement import_private_key_hex
        // 1. ensure keystore is unlocked
        // 2. decode hex to private key bytes
        // 3. encrypt private key with master_key
        // 4. create EncryptedKeyEntry, add to self.entries
        // 5. save_to_disk
        Err(KeystoreError::NotImplemented(
            "import_private_key_hex".to_string(),
        ))
    }

    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        // todo: ai: implement list_keys
        // iterate self.entries and map to KeyInfo (no decryption needed)
        Err(KeystoreError::NotImplemented("list_keys".to_string()))
    }

    /// Retrieves a signer for the given UUID. Keystore must be unlocked.
    ///
    /// # Errors
    ///
    /// Returns `KeystoreError` if the keystore is locked, key not found, or decryption fails.
    pub fn get_signer(&self, uuid: Uuid) -> Result<LocalWallet, KeystoreError> {
        // todo: ai: implement get_signer
        // 1. ensure keystore is unlocked
        // 2. find entry by uuid
        // 3. decrypt its material using self.master_key
        // 4. create LocalWallet from decrypted private key
        // 5. ensure decrypted private key is zeroized after LocalWallet creation
        Err(KeystoreError::NotImplemented("get_signer".to_string()))
    }

    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError> {
        // todo: ai: implement delete_key
        // 1. ensure keystore is unlocked (or allow deletion while locked if password provided for confirmation?)
        // 2. remove entry from self.entries
        // 3. save_to_disk
        Err(KeystoreError::NotImplemented("delete_key".to_string()))
    }

    pub fn update_alias(
        &mut self,
        uuid: Uuid,
        new_alias: Option<String>,
    ) -> Result<(), KeystoreError> {
        // todo: ai: implement update_alias
        // 1. ensure keystore is unlocked (or allow alias update while locked?)
        // 2. find entry and update alias
        // 3. save_to_disk
        Err(KeystoreError::NotImplemented("update_alias".to_string()))
    }

    /// Saves the current state of entries to disk (encrypted).
    /// This involves serializing `self.entries` (which contains `EncryptedKeyEntry` structs) to JSON
    /// and writing to `self.file_path`.
    /// The `EncryptedKeyEntry` itself stores the KDF parameters used for the master key,
    /// and the cipher parameters for its own `encrypted_material`.
    fn save_to_disk(&self) -> Result<(), KeystoreError> {
        // todo: ai: implement save_to_disk
        // 1. serialize self.entries to json
        // 2. write to self.file_path
        Err(KeystoreError::NotImplemented("save_to_disk".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_keystore_new_default_path() {
        // this test might create dirs in user's actual local data dir,
        // which is not ideal for automated tests.
        // consider how to mock dirs_next or use a test-specific base path.
        // for now, just ensure it doesn't panic.
        let ks = Keystore::new(None);
        assert!(ks.is_ok());
    }

    #[test]
    fn test_keystore_new_custom_path() {
        let dir = tempdir().unwrap();
        let custom_file_path = dir.path().join("my_keystore.json");
        let ks = Keystore::new(Some(custom_file_path.clone()));
        assert!(ks.is_ok());
        assert_eq!(ks.unwrap().file_path, custom_file_path);
    }

    // todo: ai: add more tests for:
    // - initialize_or_load (new and existing)
    // - unlock (correct and incorrect password)
    // - lock
    // - change_password
    // - import_mnemonic
    // - import_private_key_hex
    // - list_keys
    // - get_signer
    // - delete_key
    // - update_alias
    // - persistence (save and load consistency)
}
