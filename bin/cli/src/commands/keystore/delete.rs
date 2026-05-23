use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::{app_services, command_defaults};
use clap::Args;
use mfm_app_legacy::KeystoreDeleteRequest;
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
    let keystore_path = command_defaults::resolve_keystore_path(args.keystore.as_ref());
    let services = app_services::make_ephemeral_app_services();
    let response = services
        .keystore_delete(KeystoreDeleteRequest {
            id: args.id.clone(),
            by_label: args.by_label.clone(),
            yes: args.yes,
            keystore_path,
        })
        .await
        .map_err(app_services::command_error_from_app_error)?;

    Ok(CommandOutput::new(DeleteResponse {
        id: response.id,
        label: response.label,
    }))
}
