use crate::cli::command_result::{CommandError, CommandOutput, CommandResult};
use crate::cli::utils::{input, keystore::KeystoreManager, output::handle_command_result};
use crate::cli::CommandContext;
use clap::Args;
use serde::Serialize;
use std::fmt;
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
pub struct DeleteResponse {
    id: String,
    label: String,
}

impl fmt::Display for DeleteResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key '{}' deleted successfully", self.label)
    }
}

pub async fn execute(ctx: &CommandContext, args: &DeleteArgs) -> ! {
    let result = execute_internal(ctx, args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(
    _ctx: &CommandContext,
    args: &DeleteArgs,
) -> CommandResult<DeleteResponse> {
    let manager = KeystoreManager::new(args.keystore.clone());
    let mut keystore = manager
        .get_unlocked_keystore()
        .await
        .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

    // Determine which key to delete
    let key_id = if let Some(label) = &args.by_label {
        // Find key by label
        let keys = keystore
            .list_keys()
            .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;
        let matching_keys: Vec<_> = keys
            .iter()
            .filter(|k| k.alias.as_ref() == Some(label))
            .collect();

        match matching_keys.len() {
            0 => {
                return Err(CommandError::key_not_found(format!(
                    "No key found with label: {label}"
                )))
            }
            1 => matching_keys[0].id,
            _ => {
                return Err(CommandError::ambiguous_label(format!(
                    "Multiple keys found with label: {label}"
                )))
            }
        }
    } else if let Some(id_str) = &args.id {
        // Parse UUID
        Uuid::parse_str(id_str).map_err(|_| CommandError::invalid_uuid("Invalid UUID format"))?
    } else {
        return Err(CommandError::missing_argument(
            "Must specify either key ID or --by-label",
        ));
    };

    // Find the key to confirm deletion
    let keys = keystore
        .list_keys()
        .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;
    let key_to_delete = keys
        .iter()
        .find(|k| k.id == key_id)
        .ok_or_else(|| CommandError::key_not_found("Key not found"))?;

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

        if !input::confirm(&prompt).map_err(|e| CommandError::new("InputError", e.to_string()))? {
            return Err(CommandError::operation_cancelled(
                "Deletion cancelled by user",
            ));
        }
    }

    // Delete the key
    keystore
        .delete_key(key_id)
        .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

    let response = DeleteResponse {
        id: key_to_delete.id.to_string(),
        label: key_to_delete
            .alias
            .clone()
            .unwrap_or_else(|| "".to_string()),
    };

    Ok(CommandOutput::new(response))
}
