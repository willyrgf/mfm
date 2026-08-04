use super::secure_key::{HeapKeyBytes, ProtectedBytes};
use super::*;
use crate::signer::ReadAttestationKeyAccess;

impl std::fmt::Debug for Keystore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keystore")
            .field("path", &"<redacted>")
            .field("config", &"<redacted>")
            .field(
                "unlocked",
                &(self.master_key.is_some() && !self.has_unlock_expired()),
            )
            .field("auto_lock_timeout", &self.auto_lock_timeout)
            .field("entry_count", &self.entries.len())
            .field("audit_log_count", &self.audit_log.len())
            .field("kdf_params_loaded", &self.kdf_params.is_some())
            .finish()
    }
}

impl Keystore {
    /// Create or load keystore from file.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, KeystoreError> {
        Self::new_with_config(path, KeystoreConfig::default())
    }

    /// Create or load keystore with custom configuration.
    pub fn new_with_config(
        path: impl AsRef<Path>,
        config: KeystoreConfig,
    ) -> Result<Self, KeystoreError> {
        let path = path.as_ref().to_path_buf();
        let mut keystore = Self::empty(path, config);

        if keystore.path.exists() {
            keystore.load_from_disk()?;
        }

        Ok(keystore)
    }

    /// Opens and unlocks an existing keystore without initialization, audit mutation, or writes.
    pub(crate) fn open_existing_for_read_attestation(
        path: impl AsRef<Path>,
        config: KeystoreConfig,
        password: &str,
        _access: &ReadAttestationKeyAccess,
    ) -> Result<Self, KeystoreError> {
        let mut keystore = Self::empty(path.as_ref().to_path_buf(), config);
        keystore.load_from_disk()?;
        keystore.unlock_existing(password)?;
        Ok(keystore)
    }

    fn empty(path: PathBuf, config: KeystoreConfig) -> Self {
        Self {
            path,
            config,
            master_key: None,
            unlocked_at: None,
            auto_lock_timeout: Some(DEFAULT_AUTO_LOCK_TIMEOUT),
            entries: Vec::new(),
            audit_log: Vec::new(),
            kdf_params: None,
            master_key_verification: None,
            file_integrity_mac: None,
            #[cfg(test)]
            fail_next_write: Cell::new(false),
            _not_thread_safe: std::ptr::null(),
        }
    }

    /// Initialize new keystore or unlock existing one.
    pub fn unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        if let Err(err) = self.early_file_validation() {
            self.append_audit_event(AuditEvent::Unlock, false);
            return Err(err);
        }

        let result = if self.path.exists() {
            self.unlock_existing(password)
        } else {
            self.initialize_new(password)
        };

        self.append_audit_event(AuditEvent::Unlock, result.is_ok());
        result
    }

    #[cfg(test)]
    pub(super) fn set_auto_lock_timeout(&mut self, timeout: Option<Duration>) {
        self.auto_lock_timeout = timeout;
    }

    #[cfg(test)]
    pub(super) fn audit_log(&self) -> &[AuditLogEntry] {
        &self.audit_log
    }

    #[cfg(test)]
    pub(super) fn fail_next_write_for_test(&self) {
        self.fail_next_write.set(true);
    }

    pub(super) fn append_audit_event(&mut self, event: AuditEvent, success: bool) {
        let timestamp = Utc::now();
        if self.audit_log.len() >= MAX_AUDIT_LOG_ENTRIES {
            self.compact_audit_log_for_append(timestamp);
        }

        self.audit_log.push(AuditLogEntry {
            timestamp,
            event,
            success,
        });
        debug_assert!(self.audit_log.len() <= MAX_AUDIT_LOG_ENTRIES);
    }

    fn compact_audit_log_for_append(&mut self, timestamp: DateTime<Utc>) {
        if self.audit_log.len() < MAX_AUDIT_LOG_ENTRIES {
            return;
        }

        let first_is_compaction = matches!(
            self.audit_log.first().map(|entry| &entry.event),
            Some(AuditEvent::AuditLogCompacted { .. })
        );

        if first_is_compaction {
            let drop_count = self
                .audit_log
                .len()
                .saturating_sub(MAX_AUDIT_LOG_ENTRIES.saturating_sub(1));
            if drop_count > 0 {
                self.audit_log.drain(1..1 + drop_count);
            }
            if let Some(first) = self.audit_log.first_mut() {
                if let AuditEvent::AuditLogCompacted { dropped_entries } = &mut first.event {
                    *dropped_entries = dropped_entries.saturating_add(drop_count as u64);
                    first.timestamp = timestamp;
                    first.success = true;
                }
            }
        } else {
            let drop_count = (self.audit_log.len() + 2)
                .saturating_sub(MAX_AUDIT_LOG_ENTRIES)
                .min(self.audit_log.len());
            if drop_count > 0 {
                self.audit_log.drain(0..drop_count);
            }
            self.audit_log.insert(
                0,
                AuditLogEntry {
                    timestamp,
                    event: AuditEvent::AuditLogCompacted {
                        dropped_entries: drop_count as u64,
                    },
                    success: true,
                },
            );
        }
    }

    fn initialize_new(&mut self, password: &str) -> Result<(), KeystoreError> {
        self.validate_password_policy(password)?;

        let mut salt = [0u8; 32];
        OsRng.try_fill_bytes(&mut salt).map_err(|_| {
            KeystoreError::CryptoError("Failed to generate random salt".to_string())
        })?;

        let kdf_params = ArgonParams {
            salt,
            memory_kb: self.config.argon2_memory_kb,
            iterations: self.config.argon2_iterations,
            parallelism: self.config.argon2_parallelism,
        };

        let master_key = self.derive_master_key(password, &kdf_params)?;
        let master_key_verification = self.create_verification_hash(&master_key)?;

        self.master_key = Some(master_key);
        self.unlocked_at = Some(Instant::now());
        self.kdf_params = Some(kdf_params);
        self.master_key_verification = Some(master_key_verification);
        self.save_to_disk()?;

        Ok(())
    }

    fn unlock_existing(&mut self, password: &str) -> Result<(), KeystoreError> {
        let kdf_params = self
            .kdf_params
            .as_ref()
            .ok_or(KeystoreError::InvalidPassword)?;

        let master_key = self.derive_master_key(password, kdf_params)?;
        let verified_file = self.verify_file_integrity(&master_key)?;

        let computed_verification = self.create_verification_hash(&master_key)?;
        if computed_verification
            .ct_ne(&verified_file.master_key_verification)
            .into()
        {
            Err(KeystoreError::InvalidPassword)
        } else {
            self.entries = verified_file.entries;
            self.audit_log = verified_file.audit_log;
            self.kdf_params = Some(verified_file.kdf_params);
            self.master_key_verification = Some(verified_file.master_key_verification);
            self.file_integrity_mac = Some(verified_file.file_integrity_mac);
            self.master_key = Some(master_key);
            self.unlocked_at = Some(Instant::now());
            Ok(())
        }
    }

    pub(super) fn derive_master_key(
        &self,
        password: &str,
        params: &ArgonParams,
    ) -> Result<Zeroizing<[u8; 32]>, KeystoreError> {
        self.validate_kdf_params(params)?;
        let argon2_params = Params::new(
            params.memory_kb,
            params.iterations,
            params.parallelism,
            Some(32),
        )?;

        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2_params,
        );

        let mut output = Zeroizing::new([0u8; 32]);
        argon2.hash_password_into(password.as_bytes(), &params.salt, output.as_mut())?;

        Ok(output)
    }

    pub(super) fn create_verification_hash(
        &self,
        master_key: &[u8; 32],
    ) -> Result<[u8; 32], KeystoreError> {
        use ring::hmac;
        let key = hmac::Key::new(hmac::HMAC_SHA256, master_key);
        let tag = hmac::sign(&key, b"keystore_verification_v1");
        let mut verification = [0u8; 32];
        verification.copy_from_slice(tag.as_ref());
        Ok(verification)
    }

    pub(super) fn compute_file_integrity_mac(
        &self,
        master_key: &[u8; 32],
        file_data: &[u8],
    ) -> Result<[u8; 32], KeystoreError> {
        use ring::hmac;
        let key = hmac::Key::new(hmac::HMAC_SHA256, master_key);
        let mut ctx = hmac::Context::with_key(&key);
        ctx.update(FILE_INTEGRITY_CONTEXT);
        ctx.update(file_data);
        let tag = ctx.sign();
        let mut mac = [0u8; 32];
        mac.copy_from_slice(tag.as_ref());
        Ok(mac)
    }

    // TODO: check nonce-reuse-robust XChaCha20-Poly1305
    pub(super) fn encrypt_data(
        &self,
        master_key: &[u8; 32],
        nonce: &[u8; 12],
        data: &[u8],
        additional_data: &[u8],
    ) -> Result<Vec<u8>, KeystoreError> {
        let key = Key::<Aes256Gcm>::from_slice(master_key);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce);

        use aes_gcm::aead::Payload;
        let payload = Payload {
            msg: data,
            aad: additional_data,
        };

        Ok(cipher.encrypt(nonce, payload)?)
    }

    pub(super) fn decrypt_data(
        &self,
        master_key: &[u8; 32],
        nonce: &[u8; 12],
        encrypted_data: &[u8],
        additional_data: &[u8],
    ) -> Result<ProtectedBytes, KeystoreError> {
        let key = Key::<Aes256Gcm>::from_slice(master_key);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce);
        // AES-GCM appends a 16-byte detached tag. Allocate the final protected
        // plaintext buffer before authentication and decrypt in place into it.
        if encrypted_data.len() != 32 + 16 {
            return Err(KeystoreError::InvalidPrivateKey);
        }
        let (ciphertext, tag_bytes) = encrypted_data.split_at(32);
        let mut plaintext = Zeroizing::new(HeapKeyBytes::zeroed());
        plaintext.as_mut().as_mut().copy_from_slice(ciphertext);
        let tag = Tag::from_slice(tag_bytes);
        cipher
            .decrypt_in_place_detached(nonce, additional_data, plaintext.as_mut().as_mut(), tag)
            .map_err(|_| KeystoreError::InvalidPrivateKey)?;
        Ok(plaintext)
    }

    pub(super) fn ensure_unlocked_for_read(&self) -> Result<(), KeystoreError> {
        if self.master_key.is_none() || self.has_unlock_expired() {
            return Err(KeystoreError::Locked);
        }
        Ok(())
    }

    pub(super) fn ensure_master_key_available(&mut self) -> Result<(), KeystoreError> {
        if self.has_unlock_expired() && self.master_key.is_some() {
            self.append_audit_event(AuditEvent::Lock, true);
            self.master_key = None;
            self.unlocked_at = None;
        }
        if self.master_key.is_some() {
            self.unlocked_at = Some(Instant::now());
        }
        if self.master_key.is_none() {
            return Err(KeystoreError::Locked);
        }
        Ok(())
    }

    pub(super) fn has_unlock_expired(&self) -> bool {
        match (self.unlocked_at, self.auto_lock_timeout) {
            (Some(unlocked_at), Some(timeout)) => unlocked_at.elapsed() >= timeout,
            _ => false,
        }
    }

    pub(super) fn validate_password_policy(&self, password: &str) -> Result<(), KeystoreError> {
        if password.len() < MIN_PASSWORD_LEN {
            return Err(KeystoreError::InvalidInput(format!(
                "Password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }

        let lowered = password.to_ascii_lowercase();
        if PASSWORD_REJECT_LIST
            .iter()
            .any(|candidate| lowered == *candidate)
        {
            return Err(KeystoreError::InvalidInput(
                "Password is too weak".to_string(),
            ));
        }

        if let Some(first) = lowered.chars().next() {
            if lowered.chars().all(|ch| ch == first) {
                return Err(KeystoreError::InvalidInput(
                    "Password is too weak".to_string(),
                ));
            }
        }

        if password.trim().is_empty() {
            return Err(KeystoreError::InvalidInput(
                "Password is too weak".to_string(),
            ));
        }

        Ok(())
    }
}
