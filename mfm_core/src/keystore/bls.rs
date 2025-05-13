// mfm_core/src/keystore/bls.rs

use anyhow::{Context, Result};
use eth2_keystore::{Keypair, Keystore, KeystoreBuilder, PublicKeyBytes, SecretKeyBytes}; // reverted to eth2_keystore
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zeroize::Zeroize;

// default keystore directory relative to home: .local/.mfm/keystore/bls
const DEFAULT_KEYSTORE_SUBDIR: &str = ".local/.mfm/keystore/bls";

#[derive(Debug, thiserror::Error)]
pub enum BlsKeystoreError {
    #[error("keystore directory not found or inaccessible: {0}")]
    DirectoryError(String),
    #[error("failed to create keystore directory: {0}")]
    DirectoryCreationError(String),
    #[error("eth2_keystore error: {0}")] // reverted to eth2_keystore
    Eth2KeystoreError(#[from] eth2_keystore::Error),
    #[error("i/o error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("failed to serialize/deserialize json: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
    #[error("key not found for public key: {0}")]
    KeyNotFound(String),
    #[error("password is required")]
    PasswordRequired,
    #[error("invalid password")]
    InvalidPassword,
    #[error("failed to decode hex string: {0}")]
    HexError(#[from] hex::FromHexError),
    #[error("could not determine home directory")]
    HomeDirNotFound,
    #[error("bls operation failed: {0}")]
    BlsOperationError(String), // for more generic bls errors if needed
    #[error("failed to import raw key: {0}")]
    RawKeyImportError(String),
}

pub struct BlsKeystoreManager {
    keystore_dir: PathBuf,
}

impl BlsKeystoreManager {
    /// creates a new bls keystore manager.
    /// if `keystore_base_dir` is none, it uses the default path `home_dir/.local/.mfm/keystore/bls`.
    /// if `keystore_base_dir` is some, it appends `/bls` to it.
    pub fn new(keystore_base_dir: Option<PathBuf>) -> Result<Self, BlsKeystoreError> {
        let dir = match keystore_base_dir {
            Some(base) => base.join("bls"),
            None => {
                let home = dirs_next::home_dir().ok_or(BlsKeystoreError::HomeDirNotFound)?;
                home.join(DEFAULT_KEYSTORE_SUBDIR)
            }
        };

        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| BlsKeystoreError::DirectoryCreationError(e.to_string()))?;
        }
        Ok(Self { keystore_dir: dir })
    }

    /// returns the path to the keystore directory.
    pub fn keystore_dir(&self) -> &Path {
        &self.keystore_dir
    }

    /// creates a new bls12-381 keypair, encrypts it according to eip-2335,
    /// and saves it to a new keystore file in the keystore directory.
    /// returns the hex-encoded public key.
    ///
    /// the hd path for a newly generated key (not derived from a master seed explicitly here)
    /// will be set to none or a default by the eth2_keystore library.
    pub async fn create_new_key(&self, password: &str) -> Result<String, BlsKeystoreError> {
        if password.is_empty() {
            return Err(BlsKeystoreError::PasswordRequired);
        }

        // 1. generate a new random bls keypair
        // eth2_keystore uses rand::thread_rng() internally for this.
        let keypair = Keypair::random(&mut rand::thread_rng());
        let keypair_ref = (&keypair).into(); // convert to KeypairRef for the builder

        // 2. build the keystore object
        // the description can be empty or a default.
        // hd_path_string_opt is set to none, as this is a new, non-derived key.
        // the eth2_keystore library will handle the default path or empty path in the json.
        let keystore = KeystoreBuilder::new(
            keypair_ref,
            password.as_bytes(),
            "mfm generated bls key", // description
            None,                    // hd_path_string_opt
        )?
        .build()?; // this performs the kdf and encryption

        // 3. save the keystore to a file
        // the filename is typically <uuid>.json or UTC--<timestamp>--<uuid>.json
        // we'll use <uuid>.json for simplicity.
        let file_name = format!("{}.json", keystore.uuid());
        let file_path = self.keystore_dir.join(file_name);

        // note: keystore.to_json_file uses std::fs, which is blocking.
        // if this needs to be truly async, wrap in spawn_blocking.
        keystore.to_json_file(&file_path)?;

        // 4. return the hex-encoded public key
        Ok(hex::encode(keystore.pubkey().compress().as_bytes()))
    }

    /// imports a raw bls12-381 private key (hex-encoded), encrypts it,
    /// and saves it as an eip-2335 keystore file.
    /// returns the hex-encoded public key.
    ///
    /// the `path` field in the keystore json will be set to none or a default
    /// by `eth2_keystore` as this key is not derived from a master seed through this function.
    pub async fn import_raw_private_key(
        &self,
        secret_key_hex: &str,
        password: &str,
    ) -> Result<String, BlsKeystoreError> {
        if password.is_empty() {
            return Err(BlsKeystoreError::PasswordRequired);
        }

        // 1. decode hex and deserialize secret key
        let sk_bytes_vec = hex::decode(secret_key_hex)?;
        // secretkeybytes expects a [u8; 32]
        let sk_bytes_array: [u8; 32] = sk_bytes_vec.as_slice().try_into().map_err(|_| {
            BlsKeystoreError::RawKeyImportError(
                "secret key hex must decode to 32 bytes".to_string(),
            )
        })?;
        let sk = SecretKeyBytes::from(sk_bytes_array);

        // 2. create a keypair from the secret key.
        // the public key is derived from the secret key.
        let keypair = Keypair::from_secret_key_bytes(&sk);
        let keypair_ref = (&keypair).into();

        // 3. build the keystore object
        let keystore = KeystoreBuilder::new(
            keypair_ref,
            password.as_bytes(),
            "mfm imported bls key", // description
            None,                   // hd_path_string_opt (no derivation path for raw import)
        )?
        .build()?;

        // 4. save the keystore to a file
        let file_name = format!("{}.json", keystore.uuid());
        let file_path = self.keystore_dir.join(file_name);
        keystore.to_json_file(&file_path)?;

        // 5. return the hex-encoded public key
        Ok(hex::encode(keystore.pubkey().compress().as_bytes()))
    }

    // placeholder for list_keys
    pub async fn list_keys(&self) -> Result<Vec<String>, BlsKeystoreError> {
        let mut pubkeys = Vec::new();
        for entry in fs::read_dir(&self.keystore_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                // attempt to parse the keystore to get the public key
                // eth2_keystore::Keystore::from_json_file(&path) might be too slow if it decrypts
                // ideally, we just parse json to get pubkey field.
                let content = fs::read_to_string(&path)?;
                let json_value: serde_json::Value = serde_json::from_str(&content)?;
                if let Some(pk_hex) = json_value.get("pubkey").and_then(|v| v.as_str()) {
                    pubkeys.push(pk_hex.to_string());
                }
            }
        }
        Ok(pubkeys)
    }

    /// retrieves and decrypts the bls secret key from the keystore file
    /// associated with the given public key.
    pub async fn get_secret_key(
        &self,
        pubkey_hex: &str,
        password: &str,
    ) -> Result<SecretKeyBytes, BlsKeystoreError> {
        if password.is_empty() {
            return Err(BlsKeystoreError::PasswordRequired);
        }

        let keystore_path = self.find_keystore_path_by_pubkey(pubkey_hex)?;

        // note: Keystore::from_json_file and decrypt_keypair are blocking.
        // wrap in spawn_blocking if true async is needed.
        let keystore = Keystore::from_json_file(&keystore_path)?;

        // decrypt_keypair verifies the password and checksum.
        // it returns a new Keypair instance containing the decrypted secret key.
        let decrypted_keypair =
            keystore
                .decrypt_keypair(password.as_bytes())
                .map_err(|e| match e {
                    // provide a more specific error for password failures if possible
                    // eth2_keystore::Error::InvalidPassword => BlsKeystoreError::InvalidPassword,
                    // for now, just wrap the error.
                    _ => BlsKeystoreError::Eth2KeystoreError(e),
                })?;

        Ok(decrypted_keypair.sk)
    }

    /// signs a message (typically a 32-byte hash) using the bls secret key
    /// associated with the given public key.
    /// returns the serialized signature as a byte vector.
    pub async fn sign_message(
        &self,
        pubkey_hex: &str,
        message: &[u8], // typically a 32-byte hash
        password: &str,
    ) -> Result<Vec<u8>, BlsKeystoreError> {
        let secret_key = self.get_secret_key(pubkey_hex, password).await?;

        // the eth2_keystore::SecretKeyBytes has a sign method.
        // it expects the message to be a [u8; 32] if it's using certain bls contexts,
        // but the generic sign method on blst::SecretKey takes &[u8].
        // eth2_keystore::SecretKeyBytes wraps blst::SecretKey.
        // let's assume `message` is already the correct hash to be signed.
        let signature = secret_key.sign(message);

        Ok(signature.serialize().to_vec())
    }

    /// deletes the keystore file associated with the given public key,
    /// after verifying the password.
    pub async fn delete_key(
        &self,
        pubkey_hex: &str,
        password: &str,
    ) -> Result<(), BlsKeystoreError> {
        if password.is_empty() {
            return Err(BlsKeystoreError::PasswordRequired);
        }

        let keystore_path = self.find_keystore_path_by_pubkey(pubkey_hex)?;

        // load the keystore to verify the password before deleting
        let keystore = Keystore::from_json_file(&keystore_path)?;
        keystore
            .decrypt_keypair(password.as_bytes())
            .map_err(|e| match e {
                // eth2_keystore::Error::InvalidPassword => BlsKeystoreError::InvalidPassword,
                _ => BlsKeystoreError::Eth2KeystoreError(e), // or InvalidPassword if distinguishable
            })?;

        // password verified, now delete the file
        fs::remove_file(&keystore_path)?;

        Ok(())
    }

    // helper to find keystore file by pubkey
    fn find_keystore_path_by_pubkey(
        &self,
        pubkey_hex_to_find: &str,
    ) -> Result<PathBuf, BlsKeystoreError> {
        for entry in fs::read_dir(&self.keystore_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                let content = fs::read_to_string(&path)?;
                let json_value: serde_json::Value = serde_json::from_str(&content)?;
                if let Some(pk_hex) = json_value.get("pubkey").and_then(|v| v.as_str()) {
                    if pk_hex == pubkey_hex_to_find {
                        return Ok(path);
                    }
                }
            }
        }
        Err(BlsKeystoreError::KeyNotFound(
            pubkey_hex_to_find.to_string(),
        ))
    }
}

// todo ai: add unit tests for BlsKeystoreManager
// - test_new_manager_default_path
// - test_new_manager_custom_path
// - test_create_new_key_success (mock eth2_keystore interactions)
// - test_create_new_key_empty_password
// - test_import_raw_private_key_success (mock)
// - test_list_keys_empty
// - test_list_keys_with_keys (mock fs and json files)
// - test_get_secret_key_success (mock)
// - test_get_secret_key_not_found
// - test_get_secret_key_invalid_password (mock)
// - test_sign_message_success (mock)
// - test_delete_key_success (mock)
// - test_find_keystore_path_by_pubkey
