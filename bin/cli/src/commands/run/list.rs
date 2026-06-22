use std::fmt;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_services, RunStoresArgs};
use clap::Args;
use mfm_store::v1 as store;
use serde::Serialize;

/// Arguments for `mfm run list`.
#[derive(Args)]
pub(crate) struct ListArgs {
    /// Opaque cursor returned by a previous list page.
    #[arg(long)]
    pub(crate) cursor: Option<String>,

    /// Maximum run rows to return.
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: u32,

    /// Long-poll wait in milliseconds.
    #[arg(long, default_value_t = 0)]
    pub(crate) wait_ms: u64,

    /// Read changes from a cursor-capable observation page.
    #[arg(long)]
    pub(crate) watch: bool,

    /// Storage configuration for certified run events and artifacts.
    #[command(flatten)]
    pub(crate) stores: RunStoresArgs,
}

/// CLI response for observed run list/watch pages.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ListOutput {
    next_cursor: String,
    runs: Vec<ListRunOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct ListRunOutput {
    run_id: String,
    head_seq: u64,
    observed_status: String,
    started_at: String,
    updated_at: String,
    completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    change_id: Option<String>,
}

impl From<store::RunObservationPage> for ListOutput {
    fn from(page: store::RunObservationPage) -> Self {
        Self {
            next_cursor: page.next_cursor,
            runs: page.runs.into_iter().map(ListRunOutput::from).collect(),
        }
    }
}

impl From<store::RunObservation> for ListRunOutput {
    fn from(row: store::RunObservation) -> Self {
        Self {
            run_id: row.run_id.to_string(),
            head_seq: row.head_seq.as_u64(),
            observed_status: row.observed_status.as_str().to_owned(),
            started_at: row.started_at,
            updated_at: row.updated_at,
            completed_at: row.completed_at,
            change_id: row.change_id,
        }
    }
}

impl fmt::Display for ListOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "next_cursor {}", self.next_cursor)?;
        if self.runs.is_empty() {
            return writeln!(f, "runs 0");
        }
        for run in &self.runs {
            writeln!(
                f,
                "{} seq={} status={} updated_at={}",
                run.run_id, run.head_seq, run.observed_status, run.updated_at
            )?;
        }
        Ok(())
    }
}

/// Executes the list command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &ListArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ListArgs) -> CommandResult<ListOutput> {
    if args.watch && args.cursor.is_none() {
        return Err(CommandError::new(
            "WatchCursorRequired",
            "`mfm run list --watch` requires --cursor from a previous list page",
        ));
    }
    let services = connect_run_services(&args.stores).await?;
    let page = services
        .read_run_observations(store::RunObservationQuery::new(
            args.cursor.clone(),
            args.limit,
            args.wait_ms,
        ))
        .await?;

    Ok(CommandOutput::new(ListOutput::from(page)))
}
