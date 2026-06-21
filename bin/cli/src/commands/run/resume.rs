use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::typed_run::{
    connect_run_services, drive_mode, parse_typed_run_id, TypedDriveArg, TypedRunStoresArgs,
};
use clap::Args;
use mfm_app::TypedRunResponse;

/// Arguments for `mfm run resume`.
#[derive(Args)]
pub(crate) struct ResumeArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// Scheduler drive policy for typed resumes once certified spec loading is available.
    #[arg(long, value_enum, default_value_t = TypedDriveArg::UntilBlocked)]
    pub drive: TypedDriveArg,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: TypedRunStoresArgs,
}

/// Executes the resume command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ResumeArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ResumeArgs) -> CommandResult<TypedRunResponse> {
    let run_id = parse_typed_run_id(&args.run_id)?;
    let services = connect_run_services(&args.stores).await?;
    let response = services
        .resume_stored_run(&run_id, drive_mode(args.drive))
        .await?;
    Ok(CommandOutput::new(response))
}
