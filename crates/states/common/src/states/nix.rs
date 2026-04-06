//! Shared state for nix app resolution and command execution.

use std::collections::BTreeMap;

use async_trait::async_trait;
use mfm_collectors_exec::{ExecIoClient, RunProgramRequest};
use mfm_collectors_nix::{NixIoClient, RunFlakeAppRequest};
use serde::{Deserialize, Serialize};

use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::idempotency as op_idempotency;
use crate::states::meta;

/// Configuration for [`NixExecState`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NixExecStateConfig {
    /// Fully resolved nix store path to execute directly.
    #[serde(default)]
    pub program_path: Option<String>,

    /// Flake app reference to resolve before execution.
    #[serde(default)]
    pub app: Option<String>,

    /// Arguments passed to the resolved program.
    #[serde(default)]
    pub argv: Vec<String>,

    /// JSON payload written to the child process stdin.
    #[serde(default)]
    pub stdin_json: serde_json::Value,

    /// Optional context key that overrides `stdin_json` with a runtime-loaded JSON payload.
    ///
    /// This keeps step-to-step JSON handoff inside reusable execution surfaces instead of forcing
    /// planners to inline dynamic payloads into op_config.
    #[serde(default)]
    pub stdin_json_port: Option<String>,

    /// Extra non-secret environment variables passed to the child process.
    #[serde(default = "default_env")]
    pub env: serde_json::Value,

    /// Runtime host env bindings injected by the Nix transport when `app` mode is used.
    ///
    /// The map key is the target env name exposed to the flake app, and the map value is the
    /// source env name read from the current host process. Only names are persisted; values are
    /// resolved at execution time and are never recorded.
    #[serde(default)]
    pub host_env_bindings: BTreeMap<String, String>,

    /// Execution timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// Context key that receives the JSON-encoded execution result.
    #[serde(default = "default_write_result_to")]
    pub write_result_to: String,
}

fn default_timeout_ms() -> u64 {
    300_000
}

fn default_env() -> serde_json::Value {
    serde_json::json!({})
}

fn default_write_result_to() -> String {
    "result".to_string()
}

/// Validates the mutually exclusive `program_path` and `app` configuration contract.
pub fn validate_nix_exec_config(cfg: &NixExecStateConfig) -> Result<(), String> {
    match (&cfg.program_path, &cfg.app) {
        (Some(program_path), None) => {
            if !program_path.starts_with("/nix/store/") {
                return Err("program_path must start with /nix/store/".to_string());
            }
            Ok(())
        }
        (None, Some(app)) => {
            if app.trim().is_empty() {
                return Err("app must be a non-empty flake app ref".to_string());
            }
            if !app.contains('#') {
                return Err(
                    "app must contain a '#' fragment (for example github:willyrgf/mfm#jq_fmt_example)"
                        .to_string(),
                );
            }
            Ok(())
        }
        (Some(_), Some(_)) => Err("provide exactly one of program_path or app".to_string()),
        (None, None) => Err("missing program_path or app".to_string()),
    }?;
    if !cfg.env.is_object() {
        return Err("env must be a JSON object".to_string());
    }
    if cfg
        .stdin_json_port
        .as_ref()
        .is_some_and(|port| port.trim().is_empty())
    {
        return Err("stdin_json_port must be a non-empty context key".to_string());
    }
    if cfg
        .host_env_bindings
        .iter()
        .any(|(target, source)| target.trim().is_empty() || source.trim().is_empty())
    {
        return Err(
            "host_env_bindings must contain non-empty target and source env names".to_string(),
        );
    }
    Ok(())
}

/// State that resolves a nix app when needed, executes it, and writes the result to context.
#[derive(Clone, Debug)]
pub struct NixExecState {
    /// State id used for idempotency scope construction.
    pub state_id: StateId,
    /// Execution configuration for the nix call.
    pub cfg: NixExecStateConfig,
}

impl NixExecState {
    fn resolve_stdin_json(&self, ctx: &dyn DynContext) -> Result<serde_json::Value, StateError> {
        if let Some(port) = &self.cfg.stdin_json_port {
            return op_ctx::read_json_required(
                ctx,
                &ContextKey(port.clone()),
                "missing_stdin_json_input",
                "missing stdin json input in context",
            );
        }
        Ok(self.cfg.stdin_json.clone())
    }

    fn exec_request(
        &self,
        program_path: String,
        stdin_json: serde_json::Value,
    ) -> RunProgramRequest {
        RunProgramRequest {
            program_path,
            argv: self.cfg.argv.clone(),
            stdin_json,
            timeout_ms: self.cfg.timeout_ms,
            env: self.cfg.env.clone(),
        }
    }
}

#[async_trait]
impl State for NixExecState {
    fn meta(&self) -> StateMeta {
        meta::execute(op_idempotency::state_purpose(
            "nix_app",
            &self.state_id,
            "exec",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let stdin_json = self.resolve_stdin_json(ctx)?;
        if let Some(app) = &self.cfg.app {
            let mut nix = NixIoClient::new(self.state_id.clone(), io);
            let res = nix
                .run_flake_app(RunFlakeAppRequest {
                    app: app.clone(),
                    argv: self.cfg.argv.clone(),
                    stdin_json: stdin_json.clone(),
                    timeout_ms: self.cfg.timeout_ms,
                    env: self.cfg.env.clone(),
                    host_env_bindings: self.cfg.host_env_bindings.clone(),
                })
                .await
                .map_err(op_errors::state_from_io)?;

            op_ctx::write_json(
                ctx,
                ContextKey(self.cfg.write_result_to.clone()),
                res.response,
            )?;

            return Ok(StateOutcome {
                snapshot: SnapshotPolicy::OnSuccess,
            });
        }

        let program_path = match (&self.cfg.program_path, &self.cfg.app) {
            (Some(program_path), None) => program_path.clone(),
            _ => {
                return Err(op_errors::state_unknown(
                    "invalid_op_config",
                    "missing app or program_path for nix execution",
                ))
            }
        };

        let mut exec = ExecIoClient::new(self.state_id.clone(), io);
        let res = exec
            .run_program(self.exec_request(program_path, stdin_json))
            .await
            .map_err(op_errors::state_from_io)?;

        op_ctx::write_json(
            ctx,
            ContextKey(self.cfg.write_result_to.clone()),
            res.response,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}
