//! Nix-app execution op (Milestone 2).
//!
//! Source of truth: `REDESIGN.md` (v4), especially the Replay/IO contract.
//!
//! This op expands into a single state that requests external execution via `namespace="exec"`.

pub mod nix_exec_transport;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::plan::StateGraph;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_op_common::ctx as op_ctx;
use mfm_op_common::errors as op_errors;
use mfm_op_common::idempotency as op_idempotency;
use mfm_op_common::states::meta;

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

const OP_ID: &str = "nix_app";
const OP_VERSION: &str = "v1";
const NAMESPACE_NIX_EXEC: &str = "nix.exec";
const NAMESPACE_EXEC: &str = "exec";

fn sdk_err(code: &'static str, message: &'static str) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::Unknown, false, message)
}

fn state_err(code: &'static str, message: &'static str) -> StateError {
    op_errors::state_unknown(code, message)
}

#[derive(Clone, Debug, Deserialize)]
struct NixAppConfig {
    #[serde(default)]
    program_path: Option<String>,

    #[serde(default)]
    app: Option<String>,

    #[serde(default)]
    argv: Vec<String>,

    #[serde(default)]
    stdin_json: serde_json::Value,

    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,

    #[serde(default = "default_write_result_to")]
    write_result_to: String,
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

#[derive(Clone, Default)]
pub struct NixAppOp;

impl Operation for NixAppOp {
    fn op_id(&self) -> OpId {
        OpId(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: vec![PortKey("result".to_string())],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: NixAppConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid nix_app op_config"))?;
        match (&cfg.program_path, &cfg.app) {
            (Some(program_path), None) => {
                if !program_path.starts_with("/nix/store/") {
                    return Err(sdk_err(
                        "program_path_not_allowed",
                        "program_path must start with /nix/store/",
                    ));
                }
            }
            (None, Some(app)) => {
                if app.trim().is_empty() {
                    return Err(sdk_err(
                        "invalid_app_ref",
                        "app must be a non-empty flake app ref",
                    ));
                }
                if !app.contains('#') {
                    return Err(sdk_err(
                        "invalid_app_ref",
                        "app must contain a '#' fragment (for example github:willyrgf/mfm#jq_fmt_example)",
                    ));
                }
            }
            (Some(_), Some(_)) => {
                return Err(sdk_err(
                    "invalid_op_config",
                    "provide exactly one of program_path or app",
                ));
            }
            (None, None) => {
                return Err(sdk_err("invalid_op_config", "missing program_path or app"));
            }
        }

        let sid = StateId(format!("{}.run", op_path.0));
        let state = Arc::new(NixAppState {
            state_id: sid.clone(),
            cfg,
        });

        Ok(StateGraph {
            states: vec![mfm_machine::plan::StateNode { id: sid, state }],
            edges: Vec::new(),
        })
    }
}

struct NixAppState {
    state_id: StateId,
    cfg: NixAppConfig,
}

impl NixAppState {
    fn preflight_fact_key(&self, req: &serde_json::Value) -> Result<FactKey, StateError> {
        let id = artifact_id_for_json(req).map_err(|_| {
            state_err(
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
        let id = artifact_id_for_json(req)
            .map_err(|_| state_err("exec_request_not_canonical", "exec request not canonical"))?;
        Ok(FactKey(format!(
            "mfm:exec|state:{}|req:{}",
            self.state_id.0, id.0
        )))
    }
}

#[async_trait]
impl State for NixAppState {
    fn meta(&self) -> StateMeta {
        meta::execute(op_idempotency::state_scope("mfm:exec", &self.state_id))
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
                state_err(
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
                .map_err(|_| state_err("nix_preflight_failed", "nix preflight failed"))?;

            preflight
                .response
                .get("program_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    state_err(
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
            state_err(
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
            .map_err(|_| state_err("exec_io_failed", "exec io call failed"))?;

        let out_key = ContextKey(self.cfg.write_result_to.clone());
        op_ctx::write_json(ctx, out_key, res.response)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use mfm_machine::config::BuildProvenance;
    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::exec_transport::{ExecPolicy, ExecProgramTransportFactory};
    use mfm_machine::hashing::artifact_id_for_json;
    use mfm_machine::io::IoCall;
    use mfm_machine::live_io::{FactIndex, LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
    use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};
    use mfm_machine::replay_io::ReplayIo;
    use mfm_machine::runtime::{DefaultExecutionEngine, EngineFailpoints};
    use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
    use mfm_op_common::test_support as op_test_support;
    use mfm_sdk::ids::{MachineId, PortKey, StepId};
    use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
    use mfm_sdk::op::{OpIo, Operation};
    use mfm_sdk::pipeline::{Pipeline, PipelinePlanner, PipelineStep};
    use mfm_sdk::unstable::{
        single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
        SdkPlanResolver,
    };
    use tokio::sync::Mutex;

    use crate::nix_exec_transport::{NixFlakePolicy, NixFlakeTransportFactory};

    #[test]
    fn expand_accepts_flake_app_ref_config() {
        let op = NixAppOp;
        let cfg = op_test_support::run_config_live();
        let graph = mfm_sdk::op::Operation::expand(
            &op,
            OpPath("machine.main".to_string()),
            &serde_json::json!({"app": "github:willyrgf/mfm#jq_fmt_example"}),
            &cfg,
        )
        .expect("expand");
        assert_eq!(graph.states.len(), 1);
    }

    #[test]
    fn expand_rejects_both_program_path_and_app() {
        let op = NixAppOp;
        let cfg = op_test_support::run_config_live();
        let got = mfm_sdk::op::Operation::expand(
            &op,
            OpPath("machine.main".to_string()),
            &serde_json::json!({
                "program_path": "/nix/store/dummy/bin/app",
                "app": "github:willyrgf/mfm#jq_fmt_example"
            }),
            &cfg,
        );
        let err = match got {
            Ok(_) => panic!("expected error"),
            Err(err) => err,
        };
        assert_eq!(err.info.code.0, "invalid_op_config");
    }

    #[test]
    fn expand_rejects_missing_program_path_and_app() {
        let op = NixAppOp;
        let cfg = op_test_support::run_config_live();
        let got = mfm_sdk::op::Operation::expand(
            &op,
            OpPath("machine.main".to_string()),
            &serde_json::json!({}),
            &cfg,
        );
        let err = match got {
            Ok(_) => panic!("expected error"),
            Err(err) => err,
        };
        assert_eq!(err.info.code.0, "invalid_op_config");
    }

    #[derive(Clone)]
    struct CountingExecFactory {
        counts: Arc<Mutex<u64>>,
    }

    impl CountingExecFactory {
        fn new(counts: Arc<Mutex<u64>>) -> Self {
            Self { counts }
        }
    }

    impl LiveIoTransportFactory for CountingExecFactory {
        fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(CountingExecTransport {
                counts: Arc::clone(&self.counts),
            })
        }
    }

    struct CountingExecTransport {
        counts: Arc<Mutex<u64>>,
    }

    #[async_trait]
    impl LiveIoTransport for CountingExecTransport {
        async fn call(
            &mut self,
            call: IoCall,
        ) -> Result<serde_json::Value, mfm_machine::errors::IoError> {
            *self.counts.lock().await += 1;
            if call.namespace == NAMESPACE_NIX_EXEC {
                Ok(serde_json::json!({"program_path": "/nix/store/dummy/bin/app"}))
            } else {
                Ok(serde_json::json!({"ok": true, "result": {"x": 1}}))
            }
        }
    }

    struct NoopRecorder;
    #[async_trait]
    impl EventRecorder for NoopRecorder {
        async fn emit(
            &mut self,
            _event: mfm_machine::events::DomainEvent,
        ) -> Result<(), mfm_machine::errors::RunError> {
            Ok(())
        }
        async fn emit_many(
            &mut self,
            _events: Vec<mfm_machine::events::DomainEvent>,
        ) -> Result<(), mfm_machine::errors::RunError> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct MarkerOp;

    #[derive(Clone, Debug, serde::Deserialize)]
    struct MarkerConfig {
        key: String,
        value: serde_json::Value,
    }

    struct MarkerState {
        key: String,
        value: serde_json::Value,
    }

    impl Operation for MarkerOp {
        fn op_id(&self) -> OpId {
            OpId("marker".to_string())
        }

        fn op_version(&self) -> String {
            "v1".to_string()
        }

        fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, mfm_sdk::errors::SdkError> {
            let cfg: MarkerConfig = serde_json::from_value(op_config.clone())
                .map_err(|_| sdk_err("invalid_op_config", "invalid marker op_config"))?;
            Ok(OpIo {
                imports: Vec::new(),
                exports: vec![PortKey(cfg.key)],
            })
        }

        fn expand(
            &self,
            op_path: OpPath,
            op_config: &serde_json::Value,
            _run_config: &RunConfig,
        ) -> Result<mfm_machine::plan::StateGraph, mfm_sdk::errors::SdkError> {
            let cfg: MarkerConfig = serde_json::from_value(op_config.clone())
                .map_err(|_| sdk_err("invalid_op_config", "invalid marker op_config"))?;
            if cfg.key.trim().is_empty() {
                return Err(sdk_err("invalid_op_config", "marker key must be non-empty"));
            }

            let state_id = StateId(format!("{}.write", op_path.0));
            Ok(mfm_machine::plan::StateGraph {
                states: vec![mfm_machine::plan::StateNode {
                    id: state_id,
                    state: Arc::new(MarkerState {
                        key: cfg.key,
                        value: cfg.value,
                    }),
                }],
                edges: Vec::new(),
            })
        }
    }

    #[async_trait]
    impl State for MarkerState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: vec![Tag("marker".to_string())],
                depends_on: Vec::new(),
                depends_on_strategy: DependencyStrategy::Latest,
                side_effects: SideEffectKind::Pure,
                idempotency: Idempotency::None,
            }
        }

        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            _io: &mut dyn mfm_machine::io::IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            ctx.write(ContextKey(self.key.clone()), self.value.clone())
                .map_err(|_| state_err("ctx_write_failed", "context write failed"))?;
            Ok(StateOutcome {
                snapshot: SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[tokio::test]
    async fn at_live_then_replay_determinism() {
        let counts = Arc::new(Mutex::new(0));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingExecFactory::new(Arc::clone(&counts)));

        let op: mfm_sdk::op::DynOperation = Arc::new(NixAppOp);
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = single_op_pipeline(
            op.op_id(),
            op.op_version(),
            serde_json::json!({"program_path": "/nix/store/dummy/bin/app"}),
        )
        .expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine =
            DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = op_test_support::run_config_live();

        let res = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline: pipeline.clone(),
                    input: serde_json::json!({}),
                    run_config: cfg.clone(),
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(op_test_support::MapContext::default()),
                },
            )
            .await
            .expect("start");

        assert_eq!(res.phase, RunPhase::Completed);
        let final_snapshot_id = res.final_snapshot_id.clone().expect("final snapshot");

        let stream = stores
            .events
            .read_range(res.run_id, 1, None)
            .await
            .expect("read_range");
        let facts = FactIndex::from_event_stream(&stream);
        let (_manifest_id, initial_snapshot_id) = op_test_support::run_started(&stream);

        let bytes = stores
            .artifacts
            .get(&initial_snapshot_id)
            .await
            .expect("get initial snapshot");
        let initial = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
        let mut ctx = op_test_support::MapContext::from_snapshot(initial);

        let plan = planner
            .build_execution_plan(Arc::clone(&registry), &pipeline, &cfg)
            .expect("plan");
        let node = &plan.graph.states[0];
        let mut io = ReplayIo::new(
            res.run_id,
            node.id.clone(),
            0,
            Arc::clone(&stores.artifacts),
            facts.clone(),
            false,
        );
        let mut rec = NoopRecorder;
        node.state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("handle");

        let v = ctx.dump().expect("dump");
        let computed = artifact_id_for_json(&v).expect("hash");
        assert_eq!(computed, final_snapshot_id);

        assert_eq!(*counts.lock().await, 1);
    }

    #[tokio::test]
    async fn at_flake_app_mode_runs_preflight_and_exec_once_each() {
        let counts = Arc::new(Mutex::new(0));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingExecFactory::new(Arc::clone(&counts)));

        let op: mfm_sdk::op::DynOperation = Arc::new(NixAppOp);
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = single_op_pipeline(
            op.op_id(),
            op.op_version(),
            serde_json::json!({"app": "github:willyrgf/mfm#jq_fmt_example"}),
        )
        .expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine =
            DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = op_test_support::run_config_live();

        let res = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline,
                    input: serde_json::json!({}),
                    run_config: cfg,
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(op_test_support::MapContext::default()),
                },
            )
            .await
            .expect("start");

        assert_eq!(res.phase, RunPhase::Completed);
        assert_eq!(*counts.lock().await, 2);
    }

    #[tokio::test]
    async fn at_flake_app_mode_real_preflight_and_exec_local_path_ref() {
        if std::process::Command::new("nix")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let nix_bin = std::fs::canonicalize("/nix/var/nix/profiles/default/bin/nix")
            .expect("resolve nix binary");
        let system = match std::env::consts::OS {
            "macos" => format!("{}-darwin", std::env::consts::ARCH),
            other => format!("{}-{other}", std::env::consts::ARCH),
        };

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let temp_root = std::env::temp_dir().join(format!("mfm-nix-app-op-{nonce}"));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let temp_root = temp_root.canonicalize().expect("canonical temp root");
        let flake_nix = format!(
            "{{\n  outputs = {{ self }}: {{\n    apps.{system}.echo_json = {{\n      type = \"app\";\n      program = \"{}\";\n    }};\n  }};\n}}\n",
            nix_bin.display()
        );
        std::fs::write(temp_root.join("flake.nix"), flake_nix).expect("write flake.nix");

        let repo_prefix = format!("path:{}", temp_root.display());
        let app_ref = format!("{repo_prefix}#echo_json");

        let op: mfm_sdk::op::DynOperation = Arc::new(NixAppOp);
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = single_op_pipeline(
            op.op_id(),
            op.op_version(),
            serde_json::json!({
                "app": app_ref,
                "argv": ["eval", "--json", "--expr", "{ b = 1; a = 2; }"],
                "stdin_json": {"ignored": true},
                "timeout_ms": 60000,
                "write_result_to": "result"
            }),
        )
        .expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));

        let mut routes: HashMap<String, Arc<dyn LiveIoTransportFactory>> = HashMap::new();
        routes.insert(
            "exec".to_string(),
            Arc::new(ExecProgramTransportFactory::new(ExecPolicy {
                allow_prefixes: vec!["/nix/store/".to_string()],
            })),
        );
        routes.insert(
            "nix".to_string(),
            Arc::new(NixFlakeTransportFactory::new(NixFlakePolicy {
                allow_prefixes: vec![repo_prefix.clone()],
            })),
        );

        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(RouterLiveIoTransportFactory::new(routes));
        let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = op_test_support::run_config_live_with_allowlist(vec![repo_prefix]);

        let res = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline,
                    input: serde_json::json!({}),
                    run_config: cfg,
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(op_test_support::MapContext::default()),
                },
            )
            .await
            .expect("start");

        assert_eq!(res.phase, RunPhase::Completed);

        let final_snapshot_id = res.final_snapshot_id.expect("final snapshot");
        let bytes = stores
            .artifacts
            .get(&final_snapshot_id)
            .await
            .expect("get final snapshot");
        let snapshot = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json snapshot");

        let result = snapshot
            .get("nix_app.main.nix_app.main.result")
            .cloned()
            .or_else(|| snapshot.get("nix_app.main.result").cloned())
            .or_else(|| snapshot.get("machine.main.result").cloned())
            .or_else(|| snapshot.get("machine.main.machine.main.result").cloned());
        assert_eq!(result, Some(serde_json::json!({"a": 2, "b": 1})));

        std::fs::remove_file(temp_root.join("flake.nix")).expect("remove flake.nix");
        std::fs::remove_dir(temp_root).expect("remove temp root");
    }

    #[tokio::test]
    async fn at_multi_step_pipeline_runs_nix_jq_fmt_example() {
        if std::process::Command::new("nix")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let nix_bin = std::fs::canonicalize("/nix/var/nix/profiles/default/bin/nix")
            .expect("resolve nix binary");
        let system = match std::env::consts::OS {
            "macos" => format!("{}-darwin", std::env::consts::ARCH),
            other => format!("{}-{other}", std::env::consts::ARCH),
        };

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let temp_root = std::env::temp_dir().join(format!("mfm-nix-multi-{nonce}"));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let temp_root = temp_root.canonicalize().expect("canonical temp root");
        let flake_nix = format!(
            "{{\n  outputs = {{ self }}: {{\n    apps.{system}.jq_fmt_example = {{\n      type = \"app\";\n      program = \"{}\";\n    }};\n  }};\n}}\n",
            nix_bin.display()
        );
        std::fs::write(temp_root.join("flake.nix"), flake_nix).expect("write flake.nix");

        let repo_prefix = format!("path:{}", temp_root.display());
        let app_ref = format!("{repo_prefix}#jq_fmt_example");

        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(MarkerOp));
        reg.register(Arc::new(NixAppOp));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = Pipeline {
            machine_id: MachineId("nix_multi".to_string()),
            pipeline_version: "v1".to_string(),
            steps: vec![
                PipelineStep {
                    step_id: StepId("prep".to_string()),
                    op_id: OpId("marker".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({
                        "key": "prep",
                        "value": {"stage": "prep"}
                    }),
                },
                PipelineStep {
                    step_id: StepId("fmt".to_string()),
                    op_id: OpId("nix_app".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({
                        "app": app_ref,
                        "argv": ["eval", "--json", "--expr", "{ z = 1; a = { y = 2; x = 3; }; }"],
                        "stdin_json": {"ignored": true},
                        "timeout_ms": 60000,
                        "write_result_to": "result"
                    }),
                },
                PipelineStep {
                    step_id: StepId("post".to_string()),
                    op_id: OpId("marker".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({
                        "key": "post",
                        "value": {"stage": "post"}
                    }),
                },
            ],
        };

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));

        let mut routes: HashMap<String, Arc<dyn LiveIoTransportFactory>> = HashMap::new();
        routes.insert(
            "exec".to_string(),
            Arc::new(ExecProgramTransportFactory::new(ExecPolicy {
                allow_prefixes: vec!["/nix/store/".to_string()],
            })),
        );
        routes.insert(
            "nix".to_string(),
            Arc::new(NixFlakeTransportFactory::new(NixFlakePolicy {
                allow_prefixes: vec![repo_prefix.clone()],
            })),
        );

        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(RouterLiveIoTransportFactory::new(routes));
        let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = op_test_support::run_config_live_with_allowlist(vec![repo_prefix]);

        let res = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline,
                    input: serde_json::json!({}),
                    run_config: cfg,
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(op_test_support::MapContext::default()),
                },
            )
            .await
            .expect("start");

        assert_eq!(res.phase, RunPhase::Completed);

        let final_snapshot_id = res.final_snapshot_id.expect("final snapshot");
        let bytes = stores
            .artifacts
            .get(&final_snapshot_id)
            .await
            .expect("get final snapshot");
        let snapshot = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json snapshot");

        assert_eq!(
            snapshot.get("nix_multi.prep.prep"),
            Some(&serde_json::json!({"stage": "prep"}))
        );
        let formatted = snapshot
            .get("nix_multi.fmt.nix_multi.fmt.result")
            .cloned()
            .or_else(|| snapshot.get("nix_multi.fmt.result").cloned());
        assert_eq!(
            formatted,
            Some(serde_json::json!({"a": {"x": 3, "y": 2}, "z": 1}))
        );
        assert_eq!(
            snapshot.get("nix_multi.post.post"),
            Some(&serde_json::json!({"stage": "post"}))
        );

        std::fs::remove_file(temp_root.join("flake.nix")).expect("remove flake.nix");
        std::fs::remove_dir(temp_root).expect("remove temp root");
    }

    #[tokio::test]
    async fn at_crash_resume_orphan_attempt_reuses_facts() {
        let counts = Arc::new(Mutex::new(0));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingExecFactory::new(Arc::clone(&counts)));

        let failpoints = EngineFailpoints::default();
        failpoints
            .stop_after_handler_once
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let op: mfm_sdk::op::DynOperation = Arc::new(NixAppOp);
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = single_op_pipeline(
            op.op_id(),
            op.op_version(),
            serde_json::json!({"program_path": "/nix/store/dummy/bin/app"}),
        )
        .expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine = DefaultExecutionEngine::new(resolver)
            .with_live_transport_factory(Arc::clone(&factory))
            .with_failpoints(failpoints.clone());
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = op_test_support::run_config_live();

        let first = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline: pipeline.clone(),
                    input: serde_json::json!({}),
                    run_config: cfg.clone(),
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(op_test_support::MapContext::default()),
                },
            )
            .await
            .expect("start");

        assert_eq!(first.phase, RunPhase::Running);

        let resumed = launcher
            .resume(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                first.run_id,
            )
            .await
            .expect("resume");
        assert_eq!(resumed.phase, RunPhase::Completed);

        // Handler ran twice, but transport should be called once due to fact reuse.
        assert_eq!(*counts.lock().await, 1);
    }
}
