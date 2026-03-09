use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services_from_args};
use crate::support::run_stores::RunStoresArgs;
use clap::Args;
use mfm_app::{RunStartResponse, RunsStartRequest, SingleOpStartRequest};

/// Arguments for `mfm run start`.
#[derive(Args)]
pub(crate) struct StartArgs {
    /// Operation id (default: proof)
    #[arg(long, default_value = "proof")]
    pub op_id: String,

    /// Operation version (default: v1)
    #[arg(long, default_value = "v1")]
    pub op_version: String,

    /// Operation config JSON (must be canonical-json-hashable; no floats)
    #[arg(long, default_value = "{}")]
    pub op_config_json: String,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Executes the start command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StartArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StartArgs) -> CommandResult<RunStartResponse> {
    let op_config: serde_json::Value =
        serde_json::from_str(&args.op_config_json).map_err(|_| {
            CommandError::new("InvalidJson", "Failed to parse --op-config-json as JSON")
        })?;

    let services = make_app_services_from_args(&args.stores).await?;

    let response = services
        .start_run(RunsStartRequest::Single(SingleOpStartRequest {
            op_id: args.op_id.clone(),
            op_version: args.op_version.clone(),
            op_config,
        }))
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
