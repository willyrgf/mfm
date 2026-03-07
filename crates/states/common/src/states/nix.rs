//! Shared state for nix app resolution and command execution.

use async_trait::async_trait;
use mfm_collectors_exec::{ExecIoClient, RunProgramRequest};
use mfm_collectors_nix::{NixIoClient, ResolveFlakeAppRequest};
use serde::Deserialize;

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
#[derive(Clone, Debug, Deserialize)]
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
    }
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
            env: serde_json::json!({}),
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
