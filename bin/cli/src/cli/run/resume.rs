use crate::cli::command_result::{CommandError, CommandOutput, CommandResult};
use crate::cli::utils::output::handle_command_result;
use crate::cli::utils::run_stores::{make_stores, RunStoresArgs};
use crate::cli::CommandContext;
use clap::Args;
use mfm_app::{AppServices, RunResumeResponse};

use super::engine_bundle::{command_error_from_app_error, make_engine_bundle};

#[derive(Args)]
pub struct ResumeArgs {
    /// Run id (UUID)
    pub run_id: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

pub async fn execute(ctx: &CommandContext, args: &ResumeArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ResumeArgs) -> CommandResult<RunResumeResponse> {
    uuid::Uuid::parse_str(&args.run_id)
        .map_err(|_| CommandError::invalid_uuid("Invalid UUID format"))?;

    let stores = make_stores(
        args.stores.artifact_root.clone(),
        args.stores.database_url.clone(),
    )
    .await?;

    let bundle = make_engine_bundle();
    let services = AppServices::new(bundle, stores.events, stores.artifacts);

    let response = services
        .resume_run(&args.run_id)
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
