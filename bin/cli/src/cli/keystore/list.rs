use crate::cli::command_result::{CommandError, CommandOutput, CommandResult};
use crate::cli::utils::{
    keystore::KeystoreManager,
    output::{format_keys_table, handle_command_result, KeyDisplay},
};
use crate::cli::CommandContext;
use clap::Args;
use regex::Regex;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

#[derive(Args)]
pub struct ListArgs {
    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Include Ethereum addresses in output
    #[arg(long, default_value = "true")]
    pub show_addresses: bool,

    /// Filter by label pattern (regex supported)
    #[arg(long)]
    pub filter_label: Option<String>,

    /// Sort by: label, created, type
    #[arg(long, default_value = "created")]
    pub sort_by: SortBy,
}

#[derive(clap::ValueEnum, Clone)]
pub enum SortBy {
    #[value(name = "label")]
    Label,
    #[value(name = "created")]
    Created,
    #[value(name = "type")]
    Type,
}

#[derive(Serialize)]
pub struct ListResponse {
    keys: Vec<KeyDisplay>,
    show_addresses: bool,
}

impl fmt::Display for ListResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.keys.is_empty() {
            write!(f, "No keys found in keystore")
        } else {
            write!(f, "{}", format_keys_table(&self.keys, self.show_addresses))
        }
    }
}

pub async fn execute(ctx: &CommandContext, args: &ListArgs) -> ! {
    let result = execute_internal(ctx, args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(_ctx: &CommandContext, args: &ListArgs) -> CommandResult<ListResponse> {
    let manager = KeystoreManager::new(args.keystore.clone());
    let keystore = manager
        .get_unlocked_keystore()
        .await
        .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

    let keys = keystore
        .list_keys()
        .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

    if keys.is_empty() {
        return Ok(CommandOutput::new(ListResponse {
            keys: Vec::new(),
            show_addresses: args.show_addresses,
        }));
    }

    let mut key_displays: Vec<KeyDisplay> = keys
        .into_iter()
        .map(|key| KeyDisplay {
            id: key.id.to_string(),
            label: key
                .alias
                .clone()
                .unwrap_or_else(|| "<no alias>".to_string()),
            key_type: format!("{:?}", key.key_type).to_lowercase(),
            address: if args.show_addresses {
                Some(format!("{:?}", key.address))
            } else {
                None
            },
            created: key.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    // Apply label filter if specified
    if let Some(filter_pattern) = &args.filter_label {
        let regex = Regex::new(filter_pattern).map_err(|e| {
            CommandError::new("InvalidRegex", format!("Invalid regex pattern: {e}"))
        })?;
        key_displays.retain(|key| regex.is_match(&key.label));
    }

    // Sort keys
    match args.sort_by {
        SortBy::Label => key_displays.sort_by(|a, b| a.label.cmp(&b.label)),
        SortBy::Created => key_displays.sort_by(|a, b| a.created.cmp(&b.created)),
        SortBy::Type => key_displays.sort_by(|a, b| a.key_type.cmp(&b.key_type)),
    }

    Ok(CommandOutput::new(ListResponse {
        keys: key_displays,
        show_addresses: args.show_addresses,
    }))
}
