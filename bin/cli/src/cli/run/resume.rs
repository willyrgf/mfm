use crate::cli::command_result::{CommandError, CommandOutput, CommandResult};
use crate::cli::utils::output::handle_command_result;
use crate::cli::utils::run_stores::{make_stores, RunStoresArgs};
use crate::cli::CommandContext;
use clap::Args;
use mfm_machine::ids::RunId;
use mfm_sdk::launcher::RunLauncher;
use mfm_sdk::unstable::DefaultRunLauncher;
use serde::Serialize;
use std::fmt;

use super::engine_bundle::{command_error_from_run_error, make_engine_bundle};

#[derive(Args)]
pub struct ResumeArgs {
    /// Run id (UUID)
    pub run_id: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResumeResponse {
    pub run_id: String,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

impl fmt::Display for ResumeResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "run_id: {}", self.run_id)?;
        writeln!(f, "phase: {}", self.phase)?;
        if let Some(id) = &self.final_snapshot_id {
            writeln!(f, "final_snapshot_id: {id}")?;
        }
        Ok(())
    }
}

fn phase_str(p: &mfm_machine::engine::RunPhase) -> &'static str {
    match p {
        mfm_machine::engine::RunPhase::Running => "running",
        mfm_machine::engine::RunPhase::Completed => "completed",
        mfm_machine::engine::RunPhase::Failed => "failed",
        mfm_machine::engine::RunPhase::Cancelled => "cancelled",
    }
}

pub async fn execute(ctx: &CommandContext, args: &ResumeArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &ResumeArgs) -> CommandResult<ResumeResponse> {
    let uuid = uuid::Uuid::parse_str(&args.run_id)
        .map_err(|_| CommandError::invalid_uuid("Invalid UUID format"))?;
    let run_id = RunId(uuid);

    let stores = make_stores(
        args.stores.artifact_root.clone(),
        args.stores.database_url.clone(),
    )
    .await?;

    let bundle = make_engine_bundle();
    let launcher = DefaultRunLauncher;

    let run = launcher
        .resume(
            bundle.engine,
            stores,
            bundle.registry,
            bundle.planner,
            run_id,
        )
        .await
        .map_err(command_error_from_run_error)?;

    Ok(CommandOutput::new(ResumeResponse {
        run_id: run.run_id.0.to_string(),
        phase: phase_str(&run.phase).to_string(),
        final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
    }))
}
