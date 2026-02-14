use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services};
use crate::support::run_stores::{make_stores, RunStoresArgs};
use clap::Args;
use mfm_app::RunStatusResponse;

#[derive(Args)]
pub struct StatusArgs {
    /// Run id (UUID)
    pub run_id: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

pub async fn execute(ctx: &CommandContext, args: &StatusArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StatusArgs) -> CommandResult<RunStatusResponse> {
    uuid::Uuid::parse_str(&args.run_id)
        .map_err(|_| CommandError::invalid_uuid("Invalid UUID format"))?;

    let stores = make_stores(
        args.stores.artifact_root.clone(),
        args.stores.database_url.clone(),
    )
    .await?;

    let services = make_app_services(stores);

    let response = services
        .run_status(&args.run_id)
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
