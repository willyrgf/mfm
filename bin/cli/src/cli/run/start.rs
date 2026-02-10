use crate::cli::command_result::{CommandOutput, CommandResult};
use crate::cli::utils::output::handle_command_result;
use crate::cli::utils::run_stores::{make_stores, RunStoresArgs};
use crate::cli::CommandContext;
use clap::Args;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::errors::ContextError;
use mfm_machine::ids::{ContextKey, OpId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::unstable::{single_op_pipeline, DefaultRunLauncher};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;

use super::engine_bundle::{command_error_from_run_error, make_engine_bundle};

#[derive(Args)]
pub struct StartArgs {
    /// Operation id (default: proof)
    #[arg(long, default_value = "proof")]
    pub op_id: String,

    /// Operation version (default: v1)
    #[arg(long, default_value = "v1")]
    pub op_version: String,

    /// Operation config JSON (must be canonical-json-hashable; no floats)
    #[arg(long, default_value = "{}")]
    pub op_config_json: String,

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
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            // Be conservative by default: do not retry side effects.
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

pub async fn execute(ctx: &CommandContext, args: &StartArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &StartArgs) -> CommandResult<StartResponse> {
    let op_config: serde_json::Value =
        serde_json::from_str(&args.op_config_json).map_err(|_| {
            crate::cli::command_result::CommandError::new(
                "InvalidJson",
                "Failed to parse --op-config-json as JSON",
            )
        })?;

    let stores = make_stores(
        args.stores.artifact_root.clone(),
        args.stores.database_url.clone(),
    )
    .await?;

    let bundle = make_engine_bundle();

    let pipeline = single_op_pipeline(OpId(args.op_id.clone()), args.op_version.clone(), op_config)
        .map_err(|e| {
            crate::cli::command_result::CommandError::new(e.info.code.0, e.info.message)
        })?;

    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            bundle.engine,
            stores,
            bundle.registry,
            bundle.planner,
            LaunchPipeline {
                pipeline,
                input: serde_json::json!({}),
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
