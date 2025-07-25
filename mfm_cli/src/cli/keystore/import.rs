use crate::cli::utils::{input, keystore::KeystoreManager, output};
use crate::cli::OutputFormat;
use clap::Args;
use serde::Serialize;
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

#[derive(Serialize)]
struct ImportResponse {
    id: String,
    label: String,
    key_type: String,
    address: String,
    created_at: String,
}

pub async fn execute(
    args: &ImportArgs,
    output_format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
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
                match output_format {
                    OutputFormat::Text => {
                        return Err("Private key must be 64 hex characters".into())
                    }
                    OutputFormat::Json => {
                        output::print_error(
                            "InvalidPrivateKey",
                            "Private key must be 64 hex characters",
                            output_format,
                        );
                        std::process::exit(1);
                    }
                }
            }

            // Check if it's valid hex
            if hex::decode(private_key).is_err() {
                match output_format {
                    OutputFormat::Text => {
                        return Err("Private key must be valid hexadecimal".into())
                    }
                    OutputFormat::Json => {
                        output::print_error(
                            "InvalidPrivateKey",
                            "Private key must be valid hexadecimal",
                            output_format,
                        );
                        std::process::exit(1);
                    }
                }
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

            let key_id = keystore.import_private_key(Some(label.clone()), private_key)?;

            match output_format {
                OutputFormat::Text => {
                    println!("Private key imported successfully with ID: {key_id}");
                }
                OutputFormat::Json => {
                    // Get the imported key info
                    let keys = keystore.list_keys()?;
                    if let Some(key_info) = keys.iter().find(|k| k.id == key_id) {
                        let response = ImportResponse {
                            id: key_info.id.to_string(),
                            label: key_info.alias.clone().unwrap_or_else(|| "".to_string()),
                            key_type: format!("{:?}", key_info.key_type).to_lowercase(),
                            address: format!("{:?}", key_info.address),
                            created_at: key_info.created_at.to_rfc3339(),
                        };
                        output::print_success(response, output_format);
                    } else {
                        output::print_error(
                            "KeyNotFound",
                            "Failed to retrieve imported key info",
                            output_format,
                        );
                    }
                }
            }
        }
        ImportType::Mnemonic => {
            let mnemonic = if args.stdin {
                input::read_input("", true)?
            } else {
                input::read_input("Enter mnemonic phrase: ", false)?
            };

            // Basic mnemonic validation EARLY
            if mnemonic.split_whitespace().count() < 12 {
                match output_format {
                    OutputFormat::Text => return Err("Mnemonic must have at least 12 words".into()),
                    OutputFormat::Json => {
                        output::print_error(
                            "InvalidMnemonic",
                            "Mnemonic must have at least 12 words",
                            output_format,
                        );
                        std::process::exit(1);
                    }
                }
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

            let key_id =
                keystore.import_mnemonic(Some(label.clone()), &mnemonic, &args.derivation_path)?;

            match output_format {
                OutputFormat::Text => {
                    println!("Mnemonic imported successfully with ID: {key_id}");
                }
                OutputFormat::Json => {
                    // Get the imported key info
                    let keys = keystore.list_keys()?;
                    if let Some(key_info) = keys.iter().find(|k| k.id == key_id) {
                        let response = ImportResponse {
                            id: key_info.id.to_string(),
                            label: key_info.alias.clone().unwrap_or_else(|| "".to_string()),
                            key_type: format!("{:?}", key_info.key_type).to_lowercase(),
                            address: format!("{:?}", key_info.address),
                            created_at: key_info.created_at.to_rfc3339(),
                        };
                        output::print_success(response, output_format);
                    } else {
                        output::print_error(
                            "KeyNotFound",
                            "Failed to retrieve imported key info",
                            output_format,
                        );
                    }
                }
            }
        }
    }

    Ok(())
}
