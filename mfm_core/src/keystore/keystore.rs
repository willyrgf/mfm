use crate::keystore::entry::{DecryptedPrivateKey, EncryptedKeyEntry, KeyInfo};
use crate::keystore::error::KeystoreError;
use alloy_primitives::Address;
use alloy_signer::signers::private_key::PrivateKeySigner;
use atomicwrites::{AtomicFile, Overwrite};
use dirs_next::data_local_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;
use zeroize::Zeroizing;

const KEYSTORE_FILE_NAME: &str = "keystore_v1.json";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct MasterKdfParams {
    pub salt: Vec<u8>,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub output_len: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct KeystoreFileContent {
    pub version: String,
    pub master_kdf: String,
    pub master_kdf_params: MasterKdfParams,
    pub entries: Vec<EncryptedKeyEntry>,
}

pub struct Keystore {
    file_path: PathBuf,
    master_key: Option<Zeroizing<Vec<u8>>>,
    entries: Vec<EncryptedKeyEntry>,
    master_kdf_params: Option<MasterKdfParams>,
    is_unlocked: bool,
    last_activity_at: Option<Instant>,
}

impl Keystore {
    pub fn new(custom_path: Option<PathBuf>) -> Result<Self, KeystoreError> {
        let file_path = if let Some(path) = custom_path {
            path
        } else {
            data_local_dir()
                .ok_or_else(|| {
                    KeystoreError::Other("Could not determine local data directory".to_string())
                })?
                .join(".mfm")
                .join(KEYSTORE_FILE_NAME)
        };

        Ok(Self {
            file_path,
            master_key: None,
            entries: Vec::new(),
            master_kdf_params: None,
            is_unlocked: false,
            last_activity_at: None,
        })
    }

    pub fn initialize_or_load(&mut self, password: Option<&str>) -> Result<(), KeystoreError> {
        if self.file_path.exists() {
            // Load existing keystore
            let content = fs::read_to_string(&self.file_path)?;
            let keystore_content: KeystoreFileContent = serde_json::from_str(&content)?;

            if keystore_content.version != "1.0.0" {
                return Err(KeystoreError::Other(format!(
                    "Unsupported keystore version: {}",
                    keystore_content.version
                )));
            }

            self.entries = keystore_content.entries;
            self.master_kdf_params = Some(keystore_content.master_kdf_params);

            if let Some(pwd) = password {
                self.unlock(pwd)?;
            }
        } else {
            // Initialize new keystore
            let pwd = password.ok_or_else(|| {
                KeystoreError::Other(
                    "Password is required to initialize a new keystore".to_string(),
                )
            })?;

            let mut salt = vec![0u8; 16];
            use rand_core::RngCore;
            rand_core::OsRng.fill_bytes(&mut salt);

            let master_kdf_params = MasterKdfParams {
                salt,
                m_cost: 65536,
                t_cost: 3,
                p_cost: 1,
                output_len: 32,
            };

            let keystore_content = KeystoreFileContent {
                version: "1.0.0".to_string(),
                master_kdf: "argon2id".to_string(),
                master_kdf_params: master_kdf_params.clone(),
                entries: Vec::new(),
            };

            let dir = self
                .file_path
                .parent()
                .ok_or_else(|| KeystoreError::Other("Invalid keystore file path".to_string()))?;
            fs::create_dir_all(dir)?;

            let af = AtomicFile::new(&self.file_path, Overwrite);
            af.write(|f| {
                serde_json::to_writer_pretty(f, &keystore_content)?;
                Ok(())
            })?;

            self.master_kdf_params = Some(master_kdf_params);
            self.unlock(pwd)?;
        }

        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        if self.is_unlocked {
            return Err(KeystoreError::AlreadyUnlocked);
        }

        let params = self.master_kdf_params.as_ref().ok_or_else(|| {
            KeystoreError::Other(
                "Master KDF parameters not set. Keystore may not be initialized.".to_string(),
            )
        })?;

        let argon2 = argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V13,
            argon2::Params::new(
                params.m_cost,
                params.t_cost,
                params.p_cost,
                Some(params.output_len),
            )
            .map_err(|e| KeystoreError::Argon2(e))?,
        );

        let mut master_key = Zeroizing::new(vec![0u8; params.output_len]);
        argon2
            .hash_password_into(password.as_bytes(), &params.salt, &mut master_key)
            .map_err(|e| KeystoreError::Argon2(e))?;

        // TODO: Verify master key by decrypting a test entry or using a checksum

        self.master_key = Some(master_key);
        self.is_unlocked = true;
        self.last_activity_at = Some(Instant::now());

        Ok(())
    }

    pub fn lock(&mut self) {
        self.master_key = None;
        self.is_unlocked = false;
        self.last_activity_at = None;
        // TODO: Zeroize entries' decrypted private keys if they are stored in memory
    }

    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        phrase: &str,
        passphrase: Option<&str>,
        path: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let mnemonic = bip39::Mnemonic::from_phrase(phrase, bip39::Language::English)?;
        let seed = mnemonic.to_seed(passphrase.unwrap_or(""));
        let root_key = bip32::ExtendedPrivateKey::new(seed)?;
        let derivation_path: bip32::DerivationPath = path
            .parse()
            .map_err(|e: bip32::Error| KeystoreError::Bip32(e))?;
        let derived_key = root_key.derive(&derivation_path)?;
        let private_key_bytes = derived_key.private_key().to_bytes();

        self.import_private_key_bytes(alias, &private_key_bytes)
    }

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let private_key_bytes = hex::decode(pk_hex)?;
        self.import_private_key_bytes(alias, &private_key_bytes)
    }

    fn import_private_key_bytes(
        &mut self,
        alias: Option<String>,
        private_key_bytes: &[u8],
    ) -> Result<(Uuid, Address), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        if let Some(ref alias_name) = alias {
            if self
                .entries
                .iter()
                .any(|entry| entry.alias.as_deref() == Some(alias_name))
            {
                return Err(KeystoreError::AliasAlreadyExists(alias_name.clone()));
            }
        }

        let master_key = self.master_key.as_ref().ok_or_else(|| {
            KeystoreError::Other("Master key not available. Keystore may be locked.".to_string())
        })?;

        use aes_gcm::aead::{Aead, NewAead};
        use aes_gcm::Aes256Gcm;
        use rand_core::RngCore;

        let cipher = Aes256Gcm::new_from_slice(master_key.as_ref())
            .map_err(|_| KeystoreError::Other("Failed to create AES-GCM cipher".to_string()))?;

        let mut nonce = vec![0u8; 12]; // AES-GCM standard nonce size
        rand_core::OsRng.fill_bytes(&mut nonce);

        let encrypted_private_key = cipher
            .encrypt(&nonce.into(), private_key_bytes)
            .map_err(|_| KeystoreError::Other("Failed to encrypt private key".to_string()))?;

        use k256::ecdsa::SigningKey;
        use std::str::FromStr;

        let signing_key =
            SigningKey::from_bytes(private_key_bytes).map_err(|e| KeystoreError::K256(e))?;
        let verifying_key = signing_key.verifying_key();
        let address = Address::from_slice(&verifying_key.to_encoded_point(false).as_bytes()[1..]);

        let id = Uuid::new_v4();

        let new_entry = EncryptedKeyEntry {
            id,
            alias,
            address,
            encrypted_private_key,
            nonce,
            kdf_params: None, // Not using per-key KDF yet
        };

        self.entries.push(new_entry);
        self.save()?;

        Ok((id, address))
    }

    fn save(&self) -> Result<(), KeystoreError> {
        let keystore_content = KeystoreFileContent {
            version: "1.0.0".to_string(),
            master_kdf: "argon2id".to_string(),
            master_kdf_params: self.master_kdf_params.clone().ok_or_else(|| {
                KeystoreError::Other("Master KDF parameters not set for saving".to_string())
            })?,
            entries: self.entries.clone(),
        };

        let dir = self.file_path.parent().ok_or_else(|| {
            KeystoreError::Other("Invalid keystore file path for saving".to_string())
        })?;
        fs::create_dir_all(dir)?;

        let af = AtomicFile::new(&self.file_path, Overwrite);
        af.write(|f| {
            serde_json::to_writer_pretty(f, &keystore_content)?;
            Ok(())
        })?;

        Ok(())
    }

    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let key_info_list = self
            .entries
            .iter()
            .map(|entry| KeyInfo {
                id: entry.id,
                alias: entry.alias.clone(),
                address: entry.address,
            })
            .collect();

        Ok(key_info_list)
    }

    pub fn get_signer(&mut self, uuid: Uuid) -> Result<PrivateKeySigner, KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == uuid)
            .ok_or_else(|| KeystoreError::KeyNotFound(uuid))?;

        let master_key = self.master_key.as_ref().ok_or_else(|| {
            KeystoreError::Other("Master key not available. Keystore may be locked.".to_string())
        })?;

        use aes_gcm::aead::{Aead, NewAead};
        use aes_gcm::Aes256Gcm;

        let cipher = Aes256Gcm::new_from_slice(master_key.as_ref())
            .map_err(|_| KeystoreError::Other("Failed to create AES-GCM cipher".to_string()))?;

        let decrypted_private_key_bytes = cipher
            .decrypt(
                &entry.nonce.clone().into(),
                entry.encrypted_private_key.as_ref(),
            )
            .map_err(|_| KeystoreError::Other("Failed to decrypt private key".to_string()))?;

        use k256::ecdsa::SigningKey;
        use std::str::FromStr;

        let signing_key = SigningKey::from_bytes(&decrypted_private_key_bytes)
            .map_err(|e| KeystoreError::K256(e))?;

        // Zeroize the decrypted key bytes immediately after creating the SigningKey
        let mut zeroizable_bytes = Zeroizing::new(decrypted_private_key_bytes);
        zeroizable_bytes.zeroize();

        Ok(PrivateKeySigner::from(signing_key))
    }

    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.id != uuid);

        if self.entries.len() == initial_len {
            return Err(KeystoreError::KeyNotFound(uuid));
        }

        self.save()?;

        Ok(())
    }

    fn save(&self) -> Result<(), KeystoreError> {
        let keystore_content = KeystoreFileContent {
            version: "1.0.0".to_string(),
            master_kdf: "argon2id".to_string(),
            master_kdf_params: self.master_kdf_params.clone().ok_or_else(|| {
                KeystoreError::Other("Master KDF parameters not set for saving".to_string())
            })?,
            entries: self.entries.clone(),
        };

        let dir = self.file_path.parent().ok_or_else(|| {
            KeystoreError::Other("Invalid keystore file path for saving".to_string())
        })?;
        fs::create_dir_all(dir)?;

        let af = AtomicFile::new(&self.file_path, Overwrite);
        af.write(|f| {
            serde_json::to_writer_pretty(f, &keystore_content)?;
            Ok(())
        })?;

        Ok(())
    }
}
