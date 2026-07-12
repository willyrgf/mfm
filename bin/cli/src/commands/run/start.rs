use std::path::PathBuf;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::{connect_run_services, RunStoresArgs};
use clap::{Args, ValueEnum};
use mfm_app::{
    EntryPointRunLaunchInput, InvocationKey, PublicOpName, PublicOutputResponse,
    RunLaunchOutcomeStatus, RunResponse,
};
use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
use serde::Serialize;

/// Arguments for `mfm run start`.
#[derive(Args)]
pub(crate) struct StartArgs {
    /// Public entry-point operation name.
    #[arg(long, value_name = "NAME")]
    pub op: String,

    /// Authored operation config file.
    #[arg(long, value_name = "PATH")]
    pub config: PathBuf,

    /// Optional public operation version. Defaults to the latest registered version.
    #[arg(long, value_name = "VERSION")]
    pub op_version: Option<u32>,

    /// Authored config format.
    #[arg(long, value_enum, default_value_t = ConfigFormatArg::Toml)]
    pub config_format: ConfigFormatArg,

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

/// CLI spelling for authored config formats.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ConfigFormatArg {
    /// TOML authored config.
    Toml,
    /// JSON authored config.
    Json,
}

impl From<ConfigFormatArg> for AuthoredConfigFormat {
    fn from(value: ConfigFormatArg) -> Self {
        match value {
            ConfigFormatArg::Toml => Self::Toml,
            ConfigFormatArg::Json => Self::Json,
        }
    }
}

impl std::fmt::Display for ConfigFormatArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toml => f.write_str("toml"),
            Self::Json => f.write_str("json"),
        }
    }
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
    let public_op_name = PublicOpName::new(&args.op)?;
    let op_version = args.op_version.map(mfm_app::OpVersion::new).transpose()?;
    let invocation_key = args
        .invocation_key
        .as_deref()
        .map(InvocationKey::new)
        .transpose()?;
    let config_bytes = tokio::fs::read(&args.config)
        .await
        .map_err(|_| CommandError::backend("AuthoredConfigReadFailed", "Failed to read config"))?;
    let authored_config = AuthoredConfig::new(args.config_format.into(), config_bytes)?;
    let entry_point_registry = mfm_app::production_entry_point_op_registry()?;
    let certification_registry = mfm_app::production_certification_registry()?;
    let services = connect_run_services(&args.stores, args.runtime_config.as_deref()).await?;
    let store_scope_id = services.load_store_scope_id().await?;
    let prepared = mfm_app::prepare_entry_point_run_launch(EntryPointRunLaunchInput {
        entry_point_registry: &entry_point_registry,
        public_op_name,
        op_version,
        authored_config,
        certification_registry: &certification_registry,
        store_scope_id,
        invocation_key,
    })?;
    let report = services.launch_prepared_entry_point_run(prepared).await?;
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
