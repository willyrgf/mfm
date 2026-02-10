use crate::cli::command_result::{CommandError, CommandOutput, CommandResult};
use crate::cli::utils::output::handle_command_result;
use crate::cli::utils::run_stores::{make_stores, RunStoresArgs};
use crate::cli::CommandContext;
use clap::Args;
use mfm_machine::events::{Event, KernelEvent, RunStatus};
use mfm_machine::ids::RunId;
use serde::Serialize;
use std::fmt;

use super::engine_bundle::command_error_from_storage_error;

#[derive(Args)]
pub struct StatusArgs {
    /// Run id (UUID)
    pub run_id: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusResponse {
    pub run_id: String,
    pub head_seq: u64,
    pub op_id: Option<String>,
    pub manifest_id: Option<String>,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

impl fmt::Display for StatusResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "run_id: {}", self.run_id)?;
        writeln!(f, "head_seq: {}", self.head_seq)?;
        if let Some(op_id) = &self.op_id {
            writeln!(f, "op_id: {op_id}")?;
        }
        if let Some(manifest_id) = &self.manifest_id {
            writeln!(f, "manifest_id: {manifest_id}")?;
        }
        writeln!(f, "phase: {}", self.phase)?;
        if let Some(id) = &self.final_snapshot_id {
            writeln!(f, "final_snapshot_id: {id}")?;
        }
        Ok(())
    }
}

fn phase_from_status(s: &RunStatus) -> &'static str {
    match s {
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
    }
}

pub async fn execute(ctx: &CommandContext, args: &StatusArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StatusArgs) -> CommandResult<StatusResponse> {
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

    let stream = stores
        .events
        .read_range(run_id, 1, None)
        .await
        .map_err(command_error_from_storage_error)?;

    let mut op_id = None;
    let mut manifest_id = None;
    let mut completed: Option<(RunStatus, Option<String>)> = None;

    for e in &stream {
        let Event::Kernel(ke) = &e.event else {
            continue;
        };
        match ke {
            KernelEvent::RunStarted {
                op_id: oid,
                manifest_id: mid,
                initial_snapshot_id: _,
            } => {
                op_id = Some(oid.0.clone());
                manifest_id = Some(mid.0.clone());
            }
            KernelEvent::RunCompleted {
                status,
                final_snapshot_id,
            } => {
                completed = Some((
                    status.clone(),
                    final_snapshot_id.as_ref().map(|id| id.0.clone()),
                ));
            }
            _ => {}
        }
    }

    let (phase, final_snapshot_id) = match completed {
        Some((s, id)) => (phase_from_status(&s).to_string(), id),
        None => ("running".to_string(), None),
    };

    Ok(CommandOutput::new(StatusResponse {
        run_id: run_id.0.to_string(),
        head_seq: head,
        op_id,
        manifest_id,
        phase,
        final_snapshot_id,
    }))
}
