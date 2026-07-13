use std::str::FromStr;
use zeroize::Zeroizing;

use super::secure_key::ethereum_address_from_key_bytes;
use super::*;

impl Keystore {
    /// Import private key (hex format).
    pub fn import_private_key(
        &mut self,
        alias: Option<String>,
        private_key_hex: &str,
    ) -> Result<Uuid, KeystoreError> {
        let id = Uuid::new_v4();
        self.ensure_master_key_available()?;

        let private_key_hex = private_key_hex.trim_start_matches("0x");
        if private_key_hex.len() != 64 {
            return Err(KeystoreError::InvalidPrivateKey);
        }

        let private_key_bytes = Zeroizing::new(
            hex::decode(private_key_hex).map_err(|_| KeystoreError::InvalidPrivateKey)?,
        );
        if private_key_bytes.len() != 32 {
            return Err(KeystoreError::InvalidPrivateKey);
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(private_key_bytes.as_slice());
        self.import_key_material(
            id,
            alias,
            Zeroizing::new(key_array),
            KeyType::PrivateKey,
            AuditEvent::ImportPrivateKey { id },
        )
    }

    /// Import mnemonic with derivation path.
    pub fn import_mnemonic(
        &mut self,
        alias: Option<String>,
        mnemonic: &str,
        derivation_path: &str,
        passphrase: Option<&str>,
    ) -> Result<Uuid, KeystoreError> {
        let id = Uuid::new_v4();
        self.ensure_master_key_available()?;

        let mnemonic = Mnemonic::from_str(mnemonic)?;
        let derivation_path_obj = DerivationPath::from_str(derivation_path)?;

        // The mnemonic and passphrase are import inputs, not persisted wallet material.
        let seed = Zeroizing::new(mnemonic.to_seed(passphrase.unwrap_or("")));
        let derived_key: Zeroizing<[u8; 32]> = {
            let derived_xprv = XPrv::derive_from_path(*seed, &derivation_path_obj)?;
            Zeroizing::new(derived_xprv.private_key().to_bytes().into())
        };

        self.import_key_material(
            id,
            alias,
            derived_key,
            KeyType::HdDerived {
                derivation_path: derivation_path.to_string(),
            },
            AuditEvent::ImportMnemonic { id },
        )
    }

    fn import_key_material(
        &mut self,
        id: Uuid,
        alias: Option<String>,
        key_bytes: Zeroizing<[u8; 32]>,
        key_type: KeyType,
        audit_event: AuditEvent,
    ) -> Result<Uuid, KeystoreError> {
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        let address = ethereum_address_from_key_bytes(key_bytes.as_ref())?;

        let mut nonce = [0u8; 12];
        OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
            KeystoreError::CryptoError("Failed to generate random nonce".to_string())
        })?;

        let encrypted_data =
            self.encrypt_data(master_key, &nonce, key_bytes.as_ref(), id.as_bytes())?;
        let entry = KeyEntry {
            id,
            alias,
            address,
            key_type,
            encrypted_data,
            nonce,
            created_at: Utc::now(),
        };

        let previous_entry_len = self.entries.len();
        let previous_audit_log = self.audit_log.clone();
        self.entries.push(entry);
        self.append_audit_event(audit_event, true);
        if let Err(err) = self.save_to_disk() {
            self.entries.truncate(previous_entry_len);
            self.audit_log = previous_audit_log;
            return Err(err);
        }

        Ok(id)
    }

    /// Get private key for signing.
    pub fn get_private_key(&mut self, id: Uuid) -> Result<SecureKey, KeystoreError> {
        let result: Result<SecureKey, KeystoreError> = (|| {
            self.ensure_master_key_available()?;
            let key_bytes = self.decrypt_entry_key(id)?;
            Ok(SecureKey::new(*key_bytes))
        })();

        match result {
            Ok(secure_key) => {
                let previous_audit_log = self.audit_log.clone();
                self.append_audit_event(AuditEvent::GetPrivateKey { id }, true);
                if let Err(err) = self.save_to_disk() {
                    self.audit_log = previous_audit_log;
                    return Err(err);
                }

                Ok(secure_key)
            }
            Err(err) => Err(err),
        }
    }

    /// Export the private key as a hex string (0x-prefixed).
    ///
    /// - For `KeyType::PrivateKey`, this returns the stored private key.
    /// - For `KeyType::HdDerived`, this returns the one-time derived private key.
    #[cfg(feature = "dangerous-secret-export")]
    pub fn export_private_key(&mut self, id: Uuid) -> Result<Zeroizing<String>, KeystoreError> {
        let result: Result<Zeroizing<String>, KeystoreError> = (|| {
            self.ensure_secret_exports_enabled()?;
            self.ensure_master_key_available()?;
            let key_bytes = self.decrypt_entry_key(id)?;
            Ok(Zeroizing::new(format!(
                "0x{}",
                hex::encode(key_bytes.as_ref())
            )))
        })();

        match result {
            Ok(private_key_hex) => {
                let previous_audit_log = self.audit_log.clone();
                self.append_audit_event(AuditEvent::ExportPrivateKey { id }, true);
                if let Err(err) = self.save_to_disk() {
                    self.audit_log = previous_audit_log;
                    return Err(err);
                }

                Ok(private_key_hex)
            }
            Err(err) => {
                if self.master_key.is_some() && !self.has_unlock_expired() {
                    let previous_audit_log = self.audit_log.clone();
                    self.append_audit_event(AuditEvent::ExportPrivateKey { id }, false);
                    if self.save_to_disk().is_err() {
                        self.audit_log = previous_audit_log;
                    }
                }
                Err(err)
            }
        }
    }

    /// List stored keys (metadata only). Requires an unlocked session.
    pub fn list_keys(&self) -> Result<Vec<KeyInfo>, KeystoreError> {
        self.ensure_unlocked_for_read()?;
        Ok(self.entries.iter().map(KeyInfo::from).collect())
    }

    /// Remove key from keystore.
    pub fn delete_key(&mut self, id: Uuid) -> Result<(), KeystoreError> {
        let result: Result<(), KeystoreError> = (|| {
            self.ensure_master_key_available()?;

            let previous_entries = self.entries.clone();
            let previous_audit_log = self.audit_log.clone();
            let initial_len = previous_entries.len();
            self.entries.retain(|entry| entry.id != id);

            if self.entries.len() == initial_len {
                return Err(KeystoreError::KeyNotFound(id));
            }

            self.append_audit_event(AuditEvent::DeleteKey { id }, true);
            if let Err(err) = self.save_to_disk() {
                self.entries = previous_entries;
                self.audit_log = previous_audit_log;
                return Err(err);
            }
            Ok(())
        })();
        result
    }

    /// Change keystore password by re-encrypting all entries with a new derived master key.
    pub fn change_password(
        &mut self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), KeystoreError> {
        let result: Result<(), KeystoreError> = (|| {
            self.ensure_master_key_available()?;
            self.validate_password_policy(new_password)?;
            let previous_entries = self.entries.clone();
            let previous_audit_log = self.audit_log.clone();
            let previous_kdf_params = self.kdf_params.clone();
            let previous_master_key_verification = self.master_key_verification;
            let previous_master_key = self.master_key.clone();
            let previous_unlocked_at = self.unlocked_at;
            let previous_file_integrity_mac = self.file_integrity_mac;

            let kdf_params = self
                .kdf_params
                .as_ref()
                .ok_or(KeystoreError::InvalidPassword)?;

            let stored_verification = self
                .master_key_verification
                .ok_or(KeystoreError::InvalidPassword)?;

            let old_master_key = self.derive_master_key(old_password, kdf_params)?;
            let computed_verification = self.create_verification_hash(&old_master_key)?;
            if computed_verification.ct_ne(&stored_verification).into() {
                return Err(KeystoreError::InvalidPassword);
            }

            let mut salt = [0u8; 32];
            OsRng.try_fill_bytes(&mut salt).map_err(|_| {
                KeystoreError::CryptoError("Failed to generate random salt".to_string())
            })?;

            let new_kdf_params = ArgonParams {
                salt,
                memory_kb: self.config.argon2_memory_kb,
                iterations: self.config.argon2_iterations,
                parallelism: self.config.argon2_parallelism,
            };

            let new_master_key = self.derive_master_key(new_password, &new_kdf_params)?;
            let new_verification = self.create_verification_hash(&new_master_key)?;

            // Re-encrypt entries one-by-one to avoid holding all plaintexts in memory.
            let mut new_entries = Vec::with_capacity(self.entries.len());
            for entry in &self.entries {
                let plaintext = self.decrypt_data(
                    &old_master_key,
                    &entry.nonce,
                    &entry.encrypted_data,
                    entry.id.as_bytes(),
                )?;

                let mut nonce = [0u8; 12];
                OsRng.try_fill_bytes(&mut nonce).map_err(|_| {
                    KeystoreError::CryptoError("Failed to generate random nonce".to_string())
                })?;

                let encrypted =
                    self.encrypt_data(&new_master_key, &nonce, &plaintext, entry.id.as_bytes())?;
                let mut updated_entry = entry.clone();
                updated_entry.encrypted_data = encrypted;
                updated_entry.nonce = nonce;
                new_entries.push(updated_entry);
            }
            self.entries = new_entries;

            self.kdf_params = Some(new_kdf_params);
            self.master_key_verification = Some(new_verification);
            self.master_key = Some(new_master_key);
            self.unlocked_at = Some(Instant::now());
            self.append_audit_event(AuditEvent::ChangePassword, true);

            if let Err(err) = self.save_to_disk_after_rekey(&old_master_key) {
                self.entries = previous_entries;
                self.audit_log = previous_audit_log;
                self.kdf_params = previous_kdf_params;
                self.master_key_verification = previous_master_key_verification;
                self.master_key = previous_master_key;
                self.unlocked_at = previous_unlocked_at;
                self.file_integrity_mac = previous_file_integrity_mac;
                return Err(err);
            }
            Ok(())
        })();
        result
    }

    fn decrypt_entry_key(&self, id: Uuid) -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(KeystoreError::KeyNotFound(id))?;
        let decrypted_data = self.decrypt_data(
            master_key,
            &entry.nonce,
            &entry.encrypted_data,
            entry.id.as_bytes(),
        )?;
        if decrypted_data.len() != 32 {
            return Err(KeystoreError::InvalidPrivateKey);
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&decrypted_data);
        Ok(Zeroizing::new(key_bytes))
    }
}
