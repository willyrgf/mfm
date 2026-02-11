use clap::{Args, Subcommand};
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::errors::ContextError;
use mfm_machine::ids::{ContextKey, OpId};
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::DefaultRunLauncher;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use crate::cli::command_result::{CommandError, CommandOutput, CommandResult};
use crate::cli::utils::output::handle_command_result;
use crate::cli::utils::run_stores::{make_stores, RunStoresArgs};
use crate::cli::CommandContext;

use super::engine_bundle::{command_error_from_run_error, make_engine_bundle};

#[derive(Subcommand)]
pub enum PipelineCommand {
    /// Start a run from an explicit pipeline JSON payload
    Start {
        #[command(flatten)]
        args: PipelineStartArgs,
    },

    /// Start the standard deploy->configure->validate pipeline from a compact spec
    DeployConfigureValidate {
        #[command(flatten)]
        args: DeployConfigureValidateArgs,
    },
}

impl PipelineCommand {
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

#[derive(Args)]
pub struct PipelineStartArgs {
    /// Pipeline JSON payload
    #[arg(long)]
    pub pipeline_json: String,

    /// Optional pipeline input JSON (default: {})
    #[arg(long, default_value = "{}")]
    pub input_json: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

#[derive(Args)]
pub struct DeployConfigureValidateArgs {
    /// Spec JSON payload describing deploy/configure/validate op configs
    #[arg(long)]
    pub spec_json: String,

    #[command(flatten)]
    pub stores: RunStoresArgs,
}

#[derive(Default)]
struct MapContext {
    inner: HashMap<String, serde_json::Value>,
}

impl DynContext for MapContext {
    fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
        Ok(self.inner.get(&key.0).cloned())
    }

    fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
        self.inner.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
        self.inner.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, ContextError> {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.inner {
            m.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(m))
    }
}

fn default_run_config() -> RunConfig {
    RunConfig {
        nix_flake_allowlist: Vec::new(),
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff: BackoffPolicy::Fixed {
                delay: std::time::Duration::from_millis(0),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StartResponse {
    pub run_id: String,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

impl fmt::Display for StartResponse {
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

#[derive(Clone, Debug, Deserialize)]
struct DeployConfigureValidateSpec {
    #[serde(default = "default_machine_id")]
    machine_id: String,

    #[serde(default = "default_pipeline_version")]
    pipeline_version: String,

    #[serde(default = "default_empty_object")]
    input: serde_json::Value,

    deploy: serde_json::Value,
    configure: serde_json::Value,
    validate: serde_json::Value,
}

fn default_machine_id() -> String {
    "evm_deploy_configure_validate".to_string()
}

fn default_pipeline_version() -> String {
    "v1".to_string()
}

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
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

async fn start_pipeline(
    pipeline: Pipeline,
    input: serde_json::Value,
    stores_args: &RunStoresArgs,
) -> CommandResult<StartResponse> {
    let stores = make_stores(
        stores_args.artifact_root.clone(),
        stores_args.database_url.clone(),
    )
    .await?;

    let bundle = make_engine_bundle();

    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            bundle.engine,
            stores,
            bundle.registry,
            bundle.planner,
            LaunchPipeline {
                pipeline,
                input,
                run_config: default_run_config(),
                build: BuildProvenance {
                    git_commit: None,
                    cargo_lock_hash: None,
                    flake_lock_hash: None,
                    rustc_version: None,
                    target_triple: None,
                    env_allowlist: Vec::new(),
                },
                initial_context: Box::new(MapContext::default()),
            },
        )
        .await
        .map_err(command_error_from_run_error)?;

    Ok(CommandOutput::new(StartResponse {
        run_id: run.run_id.0.to_string(),
        phase: phase_str(&run.phase).to_string(),
        final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
    }))
}

async fn execute_start_internal(args: &PipelineStartArgs) -> CommandResult<StartResponse> {
    let pipeline = parse_pipeline_json(&args.pipeline_json)?;
    let input = parse_input_json(&args.input_json)?;
    start_pipeline(pipeline, input, &args.stores).await
}

async fn execute_dcv_internal(args: &DeployConfigureValidateArgs) -> CommandResult<StartResponse> {
    let spec: DeployConfigureValidateSpec =
        serde_json::from_str(&args.spec_json).map_err(|_| {
            CommandError::new(
                "InvalidJson",
                "Failed to parse --spec-json as deploy/configure/validate spec JSON",
            )
        })?;

    let pipeline = Pipeline {
        machine_id: MachineId(spec.machine_id),
        pipeline_version: spec.pipeline_version,
        steps: vec![
            PipelineStep {
                step_id: StepId("deploy".to_string()),
                op_id: OpId("evm_deploy".to_string()),
                op_version: "v1".to_string(),
                op_config: spec.deploy,
            },
            PipelineStep {
                step_id: StepId("configure".to_string()),
                op_id: OpId("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: spec.configure,
            },
            PipelineStep {
                step_id: StepId("validate".to_string()),
                op_id: OpId("evm_validate".to_string()),
                op_version: "v1".to_string(),
                op_config: spec.validate,
            },
        ],
    };

    start_pipeline(pipeline, spec.input, &args.stores).await
}
