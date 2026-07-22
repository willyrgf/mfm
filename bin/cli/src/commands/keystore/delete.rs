use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{keystore, keystore_selection};
use clap::Args;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Arguments for `mfm keystore delete`.
#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Key ID (UUID) to delete
    pub id: Option<String>,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Runtime configuration file for keystore profile selection
    #[arg(long)]
    pub runtime_config: Option<PathBuf>,

    /// Keystore profile ref inside the runtime config (default: default)
    #[arg(long)]
    pub keystore_ref: Option<String>,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Delete by label instead of ID
    #[arg(long)]
    pub by_label: Option<String>,
}

/// Response returned after successfully deleting a key.
#[derive(Serialize)]
pub(crate) struct DeleteResponse {
    id: String,
    label: String,
}

impl fmt::Display for DeleteResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key '{}' deleted successfully", self.label)
    }
}

/// Executes the delete command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &DeleteArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &DeleteArgs) -> CommandResult<DeleteResponse> {
    let selector = keystore::key_selector(args.id.clone(), args.by_label.clone())?;
    if !args.yes {
        let selected = args
            .id
            .as_deref()
            .or(args.by_label.as_deref())
            .unwrap_or("selected key");
        if !keystore::confirm(&format!(
            "Are you sure you want to delete key '{selected}'?"
        ))? {
            return Err(mfm_app::PublicError::bad_request(
                "operation_cancelled",
                "Deletion cancelled by user",
            ));
        }
    }

    let selection = keystore_selection::resolve_keystore_selection(
        keystore_selection::KeystoreSelectionArgs {
            keystore: args.keystore.as_ref(),
            runtime_config: args.runtime_config.as_ref(),
            keystore_ref: args.keystore_ref.as_deref(),
        },
    )?;
    let prepared = mfm_app::prepare_existing_keystore_access(selection).await?;
    let access = keystore::bind_prepared_access(prepared)?;
    let response = mfm_app::delete_keystore_key(access, selector).await?;

    Ok(CommandOutput::new(DeleteResponse {
        id: response.id.to_string(),
        label: response.alias.unwrap_or_default(),
    }))
}
