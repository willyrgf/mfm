use clap::{Args, Subcommand};
use mfm_app::{PipelineStartRequest, RunStartResponse, RunsStartRequest};
use mfm_sdk::pipeline::Pipeline;
use std::path::PathBuf;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::app_services::{command_error_from_app_error, make_app_services};
use crate::support::run_stores::{make_stores, RunStoresArgs};

/// Subcommands under `mfm run pipeline`.
#[derive(Subcommand)]
pub enum PipelineCommand {
    /// Start a run from an explicit pipeline JSON payload
    Start {
        /// Parsed arguments for the explicit pipeline start command.
        #[command(flatten)]
        args: PipelineStartArgs,
    },

    /// Start the standard deploy->configure->validate pipeline from a compact spec
    DeployConfigureValidate {
        /// Parsed arguments for the deploy-configure-validate helper command.
        #[command(flatten)]
        args: DeployConfigureValidateArgs,
    },
}

impl PipelineCommand {
    /// Dispatches the selected pipeline subcommand and terminates the process.
    pub async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            PipelineCommand::Start { args } => {
                handle_command_result(execute_start_internal(args).await, &ctx.output_format)
            }
            PipelineCommand::DeployConfigureValidate { args } => {
                handle_command_result(execute_dcv_internal(args).await, &ctx.output_format)
            }
        }
    }
}

/// Arguments for `mfm run pipeline start`.
#[derive(Args)]
pub struct PipelineStartArgs {
    /// Pipeline JSON payload
    #[arg(long)]
    pub pipeline_json: String,

    /// Optional pipeline input JSON (default: {})
    #[arg(long, default_value = "{}")]
    pub input_json: String,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Arguments for `mfm run pipeline deploy-configure-validate`.
#[derive(Args)]
pub struct DeployConfigureValidateArgs {
    /// Spec JSON payload describing deploy/configure/validate op configs
    #[arg(long)]
    pub spec_json: Option<String>,

    /// Path to a spec JSON file describing deploy/configure/validate op configs
    #[arg(long)]
    pub spec_file: Option<PathBuf>,

    /// Storage configuration for the run's event and artifact backends.
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

fn parse_pipeline_json(s: &str) -> Result<Pipeline, CommandError> {
    serde_json::from_str::<Pipeline>(s).map_err(|_| {
        CommandError::new(
            "InvalidJson",
            "Failed to parse --pipeline-json as a pipeline JSON payload",
        )
    })
}

fn parse_input_json(s: &str) -> Result<serde_json::Value, CommandError> {
    serde_json::from_str::<serde_json::Value>(s)
        .map_err(|_| CommandError::new("InvalidJson", "Failed to parse --input-json as JSON"))
}

async fn app_services(stores_args: &RunStoresArgs) -> Result<mfm_app::AppServices, CommandError> {
    let stores = make_stores(
        stores_args.artifact_root.clone(),
        stores_args.database_url.clone(),
    )
    .await?;

    Ok(make_app_services(stores))
}

async fn execute_start_internal(args: &PipelineStartArgs) -> CommandResult<RunStartResponse> {
    let pipeline = parse_pipeline_json(&args.pipeline_json)?;
    let input = parse_input_json(&args.input_json)?;

    let services = app_services(&args.stores).await?;

    let response = services
        .start_run(RunsStartRequest::Pipeline(PipelineStartRequest {
            pipeline,
            input,
            run_config: Some(mfm_app::default_run_config()),
        }))
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}

async fn execute_dcv_internal(
    args: &DeployConfigureValidateArgs,
) -> CommandResult<RunStartResponse> {
    let services = app_services(&args.stores).await?;
    let response = services
        .start_deploy_configure_validate_from_spec_input(
            args.spec_json.clone(),
            args.spec_file.clone(),
        )
        .await
        .map_err(command_error_from_app_error)?;

    Ok(CommandOutput::new(response))
}
