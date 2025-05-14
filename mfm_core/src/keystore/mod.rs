pub mod error;
mod types;

use crate::blockchain::Address; // Import Address from blockchain
use crate::blockchain::LocalWallet; // Import LocalWallet directly
use aes_gcm::{aead::Aead, KeyInit}; // Removed unused imports
use base64::engine::general_purpose;
use base64::Engine; // Added base64::Engine
use bip39::Mnemonic; // Added bip39::Mnemonic
use chrono::Utc; // Import Utc
use error::KeystoreError;
use hex;
use k256::ecdsa::SigningKey; // Import SigningKey from k256::ecdsa
use k256::SecretKey; // Import SecretKey
use rand_core::{OsRng, RngCore}; // Import OsRng and RngCore
use slip10::{derive_key_from_path, BIP32Path, Curve}; // Added slip10 imports
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing}; // Removed ZeroizeOnDrop

pub use types::KeyInfo; // Re-export KeyInfo for public API

pub struct Keystore {
    file_path: PathBuf,
    master_key: Option<Zeroizing<Vec<u8>>>,
    entries: Vec<types::EncryptedKeyEntry>,
    master_kdf_params: Option<types::MasterKdfParams>,
    is_unlocked: bool,
    last_activity_at: Option<Instant>,
    // key_data_lock: std::sync::Mutex<()>, // Consider if concurrent access is needed
}

impl Keystore {
    pub fn new(custom_path: Option<PathBuf>) -> Result<Self, KeystoreError> {
        let file_path = if let Some(path) = custom_path {
            path
        } else {
            dirs_next::data_local_dir()
                .ok_or(KeystoreError::Other(
                    "failed to get local data directory".to_string(),
                ))?
                .join("mfm")
                .join("keystore_v1.json")
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

    pub fn initialize_or_load(
        &mut self,
        password_for_creation: Option<&str>,
    ) -> Result<(), KeystoreError> {
        if self.file_path.exists() {
            // Load existing keystore
            let data = std::fs::read(&self.file_path).map_err(KeystoreError::FileError)?;
            let keystore_file: types::KeystoreFile =
                serde_json::from_slice(&data).map_err(KeystoreError::SerializationError)?;

            // Basic version check (can be expanded later)
            if keystore_file.version != "1.0.0" {
                return Err(KeystoreError::Other(format!(
                    "unsupported keystore version: {}",
                    keystore_file.version
                )));
            }

            self.entries = keystore_file.entries;
            self.master_kdf_params = Some(keystore_file.master_kdf_params);
            self.is_unlocked = false; // Always load as locked
        } else {
            // Create new keystore
            let _password = password_for_creation.ok_or(KeystoreError::Other(
                // Prefix with _
                "password required for new keystore creation".to_string(),
            ))?;

            // Generate salt and derive master key params
            let mut salt_bytes = [0u8; 16]; // Recommended salt size for Argon2
            let mut rng = OsRng; // Use imported OsRng
            rng.fill_bytes(&mut salt_bytes); // Use fill_bytes from RngCore
            let salt_hex = hex::encode(&salt_bytes);

            // Define Argon2 parameters (OWASP recommendations or similar)
            let m_cost = 65536; // Memory cost (KiB)
            let t_cost = 3; // Time cost (iterations)
            let p_cost = 1; // Parallelism factor
            let output_len = 32; // Desired key length (for AES-256)

            let master_kdf_params = types::MasterKdfParams {
                salt: salt_hex,
                m_cost,
                t_cost,
                p_cost,
                output_len,
            };

            let _new_keystore_file = types::KeystoreFile {
                // Prefix with _
                version: "1.0.0".to_string(),
                master_kdf: "argon2id".to_string(),
                master_kdf_params: master_kdf_params.clone(),
                entries: Vec::new(),
            };

            // Save the initial empty keystore file
            self.master_kdf_params = Some(master_kdf_params);
            self.entries = Vec::new(); // Ensure in-memory state matches the new file
            self.save_to_disk()?; // Use the save_to_disk method (to be implemented atomically)
            self.is_unlocked = false; // Always create as locked
        }

        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        if self.is_unlocked {
            return Ok(()); // Already unlocked
        }

        let params = self.master_kdf_params.as_ref().ok_or(KeystoreError::Other(
            "keystore not initialized or loaded".to_string(),
        ))?;

        let salt_bytes = hex::decode(&params.salt)
            .map_err(|_| KeystoreError::Other("failed to decode master kdf salt".to_string()))?;

        let argon2_params = argon2::Params::new(
            params.m_cost,
            params.t_cost,
            params.p_cost,
            Some(params.output_len as usize), // Cast to usize
        )
        .map_err(|e| KeystoreError::Other(format!("invalid argon2 parameters: {}", e)))?;

        let argon2 = argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2_params,
        );

        let mut master_key_bytes = vec![0u8; params.output_len as usize];
        argon2
            .hash_password_into(password.as_bytes(), &salt_bytes, &mut master_key_bytes)
            .map_err(|_| KeystoreError::InvalidPassword)?; // Treat Argon2 hash failure as invalid password

        self.master_key = Some(Zeroizing::new(master_key_bytes));
        self.is_unlocked = true;
        self.record_activity(); // Record activity on unlock

        Ok(())
    }

    pub fn lock(&mut self) {
        self.master_key = None; // Zeroizing handles the actual zeroization on drop
        self.is_unlocked = false;
        self.last_activity_at = None; // Reset activity timestamp on lock
    }

    pub fn is_locked(&self) -> bool {
        !self.is_unlocked
    }

    pub fn record_activity(&mut self) {
        if self.is_unlocked {
            self.last_activity_at = Some(Instant::now());
        }
    }

    pub fn should_auto_lock(&self, idle_duration: std::time::Duration) -> bool {
        if !self.is_unlocked {
            return false; // Already locked
        }

        match self.last_activity_at {
            Some(timestamp) => timestamp.elapsed() > idle_duration,
            None => true, // Should not happen if unlocked, but lock if no timestamp
        }
    }

    pub fn change_password(
        &mut self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        // Unlock with the old password to ensure it's correct and the master key is available
        // This also re-derives the master key using the existing parameters
        self.unlock(old_password)?;

        // Generate new salt and derive new master key params
        let mut salt_bytes = [0u8; 16]; // Recommended salt size for Argon2
        let mut rng = OsRng; // Use imported OsRng
        rng.fill_bytes(&mut salt_bytes); // Use fill_bytes from RngCore
        let new_salt_hex = hex::encode(&salt_bytes);

        // Use the same Argon2 parameters as before, but with the new salt
        let old_params = self.master_kdf_params.as_ref().ok_or(KeystoreError::Other(
            "master kdf parameters missing for password change".to_string(),
        ))?;

        let new_master_kdf_params = types::MasterKdfParams {
            salt: new_salt_hex,
            m_cost: old_params.m_cost,
            t_cost: old_params.t_cost,
            p_cost: old_params.p_cost,
            output_len: old_params.output_len,
        };

        let argon2_params = argon2::Params::new(
            new_master_kdf_params.m_cost,
            new_master_kdf_params.t_cost,
            new_master_kdf_params.p_cost,
            Some(new_master_kdf_params.output_len as usize), // Cast to usize
        )
        .map_err(|e| KeystoreError::Other(format!("invalid argon2 parameters: {}", e)))?;

        let argon2 = argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2_params,
        );

        let mut new_master_key_bytes = vec![0u8; new_master_kdf_params.output_len as usize];
        argon2
            .hash_password_into(
                new_password.as_bytes(),
                &salt_bytes,
                &mut new_master_key_bytes,
            )
            .map_err(|_| KeystoreError::Other("failed to derive new master key".to_string()))?; // Should not be InvalidPassword here

        let new_master_key = Zeroizing::new(new_master_key_bytes);

        // Re-encrypt all entries with the new master key and new nonces
        let mut updated_entries = Vec::new();
        let cipher_type = "aes-256-gcm"; // Assuming AES-256-GCM as per plan

        for entry in &self.entries {
            // Decrypt with old master key (which is currently in self.master_key)
            let old_nonce_bytes = hex::decode(&entry.encryption_details.cipher_params.nonce)
                .map_err(|_| KeystoreError::Other("failed to decode old nonce".to_string()))?;
            let old_nonce = aes_gcm::Nonce::from_slice(&old_nonce_bytes);

            let old_cipher = aes_gcm::Aes256Gcm::new_from_slice(self.master_key.as_ref().ok_or(
                KeystoreError::Other("master key missing during re-encryption".to_string()),
            )?)
            .map_err(|_| KeystoreError::Other("failed to create old cipher".to_string()))?;

            let encrypted_material_bytes = general_purpose::STANDARD
                .decode(&entry.encrypted_material)
                .map_err(|_| {
                    // Use general_purpose::STANDARD
                    KeystoreError::Other("failed to decode encrypted material".to_string())
                })?;

            let decrypted_private_key_bytes = old_cipher
                .decrypt(old_nonce, encrypted_material_bytes.as_ref())
                .map_err(|_| KeystoreError::DecryptionError)?;

            // Encrypt with new master key and new nonce
            let mut new_nonce_bytes = [0u8; 12]; // AES-GCM standard nonce size
            let mut rng = OsRng; // Use imported OsRng
            rng.fill_bytes(&mut new_nonce_bytes); // Use fill_bytes from RngCore
            let new_nonce = aes_gcm::Nonce::from_slice(&new_nonce_bytes);
            let new_nonce_hex = hex::encode(&new_nonce_bytes);

            let new_cipher = aes_gcm::Aes256Gcm::new_from_slice(&new_master_key)
                .map_err(|_| KeystoreError::Other("failed to create new cipher".to_string()))?;

            let new_encrypted_material_bytes = new_cipher
                .encrypt(new_nonce, decrypted_private_key_bytes.as_ref())
                .map_err(|_| KeystoreError::EncryptionError)?;

            let new_encrypted_material_base64 =
                general_purpose::STANDARD.encode(&new_encrypted_material_bytes); // Use general_purpose::STANDARD

            // Zeroize decrypted private key bytes immediately
            Zeroize::zeroize(&mut Zeroizing::new(decrypted_private_key_bytes));

            let updated_entry = types::EncryptedKeyEntry {
                uuid: entry.uuid,
                alias: entry.alias.clone(),
                address: entry.address,
                key_type: entry.key_type.clone(),
                encrypted_material: new_encrypted_material_base64,
                encryption_details: types::EncryptionDetails {
                    cipher: cipher_type.to_string(),
                    cipher_params: types::CipherParams {
                        nonce: new_nonce_hex,
                    },
                },
                derivation_path: entry.derivation_path.clone(),
                created_at: entry.created_at.clone(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            updated_entries.push(updated_entry);
        }

        // Update in-memory state
        self.entries = updated_entries;
        self.master_kdf_params = Some(new_master_kdf_params);
        self.master_key = Some(new_master_key); // Update master key in memory

        // Save atomically
        self.save_to_disk()?;
        self.record_activity();

        Ok(())
    }

    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        phrase: &str,
        mnemonic_password: Option<&str>, // BIP-39 passphrase
        derivation_path: &str,           // e.g., "m/44'/60'/0'/0/0"
    ) -> Result<(Uuid, Address), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        // 1. Parse mnemonic and derive seed
        let mnemonic = Mnemonic::parse_in_normalized(bip39::Language::English, phrase) // Use parse_in_normalized
            .map_err(|_| KeystoreError::InvalidMnemonic)?;
        let seed = mnemonic.to_seed(mnemonic_password.unwrap_or(""));

        // 2. Derive private key from seed and derivation path
        let path: BIP32Path = derivation_path.parse().map_err(|_| {
            KeystoreError::InvalidDerivationPath // Use specific error
        })?;

        let derived_key = derive_key_from_path(&seed, Curve::Ed25519, &path) // Correct argument order
            .map_err(|e| KeystoreError::Other(format!("failed to derive key: {}", e)))?; // Keep Other for now

        let private_key_bytes = derived_key.to_bytes() // Assuming private_key() and to_bytes() exist
        let private_key = SecretKey::from_slice(&private_key_bytes) // Use imported SecretKey
            .map_err(|_| KeystoreError::InvalidPrivateKeyHex)?; // Use specific error

        // 3. Get the public address
        let signing_key = SigningKey::from(&private_key); // Use imported SigningKey
        let address = signing_key.address();

        // 4. Encrypt the private key
        let mut rng = OsRng; // Use imported OsRng
        let mut nonce_bytes = [0u8; 12]; // AES-GCM standard nonce size
        rng.fill_bytes(&mut nonce_bytes); // Use fill_bytes from RngCore
        let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
        let nonce_hex = hex::encode(&nonce_bytes);

        let cipher = aes_gcm::Aes256Gcm::new_from_slice(self.master_key.as_ref().ok_or(
            KeystoreError::Other("master key missing during encryption".to_string()),
        )?)
        .map_err(|_| KeystoreError::Other("failed to create cipher".to_string()))?;

        let encrypted_material_bytes = cipher
            .encrypt(nonce, private_key_bytes.as_ref())
            .map_err(|_| KeystoreError::EncryptionError)?;

        let encrypted_material_base64 = general_purpose::STANDARD.encode(&encrypted_material_bytes); // Use general_purpose::STANDARD

        // 5. Create and add the new entry
        let uuid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339(); // Use imported Utc

        let new_entry = types::EncryptedKeyEntry {
            uuid,
            alias,
            address,
            key_type: types::KeyType::MnemonicDerived,
            encrypted_material: encrypted_material_base64,
            encryption_details: types::EncryptionDetails {
                cipher: "aes-256-gcm".to_string(),
                cipher_params: types::CipherParams { nonce: nonce_hex },
            },
            derivation_path: Some(derivation_path.to_string()),
            created_at: now.clone(),
            updated_at: now,
        };

        // Check if an entry with the same address or alias already exists
        if self.entries.iter().any(|entry| entry.address == address) {
            return Err(KeystoreError::AddressAlreadyExists(address));
        }
        if let Some(ref alias_str) = new_entry.alias {
            if self
                .entries
                .iter()
                .any(|entry| entry.alias.as_ref() == Some(alias_str))
            {
                return Err(KeystoreError::AliasAlreadyExists(alias_str.clone()));
            }
        }

        self.entries.push(new_entry);

        // 6. Save to disk atomically
        self.save_to_disk()?;
        self.record_activity();

        Ok((uuid, address))
    }

    pub fn import_private_key_hex(
        &mut self,
        alias: Option<String>,
        pk_hex: &str,
    ) -> Result<(Uuid, Address), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        // 1. Decode and validate the private key hex
        let private_key_bytes =
            hex::decode(pk_hex).map_err(|_| KeystoreError::InvalidPrivateKeyHex)?; // Use specific error

        if private_key_bytes.len() != 32 {
            return Err(KeystoreError::InvalidPrivateKeyHex); // Use specific error
        }

        let private_key = SecretKey::from_slice(&private_key_bytes).map_err(|_| {
            // Use imported SecretKey
            KeystoreError::InvalidPrivateKeyHex // Use specific error
        })?;

        // 2. Get the public address
        let signing_key = SigningKey::from(&private_key);
        let address = signing_key.address();

        // 3. Encrypt the private key
        let mut rng = OsRng; // Use imported OsRng
        let mut nonce_bytes = [0u8; 12]; // AES-GCM standard nonce size
        rng.fill_bytes(&mut nonce_bytes); // Use fill_bytes from RngCore
        let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
        let nonce_hex = hex::encode(&nonce_bytes);

        let cipher = aes_gcm::Aes256Gcm::new_from_slice(self.master_key.as_ref().ok_or(
            KeystoreError::Other("master key missing during encryption".to_string()),
        )?)
        .map_err(|_| KeystoreError::Other("failed to create cipher".to_string()))?;

        let encrypted_material_bytes = cipher
            .encrypt(nonce, private_key_bytes.as_ref())
            .map_err(|_| KeystoreError::EncryptionError)?;

        let encrypted_material_base64 = general_purpose::STANDARD.encode(&encrypted_material_bytes); // Use general_purpose::STANDARD

        // 4. Create and add the new entry
        let uuid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339(); // Use imported Utc

        let new_entry = types::EncryptedKeyEntry {
            uuid,
            alias,
            address,
            key_type: types::KeyType::PrivateKeyImported,
            encrypted_material: encrypted_material_base64,
            encryption_details: types::EncryptionDetails {
                cipher: "aes-256-gcm".to_string(),
                cipher_params: types::CipherParams { nonce: nonce_hex },
            },
            derivation_path: None, // No derivation path for imported private keys
            created_at: now.clone(),
            updated_at: now,
        };

        // Check if an entry with the same address or alias already exists
        if self.entries.iter().any(|entry| entry.address == address) {
            return Err(KeystoreError::AddressAlreadyExists(address));
        }
        if let Some(ref alias_str) = new_entry.alias {
            if self
                .entries
                .iter()
                .any(|entry| entry.alias.as_ref() == Some(alias_str))
            {
                return Err(KeystoreError::AliasAlreadyExists(alias_str.clone()));
            }
        }

        self.entries.push(new_entry);

        // 5. Save to disk atomically
        self.save_to_disk()?;
        self.record_activity();

        Ok((uuid, address))
    }

    pub fn delete_key(&mut self, uuid: Uuid) -> Result<(), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.uuid != uuid);

        if self.entries.len() == initial_len {
            return Err(KeystoreError::KeyNotFound(uuid));
        }

        self.save_to_disk()?;
        self.record_activity();

        Ok(())
    }

    pub fn update_alias(
        &mut self,
        uuid: Uuid,
        new_alias: Option<String>,
    ) -> Result<(), KeystoreError> {
        if !self.is_unlocked {
            return Err(KeystoreError::Locked);
        }

        // Check if alias already exists for another key
        if let Some(ref alias_str) = new_alias {
            if self
                .entries
                .iter()
                .any(|entry| entry.uuid != uuid && entry.alias.as_ref() == Some(alias_str))
            {
                return Err(KeystoreError::AliasAlreadyExists(alias_str.clone()));
            }
        }

        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.uuid == uuid)
            .ok_or(KeystoreError::KeyNotFound(uuid))?;

        entry.alias = new_alias;
        entry.updated_at = chrono::Utc::now().to_rfc3339();
        self.save_to_disk()?;
        self.record_activity();

        Ok(())
    }

    fn save_to_disk(&self) -> Result<(), KeystoreError> {
        let keystore_file = types::KeystoreFile {
            version: "1.0.0".to_string(),
            master_kdf: "argon2id".to_string(),
            master_kdf_params: self.master_kdf_params.clone().ok_or(KeystoreError::Other(
                "master kdf parameters missing for saving".to_string(),
            ))?,
            entries: self.entries.clone(),
        };

        let json_data =
            serde_json::to_vec_pretty(&keystore_file).map_err(KeystoreError::SerializationError)?;

        // Atomic write: Write to a temporary file, sync, then rename
        let temp_path = self.file_path.with_extension("tmp");
        let parent_dir = self.file_path.parent().ok_or(KeystoreError::Other(
            "failed to get parent directory for keystore file".to_string(),
        ))?;

        // Ensure the parent directory exists
        std::fs::create_dir_all(parent_dir).map_err(KeystoreError::FileError)?;

        std::fs::write(&temp_path, json_data).map_err(KeystoreError::FileError)?;

        // Ensure data is written to disk
        let mut temp_file = std::fs::File::open(&temp_path).map_err(KeystoreError::FileError)?;
        temp_file.sync_all().map_err(KeystoreError::FileError)?;

        // Rename the temporary file to the final path
        std::fs::rename(&temp_path, &self.file_path).map_err(KeystoreError::FileError)?;

        // Attempt to set restrictive permissions (best effort)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(&self.file_path) {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o600); // Owner read/write
                let _ = std::fs::set_permissions(&self.file_path, permissions); // Ignore errors
            }
        }

        Ok(())
    }
}
