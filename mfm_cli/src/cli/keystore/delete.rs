use crate::cli::utils::{input, keystore::KeystoreManager, output};
use crate::cli::OutputFormat;
use clap::Args;
use serde::Serialize;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Args)]
pub struct DeleteArgs {
    /// Key ID (UUID) to delete
    pub id: Option<String>,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Delete by label instead of ID
    #[arg(long)]
    pub by_label: Option<String>,
}

#[derive(Serialize)]
struct DeleteResponse {
    id: String,
    label: String,
}

pub async fn execute(
    args: &DeleteArgs,
    output_format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let manager = KeystoreManager::new(args.keystore.clone());
    let mut keystore = manager.get_unlocked_keystore().await?;

    // Determine which key to delete
    let key_id = if let Some(label) = &args.by_label {
        // Find key by label
        let keys = keystore.list_keys()?;
        let matching_keys: Vec<_> = keys
            .iter()
            .filter(|k| k.alias.as_ref() == Some(label))
            .collect();

        match matching_keys.len() {
            0 => match output_format {
                OutputFormat::Text => {
                    return Err(format!("No key found with label: {label}").into())
                }
                OutputFormat::Json => {
                    output::print_error(
                        "KeyNotFound",
                        &format!("No key found with label: {label}"),
                        output_format,
                    );
                    std::process::exit(1);
                }
            },
            1 => matching_keys[0].id,
            _ => match output_format {
                OutputFormat::Text => {
                    println!("Multiple keys found with label '{label}'. Please use ID instead:");
                    for key in matching_keys {
                        println!(
                            "  ID: {} ({})",
                            key.id,
                            key.alias.as_ref().unwrap_or(&"<no alias>".to_string())
                        );
                    }
                    return Err("Multiple keys found with same label".into());
                }
                OutputFormat::Json => {
                    output::print_error(
                        "AmbiguousLabel",
                        &format!("Multiple keys found with label: {label}"),
                        output_format,
                    );
                    std::process::exit(1);
                }
            },
        }
    } else if let Some(id_str) = &args.id {
        // Parse UUID
        match Uuid::parse_str(id_str) {
            Ok(id) => id,
            Err(_) => match output_format {
                OutputFormat::Text => return Err("Invalid UUID format".into()),
                OutputFormat::Json => {
                    output::print_error("InvalidUuid", "Invalid UUID format", output_format);
                    std::process::exit(1);
                }
            },
        }
    } else {
        match output_format {
            OutputFormat::Text => return Err("Must specify either key ID or --by-label".into()),
            OutputFormat::Json => {
                output::print_error(
                    "MissingArgument",
                    "Must specify either key ID or --by-label",
                    output_format,
                );
                std::process::exit(1);
            }
        }
    };

    // Find the key to confirm deletion
    let keys = keystore.list_keys()?;
    let key_to_delete = match keys.iter().find(|k| k.id == key_id) {
        Some(key) => key,
        None => match output_format {
            OutputFormat::Text => return Err("Key not found".into()),
            OutputFormat::Json => {
                output::print_error("KeyNotFound", "Key not found", output_format);
                std::process::exit(1);
            }
        },
    };

    // Confirm deletion unless --yes flag is used
    if !args.yes {
        let prompt = format!(
            "Are you sure you want to delete key '{}' (ID: {})?",
            key_to_delete
                .alias
                .as_ref()
                .unwrap_or(&"<no alias>".to_string()),
            key_to_delete.id
        );

        if !input::confirm(&prompt)? {
            match output_format {
                OutputFormat::Text => println!("Deletion cancelled"),
                OutputFormat::Json => {
                    output::print_error(
                        "OperationCancelled",
                        "Deletion cancelled by user",
                        output_format,
                    );
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
    }

    // Delete the key
    keystore.delete_key(key_id)?;

    match output_format {
        OutputFormat::Text => {
            println!(
                "Key '{}' deleted successfully",
                key_to_delete
                    .alias
                    .as_ref()
                    .unwrap_or(&"<no alias>".to_string())
            );
        }
        OutputFormat::Json => {
            let response = DeleteResponse {
                id: key_to_delete.id.to_string(),
                label: key_to_delete
                    .alias
                    .clone()
                    .unwrap_or_else(|| "".to_string()),
            };
            output::print_success(response, output_format);
        }
    }

    Ok(())
}
