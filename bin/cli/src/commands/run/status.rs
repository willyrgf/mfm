use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    command_error_from_app_error, command_error_from_typed_store_error, make_typed_artifact_store,
    make_typed_run_store, parse_typed_run_id, TypedRunStoresArgs,
};
use clap::Args;
use mfm_app::{RunServices, TypedRunResponse};

/// Arguments for `mfm run status`.
#[derive(Args)]
pub(crate) struct StatusArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: TypedRunStoresArgs,
}

/// Executes the status command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StatusArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StatusArgs) -> CommandResult<TypedRunResponse> {
    let run_id = parse_typed_run_id(&args.run_id)?;
    let store = make_typed_run_store(&args.stores).await?;
    let projection = store
        .projection_snapshot(&run_id)
        .await
        .map_err(command_error_from_typed_store_error)?;
    let artifacts = make_typed_artifact_store(&args.stores);
    let runners = mfm_app::production_typed_runner_registry(artifacts.clone())
        .map_err(command_error_from_app_error)?;
    let certification_registry =
        mfm_app::production_certification_registry().map_err(command_error_from_app_error)?;
    let services: RunServices<_> = mfm_app::make_async_typed_services_with_certification_registry(
        runners,
        store,
        artifacts,
        certification_registry,
    );
    let response = services
        .run_status_with_projection(&run_id, projection)
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
