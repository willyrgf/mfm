use crate::cli::command_result::{CommandError, CommandOutput, CommandResult};
use crate::cli::utils::output::handle_command_result;
use crate::cli::utils::run_stores::{make_stores, RunStoresArgs};
use crate::cli::CommandContext;
use clap::Args;
use mfm_machine::events::EventEnvelope;
use mfm_machine::ids::RunId;
use serde::Serialize;
use std::fmt;

use super::engine_bundle::command_error_from_storage_error;

#[derive(Args)]
pub struct EventsArgs {
    /// Run id (UUID)
    pub run_id: String,

    /// First sequence number to read (1-indexed)
    #[arg(long, default_value_t = 1)]
    pub from_seq: u64,

    /// Optional last sequence number to read (inclusive)
    #[arg(long)]
    pub to_seq: Option<u64>,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventsResponse {
    pub run_id: String,
    pub head_seq: u64,
    pub events: Vec<EventEnvelope>,
}

impl fmt::Display for EventsResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string_pretty(&self.events).unwrap_or_else(|_| "[]".to_string());
        write!(f, "{s}")
    }
}

pub async fn execute(ctx: &CommandContext, args: &EventsArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &EventsArgs) -> CommandResult<EventsResponse> {
    let uuid = uuid::Uuid::parse_str(&args.run_id)
        .map_err(|_| CommandError::invalid_uuid("Invalid UUID format"))?;
    let run_id = RunId(uuid);

    let stores = make_stores(
        args.stores.artifact_root.clone(),
        args.stores.database_url.clone(),
    )
    .await?;

    let head = stores
        .events
        .head_seq(run_id)
        .await
        .map_err(command_error_from_storage_error)?;
    if head == 0 {
        return Err(CommandError::new(
            "run_not_found",
            "run event stream was not found",
        ));
    }

    let events = stores
        .events
        .read_range(run_id, args.from_seq, args.to_seq)
        .await
        .map_err(command_error_from_storage_error)?;

    Ok(CommandOutput::new(EventsResponse {
        run_id: run_id.0.to_string(),
        head_seq: head,
        events,
    }))
}
