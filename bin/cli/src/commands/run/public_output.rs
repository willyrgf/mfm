use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::application::{
    connect_application, parse_run_id, parse_schema_id, DatabaseArgs,
};
use clap::Args;
use mfm_app::PublicOutputResponse;

/// Arguments for `mfm run public-output`.
#[derive(Args)]
pub(crate) struct PublicOutputArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Public output schema id to render.
    #[arg(long)]
    pub schema_id: String,

    /// Production database connection.
    #[command(flatten)]
    pub database: DatabaseArgs,
}

/// Executes the public-output command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &PublicOutputArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &PublicOutputArgs) -> CommandResult<PublicOutputResponse> {
    let run_id = parse_run_id(&args.run_id)?;
    let schema_id = parse_schema_id(&args.schema_id)?;
    let app = connect_application(&args.database, None).await?;
    let response = app.public_output(&run_id, &schema_id).await?;

    Ok(CommandOutput::new(response))
}
