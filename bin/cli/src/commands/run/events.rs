use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services_from_args};
use crate::support::run_stores::RunStoresArgs;
use clap::Args;
use mfm_app::{RunsEventsQuery, RunsEventsResponse};

/// Arguments for `mfm run events`.
#[derive(Args)]
pub(crate) struct EventsArgs {
    /// Run id (UUID)
    pub run_id: String,

    /// First sequence number to read (1-indexed)
    #[arg(long, default_value_t = 1)]
    pub from_seq: u64,

    /// Optional last sequence number to read (inclusive)
    #[arg(long)]
    pub to_seq: Option<u64>,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Executes the events command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &EventsArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &EventsArgs) -> CommandResult<RunsEventsResponse> {
    uuid::Uuid::parse_str(&args.run_id)
        .map_err(|_| CommandError::invalid_uuid("Invalid UUID format"))?;

    let services = make_app_services_from_args(&args.stores).await?;

    let response = services
        .run_events(
            &args.run_id,
            RunsEventsQuery {
                from_seq: args.from_seq,
                to_seq: args.to_seq,
            },
        )
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
