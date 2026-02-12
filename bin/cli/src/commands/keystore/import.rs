use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{input, keystore_manager::KeystoreManager};
use clap::Args;
use serde::Serialize;
use std::fmt;
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

    /// Optional mnemonic passphrase (BIP39). Note: providing it via CLI may expose it in shell history.
    #[arg(long)]
    pub passphrase: Option<String>,

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
pub struct ImportResponse {
    id: String,
    label: String,
    key_type: String,
    address: String,
    created_at: String,
}

impl fmt::Display for ImportResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} imported successfully with ID: {}",
            self.key_type, self.id
        )
    }
}

pub async fn execute(ctx: &CommandContext, args: &ImportArgs) -> ! {
    let result = execute_internal(ctx, args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(
    _ctx: &CommandContext,
    args: &ImportArgs,
) -> CommandResult<ImportResponse> {
    // Read and validate input BEFORE creating/unlocking keystore
    match args.import_type {
        ImportType::PrivateKey => {
            let private_key = if args.stdin {
                input::read_input("", true)
                    .map_err(|e| CommandError::new("InputError", e.to_string()))?
            } else {
                input::read_input("Enter private key (hex): ", false)
                    .map_err(|e| CommandError::new("InputError", e.to_string()))?
            };

            // Validate private key format EARLY
            let private_key = private_key.strip_prefix("0x").unwrap_or(&private_key);
            if private_key.len() != 64 {
                return Err(CommandError::new(
                    "InvalidPrivateKey",
                    "Private key must be 64 hex characters",
                ));
            }

            // Check if it's valid hex
            if hex::decode(private_key).is_err() {
                return Err(CommandError::new(
                    "InvalidPrivateKey",
                    "Private key must be valid hexadecimal",
                ));
            }

            // Only create keystore after validation passes
            let manager = KeystoreManager::new(args.keystore.clone());
            let mut keystore = manager
                .create_keystore_if_needed()
                .await
                .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

            let label = args.label.clone().unwrap_or_else(|| {
                format!(
                    "imported-key-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                )
            });

            let key_id = keystore
                .import_private_key(Some(label.clone()), private_key)
                .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

            // Get the imported key info
            let keys = keystore
                .list_keys()
                .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;
            let key_info = keys.iter().find(|k| k.id == key_id).ok_or_else(|| {
                CommandError::key_not_found("Failed to retrieve imported key info")
            })?;

            let response = ImportResponse {
                id: key_info.id.to_string(),
                label: key_info.alias.clone().unwrap_or_else(|| "".to_string()),
                key_type: "private key".to_string(),
                address: format!("{:?}", key_info.address),
                created_at: key_info.created_at.to_rfc3339(),
            };

            Ok(CommandOutput::new(response))
        }
        ImportType::Mnemonic => {
            let mnemonic = if args.stdin {
                input::read_input("", true)
                    .map_err(|e| CommandError::new("InputError", e.to_string()))?
            } else {
                input::read_input("Enter mnemonic phrase: ", false)
                    .map_err(|e| CommandError::new("InputError", e.to_string()))?
            };

            // Basic mnemonic validation EARLY
            if mnemonic.split_whitespace().count() < 12 {
                return Err(CommandError::new(
                    "InvalidMnemonic",
                    "Mnemonic must have at least 12 words",
                ));
            }

            // Only create keystore after validation passes
            let manager = KeystoreManager::new(args.keystore.clone());
            let mut keystore = manager
                .create_keystore_if_needed()
                .await
                .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

            let label = args.label.clone().unwrap_or_else(|| {
                format!(
                    "imported-mnemonic-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                )
            });

            let key_id = keystore
                .import_mnemonic(
                    Some(label.clone()),
                    &mnemonic,
                    &args.derivation_path,
                    args.passphrase.as_deref(),
                )
                .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

            // Get the imported key info
            let keys = keystore
                .list_keys()
                .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;
            let key_info = keys.iter().find(|k| k.id == key_id).ok_or_else(|| {
                CommandError::key_not_found("Failed to retrieve imported key info")
            })?;

            let response = ImportResponse {
                id: key_info.id.to_string(),
                label: key_info.alias.clone().unwrap_or_else(|| "".to_string()),
                key_type: "mnemonic".to_string(),
                address: format!("{:?}", key_info.address),
                created_at: key_info.created_at.to_rfc3339(),
            };

            Ok(CommandOutput::new(response))
        }
    }
}
