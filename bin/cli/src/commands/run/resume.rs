use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services};
use crate::support::run_stores::{make_stores, RunStoresArgs};
use clap::Args;
use mfm_app::RunResumeResponse;

/// Arguments for `mfm run resume`.
#[derive(Args)]
pub struct ResumeArgs {
    /// Run id (UUID)
    pub run_id: String,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Executes the resume command and terminates the process.
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

    let services = make_app_services(stores);

    let response = services
        .resume_run(&args.run_id)
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
