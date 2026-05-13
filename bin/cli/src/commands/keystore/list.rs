use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::{format_keys_table, handle_command_result, KeyDisplay};
use crate::support::{app_services, command_defaults};
use clap::Args;
use mfm_app::{KeystoreListRequest, KeystoreListSortBy};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Arguments for `mfm keystore list`.
#[derive(Args)]
pub(crate) struct ListArgs {
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
    let keystore_path = command_defaults::resolve_keystore_path(args.keystore.as_ref());
    let services = app_services::make_ephemeral_app_services();
    let response = services
        .keystore_list(KeystoreListRequest {
            keystore_path,
            show_addresses: args.show_addresses,
            filter_label: args.filter_label.clone(),
            sort_by: match args.sort_by {
                SortBy::Label => KeystoreListSortBy::Label,
                SortBy::Created => KeystoreListSortBy::Created,
                SortBy::Type => KeystoreListSortBy::Type,
            },
        })
        .await
        .map_err(app_services::command_error_from_app_error)?;

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
