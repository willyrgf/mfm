use fs2::FileExt;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::*;

struct MutationLockGuard {
    file: std::fs::File,
}

impl Drop for MutationLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Keystore {
    pub(super) fn save_to_disk(&mut self) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        let parent = self.ensure_parent_directory_safe()?;
        let _lock = self.acquire_mutation_lock(&parent)?;
        self.ensure_master_key_available()?;
        let master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        self.verify_current_file_matches_memory_mac(master_key)?;
        let file_integrity_mac = self.write_current_keystore_file_locked(&parent, master_key)?;
        self.file_integrity_mac = Some(file_integrity_mac);

        Ok(())
    }

    pub(super) fn save_to_disk_after_rekey(
        &mut self,
        current_file_master_key: &[u8; 32],
    ) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        let parent = self.ensure_parent_directory_safe()?;
        let _lock = self.acquire_mutation_lock(&parent)?;
        self.verify_current_file_matches_memory_mac(current_file_master_key)?;
        self.ensure_master_key_available()?;
        let new_master_key = self.master_key.as_ref().ok_or(KeystoreError::Locked)?;
        let file_integrity_mac =
            self.write_current_keystore_file_locked(&parent, new_master_key)?;
        self.file_integrity_mac = Some(file_integrity_mac);

        Ok(())
    }

    fn write_current_keystore_file_locked(
        &self,
        parent: &Path,
        master_key: &[u8; 32],
    ) -> Result<[u8; 32], KeystoreError> {
        let kdf_params = self
            .kdf_params
            .as_ref()
            .ok_or(KeystoreError::InvalidInput("No KDF parameters".to_string()))?;

        let master_key_verification =
            self.master_key_verification
                .ok_or(KeystoreError::InvalidInput(
                    "No verification hash".to_string(),
                ))?;

        let keystore_file_without_mac = KeystoreFile {
            version: KEYSTORE_FILE_VERSION,
            kdf_params: kdf_params.clone(),
            master_key_verification,
            audit_log: self.audit_log.clone(),
            entries: self.entries.clone(),
            file_integrity_mac: [0u8; 32],
        };
        self.validate_keystore_shape(&keystore_file_without_mac)?;

        let mac_preimage = Self::canonical_file_mac_preimage_from_file(&keystore_file_without_mac)?;
        let file_integrity_mac = self.compute_file_integrity_mac(master_key, &mac_preimage)?;

        let keystore_file = KeystoreFile {
            version: KEYSTORE_FILE_VERSION,
            kdf_params: kdf_params.clone(),
            master_key_verification,
            audit_log: self.audit_log.clone(),
            entries: self.entries.clone(),
            file_integrity_mac,
        };
        self.validate_keystore_shape(&keystore_file)?;

        let json_data = serde_json::to_vec(&keystore_file)?;
        self.atomic_write_keystore_file(parent, &json_data)?;

        Ok(file_integrity_mac)
    }

    pub(super) fn load_from_disk(&mut self) -> Result<(), KeystoreError> {
        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;
        self.early_file_validation()?;

        let value = self.read_keystore_file_value()?;
        let header = self.parse_bounded_header(&value)?;

        self.kdf_params = Some(header.kdf_params);
        self.master_key_verification = Some(header.master_key_verification);
        self.file_integrity_mac = Some(header.file_integrity_mac);
        self.entries.clear();
        self.audit_log.clear();
        self.master_key = None;
        self.unlocked_at = None;

        Ok(())
    }

    pub(super) fn early_file_validation(&self) -> Result<(), KeystoreError> {
        // Early validation with dummy key to detect obviously malformed files
        // before expensive password derivation.
        if !self.path.exists() {
            return Ok(());
        }

        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;

        let value = self.read_keystore_file_value()?;
        self.parse_bounded_header(&value)?;

        Ok(())
    }

    pub(super) fn verify_file_integrity(
        &self,
        master_key: &[u8; 32],
    ) -> Result<KeystoreFile, KeystoreError> {
        self.ensure_target_path_is_safe()?;
        self.ensure_parent_directory_safe()?;

        let value = self.read_keystore_file_value()?;
        let header = self.parse_bounded_header(&value)?;

        if let Some(expected_mac) = self.file_integrity_mac {
            if expected_mac.ct_ne(&header.file_integrity_mac).into() {
                return Err(KeystoreError::InvalidInput(
                    "Concurrent modification detected while unlocking keystore".to_string(),
                ));
            }
        }
        let stored_mac = header.file_integrity_mac;
        let mac_preimage = Self::canonical_file_mac_preimage_from_value(value.clone())?;
        let computed_mac = self.compute_file_integrity_mac(master_key, &mac_preimage)?;

        if computed_mac.ct_ne(&stored_mac).into() {
            return Err(KeystoreError::InvalidInput(
                "File integrity verification failed - keystore may have been tampered with"
                    .to_string(),
            ));
        }

        let keystore_file: KeystoreFile = serde_json::from_value(value).map_err(|_| {
            KeystoreError::InvalidInput(
                "Malformed authenticated keystore payload - likely corrupted".to_string(),
            )
        })?;
        self.validate_keystore_shape(&keystore_file)?;

        Ok(keystore_file)
    }

    fn read_keystore_file_value(&self) -> Result<serde_json::Value, KeystoreError> {
        let data = fs::read(&self.path)
            .map_err(|_| KeystoreError::InvalidInput("Cannot read keystore file".to_string()))?;
        Self::parse_keystore_file_value(&data)
    }

    fn parse_keystore_file_value(data: &[u8]) -> Result<serde_json::Value, KeystoreError> {
        if data.len() > MAX_KEYSTORE_SIZE {
            return Err(KeystoreError::InvalidInput(
                "Keystore file too large - possible DoS attempt".to_string(),
            ));
        }

        if data.len() < MIN_KEYSTORE_SIZE {
            return Err(KeystoreError::InvalidInput(
                "Keystore file too small - likely corrupted".to_string(),
            ));
        }

        let value: serde_json::Value = serde_json::from_slice(data).map_err(|_| {
            KeystoreError::InvalidInput("Malformed keystore file - invalid JSON".to_string())
        })?;
        Self::validate_top_level_file_shape(&value)?;
        Ok(value)
    }

    fn validate_top_level_file_shape(
        value: &serde_json::Value,
    ) -> Result<&serde_json::Map<String, serde_json::Value>, KeystoreError> {
        let object = value.as_object().ok_or_else(|| {
            KeystoreError::InvalidInput("Keystore file must be a JSON object".to_string())
        })?;

        for key in object.keys() {
            if !KEYSTORE_FILE_FIELDS.contains(&key.as_str()) {
                return Err(KeystoreError::InvalidInput(format!(
                    "Unknown keystore file field: {key}"
                )));
            }
        }

        for key in KEYSTORE_FILE_FIELDS {
            if !object.contains_key(*key) {
                return Err(KeystoreError::InvalidInput(format!(
                    "Missing keystore file field: {key}"
                )));
            }
        }

        Ok(object)
    }

    fn parse_bounded_header(
        &self,
        value: &serde_json::Value,
    ) -> Result<KeystoreHeader, KeystoreError> {
        let object = Self::validate_top_level_file_shape(value)?;
        let header_value = serde_json::json!({
            "version": object.get("version").cloned().unwrap_or(serde_json::Value::Null),
            "kdf_params": object.get("kdf_params").cloned().unwrap_or(serde_json::Value::Null),
            "master_key_verification": object
                .get("master_key_verification")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "file_integrity_mac": object
                .get("file_integrity_mac")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        });
        let header: KeystoreHeader = serde_json::from_value(header_value).map_err(|_| {
            KeystoreError::InvalidInput(
                "Malformed keystore header - invalid KDF or MAC fields".to_string(),
            )
        })?;

        if header.version != KEYSTORE_FILE_VERSION {
            return Err(KeystoreError::InvalidInput(format!(
                "Unsupported keystore version: {}",
                header.version
            )));
        }
        self.validate_kdf_params(&header.kdf_params)?;
        Ok(header)
    }

    pub(super) fn validate_kdf_params(&self, params: &ArgonParams) -> Result<(), KeystoreError> {
        if params.memory_kb < MIN_KDF_MEMORY_KB || params.memory_kb > self.config.argon2_memory_kb {
            return Err(KeystoreError::InvalidInput(
                "KDF memory cost out of bounds".to_string(),
            ));
        }
        if params.iterations < MIN_KDF_ITERATIONS
            || params.iterations > self.config.argon2_iterations
        {
            return Err(KeystoreError::InvalidInput(
                "KDF iteration count out of bounds".to_string(),
            ));
        }
        if params.parallelism < MIN_KDF_PARALLELISM
            || params.parallelism > self.config.argon2_parallelism
        {
            return Err(KeystoreError::InvalidInput(
                "KDF parallelism out of bounds".to_string(),
            ));
        }
        Ok(())
    }

    fn canonical_file_mac_preimage_from_file(
        keystore_file: &KeystoreFile,
    ) -> Result<Vec<u8>, KeystoreError> {
        let value = serde_json::to_value(keystore_file)?;
        Self::canonical_file_mac_preimage_from_value(value)
    }

    pub(super) fn canonical_file_mac_preimage_from_value(
        mut value: serde_json::Value,
    ) -> Result<Vec<u8>, KeystoreError> {
        let object = value.as_object_mut().ok_or_else(|| {
            KeystoreError::InvalidInput("Keystore file must be a JSON object".to_string())
        })?;
        object.insert(
            "file_integrity_mac".to_string(),
            serde_json::to_value([0u8; 32])?,
        );
        Ok(serde_json::to_vec(&value)?)
    }

    fn ensure_target_path_is_safe(&self) -> Result<(), KeystoreError> {
        if !self.path.exists() {
            return Ok(());
        }

        let metadata = fs::symlink_metadata(&self.path)?;
        if metadata.file_type().is_symlink() {
            return Err(KeystoreError::InvalidInput(
                "Refusing to use symlinked keystore path".to_string(),
            ));
        }
        if !metadata.file_type().is_file() {
            return Err(KeystoreError::InvalidInput(
                "Refusing to use non-regular keystore path".to_string(),
            ));
        }

        Ok(())
    }

    fn ensure_parent_directory_safe(&self) -> Result<PathBuf, KeystoreError> {
        let parent = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        if parent.exists() {
            let metadata = fs::symlink_metadata(&parent)?;
            if metadata.file_type().is_symlink() {
                return Err(KeystoreError::InvalidInput(
                    "Refusing to use symlinked parent directory".to_string(),
                ));
            }
            if !metadata.file_type().is_dir() {
                return Err(KeystoreError::InvalidInput(
                    "Keystore parent path must be a directory".to_string(),
                ));
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();
                let world_writable = (mode & 0o002) != 0;
                let sticky = (mode & 0o1000) != 0;
                if world_writable && !sticky {
                    return Err(KeystoreError::InvalidInput(
                        "Refusing unsafe parent directory permissions".to_string(),
                    ));
                }
            }
        } else {
            fs::create_dir_all(&parent)?;
            Self::set_restrictive_permissions_for_directory(&parent)?;
        }

        Ok(parent)
    }

    fn validate_keystore_shape(&self, keystore_file: &KeystoreFile) -> Result<(), KeystoreError> {
        if keystore_file.entries.len() > MAX_ENTRIES {
            return Err(KeystoreError::InvalidInput(
                "Too many entries - possible DoS attempt".to_string(),
            ));
        }
        if keystore_file.audit_log.len() > MAX_AUDIT_LOG_ENTRIES {
            return Err(KeystoreError::InvalidInput(
                "Audit log too large - possible DoS attempt".to_string(),
            ));
        }
        if keystore_file
            .entries
            .iter()
            .any(|e| e.encrypted_data.len() > MAX_ENTRY_DATA)
        {
            return Err(KeystoreError::InvalidInput(
                "Entry too large – likely corrupted".to_string(),
            ));
        }

        let mut ids = HashSet::with_capacity(keystore_file.entries.len());
        for entry in &keystore_file.entries {
            if !ids.insert(entry.id) {
                return Err(KeystoreError::InvalidInput(
                    "Duplicate key entry id detected".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn mutation_lock_path(&self, parent: &Path) -> PathBuf {
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("keystore");
        parent.join(format!(".{file_name}.lock"))
    }

    fn acquire_mutation_lock(&self, parent: &Path) -> Result<MutationLockGuard, KeystoreError> {
        let lock_path = self.mutation_lock_path(parent);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(MutationLockGuard { file })
    }

    fn concurrent_modification_error() -> KeystoreError {
        KeystoreError::InvalidInput(
            "Concurrent modification detected while writing keystore".to_string(),
        )
    }

    pub(super) fn verify_current_file_matches_memory_mac(
        &self,
        master_key: &[u8; 32],
    ) -> Result<(), KeystoreError> {
        if !self.path.exists() {
            if self.file_integrity_mac.is_some() {
                return Err(Self::concurrent_modification_error());
            }
            return Ok(());
        }

        let expected_mac = self
            .file_integrity_mac
            .ok_or_else(Self::concurrent_modification_error)?;

        let data = fs::read(&self.path).map_err(|_| Self::concurrent_modification_error())?;
        let value = Self::parse_keystore_file_value(&data)
            .map_err(|_| Self::concurrent_modification_error())?;
        let header = self
            .parse_bounded_header(&value)
            .map_err(|_| Self::concurrent_modification_error())?;
        let current_file: KeystoreFile = serde_json::from_value(value.clone())
            .map_err(|_| Self::concurrent_modification_error())?;
        self.validate_keystore_shape(&current_file)
            .map_err(|_| Self::concurrent_modification_error())?;

        let stored_mac = header.file_integrity_mac;
        let mac_preimage = Self::canonical_file_mac_preimage_from_value(value)
            .map_err(|_| Self::concurrent_modification_error())?;
        let computed_mac = self
            .compute_file_integrity_mac(master_key, &mac_preimage)
            .map_err(|_| Self::concurrent_modification_error())?;

        let authenticated_match = computed_mac.ct_eq(&stored_mac)
            & computed_mac.ct_eq(&expected_mac)
            & stored_mac.ct_eq(&expected_mac);
        if !bool::from(authenticated_match) {
            return Err(Self::concurrent_modification_error());
        }

        Ok(())
    }

    fn atomic_write_keystore_file(&self, parent: &Path, data: &[u8]) -> Result<(), KeystoreError> {
        if self.path.exists() {
            self.ensure_target_path_is_safe()?;
        }

        #[cfg(test)]
        if self.fail_next_write.replace(false) {
            return Err(KeystoreError::FileError(
                "injected keystore write failure".to_string(),
            ));
        }

        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("keystore");
        let tmp_path = parent.join(format!(".{file_name}.tmp-{}", Uuid::new_v4()));

        let write_result = (|| -> Result<(), KeystoreError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)?;
            Self::set_restrictive_permissions_for_file(&file)?;

            file.write_all(data)?;
            file.sync_all()?;
            drop(file);

            fs::rename(&tmp_path, &self.path)?;
            Self::set_restrictive_permissions_for_path(&self.path)?;

            #[cfg(unix)]
            {
                let parent_dir = OpenOptions::new().read(true).open(parent)?;
                parent_dir.sync_all()?;
            }
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_result
    }

    #[cfg(unix)]
    fn set_restrictive_permissions_for_file(file: &std::fs::File) -> Result<(), KeystoreError> {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_restrictive_permissions_for_file(_file: &std::fs::File) -> Result<(), KeystoreError> {
        Ok(())
    }

    #[cfg(unix)]
    fn set_restrictive_permissions_for_path(path: &Path) -> Result<(), KeystoreError> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_restrictive_permissions_for_path(_path: &Path) -> Result<(), KeystoreError> {
        Ok(())
    }

    #[cfg(unix)]
    fn set_restrictive_permissions_for_directory(path: &Path) -> Result<(), KeystoreError> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_restrictive_permissions_for_directory(_path: &Path) -> Result<(), KeystoreError> {
        Ok(())
    }
}
