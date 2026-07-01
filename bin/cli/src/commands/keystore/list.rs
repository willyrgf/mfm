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
    let access =
        keystore_selection::resolve_keystore_access(keystore_selection::KeystoreSelectionArgs {
            keystore: args.keystore.as_ref(),
            runtime_config: args.runtime_config.as_ref(),
            keystore_ref: args.keystore_ref.as_deref(),
        })?;
    let response = keystore::list_keys(keystore::ListKeysRequest {
        access,
        show_addresses: args.show_addresses,
        filter_label: args.filter_label.clone(),
        sort_by: match args.sort_by {
            SortBy::Label => keystore::ListSortBy::Label,
            SortBy::Created => keystore::ListSortBy::Created,
            SortBy::Type => keystore::ListSortBy::Type,
        },
    })?;

    let keys = response
        .keys
        .into_iter()
        .map(|key| KeyDisplay {
            id: key.id,
            label: key.label,
            key_type: key.key_type,
            address: key.address,
            created: key.created,
        })
        .collect();

    Ok(CommandOutput::new(ListResponse {
        keys,
        show_addresses: response.show_addresses,
    }))
}
