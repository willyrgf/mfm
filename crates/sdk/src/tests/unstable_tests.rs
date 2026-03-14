use super::*;
use mfm_machine::config::{
    BackoffPolicy, ContextCheckpointing, EventProfile, ExecutionMode, IoMode, RetryPolicy,
    RunConfig,
};
use mfm_machine::errors::{ContextError, StateError};
use mfm_machine::ids::ArtifactId;
use mfm_machine::runtime::{DefaultExecutionEngine, PlanResolver};
use mfm_machine::stores::{ArtifactStore, StreamAppend, StreamId, StreamRecord, StreamStore};
use std::sync::Arc;
use tokio::sync::Mutex;

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

#[derive(Clone, Default)]
struct MemStreamStore {
    inner: Arc<Mutex<HashMap<StreamId, Vec<StreamRecord>>>>,
}

#[async_trait]
impl StreamStore for MemStreamStore {
    async fn head_seq(
        &self,
        stream_id: &StreamId,
    ) -> Result<u64, mfm_machine::errors::StorageError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .get(stream_id)
            .and_then(|v| v.last())
            .map(|record| record.seq)
            .unwrap_or(0))
    }

    async fn append(&self, append: StreamAppend) -> Result<u64, mfm_machine::errors::StorageError> {
        let mut inner = self.inner.lock().await;
        let stream = inner.entry(append.stream_id.clone()).or_default();
        let head = stream.last().map(|record| record.seq).unwrap_or(0);
        if head != append.expected_seq {
            return Err(mfm_machine::errors::StorageError::Concurrency(info(
                "stream_store_concurrency",
                ErrorCategory::Storage,
                false,
                "head seq did not match expected seq",
            )));
        }
        let mut next_seq = append.expected_seq + 1;
        for record in append.records {
            stream.push(StreamRecord {
                stream_id: append.stream_id.clone(),
                seq: next_seq,
                ts_millis: record.ts_millis,
                kind: record.kind,
                payload: record.payload,
            });
            next_seq += 1;
        }
        Ok(stream.last().map(|record| record.seq).unwrap_or(head))
    }

    async fn append_batch(
        &self,
        appends: Vec<StreamAppend>,
    ) -> Result<mfm_machine::stores::AppendBatchResult, mfm_machine::errors::StorageError> {
        let mut inner = self.inner.lock().await;
        let mut stream_heads = Vec::with_capacity(appends.len());

        for append in &appends {
            let head = inner
                .get(&append.stream_id)
                .and_then(|records| records.last())
                .map(|record| record.seq)
                .unwrap_or(0);
            if head != append.expected_seq {
                return Err(mfm_machine::errors::StorageError::Concurrency(info(
                    "stream_store_concurrency",
                    ErrorCategory::Storage,
                    false,
                    "head seq did not match expected seq",
                )));
            }
        }

        for append in appends {
            let stream = inner.entry(append.stream_id.clone()).or_default();
            let mut next_seq = append.expected_seq + 1;
            for record in append.records {
                stream.push(StreamRecord {
                    stream_id: append.stream_id.clone(),
                    seq: next_seq,
                    ts_millis: record.ts_millis,
                    kind: record.kind,
                    payload: record.payload,
                });
                next_seq += 1;
            }
            let head = stream
                .last()
                .map(|record| record.seq)
                .unwrap_or(append.expected_seq);
            stream_heads.push((append.stream_id, head));
        }

        Ok(mfm_machine::stores::AppendBatchResult { stream_heads })
    }

    async fn read_range(
        &self,
        stream_id: &StreamId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, mfm_machine::errors::StorageError> {
        let inner = self.inner.lock().await;
        let Some(stream) = inner.get(stream_id) else {
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
        let id = artifact_id_for_bytes(&bytes);
        self.inner.lock().await.insert(id.clone(), bytes);
        Ok(id)
    }

    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, mfm_machine::errors::StorageError> {
        let inner = self.inner.lock().await;
        inner.get(id).cloned().ok_or_else(|| {
            mfm_machine::errors::StorageError::NotFound(info(
                "artifact_not_found",
                ErrorCategory::Storage,
                false,
                "artifact not found",
            ))
        })
    }

    async fn exists(&self, id: &ArtifactId) -> Result<bool, mfm_machine::errors::StorageError> {
        Ok(self.inner.lock().await.contains_key(id))
    }
}

#[derive(Clone)]
struct WriteKeyState {
    key: &'static str,
    value: serde_json::Value,
}

#[async_trait]
impl State for WriteKeyState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
            side_effects: mfm_machine::meta::SideEffectKind::Pure,
            idempotency: mfm_machine::meta::Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        ctx.write(ContextKey(self.key.to_string()), self.value.clone())
            .map_err(|_| StateError {
                state_id: None,
                info: info(
                    "ctx_write_failed",
                    ErrorCategory::Context,
                    false,
                    "context write failed",
                ),
            })?;
        Ok(StateOutcome {
            snapshot: mfm_machine::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone)]
struct ReadThenWriteState {
    read_key: &'static str,
    write_key: &'static str,
}

#[async_trait]
impl State for ReadThenWriteState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
            side_effects: mfm_machine::meta::SideEffectKind::Pure,
            idempotency: mfm_machine::meta::Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let v = ctx
            .read(&ContextKey(self.read_key.to_string()))
            .map_err(|_| StateError {
                state_id: None,
                info: info(
                    "ctx_read_failed",
                    ErrorCategory::Context,
                    false,
                    "context read failed",
                ),
            })?
            .unwrap_or(serde_json::Value::Null);

        ctx.write(ContextKey(self.write_key.to_string()), v)
            .map_err(|_| StateError {
                state_id: None,
                info: info(
                    "ctx_write_failed",
                    ErrorCategory::Context,
                    false,
                    "context write failed",
                ),
            })?;

        Ok(StateOutcome {
            snapshot: mfm_machine::state::SnapshotPolicy::OnSuccess,
        })
    }
}

struct TestOp {
    op_id: OpId,
    op_version: String,
    io: OpIo,
    graph: StateGraph,
}

impl TestOp {
    fn new_write(
        op_id: &str,
        op_version: &str,
        state_id: &str,
        key: &'static str,
        value: serde_json::Value,
        io: OpIo,
    ) -> Self {
        let state: DynState = Arc::new(WriteKeyState { key, value });
        Self {
            op_id: OpId::must_new(op_id.to_string()),
            op_version: op_version.to_string(),
            io,
            graph: StateGraph {
                states: vec![StateNode {
                    id: StateId::must_new(state_id.to_string()),
                    state,
                }],
                edges: Vec::new(),
            },
        }
    }

    fn new_read_then_write(
        op_id: &str,
        op_version: &str,
        state_id: &str,
        read_key: &'static str,
        write_key: &'static str,
        io: OpIo,
    ) -> Self {
        let state: DynState = Arc::new(ReadThenWriteState {
            read_key,
            write_key,
        });
        Self {
            op_id: OpId::must_new(op_id.to_string()),
            op_version: op_version.to_string(),
            io,
            graph: StateGraph {
                states: vec![StateNode {
                    id: StateId::must_new(state_id.to_string()),
                    state,
                }],
                edges: Vec::new(),
            },
        }
    }
}

impl crate::op::Operation for TestOp {
    fn op_id(&self) -> OpId {
        self.op_id.clone()
    }

    fn op_version(&self) -> String {
        self.op_version.clone()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(self.io.clone())
    }

    fn expand(
        &self,
        _op_path: OpPath,
        _op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        Ok(self.graph.clone())
    }
}

#[test]
fn planner_adds_step_barrier_edges() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp::new_write(
        "op1",
        "v1",
        "m.step1.s1",
        "k",
        serde_json::json!("v1"),
        OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
    )));
    reg.register(Arc::new(TestOp::new_write(
        "op2",
        "v1",
        "m.step2.s1",
        "k",
        serde_json::json!("v2"),
        OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
    )));

    let pipeline = Pipeline {
        machine_id: MachineId("m".to_string()),
        pipeline_version: "v".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("step1".to_string()),
                op_id: OpId::must_new("op1".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            },
            PipelineStep {
                step_id: StepId("step2".to_string()),
                op_id: OpId::must_new("op2".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            },
        ],
    };

    let plan = DefaultPipelinePlanner
        .build_execution_plan(Arc::new(reg), &pipeline, &run_config_live())
        .expect("plan should build");

    assert!(
        plan.graph
            .edges
            .iter()
            .any(|e| e.from.as_str() == "m.step1.s1" && e.to.as_str() == "m.step2.s1"),
        "expected barrier edge from step1 to step2"
    );
}

#[test]
fn planner_rejects_apply_side_effect_without_idempotency_key() {
    #[derive(Clone)]
    struct ApplyNoIdemState;

    #[async_trait]
    impl State for ApplyNoIdemState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
                side_effects: mfm_machine::meta::SideEffectKind::ApplySideEffect,
                idempotency: mfm_machine::meta::Idempotency::None,
            }
        }

        async fn handle(
            &self,
            _ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            Ok(StateOutcome {
                snapshot: mfm_machine::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp {
        op_id: OpId::must_new("op".to_string()),
        op_version: "v1".to_string(),
        io: OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
        graph: StateGraph {
            states: vec![StateNode {
                id: StateId::must_new("op.main.s1".to_string()),
                state: Arc::new(ApplyNoIdemState),
            }],
            edges: Vec::new(),
        },
    }));

    let pipeline = single_op_pipeline(
        OpId::must_new("op".to_string()),
        "v1".to_string(),
        serde_json::json!({}),
    )
    .expect("pipeline");

    let err = match DefaultPipelinePlanner.build_execution_plan(
        Arc::new(reg),
        &pipeline,
        &run_config_live(),
    ) {
        Ok(_) => panic!("expected planner error"),
        Err(e) => e,
    };
    assert_eq!(err.info.code.0, "missing_idempotency_key");
}

#[test]
fn planner_validates_unsatisfied_import() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp::new_read_then_write(
        "op1",
        "v1",
        "m.step1.s1",
        "x",
        "y",
        OpIo {
            imports: vec![PortKey("x".to_string())],
            exports: Vec::new(),
        },
    )));

    let pipeline = Pipeline {
        machine_id: MachineId("m".to_string()),
        pipeline_version: "v".to_string(),
        steps: vec![PipelineStep {
            step_id: StepId("step1".to_string()),
            op_id: OpId::must_new("op1".to_string()),
            op_version: "v1".to_string(),
            op_config: serde_json::json!({}),
        }],
    };

    let err = match DefaultPipelinePlanner.build_execution_plan(
        Arc::new(reg),
        &pipeline,
        &run_config_live(),
    ) {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert_eq!(err.info.code.0, "unsatisfied_import");
}

#[test]
fn planner_rejects_secrets_in_op_config() {
    let reg = HashMapOperationRegistry::default();
    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);

    let pipeline = Pipeline {
        machine_id: MachineId("m".to_string()),
        pipeline_version: "v".to_string(),
        steps: vec![PipelineStep {
            step_id: StepId("step1".to_string()),
            op_id: OpId::must_new("op1".to_string()),
            op_version: "v1".to_string(),
            op_config: serde_json::json!({ "password": "x" }),
        }],
    };

    let err = match planner.build_execution_plan(Arc::new(reg), &pipeline, &run_config_live()) {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert_eq!(err.info.code.0, "secrets_detected");
}

#[tokio::test]
async fn launcher_rejects_secrets_in_manifest_input() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp::new_write(
        "op1",
        "v1",
        "m.step1.s1",
        "k",
        serde_json::json!("v1"),
        OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
    )));

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);

    struct NeverResolver;
    impl mfm_machine::runtime::PlanResolver for NeverResolver {
        fn resolve(&self, _manifest: &RunManifest) -> Result<ExecutionPlan, RunError> {
            Err(RunError::InvalidPlan(info(
                "resolver_unavailable",
                ErrorCategory::Unknown,
                false,
                "resolver unavailable",
            )))
        }
    }

    let engine: Arc<dyn ExecutionEngine> =
        Arc::new(DefaultExecutionEngine::new(Arc::new(NeverResolver)));
    let stores = Stores {
        streams: Arc::new(MemStreamStore::default()),
        artifacts: Arc::new(MemArtifactStore::default()),
    };

    let pipeline = Pipeline {
        machine_id: MachineId("m".to_string()),
        pipeline_version: "v".to_string(),
        steps: vec![PipelineStep {
            step_id: StepId("step1".to_string()),
            op_id: OpId::must_new("op1".to_string()),
            op_version: "v1".to_string(),
            op_config: serde_json::json!({}),
        }],
    };

    let err = launcher
        .start_pipeline(
            engine,
            stores,
            Arc::new(reg),
            planner,
            LaunchPipeline {
                pipeline,
                input: serde_json::json!({ "authorization": "Bearer x" }),
                run_config: run_config_live(),
                build: mfm_machine::config::BuildProvenance {
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
        .unwrap_err();

    match err {
        RunError::InvalidPlan(info) => assert_eq!(info.code.0, "secrets_detected"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn namespacing_prevents_context_collisions_and_wires_imports() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp::new_write(
        "op1",
        "v1",
        "m.step1.s1",
        "x",
        serde_json::json!("v1"),
        OpIo {
            imports: Vec::new(),
            exports: vec![PortKey("x".to_string())],
        },
    )));
    reg.register(Arc::new(TestOp::new_read_then_write(
        "op2",
        "v1",
        "m.step2.s1",
        "x",
        "y",
        OpIo {
            imports: vec![PortKey("x".to_string())],
            exports: Vec::new(),
        },
    )));
    reg.register(Arc::new(TestOp::new_write(
        "op3",
        "v1",
        "m.step3.s1",
        "x",
        serde_json::json!("v2"),
        OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
    )));

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);

    // A resolver is required by the engine type but isn't used for start().
    struct NeverResolver;
    impl mfm_machine::runtime::PlanResolver for NeverResolver {
        fn resolve(&self, _manifest: &RunManifest) -> Result<ExecutionPlan, RunError> {
            Err(RunError::InvalidPlan(info(
                "resolver_unavailable",
                ErrorCategory::Unknown,
                false,
                "resolver unavailable",
            )))
        }
    }

    let engine: Arc<dyn ExecutionEngine> =
        Arc::new(DefaultExecutionEngine::new(Arc::new(NeverResolver)));
    let stores = Stores {
        streams: Arc::new(MemStreamStore::default()),
        artifacts: Arc::new(MemArtifactStore::default()),
    };

    let pipeline = Pipeline {
        machine_id: MachineId("m".to_string()),
        pipeline_version: "v".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("step1".to_string()),
                op_id: OpId::must_new("op1".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            },
            PipelineStep {
                step_id: StepId("step2".to_string()),
                op_id: OpId::must_new("op2".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            },
            PipelineStep {
                step_id: StepId("step3".to_string()),
                op_id: OpId::must_new("op3".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            },
        ],
    };

    let result = launcher
        .start_pipeline(
            engine,
            Stores {
                streams: Arc::clone(&stores.streams),
                artifacts: Arc::clone(&stores.artifacts),
            },
            Arc::new(reg),
            planner,
            LaunchPipeline {
                pipeline,
                input: serde_json::json!({}),
                run_config: run_config_live(),
                build: mfm_machine::config::BuildProvenance {
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
        .expect("run should succeed");

    let snap_id = result.final_snapshot_id.expect("snapshot id");
    let bytes = stores.artifacts.get(&snap_id).await.expect("get snapshot");
    let v = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
    let obj = v.as_object().expect("object");

    // step1 writes x, but it must be namespaced.
    assert_eq!(obj.get("m.step1.x"), Some(&serde_json::json!("v1")));

    // step2 reads imported x (from step1) and writes y in its own namespace.
    assert_eq!(obj.get("m.step2.y"), Some(&serde_json::json!("v1")));

    // step3 writes x too; it must not collide with step1.
    assert_eq!(obj.get("m.step3.x"), Some(&serde_json::json!("v2")));
}

#[tokio::test]
async fn sdk_plan_resolver_rebuilds_plan_from_manifest() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp::new_write(
        "op1",
        "v1",
        "m.step1.s1",
        "k",
        serde_json::json!("v1"),
        OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
    )));

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let resolver = SdkPlanResolver::new(Arc::new(reg), Arc::clone(&planner));

    let pipeline = Pipeline {
        machine_id: MachineId("m".to_string()),
        pipeline_version: "v".to_string(),
        steps: vec![PipelineStep {
            step_id: StepId("step1".to_string()),
            op_id: OpId::must_new("op1".to_string()),
            op_version: "v1".to_string(),
            op_config: serde_json::json!({}),
        }],
    };

    let manifest = RunManifest {
        op_id: OpId::must_new("m".to_string()),
        op_version: "v".to_string(),
        input_params: serde_json::to_value(PipelineManifestInput {
            pipeline: pipeline.clone(),
            input: serde_json::json!({}),
        })
        .unwrap(),
        run_config: run_config_live(),
        build: mfm_machine::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };

    let plan = resolver.resolve(&manifest).expect("resolve plan");
    assert_eq!(plan.op_id.as_str(), "m");
    assert_eq!(plan.graph.states.len(), 1);
    assert_eq!(plan.graph.states[0].id.as_str(), "m.step1.s1");
}

#[tokio::test]
async fn single_op_report_returns_typed_report() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp::new_write(
        "report_op",
        "v1",
        "report_op.main.s1",
        "report",
        serde_json::json!({"value": 42}),
        OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
    )));

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let registry: Arc<dyn OperationRegistry> = Arc::new(reg);
    let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));
    let engine: Arc<dyn ExecutionEngine> = Arc::new(DefaultExecutionEngine::new(resolver));
    let stores = Stores {
        streams: Arc::new(MemStreamStore::default()),
        artifacts: Arc::new(MemArtifactStore::default()),
    };

    let report: serde_json::Value = execute_single_op_report(
        engine,
        stores,
        registry,
        planner,
        SingleOpReportRequest {
            op_id: "report_op".to_string(),
            op_version: "v1".to_string(),
            op_config: serde_json::json!({}),
            report_context_key: "report_op.main.report".to_string(),
        },
    )
    .await
    .expect("single-op report should decode");

    assert_eq!(report, serde_json::json!({"value": 42}));
}

#[tokio::test]
async fn single_op_report_errors_when_report_key_missing() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(TestOp::new_write(
        "report_missing_op",
        "v1",
        "report_missing_op.main.s1",
        "unused",
        serde_json::json!({"value": 1}),
        OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        },
    )));

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let registry: Arc<dyn OperationRegistry> = Arc::new(reg);
    let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));
    let engine: Arc<dyn ExecutionEngine> = Arc::new(DefaultExecutionEngine::new(resolver));
    let stores = Stores {
        streams: Arc::new(MemStreamStore::default()),
        artifacts: Arc::new(MemArtifactStore::default()),
    };

    let err = execute_single_op_report::<serde_json::Value>(
        engine,
        stores,
        registry,
        planner,
        SingleOpReportRequest {
            op_id: "report_missing_op".to_string(),
            op_version: "v1".to_string(),
            op_config: serde_json::json!({}),
            report_context_key: "report_missing_op.main.report".to_string(),
        },
    )
    .await
    .expect_err("missing report key should fail");

    assert_eq!(err.code, "MissingReport");
}

#[derive(Clone)]
struct FailingState {
    code: &'static str,
    message: &'static str,
}

#[async_trait]
impl State for FailingState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
            side_effects: mfm_machine::meta::SideEffectKind::Pure,
            idempotency: mfm_machine::meta::Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        Err(StateError {
            state_id: Some(StateId::must_new("fail_op.main.s1".to_string())),
            info: ErrorInfo {
                code: ErrorCode(self.code.to_string()),
                category: ErrorCategory::Unknown,
                retryable: false,
                message: self.message.to_string(),
                details: None,
            },
        })
    }
}

struct FailOp;

impl crate::op::Operation for FailOp {
    fn op_id(&self) -> OpId {
        OpId::must_new("fail_op".to_string())
    }

    fn op_version(&self) -> String {
        "v1".to_string()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: Vec::new(),
        })
    }

    fn expand(
        &self,
        _op_path: OpPath,
        _op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        Ok(StateGraph {
            states: vec![StateNode {
                id: StateId::must_new("fail_op.main.s1".to_string()),
                state: Arc::new(FailingState {
                    code: "IntentionalFailure",
                    message: "intentional test failure",
                }),
            }],
            edges: Vec::new(),
        })
    }
}

#[tokio::test]
async fn single_op_report_maps_failed_run_state_error() {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(FailOp));

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let registry: Arc<dyn OperationRegistry> = Arc::new(reg);
    let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));
    let engine: Arc<dyn ExecutionEngine> = Arc::new(DefaultExecutionEngine::new(resolver));
    let stores = Stores {
        streams: Arc::new(MemStreamStore::default()),
        artifacts: Arc::new(MemArtifactStore::default()),
    };

    let err = execute_single_op_report::<serde_json::Value>(
        engine,
        stores,
        registry,
        planner,
        SingleOpReportRequest {
            op_id: "fail_op".to_string(),
            op_version: "v1".to_string(),
            op_config: serde_json::json!({}),
            report_context_key: "fail_op.main.report".to_string(),
        },
    )
    .await
    .expect_err("failed run should surface state failure");

    assert_eq!(err.code, "IntentionalFailure");
    assert_eq!(err.message, "intentional test failure");
}
