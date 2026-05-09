use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::command_error_from_app_error;
use crate::support::run_stores::{make_artifact_store, RunStoresArgs};
use clap::{Args, Subcommand};
use mfm_app::{get_artifact_from_store, parse_artifact_id, ArtifactGetResponse};

/// Subcommands under `mfm run artifacts`.
#[derive(Subcommand)]
pub(crate) enum ArtifactsCommand {
    /// Fetch an artifact by id
    Get {
        /// Parsed arguments for the artifact fetch command.
        #[command(flatten)]
        args: GetArgs,
    },
}

impl ArtifactsCommand {
    /// Dispatches the selected artifact subcommand and terminates the process.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            ArtifactsCommand::Get { args } => execute_get(ctx, args).await,
        }
    }
}

/// Arguments for `mfm run artifacts get`.
#[derive(Args)]
pub(crate) struct GetArgs {
    /// Artifact id (SHA-256 lowercase hex, 64 chars)
    pub artifact_id: String,

    /// Storage configuration for artifact lookup.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

async fn execute_get(ctx: &CommandContext, args: &GetArgs) -> ! {
    let result = execute_get_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_get_internal(args: &GetArgs) -> CommandResult<ArtifactGetResponse> {
    let artifacts = make_artifact_store(args.stores.artifact_root.clone());
    let artifact_id = parse_artifact_id(&args.artifact_id).map_err(command_error_from_app_error)?;
    let response = get_artifact_from_store(artifacts, &artifact_id)
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
