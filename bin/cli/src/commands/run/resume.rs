use std::path::PathBuf;

use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::application::{connect_application, parse_run_id, DatabaseArgs};
use clap::Args;
use mfm_app::RunResponse;

/// Arguments for `mfm run resume`.
#[derive(Args)]
pub(crate) struct ResumeArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Production database connection.
    #[command(flatten)]
    pub database: DatabaseArgs,

    /// Explicit runtime configuration file for live capabilities.
    #[arg(long, value_name = "PATH")]
    pub runtime_config: Option<PathBuf>,
}

/// Executes the resume command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ResumeArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ResumeArgs) -> CommandResult<RunResponse> {
    let run_id = parse_run_id(&args.run_id)?;
    let app = connect_application(&args.database, args.runtime_config.as_deref()).await?;
    let response = app.resume_run(&run_id).await?;
    Ok(CommandOutput::new(response))
}
