use super::*;
use crate::context::DynContext;
use crate::context_runtime::JsonContext;
use crate::errors::StateError;
use crate::errors::StorageError;
use crate::errors::{ErrorCategory, ErrorInfo, IoError};
use crate::events::{DomainEvent, FactRecorded, DOMAIN_EVENT_FACT_RECORDED};
use crate::hashing::artifact_id_for_bytes;
use crate::ids::ContextKey;
use crate::ids::ErrorCode;
use crate::ids::FactKey;
use crate::io::IoCall;
use crate::io::IoProvider;
use crate::live_io::{LiveIoTransport, LiveIoTransportFactory};
use crate::meta::{standard_tags, DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};
use crate::plan::StateGraph;
use crate::recorder::EventRecorder;
use crate::state::State;
use crate::stores::{ArtifactKind, ArtifactStore, EventStore};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct MemEventStore {
    inner: Arc<Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
}

#[async_trait]
impl EventStore for MemEventStore {
    async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError> {
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
    ) -> Result<u64, StorageError> {
        let mut inner = self.inner.lock().await;
        let stream = inner.entry(run_id).or_default();
        let head = stream.last().map(|e| e.seq).unwrap_or(0);
        if head != expected_seq {
            return Err(StorageError::Concurrency(info(
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
    ) -> Result<Vec<EventEnvelope>, StorageError> {
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
    async fn put(&self, _kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        let id = artifact_id_for_bytes(&bytes);
        self.inner.lock().await.insert(id.clone(), bytes);
        Ok(id)
    }

    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
        let inner = self.inner.lock().await;
        inner
            .get(id)
            .cloned()
            .ok_or_else(|| storage_not_found("not_found", "artifact not found"))
    }

    async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
        Ok(self.inner.lock().await.contains_key(id))
    }
}

struct FixedResolver {
    plan: ExecutionPlan,
}

impl PlanResolver for FixedResolver {
    fn resolve(&self, _manifest: &RunManifest) -> Result<ExecutionPlan, RunError> {
        Ok(self.plan.clone())
    }
}

#[derive(Clone)]
struct SetKeyState;

#[async_trait]
impl State for SetKeyState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::Pure,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        let existing = ctx.read(&ContextKey("x".to_string())).expect("read");
        if existing.is_some() {
            return Err(StateError {
                state_id: None,
                info: ErrorInfo {
                    code: ErrorCode("unexpected_context".to_string()),
                    category: ErrorCategory::Context,
                    retryable: false,
                    message: "unexpected context value".to_string(),
                    details: None,
                },
            });
        }

        ctx.write(ContextKey("x".to_string()), serde_json::json!(1))
            .expect("write");
        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone)]
struct TaggedWriteState {
    called: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait]
impl State for TaggedWriteState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: vec![Tag(standard_tags::APPLY_SIDE_EFFECT.to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ApplySideEffect,
            idempotency: Idempotency::Key("test:tagged_write".to_string()),
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        self.called
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ctx.write(ContextKey("y".to_string()), serde_json::json!(2))
            .expect("write");
        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone)]
struct RecordFactAndWriteState;

#[async_trait]
impl State for RecordFactAndWriteState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        let got = io
            .call(IoCall {
                namespace: "test".to_string(),
                request: serde_json::json!({"q": 1}),
                fact_key: Some(FactKey("k".to_string())),
            })
            .await
            .expect("io");
        assert_eq!(got.response, serde_json::json!({ "n": 0 }));

        ctx.write(ContextKey("x".to_string()), serde_json::json!(1))
            .expect("write");

        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

fn base_run_config() -> RunConfig {
    RunConfig {
        io_mode: crate::config::IoMode::Live,
        retry_policy: crate::config::RetryPolicy {
            max_attempts: 3,
            backoff: BackoffPolicy::Fixed {
                delay: Duration::from_millis(0),
            },
        },
        event_profile: crate::config::EventProfile::Minimal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: crate::config::ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
        nix_flake_allowlist: crate::config::default_nix_flake_allowlist(),
    }
}

fn replay_run_config() -> RunConfig {
    let mut cfg = base_run_config();
    cfg.io_mode = crate::config::IoMode::Replay;
    cfg
}

async fn store_manifest(artifacts: &dyn ArtifactStore, manifest: &RunManifest) -> ArtifactId {
    let value = serde_json::to_value(manifest).unwrap();
    let bytes = crate::hashing::canonical_json_bytes(&value).unwrap();
    artifacts.put(ArtifactKind::Manifest, bytes).await.unwrap()
}

const SECRET_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const SECRET_PASSWORD: &str = "hunter2";

#[derive(Clone)]
struct SecretErrorState;

#[async_trait]
impl State for SecretErrorState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::Pure,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        Err(StateError {
            state_id: None,
            info: ErrorInfo {
                code: ErrorCode("secret_error".to_string()),
                category: ErrorCategory::Unknown,
                retryable: false,
                message: format!("password leaked: {SECRET_PASSWORD}"),
                details: Some(serde_json::json!({
                    "private_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                })),
            },
        })
    }
}

#[derive(Clone)]
struct EmitSecretDomainEventState;

#[async_trait]
impl State for EmitSecretDomainEventState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::Pure,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        let res = rec
            .emit(DomainEvent {
                name: "test".to_string(),
                payload: serde_json::json!({ "password": SECRET_PASSWORD }),
                payload_ref: None,
            })
            .await;

        if res.is_err() {
            return Err(StateError {
                state_id: None,
                info: ErrorInfo {
                    code: ErrorCode("emit_failed".to_string()),
                    category: ErrorCategory::Unknown,
                    retryable: false,
                    message: "emit failed".to_string(),
                    details: None,
                },
            });
        }

        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone)]
struct SecretFactIoState;

#[async_trait]
impl State for SecretFactIoState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        io.call(IoCall {
            namespace: "test".to_string(),
            request: serde_json::json!({}),
            fact_key: Some(FactKey("test:fact".to_string())),
        })
        .await
        .map_err(|_| StateError {
            state_id: None,
            info: ErrorInfo {
                code: ErrorCode("io_failed".to_string()),
                category: ErrorCategory::Unknown,
                retryable: false,
                message: "io failed".to_string(),
                details: None,
            },
        })?;

        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[derive(Clone)]
struct SecretTransportFactory;

struct SecretTransport;

#[async_trait]
impl LiveIoTransport for SecretTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            "test" => Ok(serde_json::json!({ "password": SECRET_PASSWORD })),
            _ => Ok(serde_json::json!({})),
        }
    }
}

impl LiveIoTransportFactory for SecretTransportFactory {
    fn namespace_group(&self) -> &str {
        "test"
    }

    fn make(&self, _env: crate::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(SecretTransport)
    }
}

async fn assert_no_secret_bytes_in_artifacts(artifacts: &MemArtifactStore, needles: &[&str]) {
    let inner = artifacts.inner.lock().await;
    for bytes in inner.values() {
        let s = String::from_utf8_lossy(bytes);
        for n in needles {
            assert!(!s.contains(n));
        }
    }
}

#[tokio::test]
async fn secrets_in_initial_context_are_rejected_and_not_persisted() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = base_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(SetKeyState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver);

    let mut initial = JsonContext::new();
    initial
        .write(
            ContextKey("mnemonic".to_string()),
            serde_json::json!(SECRET_MNEMONIC),
        )
        .unwrap();

    let err = engine
        .start(
            stores,
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(initial),
            },
        )
        .await
        .unwrap_err();

    match err {
        RunError::Context(crate::errors::ContextError::Other(info)) => {
            assert_eq!(info.code.0, "secrets_detected");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    // No event stream was created, and no artifact contains the secret.
    assert!(events.inner.lock().await.is_empty());
    assert_no_secret_bytes_in_artifacts(&artifacts, &[SECRET_MNEMONIC, "mnemonic"]).await;
}

#[tokio::test]
async fn state_failed_error_messages_are_redacted_before_persisting() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = base_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id.clone(),
                state: Arc::new(SecretErrorState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver);

    let r = engine
        .start(
            stores,
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect("start");

    assert_eq!(r.phase, RunPhase::Failed);

    let stream = events.read_range(r.run_id, 1, None).await.expect("read");
    let failed = stream.iter().find_map(|e| match &e.event {
        Event::Kernel(KernelEvent::StateFailed { error, .. }) => Some(error.clone()),
        _ => None,
    });
    let failed = failed.expect("StateFailed event");
    assert_eq!(failed.state_id, Some(state_id));
    assert_eq!(failed.info.message, "error details redacted");
    assert!(failed.info.details.is_none());

    // Persisted surfaces must not contain the leaked secret.
    let serialized = serde_json::to_string(&stream).unwrap();
    assert!(!serialized.contains(SECRET_PASSWORD));
    assert!(!serialized.to_ascii_lowercase().contains("private_key"));
}

#[tokio::test]
async fn domain_events_with_secrets_are_rejected() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let mut run_config = base_run_config();
    run_config.event_profile = crate::config::EventProfile::Normal;

    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: StateId::must_new("machine.main.s1".to_string()),
                state: Arc::new(EmitSecretDomainEventState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver);

    let r = engine
        .start(
            stores,
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect("start");

    assert_eq!(r.phase, RunPhase::Failed);

    let stream = events.read_range(r.run_id, 1, None).await.expect("read");
    assert!(!stream.iter().any(|e| matches!(e.event, Event::Domain(_))));

    let serialized = serde_json::to_string(&stream).unwrap();
    assert!(!serialized.contains(SECRET_PASSWORD));
    assert!(!serialized.to_ascii_lowercase().contains("password"));
}

#[tokio::test]
async fn fact_payloads_with_secrets_are_rejected() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let mut run_config = base_run_config();
    run_config.event_profile = crate::config::EventProfile::Normal;

    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: StateId::must_new("machine.main.s1".to_string()),
                state: Arc::new(SecretFactIoState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver)
        .with_live_transport_factory(Arc::new(SecretTransportFactory));

    let r = engine
        .start(
            stores,
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect("start");

    assert_eq!(r.phase, RunPhase::Failed);

    let stream = events.read_range(r.run_id, 1, None).await.expect("read");
    assert!(!stream.iter().any(|e| match &e.event {
        Event::Domain(de) => de.name == DOMAIN_EVENT_FACT_RECORDED,
        _ => false,
    }));

    // No persisted bytes should include the secret-bearing "password" field.
    let serialized = serde_json::to_string(&stream).unwrap();
    assert!(!serialized.to_ascii_lowercase().contains("password"));
    assert_no_secret_bytes_in_artifacts(&artifacts, &["password", SECRET_PASSWORD]).await;
}

#[tokio::test]
async fn start_then_resume_retries_orphan_attempt_from_base_snapshot() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = base_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id.clone(),
                state: Arc::new(SetKeyState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let failpoints = EngineFailpoints {
        stop_after_handler_once: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    };
    let engine = DefaultExecutionEngine::new(resolver).with_failpoints(failpoints);

    let initial_ctx = Box::new(JsonContext::new());

    let r1 = engine
        .start(
            stores(),
            StartRun {
                manifest: manifest.clone(),
                manifest_id: manifest_id.clone(),
                plan: plan.clone(),
                run_config: run_config.clone(),
                initial_context: initial_ctx,
            },
        )
        .await
        .expect("start");

    assert_eq!(r1.phase, RunPhase::Running);

    let r2 = engine.resume(stores(), r1.run_id).await.expect("resume");
    assert_eq!(r2.phase, RunPhase::Completed);
    let final_snapshot_id = r2.final_snapshot_id.expect("final snapshot");

    let snapshot =
        crate::context_runtime::read_full_snapshot_value(artifacts.as_ref(), &final_snapshot_id)
            .await
            .expect("read snapshot");
    assert_eq!(snapshot, serde_json::json!({"x": 1}));

    let stream = events.read_range(r1.run_id, 1, None).await.expect("read");
    let entered: Vec<u32> = stream
        .iter()
        .filter_map(|e| match &e.event {
            Event::Kernel(KernelEvent::StateEntered {
                state_id: sid,
                attempt,
                ..
            }) if sid == &state_id => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![0, 1]);
}

#[derive(Clone)]
struct FlakyRetryableState {
    calls: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait]
impl State for FlakyRetryableState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::Pure,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            return Err(StateError {
                state_id: None,
                info: ErrorInfo {
                    code: ErrorCode("flaky".to_string()),
                    category: ErrorCategory::Unknown,
                    retryable: true,
                    message: "flaky".to_string(),
                    details: None,
                },
            });
        }
        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[tokio::test]
async fn retry_policy_retries_retryable_errors() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = base_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id.clone(),
                state: Arc::new(FlakyRetryableState {
                    calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                }),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver);

    let r = engine
        .start(
            stores(),
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect("start");
    assert_eq!(r.phase, RunPhase::Completed);

    let stream = events.read_range(r.run_id, 1, None).await.expect("read");
    let entered: Vec<u32> = stream
        .iter()
        .filter_map(|e| match &e.event {
            Event::Kernel(KernelEvent::StateEntered {
                state_id: sid,
                attempt,
                ..
            }) if sid == &state_id => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![0, 1]);
}

#[tokio::test]
async fn rejects_fanout_join_execution_mode() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let mut run_config = base_run_config();
    run_config.execution_mode = ExecutionMode::FanOutJoin { max_concurrency: 2 };
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: StateId::must_new("machine.main.s1".to_string()),
                state: Arc::new(SetKeyState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver);

    let err = engine
        .start(
            stores(),
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect_err("expected error");

    match err {
        RunError::InvalidPlan(info) => assert_eq!(info.code.0, CODE_UNSUPPORTED_EXECUTION_MODE),
        other => panic!("expected InvalidPlan, got: {other:?}"),
    }
}

#[tokio::test]
async fn skip_tags_skips_tagged_states_without_running_handler() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let mut run_config = base_run_config();
    run_config.skip_tags = vec![Tag(standard_tags::APPLY_SIDE_EFFECT.to_string())];
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let called = Arc::new(std::sync::atomic::AtomicU32::new(0));

    let s1 = StateId::must_new("machine.main.s1".to_string());
    let s2 = StateId::must_new("machine.main.s2".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![
                StateNode {
                    id: s1.clone(),
                    state: Arc::new(SetKeyState),
                },
                StateNode {
                    id: s2.clone(),
                    state: Arc::new(TaggedWriteState {
                        called: Arc::clone(&called),
                    }),
                },
            ],
            edges: vec![DependencyEdge {
                from: s1.clone(),
                to: s2.clone(),
            }],
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver);

    let r = engine
        .start(
            stores(),
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect("start");

    assert_eq!(r.phase, RunPhase::Completed);
    assert_eq!(
        called.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "tagged state should be skipped"
    );

    let final_snapshot_id = r.final_snapshot_id.expect("final snapshot");
    let snapshot =
        crate::context_runtime::read_full_snapshot_value(artifacts.as_ref(), &final_snapshot_id)
            .await
            .expect("read snapshot");
    assert_eq!(snapshot, serde_json::json!({"x": 1}));

    let stream = events.read_range(r.run_id, 1, None).await.expect("read");
    let mut s1_snapshot = None;
    let mut s2_enter_base = None;
    let mut s2_completed_snapshot = None;

    for e in &stream {
        match &e.event {
            Event::Kernel(KernelEvent::StateCompleted {
                state_id,
                context_snapshot_id,
            }) if state_id == &s1 => {
                s1_snapshot = Some(context_snapshot_id.clone());
            }
            Event::Kernel(KernelEvent::StateEntered {
                state_id,
                base_snapshot_id,
                ..
            }) if state_id == &s2 => {
                s2_enter_base = Some(base_snapshot_id.clone());
            }
            Event::Kernel(KernelEvent::StateCompleted {
                state_id,
                context_snapshot_id,
            }) if state_id == &s2 => {
                s2_completed_snapshot = Some(context_snapshot_id.clone());
            }
            _ => {}
        }
    }

    let s1_snapshot = s1_snapshot.expect("s1 snapshot");
    assert_eq!(s2_enter_base, Some(s1_snapshot.clone()));
    assert_eq!(s2_completed_snapshot, Some(s1_snapshot.clone()));
    assert_eq!(final_snapshot_id, s1_snapshot);
}

#[tokio::test]
async fn crash_resume_orphan_attempt_reuses_facts() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = base_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(CountingTransportFactory {
        calls: Arc::clone(&calls),
    });

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id.clone(),
                state: Arc::new(RecordFactAndWriteState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let failpoints = EngineFailpoints {
        stop_after_handler_once: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    };
    let engine = DefaultExecutionEngine::new(resolver)
        .with_live_transport_factory(factory)
        .with_failpoints(failpoints);

    let initial_ctx = Box::new(JsonContext::new());
    let r1 = engine
        .start(
            stores(),
            StartRun {
                manifest: manifest.clone(),
                manifest_id: manifest_id.clone(),
                plan: plan.clone(),
                run_config: run_config.clone(),
                initial_context: initial_ctx,
            },
        )
        .await
        .expect("start");

    assert_eq!(r1.phase, RunPhase::Running);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    let r2 = engine.resume(stores(), r1.run_id).await.expect("resume");
    assert_eq!(r2.phase, RunPhase::Completed);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    let final_snapshot_id = r2.final_snapshot_id.expect("final snapshot");
    let snapshot =
        crate::context_runtime::read_full_snapshot_value(artifacts.as_ref(), &final_snapshot_id)
            .await
            .expect("read snapshot");
    assert_eq!(snapshot, serde_json::json!({"x": 1}));

    let stream = events.read_range(r1.run_id, 1, None).await.expect("read");
    let entered: Vec<u32> = stream
        .iter()
        .filter_map(|e| match &e.event {
            Event::Kernel(KernelEvent::StateEntered {
                state_id: sid,
                attempt,
                ..
            }) if sid == &state_id => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![0, 1]);

    let facts: Vec<FactRecorded> = stream
        .iter()
        .filter_map(|e| match &e.event {
            Event::Domain(de) if de.name == DOMAIN_EVENT_FACT_RECORDED => {
                serde_json::from_value::<FactRecorded>(de.payload.clone()).ok()
            }
            _ => None,
        })
        .collect();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].key.0, "k");
}

struct CountingTransport {
    calls: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait]
impl LiveIoTransport for CountingTransport {
    async fn call(&mut self, _call: IoCall) -> Result<serde_json::Value, IoError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(serde_json::json!({ "n": n }))
    }
}

struct CountingTransportFactory {
    calls: Arc<std::sync::atomic::AtomicU32>,
}

impl LiveIoTransportFactory for CountingTransportFactory {
    fn namespace_group(&self) -> &str {
        "test"
    }

    fn make(&self, _env: crate::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(CountingTransport {
            calls: Arc::clone(&self.calls),
        })
    }
}

struct TrackingTransportFactory {
    makes: Arc<std::sync::atomic::AtomicU32>,
    calls: Arc<std::sync::atomic::AtomicU32>,
}

impl LiveIoTransportFactory for TrackingTransportFactory {
    fn namespace_group(&self) -> &str {
        "test"
    }

    fn make(&self, _env: crate::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
        self.makes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::new(TrackingTransport {
            calls: Arc::clone(&self.calls),
        })
    }
}

struct TrackingTransport {
    calls: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait]
impl LiveIoTransport for TrackingTransport {
    async fn call(&mut self, _call: IoCall) -> Result<serde_json::Value, IoError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(serde_json::json!({ "n": 999 }))
    }
}

#[derive(Clone)]
struct RecordFactThenFailOnce {
    handled: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait]
impl State for RecordFactThenFailOnce {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        let got = io
            .call(IoCall {
                namespace: "test".to_string(),
                request: serde_json::json!({"q": 1}),
                fact_key: Some(FactKey("k".to_string())),
            })
            .await
            .expect("io");

        assert_eq!(got.response, serde_json::json!({ "n": 0 }));

        let n = self
            .handled
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            return Err(StateError {
                state_id: None,
                info: ErrorInfo {
                    code: ErrorCode("flaky".to_string()),
                    category: ErrorCategory::Unknown,
                    retryable: true,
                    message: "flaky".to_string(),
                    details: None,
                },
            });
        }

        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[tokio::test]
async fn facts_are_single_assignment_and_reused_across_retries() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = base_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(CountingTransportFactory {
        calls: Arc::clone(&calls),
    });

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id.clone(),
                state: Arc::new(RecordFactThenFailOnce {
                    handled: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                }),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);

    let r = engine
        .start(
            stores(),
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect("start");
    assert_eq!(r.phase, RunPhase::Completed);
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "transport call should be deduped by fact key"
    );

    let stream = events.read_range(r.run_id, 1, None).await.expect("read");
    let facts: Vec<FactRecorded> = stream
        .iter()
        .filter_map(|e| match &e.event {
            Event::Domain(de) if de.name == DOMAIN_EVENT_FACT_RECORDED => {
                serde_json::from_value::<FactRecorded>(de.payload.clone()).ok()
            }
            _ => None,
        })
        .collect();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].key.0, "k");
}

#[derive(Clone)]
struct TimeAndRandomState;

#[async_trait]
impl State for TimeAndRandomState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        let _t0 = io.now_millis().await.expect("time");
        let _t1 = io.now_millis().await.expect("time");
        let r = io.random_bytes(8).await.expect("random");
        assert_eq!(r.len(), 8);

        Ok(crate::state::StateOutcome {
            snapshot: crate::state::SnapshotPolicy::OnSuccess,
        })
    }
}

#[tokio::test]
async fn time_and_random_are_recorded_as_facts() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = base_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id.clone(),
                state: Arc::new(TimeAndRandomState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver);

    let r = engine
        .start(
            stores(),
            StartRun {
                manifest,
                manifest_id,
                plan,
                run_config,
                initial_context: Box::new(JsonContext::new()),
            },
        )
        .await
        .expect("start");
    assert_eq!(r.phase, RunPhase::Completed);

    let stream = events.read_range(r.run_id, 1, None).await.expect("read");
    let facts: Vec<FactRecorded> = stream
        .iter()
        .filter_map(|e| match &e.event {
            Event::Domain(de) if de.name == DOMAIN_EVENT_FACT_RECORDED => {
                serde_json::from_value::<FactRecorded>(de.payload.clone()).ok()
            }
            _ => None,
        })
        .collect();
    assert_eq!(facts.len(), 3);

    assert!(facts[0].key.0.starts_with("mfm:now_millis|"));
    assert!(facts[1].key.0.starts_with("mfm:now_millis|"));
    assert!(facts[2].key.0.starts_with("mfm:random_bytes|"));

    for fr in facts {
        assert!(artifacts.exists(&fr.payload_id).await.expect("exists"));
    }
}

#[tokio::test]
async fn replay_mode_serves_recorded_facts_without_live_io() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = replay_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let initial_snapshot_id = write_full_snapshot_value(artifacts.as_ref(), serde_json::json!({}))
        .await
        .expect("initial snapshot");

    // Seed a recorded fact so the run can execute without hitting live IO.
    let payload = serde_json::json!({ "n": 0 });
    let bytes = crate::hashing::canonical_json_bytes(&payload).expect("canonical payload");
    let payload_id = artifacts
        .put(ArtifactKind::FactPayload, bytes)
        .await
        .expect("store payload");

    let domain = DomainEvent {
        name: DOMAIN_EVENT_FACT_RECORDED.to_string(),
        payload: serde_json::to_value(FactRecorded {
            key: FactKey("k".to_string()),
            payload_id,
            meta: serde_json::json!({}),
        })
        .expect("payload"),
        payload_ref: None,
    };

    let run_id = RunId(uuid::Uuid::new_v4());
    events
        .append(
            run_id,
            0,
            vec![
                EventEnvelope {
                    run_id,
                    seq: 1,
                    ts_millis: None,
                    event: Event::Kernel(KernelEvent::RunStarted {
                        op_id: manifest.op_id.clone(),
                        manifest_id: manifest_id.clone(),
                        initial_snapshot_id: initial_snapshot_id.clone(),
                    }),
                },
                EventEnvelope {
                    run_id,
                    seq: 2,
                    ts_millis: None,
                    event: Event::Domain(domain),
                },
            ],
        )
        .await
        .expect("seed run");

    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let makes = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(TrackingTransportFactory {
        makes: Arc::clone(&makes),
        calls: Arc::clone(&calls),
    });

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(RecordFactAndWriteState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);

    let r = engine.resume(stores(), run_id).await.expect("resume");
    assert_eq!(r.phase, RunPhase::Completed);
    assert_eq!(makes.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[derive(Clone)]
struct ReplayMissingFactState;

#[async_trait]
impl State for ReplayMissingFactState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<crate::state::StateOutcome, StateError> {
        let err = io
            .call(IoCall {
                namespace: "test".to_string(),
                request: serde_json::json!({"q": 1}),
                fact_key: Some(FactKey("k".to_string())),
            })
            .await
            .expect_err("missing fact");

        let info = match err {
            IoError::MissingFact { info, .. } => info,
            other => panic!("expected MissingFact, got: {other:?}"),
        };

        Err(StateError {
            state_id: None,
            info,
        })
    }
}

#[tokio::test]
async fn replay_mode_missing_fact_fails_without_live_io() {
    let events = Arc::new(MemEventStore::default());
    let artifacts = Arc::new(MemArtifactStore::default());
    let stores = || Stores {
        events: events.clone(),
        artifacts: artifacts.clone(),
    };

    let run_config = replay_run_config();
    let manifest = RunManifest {
        op_id: OpId::must_new("op".to_string()),
        op_version: "0".to_string(),
        input_params: serde_json::json!({}),
        run_config: run_config.clone(),
        build: crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        },
    };
    let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

    let initial_snapshot_id = write_full_snapshot_value(artifacts.as_ref(), serde_json::json!({}))
        .await
        .expect("initial snapshot");

    let run_id = RunId(uuid::Uuid::new_v4());
    events
        .append(
            run_id,
            0,
            vec![EventEnvelope {
                run_id,
                seq: 1,
                ts_millis: None,
                event: Event::Kernel(KernelEvent::RunStarted {
                    op_id: manifest.op_id.clone(),
                    manifest_id: manifest_id.clone(),
                    initial_snapshot_id,
                }),
            }],
        )
        .await
        .expect("seed run");

    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let makes = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(TrackingTransportFactory {
        makes: Arc::clone(&makes),
        calls: Arc::clone(&calls),
    });

    let state_id = StateId::must_new("machine.main.s1".to_string());
    let plan = ExecutionPlan {
        op_id: manifest.op_id.clone(),
        graph: StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(ReplayMissingFactState),
            }],
            edges: Vec::new(),
        },
    };

    let resolver = Arc::new(FixedResolver { plan: plan.clone() });
    let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);

    let r = engine.resume(stores(), run_id).await.expect("resume");
    assert_eq!(r.phase, RunPhase::Failed);
    assert_eq!(makes.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}
