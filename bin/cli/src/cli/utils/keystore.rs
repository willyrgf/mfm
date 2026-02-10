use mfm_core::keystore::{Keystore, KeystoreConfig};
use std::path::PathBuf;

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

        // Get password from environment variable or prompt
        let password = if let Ok(env_password) = std::env::var("MFM_KEYSTORE_PASSWORD") {
            env_password
        } else {
            super::input::read_password("Enter keystore password: ")?
        };

        // Unlock
        keystore.unlock(&password)?;
        Ok(keystore)
    }

    /// Create new keystore if it doesn't exist
    pub async fn create_keystore_if_needed(&self) -> Result<Keystore, Box<dyn std::error::Error>> {
        if self.keystore_path.exists() {
            return self.get_unlocked_keystore().await;
        }

        println!("Creating new keystore at: {}", self.keystore_path.display());

        // Create parent directory if needed
        if let Some(parent) = self.keystore_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Get password from environment variable or prompt
        let password = if let Ok(env_password) = std::env::var("MFM_KEYSTORE_PASSWORD") {
            env_password
        } else {
            let password = super::input::read_password("Enter password for new keystore: ")?;
            let confirm_password = super::input::read_password("Confirm password: ")?;

            if password != confirm_password {
                return Err("Passwords do not match".into());
            }
            password
        };

        // Create keystore with fast config for integration tests
        let config = if std::env::var("MFM_INTEGRATION_TEST").is_ok() {
            KeystoreConfig::insecure_integration_test()
        } else {
            KeystoreConfig::default()
        };
        let mut keystore = Keystore::new_with_config(&self.keystore_path, config)?;
        keystore.unlock(&password)?;
        Ok(keystore)
    }
}
