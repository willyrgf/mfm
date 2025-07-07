use crate::cli::utils::{input, keystore::KeystoreManager};
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub struct ImportArgs {
    /// Import type: privatekey, mnemonic
    #[arg(short = 't', long, value_enum)]
    pub import_type: ImportType,

    /// Human-readable label for the key
    #[arg(short, long)]
    pub label: Option<String>,

    /// Derivation path for mnemonic (default: m/44'/60'/0'/0/0)
    #[arg(short = 'p', long, default_value = "m/44'/60'/0'/0/0")]
    pub derivation_path: String,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Interactive input mode (default if --stdin not specified)
    #[arg(short, long)]
    pub interactive: bool,

    /// Read from stdin
    #[arg(long)]
    pub stdin: bool,
}

#[derive(clap::ValueEnum, Clone)]
pub enum ImportType {
    #[value(name = "privatekey", alias = "private-key")]
    PrivateKey,
    #[value(name = "mnemonic")]
    Mnemonic,
}

pub async fn execute(args: &ImportArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Read and validate input BEFORE creating/unlocking keystore
    match args.import_type {
        ImportType::PrivateKey => {
            let private_key = if args.stdin {
                input::read_input("", true)?
            } else {
                input::read_input("Enter private key (hex): ", false)?
            };

            // Validate private key format EARLY
            let private_key = private_key.strip_prefix("0x").unwrap_or(&private_key);
            if private_key.len() != 64 {
                return Err("Private key must be 64 hex characters".into());
            }

            // Check if it's valid hex
            if hex::decode(private_key).is_err() {
                return Err("Private key must be valid hexadecimal".into());
            }

            // Only create keystore after validation passes
            let manager = KeystoreManager::new(args.keystore.clone());
            let mut keystore = manager.create_keystore_if_needed().await?;

            let label = args.label.clone().unwrap_or_else(|| {
                format!(
                    "imported-key-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                )
            });

            let key_id = keystore.import_private_key(Some(label), private_key)?;
            println!("Private key imported successfully with ID: {key_id}");
        }
        ImportType::Mnemonic => {
            let mnemonic = if args.stdin {
                input::read_input("", true)?
            } else {
                input::read_input("Enter mnemonic phrase: ", false)?
            };

            // Basic mnemonic validation EARLY
            if mnemonic.split_whitespace().count() < 12 {
                return Err("Mnemonic must have at least 12 words".into());
            }

            // Only create keystore after validation passes
            let manager = KeystoreManager::new(args.keystore.clone());
            let mut keystore = manager.create_keystore_if_needed().await?;

            let label = args.label.clone().unwrap_or_else(|| {
                format!(
                    "imported-mnemonic-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                )
            });

            let key_id = keystore.import_mnemonic(Some(label), &mnemonic, &args.derivation_path)?;
            println!("Mnemonic imported successfully with ID: {key_id}");
        }
    }

    Ok(())
}
