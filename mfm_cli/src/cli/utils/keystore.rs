use mfm_core::keystore::Keystore;
use std::path::PathBuf;

/// Keystore operations wrapper with unlock handling
pub struct KeystoreManager {
    keystore_path: PathBuf,
}

impl KeystoreManager {
    pub fn new(keystore_path: Option<PathBuf>) -> Self {
        let keystore_path = keystore_path.unwrap_or_else(|| {
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

        // Prompt for password
        let password = super::input::read_password("Enter keystore password: ")?;

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

        // Get password
        let password = super::input::read_password("Enter password for new keystore: ")?;
        let confirm_password = super::input::read_password("Confirm password: ")?;

        if password != confirm_password {
            return Err("Passwords do not match".into());
        }

        // Create keystore
        let mut keystore = Keystore::new(&self.keystore_path)?;
        keystore.unlock(&password)?;
        Ok(keystore)
    }
}
