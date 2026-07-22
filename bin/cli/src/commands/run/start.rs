use std::path::PathBuf;

use crate::commands::result::{CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::application::{connect_application, DatabaseArgs};
use clap::Args;
use mfm_app::{InvocationKey, PublicOutputResponse, RunLaunchOutcomeStatus, RunResponse};
use serde::Serialize;

/// Arguments for `mfm run start`.
#[derive(Args)]
pub(crate) struct StartArgs {
    /// Exact public entry-point id, including namespace and version.
    #[arg(value_name = "ENTRY_POINT")]
    pub entry_point: String,

    /// Stable configured target selected by the entry point.
    #[arg(value_name = "TARGET")]
    pub target: String,

    /// Caller-supplied key that forces a invocation of otherwise identical certified work.
    #[arg(long, value_name = "KEY")]
    pub invocation_key: Option<String>,

    /// Explicit runtime configuration file for live capabilities.
    #[arg(long, value_name = "PATH")]
    pub runtime_config: Option<PathBuf>,

    /// Production database connection.
    #[command(flatten)]
    pub database: DatabaseArgs,
}

#[derive(Debug, Clone, Serialize)]
struct StartOutput {
    outcome: RunLaunchOutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<RunResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_output: Option<PublicOutputResponse>,
}

impl std::fmt::Display for StartOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(public_output) = &self.public_output {
            return write!(f, "launch_outcome={} {public_output}", self.outcome);
        }
        if let Some(run) = &self.run {
            return write!(f, "launch_outcome={} {run}", self.outcome);
        }
        match &self.active_run_id {
            Some(active_run_id) => {
                write!(
                    f,
                    "launch_outcome={} active_run_id={active_run_id}",
                    self.outcome
                )
            }
            None => write!(f, "launch_outcome={}", self.outcome),
        }
    }
}

/// Executes the start command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &StartArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StartArgs) -> CommandResult<StartOutput> {
    let invocation_key = args
        .invocation_key
        .as_deref()
        .map(InvocationKey::new)
        .transpose()?;
    let app = connect_application(&args.database, args.runtime_config.as_deref()).await?;
    let report = app
        .start_entry_point_run(&args.entry_point, &args.target, invocation_key)
        .await?;
    Ok(CommandOutput::new(StartOutput {
        outcome: report.outcome,
        run: report.run,
        active_run_id: report.active_run_id,
        public_output: report.public_output,
    }))
}

#[cfg(test)]
#[path = "start_tests.rs"]
mod tests;
