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
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StateError};
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{ContextKey, ErrorCode, FactKey, OpId, OpPath, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};
use mfm_machine::plan::StateGraph;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

const OP_ID: &str = "nix_app";
const OP_VERSION: &str = "v1";
const NAMESPACE_NIX_EXEC: &str = "nix.exec";
const NAMESPACE_EXEC: &str = "exec";

fn info(code: &'static str, category: ErrorCategory, retryable: bool, message: &str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details: None,
    }
}

fn sdk_err(code: &'static str, message: &'static str) -> SdkError {
    SdkError {
        info: info(code, ErrorCategory::Unknown, false, message),
    }
}

fn state_err(code: &'static str, message: &'static str) -> StateError {
    StateError {
        state_id: None,
        info: info(code, ErrorCategory::Unknown, false, message),
    }
}

fn ctx_key(op_path: &OpPath, suffix: &str) -> ContextKey {
    ContextKey(format!("{}.{}", op_path.0, suffix))
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
            op_path,
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
    op_path: OpPath,
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
        StateMeta {
            tags: vec![Tag("execute".to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ApplySideEffect,
            idempotency: Idempotency::Key(format!("mfm:exec|state:{}", self.state_id.0)),
        }
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

        let out_key = ctx_key(&self.op_path, &self.cfg.write_result_to);
        ctx.write(out_key, res.response)
            .map_err(|_| state_err("ctx_write_failed", "context write failed"))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use mfm_machine::config::{
        BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
        RetryPolicy,
    };
    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::events::{Event, EventEnvelope, KernelEvent};
    use mfm_machine::exec_transport::{ExecPolicy, ExecProgramTransportFactory};
    use mfm_machine::hashing::artifact_id_for_json;
    use mfm_machine::ids::{ArtifactId, RunId};
    use mfm_machine::io::IoCall;
    use mfm_machine::live_io::{FactIndex, LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
    use mfm_machine::replay_io::ReplayIo;
    use mfm_machine::runtime::{DefaultExecutionEngine, EngineFailpoints};
    use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};
    use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
    use mfm_sdk::pipeline::PipelinePlanner;
    use mfm_sdk::unstable::{
        single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
        SdkPlanResolver,
    };
    use tokio::sync::Mutex;

    use crate::nix_exec_transport::{NixFlakePolicy, NixFlakeTransportFactory};

    fn run_config_live() -> RunConfig {
        RunConfig {
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
            nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
        }
    }

    fn run_config_live_with_allowlist(prefixes: Vec<String>) -> RunConfig {
        let mut cfg = run_config_live();
        cfg.nix_flake_allowlist = prefixes;
        cfg
    }

    #[test]
    fn expand_accepts_flake_app_ref_config() {
        let op = NixAppOp;
        let cfg = run_config_live();
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
        let cfg = run_config_live();
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
        let cfg = run_config_live();
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

    #[derive(Default)]
    struct MapContext {
        inner: HashMap<String, serde_json::Value>,
    }

    impl MapContext {
        fn from_snapshot(v: serde_json::Value) -> Self {
            let obj = v.as_object().cloned().unwrap_or_default();
            let mut inner = HashMap::new();
            for (k, v) in obj {
                inner.insert(k, v);
            }
            Self { inner }
        }
    }

    impl DynContext for MapContext {
        fn read(
            &self,
            key: &ContextKey,
        ) -> Result<Option<serde_json::Value>, mfm_machine::errors::ContextError> {
            Ok(self.inner.get(&key.0).cloned())
        }

        fn write(
            &mut self,
            key: ContextKey,
            value: serde_json::Value,
        ) -> Result<(), mfm_machine::errors::ContextError> {
            self.inner.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), mfm_machine::errors::ContextError> {
            self.inner.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, mfm_machine::errors::ContextError> {
            let mut m = serde_json::Map::new();
            for (k, v) in &self.inner {
                m.insert(k.clone(), v.clone());
            }
            Ok(serde_json::Value::Object(m))
        }
    }

    fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    #[derive(Clone, Default)]
    struct MemEventStore {
        inner: Arc<Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
    }

    #[async_trait]
    impl EventStore for MemEventStore {
        async fn head_seq(&self, run_id: RunId) -> Result<u64, mfm_machine::errors::StorageError> {
            let inner = self.inner.lock().await;
            Ok(inner
                .get(&run_id)
                .and_then(|v| v.last())
                .map(|e| e.seq)
                .unwrap_or(0))
        }

        async fn append(
            &self,
            run_id: RunId,
            expected_seq: u64,
            events: Vec<EventEnvelope>,
        ) -> Result<u64, mfm_machine::errors::StorageError> {
            let mut inner = self.inner.lock().await;
            let stream = inner.entry(run_id).or_default();
            let head = stream.last().map(|e| e.seq).unwrap_or(0);
            if head != expected_seq {
                return Err(mfm_machine::errors::StorageError::Concurrency(info(
                    "event_store_concurrency",
                    ErrorCategory::Storage,
                    "head seq did not match expected seq",
                )));
            }

            stream.extend(events);
            Ok(stream.last().map(|e| e.seq).unwrap_or(head))
        }

        async fn read_range(
            &self,
            run_id: RunId,
            from_seq: u64,
            to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, mfm_machine::errors::StorageError> {
            let inner = self.inner.lock().await;
            let Some(stream) = inner.get(&run_id) else {
                return Ok(Vec::new());
            };

            let from = from_seq.max(1);
            let to = to_seq.unwrap_or(u64::MAX);
            Ok(stream
                .iter()
                .filter(|e| e.seq >= from && e.seq <= to)
                .cloned()
                .collect())
        }
    }

    #[derive(Clone, Default)]
    struct MemArtifactStore {
        inner: Arc<Mutex<HashMap<ArtifactId, Vec<u8>>>>,
    }

    #[async_trait]
    impl ArtifactStore for MemArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            bytes: Vec<u8>,
        ) -> Result<ArtifactId, mfm_machine::errors::StorageError> {
            let id = mfm_machine::hashing::artifact_id_for_bytes(&bytes);
            self.inner.lock().await.insert(id.clone(), bytes);
            Ok(id)
        }

        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, mfm_machine::errors::StorageError> {
            let inner = self.inner.lock().await;
            inner.get(id).cloned().ok_or_else(|| {
                mfm_machine::errors::StorageError::NotFound(info(
                    "not_found",
                    ErrorCategory::Storage,
                    "artifact not found",
                ))
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, mfm_machine::errors::StorageError> {
            Ok(self.inner.lock().await.contains_key(id))
        }
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

    fn run_started(stream: &[EventEnvelope]) -> (ArtifactId, ArtifactId) {
        for e in stream {
            if let Event::Kernel(KernelEvent::RunStarted {
                manifest_id,
                initial_snapshot_id,
                ..
            }) = &e.event
            {
                return (manifest_id.clone(), initial_snapshot_id.clone());
            }
        }
        panic!("missing RunStarted");
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

        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = run_config_live();

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
                    initial_context: Box::new(MapContext::default()),
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
        let (_manifest_id, initial_snapshot_id) = run_started(&stream);

        let bytes = stores
            .artifacts
            .get(&initial_snapshot_id)
            .await
            .expect("get initial snapshot");
        let initial = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
        let mut ctx = MapContext::from_snapshot(initial);

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

        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = run_config_live();

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
                    initial_context: Box::new(MapContext::default()),
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

        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = run_config_live_with_allowlist(vec![repo_prefix]);

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
                    initial_context: Box::new(MapContext::default()),
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

        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = run_config_live();

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
                    initial_context: Box::new(MapContext::default()),
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
