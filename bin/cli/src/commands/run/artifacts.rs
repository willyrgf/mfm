use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_stores::{make_artifact_store, RunStoresArgs};
use clap::{Args, Subcommand};
use mfm_app::{get_artifact_from_store, ArtifactGetResponse};

use super::engine_bundle::command_error_from_app_error;

#[derive(Subcommand)]
pub enum ArtifactsCommand {
    /// Fetch an artifact by id
    Get {
        #[command(flatten)]
        args: GetArgs,
    },
}

impl ArtifactsCommand {
    pub async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            ArtifactsCommand::Get { args } => execute_get(ctx, args).await,
        }
    }
}

#[derive(Args)]
pub struct GetArgs {
    /// Artifact id (SHA-256 lowercase hex, 64 chars)
    pub artifact_id: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

async fn execute_get(ctx: &CommandContext, args: &GetArgs) -> ! {
    let result = execute_get_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_get_internal(args: &GetArgs) -> CommandResult<ArtifactGetResponse> {
    let artifacts = make_artifact_store(args.stores.artifact_root.clone());
    let response = get_artifact_from_store(artifacts, &args.artifact_id)
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
