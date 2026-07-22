use crate::commands::result::{CommandOutput, CommandResult, PublicError};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::application::{connect_application, parse_run_id, DatabaseArgs};
use clap::Args;
use mfm_app::RunStreamResponse;

/// Arguments for `mfm run stream`.
#[derive(Args)]
pub(crate) struct StreamArgs {
    /// Typed run id (`run:<algorithm>:<digest>`)
    pub run_id: String,

    /// First sequence number to read (1-indexed)
    #[arg(long, default_value_t = 1)]
    pub from_seq: u64,

    /// Optional last sequence number to read (inclusive)
    #[arg(long)]
    pub to_seq: Option<u64>,

    /// Production database connection.
    #[command(flatten)]
    pub database: DatabaseArgs,
}

/// Executes the stream command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StreamArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StreamArgs) -> CommandResult<RunStreamResponse> {
    if args.from_seq == 0 {
        return Err(PublicError::bad_request(
            "InvalidSequenceRange",
            "--from-seq must be greater than zero",
        ));
    }
    if let Some(to_seq) = args.to_seq {
        if to_seq < args.from_seq {
            return Err(PublicError::bad_request(
                "InvalidSequenceRange",
                "--to-seq must be greater than or equal to --from-seq",
            ));
        }
    }

    let run_id = parse_run_id(&args.run_id)?;
    let app = connect_application(&args.database, None).await?;
    let response = app.run_stream(&run_id).await?;
    let response = RunStreamResponse {
        run_id: response.run_id,
        head_seq: response.head_seq,
        events: response
            .events
            .into_iter()
            .filter(|event| {
                event.seq >= args.from_seq
                    && match args.to_seq {
                        Some(to_seq) => event.seq <= to_seq,
                        None => true,
                    }
            })
            .collect(),
    };

    Ok(CommandOutput::new(response))
}
