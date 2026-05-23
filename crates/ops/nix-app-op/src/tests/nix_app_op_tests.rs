#![allow(clippy::disallowed_methods)]

use super::*;

use std::collections::HashMap;

use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase};
use mfm_machine::errors::StateError;
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::ContextKey;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{FactIndex, LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::replay_io::ReplayIo;
use mfm_machine::runtime::{DefaultExecutionEngine, EngineFailpoints};
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_sdk::ids::{MachineId, PortKey, StepId};
use mfm_sdk::op::{leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::SdkPlanResolver;
use mfm_state_common::test_support as op_test_support;
use mfm_transports_exec::{ExecPolicy, ExecProgramTransportFactory};
use tokio::sync::Mutex;

use mfm_collectors_nix_exec::{NixFlakePolicy, NixFlakeTransportFactory, NAMESPACE_NIX_EXEC};
use mfm_machine::events::event_envelopes_from_stream_records;
use mfm_machine::stores::StreamId;

fn into_leaf(planned: PlannedOp) -> LeafOpSpec {
    match planned.kind {
        PlannedOpKind::Leaf(spec) => spec,
        PlannedOpKind::Composite(_) => panic!("expected leaf planned op"),
    }
}

fn nix_can_fetch_local_flake(root: &std::path::Path) -> bool {
    std::process::Command::new("nix")
        .arg("flake")
        .arg("metadata")
        .arg("--no-write-lock-file")
        .arg(format!("path:{}", root.display()))
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn expand_accepts_flake_app_ref_config() {
    let op = NixAppOp;
    let cfg = op_test_support::run_config_live();
    let graph = into_leaf(
        mfm_sdk::op::Operation::expand(
            &op,
            OpPath("machine.main".to_string()),
            &serde_json::json!({"app": "github:willyrgf/mfm#jq_fmt_example"}),
            &cfg,
        )
        .expect("expand"),
    );
    assert_eq!(graph.states.len(), 1);
}

#[test]
fn expand_exports_configured_write_result_port() {
    let op = NixAppOp;
    let cfg = op_test_support::run_config_live();
    let planned = mfm_sdk::op::Operation::expand(
        &op,
        OpPath("machine.main".to_string()),
        &serde_json::json!({
            "program_path": "/nix/store/dummy/bin/app",
            "write_result_to": "fetch_origin_result"
        }),
        &cfg,
    )
    .expect("expand");

    assert!(planned
        .interface
        .exports
        .iter()
        .any(|port| port.0 == "fetch_origin_result"));
}

#[test]
fn expand_imports_configured_stdin_json_port() {
    let op = NixAppOp;
    let cfg = op_test_support::run_config_live();
    let planned = mfm_sdk::op::Operation::expand(
        &op,
        OpPath("machine.main".to_string()),
        &serde_json::json!({
            "app": "github:willyrgf/mfm#jq_fmt_example",
            "stdin_json_port": "fetch_origin_result"
        }),
        &cfg,
    )
    .expect("expand");

    assert!(planned
        .interface
        .imports
        .iter()
        .any(|port| port.0 == "fetch_origin_result"));
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
    assert_eq!(err.info.code.as_str(), "invalid_op_config");
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
    assert_eq!(err.info.code.as_str(), "invalid_op_config");
}

#[test]
fn expand_accepts_non_secret_env_object() {
    let op = NixAppOp;
    let cfg = op_test_support::run_config_live();
    let planned = mfm_sdk::op::Operation::expand(
        &op,
        OpPath("machine.main".to_string()),
        &serde_json::json!({
            "app": "github:willyrgf/mfm#jq_fmt_example",
            "env": {
                "MFM_SELECTOR_ENV": "MFM_RUNTIME_VALUE"
            }
        }),
        &cfg,
    )
    .expect("expand");

    assert_eq!(planned.interface.exports.len(), 1);
}

#[test]
fn expand_accepts_host_env_bindings() {
    let op = NixAppOp;
    let cfg = op_test_support::run_config_live();
    let planned = mfm_sdk::op::Operation::expand(
        &op,
        OpPath("machine.main".to_string()),
        &serde_json::json!({
            "app": "github:willyrgf/mfm#jq_fmt_example",
            "host_env_bindings": {
                "MFM_TARGET_PRIVATE_KEY": "MFM_SOURCE_SIGNING_KEY"
            }
        }),
        &cfg,
    )
    .expect("expand");

    assert_eq!(planned.interface.exports.len(), 1);
}

#[test]
fn expand_rejects_non_object_env() {
    let op = NixAppOp;
    let cfg = op_test_support::run_config_live();
    let err = mfm_sdk::op::Operation::expand(
        &op,
        OpPath("machine.main".to_string()),
        &serde_json::json!({
            "app": "github:willyrgf/mfm#jq_fmt_example",
            "env": "not-an-object"
        }),
        &cfg,
    )
    .err()
    .expect("invalid env must fail");

    assert_eq!(err.info.code.as_str(), "invalid_op_config");
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
    fn namespace_group(&self) -> &str {
        "exec"
    }

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
            let kind = call
                .request
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if kind == "run_flake_app_v1" {
                Ok(serde_json::json!({"x": 1}))
            } else {
                Ok(serde_json::json!({"program_path": "/nix/store/dummy/bin/app"}))
            }
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
        OpId::must_new("marker".to_string())
    }

    fn op_version(&self) -> String {
        "v1".to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, mfm_sdk::errors::SdkError> {
        let cfg: MarkerConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_unknown_error("invalid_op_config", "invalid marker op_config")
        })?;
        if cfg.key.trim().is_empty() {
            return Err(op_errors::sdk_unknown_error(
                "invalid_op_config",
                "marker key must be non-empty",
            ));
        }

        let export_key = cfg.key.clone();
        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![PortKey(export_key)],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    "write",
                    Arc::new(MarkerState {
                        key: cfg.key,
                        value: cfg.value,
                    }),
                )?],
                edges: Vec::new(),
            }),
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
            .map_err(|_| op_errors::state_unknown("ctx_write_failed", "context write failed"))?;
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
    let (registry, planner, pipeline) = op_test_support::single_op_plan(
        op,
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

    let cfg = op_test_support::run_config_live();

    let res = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline.clone(),
        cfg.clone(),
    )
    .await
    .expect("start");

    assert_eq!(res.phase, RunPhase::Completed);
    let final_snapshot_id = res.final_snapshot_id.clone().expect("final snapshot");

    let stream = stores
        .streams
        .read_range(&StreamId::run(res.run_id), 1, None)
        .await
        .and_then(|records| event_envelopes_from_stream_records(res.run_id, records))
        .expect("read_range");
    let facts = FactIndex::from_event_stream(&stream).expect("fact index");
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
async fn at_flake_app_mode_runs_via_nix_transport_once() {
    let counts = Arc::new(Mutex::new(0));
    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(CountingExecFactory::new(Arc::clone(&counts)));

    let op: mfm_sdk::op::DynOperation = Arc::new(NixAppOp);
    let (registry, planner, pipeline) = op_test_support::single_op_plan(
        op,
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

    let cfg = op_test_support::run_config_live();

    let res = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline,
        cfg,
    )
    .await
    .expect("start");

    assert_eq!(res.phase, RunPhase::Completed);
    assert_eq!(*counts.lock().await, 1);
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

    let nix_bin =
        std::fs::canonicalize("/nix/var/nix/profiles/default/bin/nix").expect("resolve nix binary");
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
    if !nix_can_fetch_local_flake(&temp_root) {
        std::fs::remove_file(temp_root.join("flake.nix")).expect("remove flake.nix");
        std::fs::remove_dir(temp_root).expect("remove temp root");
        return;
    }

    let repo_prefix = format!("path:{}", temp_root.display());
    let app_ref = format!("{repo_prefix}#echo_json");

    let op: mfm_sdk::op::DynOperation = Arc::new(NixAppOp);
    let (registry, planner, pipeline) = op_test_support::single_op_plan(
        op,
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
        NAMESPACE_NIX_EXEC.to_string(),
        Arc::new(NixFlakeTransportFactory::new(NixFlakePolicy {
            allow_prefixes: vec![repo_prefix.clone()],
        })),
    );

    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(RouterLiveIoTransportFactory::new(routes));
    let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);
    let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

    let stores = op_test_support::in_memory_stores();

    let cfg = op_test_support::run_config_live_with_allowlist(vec![repo_prefix]);

    let res = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline,
        cfg,
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
        .or_else(|| snapshot.get("nix_app.main.out.result").cloned())
        .or_else(|| snapshot.get("nix_app.main.result").cloned())
        .or_else(|| snapshot.get("machine.main.out.result").cloned())
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

    let nix_bin =
        std::fs::canonicalize("/nix/var/nix/profiles/default/bin/nix").expect("resolve nix binary");
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
    if !nix_can_fetch_local_flake(&temp_root) {
        std::fs::remove_file(temp_root.join("flake.nix")).expect("remove flake.nix");
        std::fs::remove_dir(temp_root).expect("remove temp root");
        return;
    }

    let repo_prefix = format!("path:{}", temp_root.display());
    let app_ref = format!("{repo_prefix}#jq_fmt_example");

    let registry = op_test_support::registry_with_ops([
        Arc::new(MarkerOp) as mfm_sdk::op::DynOperation,
        Arc::new(NixAppOp) as mfm_sdk::op::DynOperation,
    ]);

    let planner = op_test_support::default_pipeline_planner();
    let pipeline = Pipeline {
        machine_id: MachineId("nix_multi".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("prep".to_string()),
                op_id: OpId::must_new("marker".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "key": "prep",
                    "value": {"stage": "prep"}
                }),
            },
            PipelineStep {
                step_id: StepId("fmt".to_string()),
                op_id: OpId::must_new("nix_app".to_string()),
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
                op_id: OpId::must_new("marker".to_string()),
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
        NAMESPACE_NIX_EXEC.to_string(),
        Arc::new(NixFlakeTransportFactory::new(NixFlakePolicy {
            allow_prefixes: vec![repo_prefix.clone()],
        })),
    );

    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(RouterLiveIoTransportFactory::new(routes));
    let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);
    let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

    let stores = op_test_support::in_memory_stores();

    let cfg = op_test_support::run_config_live_with_allowlist(vec![repo_prefix]);

    let res = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline,
        cfg,
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
        snapshot.get("nix_multi.prep.out.prep"),
        Some(&serde_json::json!({"stage": "prep"}))
    );
    let formatted = snapshot
        .get("nix_multi.fmt.nix_multi.fmt.result")
        .cloned()
        .or_else(|| snapshot.get("nix_multi.fmt.out.result").cloned())
        .or_else(|| snapshot.get("nix_multi.fmt.result").cloned());
    assert_eq!(
        formatted,
        Some(serde_json::json!({"a": {"x": 3, "y": 2}, "z": 1}))
    );
    assert_eq!(
        snapshot.get("nix_multi.post.out.post"),
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
    let (registry, planner, pipeline) = op_test_support::single_op_plan(
        op,
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

    let cfg = op_test_support::run_config_live();

    let first = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline.clone(),
        cfg.clone(),
    )
    .await
    .expect("start");

    assert_eq!(first.phase, RunPhase::Running);

    let resumed = op_test_support::resume_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
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
