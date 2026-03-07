//! Shared helpers for ops crate tests.
//!
//! Keep this module limited to non-domain, reusable test wiring helpers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use mfm_machine::config::BuildProvenance;
use mfm_machine::config::{
    BackoffPolicy, ContextCheckpointing, EventProfile, ExecutionMode, IoMode, RetryPolicy,
    RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunResult, Stores};
use mfm_machine::errors::{ContextError, ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::events::{Event, EventEnvelope, KernelEvent};
use mfm_machine::hashing::artifact_id_for_bytes;
use mfm_machine::ids::{ArtifactId, ContextKey, ErrorCode, RunId};
use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};
use mfm_sdk::errors::SdkError;
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::op::{DynOperation, OperationRegistry};
use mfm_sdk::pipeline::{Pipeline, PipelinePlanner};
use mfm_sdk::unstable::{
    single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
};

/// Returns a standard live [`RunConfig`] with a single attempt.
pub fn run_config_live() -> RunConfig {
    run_config_live_with_retry_attempts(1)
}

/// Returns a standard live [`RunConfig`] with the supplied retry-attempt budget.
pub fn run_config_live_with_retry_attempts(max_attempts: u32) -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts,
            backoff: BackoffPolicy::Fixed {
                delay: Duration::from_millis(0),
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

/// Returns the default live run config with a custom nix flake allowlist.
pub fn run_config_live_with_allowlist(prefixes: Vec<String>) -> RunConfig {
    let mut cfg = run_config_live();
    cfg.nix_flake_allowlist = prefixes;
    cfg
}

/// Returns an empty [`BuildProvenance`] value suitable for most tests.
pub fn build_provenance_default() -> BuildProvenance {
    BuildProvenance {
        git_commit: None,
        cargo_lock_hash: None,
        flake_lock_hash: None,
        rustc_version: None,
        target_triple: None,
        env_allowlist: Vec::new(),
    }
}

/// Convenience bundle used by tests that plan a single operation.
pub type SingleOpPlan = (
    Arc<dyn OperationRegistry>,
    Arc<dyn PipelinePlanner>,
    Pipeline,
);

/// Builds an in-memory operation registry preloaded with the supplied operations.
pub fn registry_with_ops(
    ops: impl IntoIterator<Item = DynOperation>,
) -> Arc<dyn OperationRegistry> {
    let mut reg = HashMapOperationRegistry::default();
    for op in ops {
        reg.register(op);
    }
    Arc::new(reg)
}

/// Returns the default pipeline planner used by op/state tests.
pub fn default_pipeline_planner() -> Arc<dyn PipelinePlanner> {
    Arc::new(DefaultPipelinePlanner)
}

/// Builds a single-op pipeline manifest from an operation and its JSON config.
pub fn single_op_pipeline_for(
    op: &DynOperation,
    op_config: serde_json::Value,
) -> Result<Pipeline, SdkError> {
    single_op_pipeline(op.op_id(), op.op_version(), op_config)
}

/// Builds the standard registry/planner/pipeline tuple for a single operation.
pub fn single_op_plan(
    op: DynOperation,
    op_config: serde_json::Value,
) -> Result<SingleOpPlan, SdkError> {
    let registry = registry_with_ops([Arc::clone(&op)]);
    let planner = default_pipeline_planner();
    let pipeline = single_op_pipeline_for(&op, op_config)?;
    Ok((registry, planner, pipeline))
}

/// Starts a pipeline with the default launcher, provenance, and empty in-memory context.
pub async fn start_pipeline_with_defaults(
    engine: Arc<dyn ExecutionEngine>,
    stores: &Stores,
    registry: Arc<dyn OperationRegistry>,
    planner: Arc<dyn PipelinePlanner>,
    pipeline: Pipeline,
    run_config: RunConfig,
) -> Result<RunResult, mfm_machine::errors::RunError> {
    let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
    launcher
        .start_pipeline(
            engine,
            Stores {
                events: Arc::clone(&stores.events),
                artifacts: Arc::clone(&stores.artifacts),
            },
            registry,
            planner,
            LaunchPipeline {
                pipeline,
                input: serde_json::json!({}),
                run_config,
                build: build_provenance_default(),
                initial_context: Box::new(MapContext::default()),
            },
        )
        .await
}

/// Resumes a pipeline with the default launcher and the supplied run id.
pub async fn resume_pipeline_with_defaults(
    engine: Arc<dyn ExecutionEngine>,
    stores: &Stores,
    registry: Arc<dyn OperationRegistry>,
    planner: Arc<dyn PipelinePlanner>,
    run_id: RunId,
) -> Result<RunResult, mfm_machine::errors::RunError> {
    let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
    launcher
        .resume(
            engine,
            Stores {
                events: Arc::clone(&stores.events),
                artifacts: Arc::clone(&stores.artifacts),
            },
            registry,
            planner,
            run_id,
        )
        .await
}

/// Minimal in-memory context implementation used by tests.
#[derive(Default)]
pub struct MapContext {
    inner: HashMap<String, serde_json::Value>,
}

impl MapContext {
    /// Rehydrates the context from a previously dumped JSON snapshot.
    pub fn from_snapshot(v: serde_json::Value) -> Self {
        let obj = v.as_object().cloned().unwrap_or_default();
        let mut inner = HashMap::new();
        for (k, v) in obj {
            inner.insert(k, v);
        }
        Self { inner }
    }
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

fn storage_info(code: &'static str, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category: ErrorCategory::Storage,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn lock_map<'a, T>(mutex: &'a Mutex<T>) -> Result<MutexGuard<'a, T>, StorageError> {
    mutex.lock().map_err(|_| {
        StorageError::Other(storage_info("storage_lock_failed", "storage lock failed"))
    })
}

/// In-memory [`EventStore`] used by tests.
#[derive(Clone, Default)]
pub struct MemEventStore {
    inner: Arc<Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
}

#[async_trait]
impl EventStore for MemEventStore {
    async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError> {
        let inner = lock_map(&self.inner)?;
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
        let mut inner = lock_map(&self.inner)?;
        let stream = inner.entry(run_id).or_default();
        let head = stream.last().map(|e| e.seq).unwrap_or(0);
        if head != expected_seq {
            return Err(StorageError::Concurrency(storage_info(
                "event_store_concurrency",
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
        let inner = lock_map(&self.inner)?;
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

/// In-memory [`ArtifactStore`] used by tests.
#[derive(Clone, Default)]
pub struct MemArtifactStore {
    inner: Arc<Mutex<HashMap<ArtifactId, Vec<u8>>>>,
}

#[async_trait]
impl ArtifactStore for MemArtifactStore {
    async fn put(&self, _kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        let id = artifact_id_for_bytes(&bytes);
        lock_map(&self.inner)?.insert(id.clone(), bytes);
        Ok(id)
    }

    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
        lock_map(&self.inner)?
            .get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(storage_info("not_found", "artifact not found")))
    }

    async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
        Ok(lock_map(&self.inner)?.contains_key(id))
    }
}

/// Returns a [`Stores`] bundle backed by in-memory event and artifact stores.
pub fn in_memory_stores() -> Stores {
    Stores {
        events: Arc::new(MemEventStore::default()),
        artifacts: Arc::new(MemArtifactStore::default()),
    }
}

/// Extracts the manifest id and initial snapshot id from a run's event stream.
///
/// Panics if the stream does not contain a `RunStarted` event.
pub fn run_started(stream: &[EventEnvelope]) -> (ArtifactId, ArtifactId) {
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

/// Returns the final snapshot id recorded by `RunCompleted`, if one exists.
pub fn run_completed_snapshot_id(stream: &[EventEnvelope]) -> Option<ArtifactId> {
    for e in stream {
        if let Event::Kernel(KernelEvent::RunCompleted {
            final_snapshot_id, ..
        }) = &e.event
        {
            return final_snapshot_id.clone();
        }
    }
    None
}
