use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::{format_keys_table, handle_command_result, KeyDisplay};
use crate::support::{keystore, keystore_selection};
use clap::Args;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Arguments for `mfm keystore list`.
#[derive(Args)]
pub(crate) struct ListArgs {
    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Runtime configuration file for keystore profile selection
    #[arg(long)]
    pub runtime_config: Option<PathBuf>,

    /// Keystore profile ref inside the runtime config (default: default)
    #[arg(long)]
    pub keystore_ref: Option<String>,

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

/// Sort orders supported by the list command.
#[derive(clap::ValueEnum, Clone)]
pub(crate) enum SortBy {
    /// Sort keys lexicographically by label.
    #[value(name = "label")]
    Label,
    /// Sort keys by creation timestamp.
    #[value(name = "created")]
    Created,
    /// Sort keys by stored key type.
    #[value(name = "type")]
    Type,
}

/// Response returned by the list command.
#[derive(Serialize)]
pub(crate) struct ListResponse {
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

/// Executes the list command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ListArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ListArgs) -> CommandResult<ListResponse> {
    let selection = keystore_selection::resolve_keystore_selection(
        keystore_selection::KeystoreSelectionArgs {
            keystore: args.keystore.as_ref(),
            runtime_config: args.runtime_config.as_ref(),
            keystore_ref: args.keystore_ref.as_deref(),
        },
    )?;
    let prepared = mfm_app::prepare_existing_keystore_access(selection).await?;
    let access = keystore::bind_prepared_access(prepared)?;
    let mut keys = mfm_app::list_keystore_keys(access)
        .await?
        .into_iter()
        .map(|key| KeyDisplay {
            id: key.id.to_string(),
            label: key.alias.unwrap_or_else(|| "<no alias>".to_owned()),
            key_type: keystore::key_type_code(&key.key_type).to_owned(),
            address: args.show_addresses.then(|| format!("{:?}", key.address)),
            created: key.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect::<Vec<_>>();

    if let Some(pattern) = args.filter_label.as_ref() {
        let regex = regex::Regex::new(pattern).map_err(|_| {
            mfm_app::PublicError::bad_request("invalid_regex", "Invalid regex pattern")
        })?;
        keys.retain(|key| regex.is_match(&key.label));
    }

    match args.sort_by {
        SortBy::Label => keys.sort_by(|left, right| left.label.cmp(&right.label)),
        SortBy::Created => keys.sort_by(|left, right| left.created.cmp(&right.created)),
        SortBy::Type => keys.sort_by(|left, right| left.key_type.cmp(&right.key_type)),
    }

    Ok(CommandOutput::new(ListResponse {
        keys,
        show_addresses: args.show_addresses,
    }))
}
