use crate::commands::result::CommandError;
use mfm_op_keystore::{Keystore, KeystoreConfig};
use std::path::PathBuf;
use uuid::Uuid;
use zeroize::Zeroizing;

const ENV_PASSWORD_FILE: &str = "MFM_KEYSTORE_PASSWORD_FILE";
const ENV_PASSWORD: &str = "MFM_KEYSTORE_PASSWORD";
use tracing::info;

/// Keystore operations wrapper with unlock handling
pub struct KeystoreManager {
    keystore_path: PathBuf,
}

impl KeystoreManager {
    pub fn new(keystore_path: Option<PathBuf>) -> Self {
        let keystore_path = keystore_path
            .or_else(|| {
                // Check for environment variable
                std::env::var("MFM_KEYSTORE_PATH").ok().map(PathBuf::from)
            })
            .unwrap_or_else(|| {
                // Default to ~/.mfm/keystore
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".mfm").join("keystore")
            });

        Self { keystore_path }
    }

    /// Get unlocked keystore, prompting for password if needed
    pub async fn get_unlocked_keystore(&self) -> Result<Keystore, Box<dyn std::error::Error>> {
        // Check if keystore exists
        if !self.keystore_path.exists() {
            return Err(format!("Keystore not found at: {}", self.keystore_path.display()).into());
        }

        // Load keystore
        let mut keystore = Keystore::new(&self.keystore_path)?;

        let password = self.get_unlock_password()?;

        // Unlock
        keystore.unlock(password.as_str())?;
        Ok(keystore)
    }

    /// Create new keystore if it doesn't exist
    pub async fn create_keystore_if_needed(&self) -> Result<Keystore, Box<dyn std::error::Error>> {
        if self.keystore_path.exists() {
            return self.get_unlocked_keystore().await;
        }

        info!(
            keystore_path = %self.keystore_path.display(),
            "creating new keystore file"
        );

        // Create parent directory if needed
        if let Some(parent) = self.keystore_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let password = self.get_create_password()?;

        // Create keystore with fast config for integration tests
        let config = if std::env::var("MFM_INTEGRATION_TEST").is_ok() {
            KeystoreConfig::insecure_integration_test()
        } else {
            KeystoreConfig::default()
        };
        let mut keystore = Keystore::new_with_config(&self.keystore_path, config)?;
        keystore.unlock(password.as_str())?;
        Ok(keystore)
    }

    fn get_unlock_password(&self) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
        if let Some(password) = self.password_from_env_sources()? {
            return Ok(password);
        }

        super::input::read_password("Enter keystore password: ")
    }

    fn get_create_password(&self) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
        if let Some(password) = self.password_from_env_sources()? {
            return Ok(password);
        }

        let password = super::input::read_password("Enter password for new keystore: ")?;
        let confirm_password = super::input::read_password("Confirm password: ")?;
        if password.as_str() != confirm_password.as_str() {
            return Err("Passwords do not match".into());
        }

        Ok(password)
    }

    fn password_from_env_sources(
        &self,
    ) -> Result<Option<Zeroizing<String>>, Box<dyn std::error::Error>> {
        if let Ok(password_file) = std::env::var(ENV_PASSWORD_FILE) {
            return Ok(Some(Self::read_password_file(&password_file)?));
        }

        if let Ok(password) = std::env::var(ENV_PASSWORD) {
            tracing::warn!(
                "{ENV_PASSWORD} may expose secrets; prefer {ENV_PASSWORD_FILE} for non-interactive use"
            );
            return Ok(Some(Zeroizing::new(password)));
        }

        Ok(None)
    }

    fn read_password_file(path: &str) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
        let raw = Zeroizing::new(std::fs::read_to_string(path)?);
        let trimmed = raw.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return Err(format!("Password file at '{path}' was empty").into());
        }
        Ok(Zeroizing::new(trimmed.to_string()))
    }

    pub fn resolve_key_id(
        keystore: &Keystore,
        id: Option<&str>,
        by_label: Option<&str>,
    ) -> Result<Uuid, CommandError> {
        match (id, by_label) {
            (Some(_), Some(_)) => Err(CommandError::missing_argument(
                "Specify exactly one key selector: --id or --by-label",
            )),
            (None, None) => Err(CommandError::missing_argument(
                "Must specify one key selector: --id or --by-label",
            )),
            (Some(raw), None) => {
                Uuid::parse_str(raw).map_err(|_| CommandError::invalid_uuid("Invalid UUID format"))
            }
            (None, Some(label)) => {
                let keys = keystore
                    .list_keys()
                    .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;
                let matching_keys: Vec<_> = keys
                    .iter()
                    .filter(|k| k.alias.as_deref() == Some(label))
                    .collect();

                match matching_keys.len() {
                    0 => Err(CommandError::key_not_found(format!(
                        "No key found with label: {label}"
                    ))),
                    1 => Ok(matching_keys[0].id),
                    _ => Err(CommandError::ambiguous_label(format!(
                        "Multiple keys found with label: {label}"
                    ))),
                }
            }
        }
    }
}
