use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_services, parse_run_id, RunStoresArgs};
use clap::Args;
use mfm_app::RunResponse;

/// Arguments for `mfm run status`.
#[derive(Args)]
pub(crate) struct StatusArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Executes the status command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StatusArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StatusArgs) -> CommandResult<RunResponse> {
    let run_id = parse_run_id(&args.run_id)?;
    let services = connect_run_services(&args.stores).await?;
    let response = services.run_status(&run_id).await?;

    Ok(CommandOutput::new(response))
}
