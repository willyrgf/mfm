use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{ContextKey, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::idempotency as op_idempotency;
use crate::states::meta;

const NAMESPACE_NIX_EXEC: &str = "nix.exec";
const NAMESPACE_EXEC: &str = "exec";

#[derive(Clone, Debug, Deserialize)]
pub struct NixExecStateConfig {
    #[serde(default)]
    pub program_path: Option<String>,

    #[serde(default)]
    pub app: Option<String>,

    #[serde(default)]
    pub argv: Vec<String>,

    #[serde(default)]
    pub stdin_json: serde_json::Value,

    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    #[serde(default = "default_write_result_to")]
    pub write_result_to: String,
}

fn default_timeout_ms() -> u64 {
    300_000
}

fn default_write_result_to() -> String {
    "result".to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum ExecRequest {
    #[serde(rename = "run_program_v1")]
    RunProgramV1 {
        program_path: String,
        argv: Vec<String>,
        stdin_json: serde_json::Value,
        timeout_ms: u64,
        #[serde(default)]
        env: serde_json::Value,
    },
}

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

#[derive(Clone, Debug)]
pub struct NixExecState {
    pub state_id: StateId,
    pub cfg: NixExecStateConfig,
}

impl NixExecState {
    fn preflight_fact_key(&self, req: &serde_json::Value) -> Result<FactKey, StateError> {
        let id = artifact_id_for_json(req).map_err(|_| {
            op_errors::state_unknown(
                "nix_preflight_request_not_canonical",
                "nix preflight request not canonical",
            )
        })?;
        Ok(FactKey(format!(
            "mfm:nix:preflight|state:{}|req:{}",
            self.state_id.0, id.0
        )))
    }

    fn fact_key(&self, req: &serde_json::Value) -> Result<FactKey, StateError> {
        let id = artifact_id_for_json(req).map_err(|_| {
            op_errors::state_unknown("exec_request_not_canonical", "exec request not canonical")
        })?;
        Ok(FactKey(format!(
            "mfm:exec|state:{}|req:{}",
            self.state_id.0, id.0
        )))
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

            let preflight_req = serde_json::json!({
                "kind": "resolve_flake_app_v1",
                "app": app,
                "timeout_ms": self.cfg.timeout_ms,
            });
            let preflight_key = self.preflight_fact_key(&preflight_req)?;
            let preflight = io
                .call(IoCall {
                    namespace: NAMESPACE_NIX_EXEC.to_string(),
                    request: preflight_req,
                    fact_key: Some(preflight_key),
                })
                .await
                .map_err(|_| {
                    op_errors::state_unknown("nix_preflight_failed", "nix preflight failed")
                })?;

            preflight
                .response
                .get("program_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    op_errors::state_unknown(
                        "nix_preflight_invalid_response",
                        "nix preflight response missing program_path",
                    )
                })?
        };

        let req = serde_json::to_value(ExecRequest::RunProgramV1 {
            program_path,
            argv: self.cfg.argv.clone(),
            stdin_json: self.cfg.stdin_json.clone(),
            timeout_ms: self.cfg.timeout_ms,
            env: serde_json::json!({}),
        })
        .map_err(|_| {
            op_errors::state_unknown(
                "exec_request_encode_failed",
                "failed to encode exec request",
            )
        })?;

        let key = self.fact_key(&req)?;
        let res = io
            .call(IoCall {
                namespace: NAMESPACE_EXEC.to_string(),
                request: req,
                fact_key: Some(key),
            })
            .await
            .map_err(|_| op_errors::state_unknown("exec_io_failed", "exec io call failed"))?;

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
