use crate::cli::utils::{input, keystore::KeystoreManager};
use clap::Args;
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

pub async fn execute(args: &DeleteArgs) -> Result<(), Box<dyn std::error::Error>> {
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
            0 => return Err(format!("No key found with label: {label}").into()),
            1 => matching_keys[0].id,
            _ => {
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
        }
    } else if let Some(id_str) = &args.id {
        // Parse UUID
        match Uuid::parse_str(id_str) {
            Ok(id) => id,
            Err(_) => return Err("Invalid UUID format".into()),
        }
    } else {
        return Err("Must specify either key ID or --by-label".into());
    };

    // Find the key to confirm deletion
    let keys = keystore.list_keys()?;
    let key_to_delete = keys
        .iter()
        .find(|k| k.id == key_id)
        .ok_or("Key not found")?;

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
            println!("Deletion cancelled");
            return Ok(());
        }
    }

    // Delete the key
    keystore.delete_key(key_id)?;
    println!(
        "Key '{}' deleted successfully",
        key_to_delete
            .alias
            .as_ref()
            .unwrap_or(&"<no alias>".to_string())
    );

    Ok(())
}
