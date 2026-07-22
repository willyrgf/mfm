use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::application::{connect_application, parse_run_id, DatabaseArgs};
use clap::Args;
use mfm_app::ReplayResponse;

/// Arguments for `mfm run replay`.
#[derive(Args)]
pub(crate) struct ReplayArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Production database connection.
    #[command(flatten)]
    pub database: DatabaseArgs,
}

/// Executes the replay command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ReplayArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ReplayArgs) -> CommandResult<ReplayResponse> {
    let run_id = parse_run_id(&args.run_id)?;
    let app = connect_application(&args.database, None).await?;
    let response = app.verify_replay(&run_id).await?;
    Ok(CommandOutput::new(response))
}
