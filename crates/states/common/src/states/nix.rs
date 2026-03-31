//! Shared state for nix app resolution and command execution.

use std::collections::BTreeMap;

use async_trait::async_trait;
use mfm_collectors_exec::{ExecIoClient, RunProgramRequest};
use mfm_collectors_nix::{NixIoClient, ResolveFlakeAppRequest, RunFlakeAppRequest};
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
    fn exec_request(&self, program_path: String) -> RunProgramRequest {
        RunProgramRequest {
            program_path,
            argv: self.cfg.argv.clone(),
            stdin_json: self.cfg.stdin_json.clone(),
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
        if let Some(app) = &self.cfg.app {
            let mut nix = NixIoClient::new(self.state_id.clone(), io);
            let res = nix
                .run_flake_app(RunFlakeAppRequest {
                    app: app.clone(),
                    argv: self.cfg.argv.clone(),
                    stdin_json: self.cfg.stdin_json.clone(),
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

        let program_path = if let Some(program_path) = &self.cfg.program_path {
            program_path.clone()
        } else {
            let app = self.cfg.app.clone().ok_or_else(|| {
                op_errors::state_unknown(
                    "invalid_op_config",
                    "missing app for nix preflight resolution",
                )
            })?;

            let mut nix = NixIoClient::new(self.state_id.clone(), io);
            nix.resolve_flake_app(ResolveFlakeAppRequest {
                app,
                timeout_ms: self.cfg.timeout_ms,
            })
            .await
            .map_err(op_errors::state_from_io)?
            .program_path
        };

        let mut exec = ExecIoClient::new(self.state_id.clone(), io);
        let res = exec
            .run_program(self.exec_request(program_path))
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
