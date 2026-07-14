use std::path::PathBuf;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_services, RunStoresArgs};
use clap::Args;
use mfm_app::{InvocationKey, PublicOutputResponse, RunLaunchOutcomeStatus, RunResponse};
use serde::Serialize;

/// Arguments for `mfm run start`.
#[derive(Args)]
pub(crate) struct StartArgs {
    /// Exact public entry-point id, including namespace and version.
    #[arg(long = "entry-point", value_name = "ID")]
    pub entry_point: String,

    /// Strict JSON request file containing exact catalog references.
    #[arg(long, value_name = "PATH")]
    pub request: PathBuf,

    /// Caller-supplied key that forces a invocation of otherwise identical certified work.
    #[arg(long, value_name = "KEY")]
    pub invocation_key: Option<String>,

    /// Runtime configuration file for live capabilities (default: $MFM_RUNTIME_CONFIG_FILE).
    #[arg(long, value_name = "PATH")]
    pub runtime_config: Option<PathBuf>,

    /// Storage configuration for certified typed run events and artifacts.
    #[command(flatten)]
    pub stores: RunStoresArgs,
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
    let request_bytes = tokio::fs::read(&args.request).await.map_err(|_| {
        CommandError::backend("EntryPointRequestReadFailed", "Failed to read request")
    })?;
    let request: serde_json::Value = serde_json::from_slice(&request_bytes).map_err(|_| {
        CommandError::backend("EntryPointRequestInvalid", "Request must be valid JSON")
    })?;
    let certification_registry = mfm_app::production_certification_registry()?;
    let services = connect_run_services(&args.stores, args.runtime_config.as_deref()).await?;
    let store_scope_id = services.load_store_scope_id().await?;
    let request = mfm_app::prepare_entry_point_run_launch(
        services.store(),
        &args.entry_point,
        &request,
        &certification_registry,
        store_scope_id,
        invocation_key,
    )
    .await?;
    let report = services.launch_run_and_render(request).await?;
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
