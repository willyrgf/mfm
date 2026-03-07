//! Engine-managed child runs.
//!
//! This module wraps a live IO transport factory with two extra namespaces that let
//! a parent run spawn and await child runs through the same runtime invariants.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::config::{RunConfig, RunManifest};
use crate::context_runtime::write_full_snapshot_value;
use crate::engine::{RunPhase, RunResult, StartRun, Stores};
use crate::errors::{ErrorCategory, ErrorInfo, IoError, RunError, StorageError};
use crate::events::RunStatus;
use crate::ids::{ArtifactId, ErrorCode, OpId, RunId};
use crate::io::IoCall;
use crate::live_io::{FactIndex, LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use crate::stores::ArtifactKind;

use super::{
    invalid_plan, next_attempt, read_manifest, read_run_history, run_states, storage_not_found,
    topological_order, validate_execution_mode, validate_start_run_contract, EngineFailpoints,
    EventWriter, PlanResolver, SharedEventWriter,
};

const NAMESPACE_CHILD_RUN_SPAWN: &str = "machine.child_run.spawn";
const NAMESPACE_CHILD_RUN_AWAIT: &str = "machine.child_run.await";

const CODE_CHILD_RUN_REQUEST_INVALID: &str = "child_run_request_invalid";
const CODE_CHILD_RUN_ENGINE_FAILED: &str = "child_run_engine_failed";
const CODE_CHILD_RUN_TASK_FAILED: &str = "child_run_task_failed";

type ChildRunTask = JoinHandle<Result<RunResult, RunError>>;
type ChildRunTaskMap = HashMap<RunId, ChildRunTask>;

#[derive(Clone, Default)]
struct ChildRunSupervisor {
    inner: Arc<Mutex<ChildRunTaskMap>>,
}

impl ChildRunSupervisor {
    async fn spawn_if_absent<F>(&self, run_id: RunId, f: F) -> bool
    where
        F: FnOnce() -> ChildRunTask,
    {
        let mut inner = self.inner.lock().await;
        if inner.contains_key(&run_id) {
            return false;
        }
        inner.insert(run_id, f());
        true
    }

    async fn take(&self, run_id: RunId) -> Option<ChildRunTask> {
        self.inner.lock().await.remove(&run_id)
    }
}

#[derive(Clone)]
struct ChildRunEngine {
    resolver: Arc<dyn PlanResolver>,
    live_transport_factory: Arc<dyn LiveIoTransportFactory>,
    failpoints: Option<EngineFailpoints>,
}

impl ChildRunEngine {
    async fn start_with_id(
        &self,
        stores: Stores,
        run: StartRun,
        run_id: RunId,
    ) -> Result<RunResult, RunError> {
        validate_execution_mode(&run.run_config)?;
        validate_start_run_contract(&run)?;

        let exists = stores
            .artifacts
            .exists(&run.manifest_id)
            .await
            .map_err(RunError::Storage)?;
        if !exists {
            return Err(RunError::Storage(storage_not_found(
                "manifest_not_found",
                "manifest artifact was not found",
            )));
        }

        let head = stores
            .events
            .head_seq(run_id)
            .await
            .map_err(RunError::Storage)?;
        if head != 0 {
            return Err(RunError::Storage(StorageError::Concurrency(super::info(
                "run_already_exists",
                ErrorCategory::Storage,
                "run already exists",
            ))));
        }

        let initial_snapshot = run.initial_context.dump().map_err(RunError::Context)?;
        let initial_snapshot_id =
            write_full_snapshot_value(stores.artifacts.as_ref(), initial_snapshot).await?;

        let writer: SharedEventWriter = Arc::new(Mutex::new(
            EventWriter::new(Arc::clone(&stores.events), run_id)
                .await
                .map_err(RunError::Storage)?,
        ));

        writer
            .lock()
            .await
            .append_kernel(crate::events::KernelEvent::RunStarted {
                op_id: run.plan.op_id.clone(),
                manifest_id: run.manifest_id.clone(),
                initial_snapshot_id: initial_snapshot_id.clone(),
            })
            .await
            .map_err(RunError::Storage)?;

        run_states(
            &stores,
            &run.plan,
            &run.run_config,
            run_id,
            writer,
            initial_snapshot_id,
            &HashSet::new(),
            None,
            FactIndex::default(),
            Arc::clone(&self.live_transport_factory),
            self.failpoints.clone(),
        )
        .await
    }

    async fn resume(&self, stores: Stores, run_id: RunId) -> Result<RunResult, RunError> {
        let head = stores
            .events
            .head_seq(run_id)
            .await
            .map_err(RunError::Storage)?;
        if head == 0 {
            return Err(RunError::Storage(storage_not_found(
                "run_not_found",
                "run event stream was not found",
            )));
        }

        let stream = stores
            .events
            .read_range(run_id, 1, None)
            .await
            .map_err(RunError::Storage)?;

        let facts = FactIndex::from_event_stream(&stream);
        let history = read_run_history(run_id, &stream)?;

        if let Some((status, final_snapshot_id)) = &history.run_completed {
            return Ok(RunResult {
                run_id,
                phase: match status {
                    RunStatus::Completed => RunPhase::Completed,
                    RunStatus::Failed => RunPhase::Failed,
                    RunStatus::Cancelled => RunPhase::Cancelled,
                },
                final_snapshot_id: final_snapshot_id.clone(),
            });
        }

        let manifest =
            read_manifest(stores.artifacts.as_ref(), &history.started.manifest_id).await?;
        validate_execution_mode(&manifest.run_config)?;

        if history.started.op_id != manifest.op_id {
            return Err(invalid_plan(
                "run_started_op_id_mismatch",
                "RunStarted.op_id did not match manifest.op_id",
            ));
        }

        let plan = self.resolver.resolve(&manifest)?;
        if plan.op_id != manifest.op_id {
            return Err(invalid_plan(
                "plan_op_id_mismatch",
                "resolved plan.op_id did not match manifest.op_id",
            ));
        }

        let writer: SharedEventWriter = Arc::new(Mutex::new(
            EventWriter::new(Arc::clone(&stores.events), run_id)
                .await
                .map_err(RunError::Storage)?,
        ));

        if let Some(orphan) = &history.orphan_attempt {
            let start = (
                orphan.state_id.clone(),
                orphan.attempt + 1,
                orphan.base_snapshot_id.clone(),
            );
            return run_states(
                &stores,
                &plan,
                &manifest.run_config,
                run_id,
                writer,
                history.last_checkpoint.clone(),
                &history.completed_states,
                Some(start),
                facts.clone(),
                Arc::clone(&self.live_transport_factory),
                self.failpoints.clone(),
            )
            .await;
        }

        let ordered = topological_order(&plan)
            .map_err(|_| invalid_plan("invalid_plan", "execution plan failed validation"))?;
        let next_state = ordered
            .iter()
            .find(|n| !history.completed_states.contains(&n.id))
            .map(|n| n.id.clone());

        let Some(next_state_id) = next_state else {
            writer
                .lock()
                .await
                .append_kernel(crate::events::KernelEvent::RunCompleted {
                    status: RunStatus::Completed,
                    final_snapshot_id: Some(history.last_checkpoint.clone()),
                })
                .await
                .map_err(RunError::Storage)?;
            return Ok(RunResult {
                run_id,
                phase: RunPhase::Completed,
                final_snapshot_id: Some(history.last_checkpoint.clone()),
            });
        };

        if let Some((attempt, base_snapshot, retryable)) =
            history.last_failure_by_state.get(&next_state_id)
        {
            let next = attempt + 1;
            if !*retryable || next >= manifest.run_config.retry_policy.max_attempts {
                writer
                    .lock()
                    .await
                    .append_kernel(crate::events::KernelEvent::RunCompleted {
                        status: RunStatus::Failed,
                        final_snapshot_id: Some(history.last_checkpoint.clone()),
                    })
                    .await
                    .map_err(RunError::Storage)?;
                return Ok(RunResult {
                    run_id,
                    phase: RunPhase::Failed,
                    final_snapshot_id: Some(history.last_checkpoint.clone()),
                });
            }

            let start = (next_state_id.clone(), next, base_snapshot.clone());
            return run_states(
                &stores,
                &plan,
                &manifest.run_config,
                run_id,
                writer,
                history.last_checkpoint.clone(),
                &history.completed_states,
                Some(start),
                facts.clone(),
                Arc::clone(&self.live_transport_factory),
                self.failpoints.clone(),
            )
            .await;
        }

        let start = (
            next_state_id.clone(),
            next_attempt(&history.last_attempt_by_state, &next_state_id),
            history.last_checkpoint.clone(),
        );
        run_states(
            &stores,
            &plan,
            &manifest.run_config,
            run_id,
            writer,
            history.last_checkpoint.clone(),
            &history.completed_states,
            Some(start),
            facts,
            Arc::clone(&self.live_transport_factory),
            self.failpoints.clone(),
        )
        .await
    }
}

/// Live IO transport factory that adds child-run spawn and await namespaces.
///
/// It forwards all unrelated namespaces to the wrapped factory.
#[derive(Clone)]
pub struct ChildRunLiveIoTransportFactory {
    inner: Arc<dyn LiveIoTransportFactory>,
    child_engine: ChildRunEngine,
    supervisor: ChildRunSupervisor,
}

impl ChildRunLiveIoTransportFactory {
    /// Wraps `inner` with child-run spawn/await handling backed by `resolver`.
    ///
    /// Calls outside the reserved child-run namespaces are forwarded to `inner`.
    pub fn new(resolver: Arc<dyn PlanResolver>, inner: Arc<dyn LiveIoTransportFactory>) -> Self {
        Self {
            inner: Arc::clone(&inner),
            child_engine: ChildRunEngine {
                resolver,
                live_transport_factory: inner,
                failpoints: None,
            },
            supervisor: ChildRunSupervisor::default(),
        }
    }
}

impl LiveIoTransportFactory for ChildRunLiveIoTransportFactory {
    fn namespace_group(&self) -> &str {
        "child_run_wrapper"
    }

    fn make(&self, env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(ChildRunLiveIoTransport {
            env: env.clone(),
            inner: self.inner.make(env),
            child_engine: self.child_engine.clone(),
            supervisor: self.supervisor.clone(),
        })
    }
}

struct ChildRunLiveIoTransport {
    env: LiveIoEnv,
    inner: Box<dyn LiveIoTransport>,
    child_engine: ChildRunEngine,
    supervisor: ChildRunSupervisor,
}

#[derive(Clone, Debug, Deserialize)]
struct ChildRunSpawnRequestV1 {
    kind: String,
    op_id: String,
    op_version: String,
    #[serde(default)]
    op_config: serde_json::Value,
    #[serde(default)]
    input: serde_json::Value,
    run_config: RunConfig,
    #[serde(default)]
    initial_context: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChildRunSpawnResponseV1 {
    parent_run_id: RunId,
    child_run_id: RunId,
    child_manifest_id: ArtifactId,
}

#[derive(Clone, Debug, Deserialize)]
struct ChildRunAwaitRequestV1 {
    kind: String,
    child_run_id: RunId,
    child_manifest_id: ArtifactId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChildRunAwaitResponseV1 {
    child_run_id: RunId,
    status: RunStatus,
    final_snapshot_id: Option<ArtifactId>,
    #[serde(default)]
    final_snapshot: serde_json::Value,
}

fn child_io_error(code: &'static str, category: ErrorCategory, message: &'static str) -> IoError {
    IoError::Other(ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    })
}

impl ChildRunLiveIoTransport {
    fn missing_fact_key() -> IoError {
        IoError::MissingFactKey(ErrorInfo {
            code: ErrorCode(crate::errors::CODE_MISSING_FACT_KEY.to_string()),
            category: ErrorCategory::ParsingInput,
            retryable: false,
            message: "missing fact key for child-run call".to_string(),
            details: None,
        })
    }

    fn child_run_id(&self, fact_key: &crate::ids::FactKey) -> RunId {
        // Deterministic, per-parent-run ids avoid cross-run collisions while remaining stable for
        // resume/replay within the same parent run.
        RunId(uuid::Uuid::new_v5(
            &self.env.run_id.0,
            fact_key.0.as_bytes(),
        ))
    }

    fn default_build() -> crate::config::BuildProvenance {
        crate::config::BuildProvenance {
            git_commit: None,
            cargo_lock_hash: None,
            flake_lock_hash: None,
            rustc_version: None,
            target_triple: None,
            env_allowlist: Vec::new(),
        }
    }

    fn single_op_manifest_input(
        op_id: &str,
        op_version: &str,
        op_config: serde_json::Value,
        input: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "pipeline": {
                "machine_id": op_id,
                "pipeline_version": op_version,
                "steps": [{
                    "step_id": "main",
                    "op_id": op_id,
                    "op_version": op_version,
                    "op_config": op_config,
                }]
            },
            "input": input,
        })
    }

    fn parse_spawn_request(call: &IoCall) -> Result<ChildRunSpawnRequestV1, IoError> {
        let req = serde_json::from_value::<ChildRunSpawnRequestV1>(call.request.clone()).map_err(
            |_| {
                child_io_error(
                    CODE_CHILD_RUN_REQUEST_INVALID,
                    ErrorCategory::ParsingInput,
                    "invalid child run spawn request",
                )
            },
        )?;
        if req.kind != "child_run_spawn_v1" {
            return Err(child_io_error(
                CODE_CHILD_RUN_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "unsupported child run spawn request kind",
            ));
        }
        Ok(req)
    }

    fn parse_await_request(call: &IoCall) -> Result<ChildRunAwaitRequestV1, IoError> {
        let req = serde_json::from_value::<ChildRunAwaitRequestV1>(call.request.clone()).map_err(
            |_| {
                child_io_error(
                    CODE_CHILD_RUN_REQUEST_INVALID,
                    ErrorCategory::ParsingInput,
                    "invalid child run await request",
                )
            },
        )?;
        if req.kind != "child_run_await_v1" {
            return Err(child_io_error(
                CODE_CHILD_RUN_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "unsupported child run await request kind",
            ));
        }
        Ok(req)
    }

    async fn spawn_child(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let Some(fact_key) = &call.fact_key else {
            return Err(Self::missing_fact_key());
        };
        if crate::secrets::string_contains_secrets(&fact_key.0) {
            return Err(child_io_error(
                "secrets_detected",
                ErrorCategory::Unknown,
                "fact key contained secrets (policy forbids persisting secrets)",
            ));
        }

        let req = Self::parse_spawn_request(&call)?;
        let child_run_id = self.child_run_id(fact_key);

        let build = Self::default_build();
        let input_params = Self::single_op_manifest_input(
            &req.op_id,
            &req.op_version,
            req.op_config.clone(),
            req.input.clone(),
        );
        let op_id = OpId::new(req.op_id.clone()).map_err(|_| {
            child_io_error(
                CODE_CHILD_RUN_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "invalid child run op_id",
            )
        })?;

        let manifest = RunManifest {
            op_id,
            op_version: req.op_version.clone(),
            input_params,
            run_config: req.run_config.clone(),
            build,
        };

        let value = serde_json::to_value(&manifest).map_err(|_| {
            child_io_error(
                CODE_CHILD_RUN_ENGINE_FAILED,
                ErrorCategory::ParsingInput,
                "failed to serialize child run manifest",
            )
        })?;
        let bytes = crate::hashing::canonical_json_bytes(&value).map_err(|_| {
            child_io_error(
                CODE_CHILD_RUN_ENGINE_FAILED,
                ErrorCategory::ParsingInput,
                "child run manifest is not canonical-json-hashable",
            )
        })?;
        let computed_id = crate::hashing::artifact_id_for_bytes(&bytes);

        let stored_id = self
            .env
            .stores
            .artifacts
            .put(ArtifactKind::Manifest, bytes)
            .await
            .map_err(|_| {
                child_io_error(
                    CODE_CHILD_RUN_ENGINE_FAILED,
                    ErrorCategory::Storage,
                    "failed to store child run manifest",
                )
            })?;
        if stored_id != computed_id {
            return Err(child_io_error(
                "child_run_manifest_id_mismatch",
                ErrorCategory::Storage,
                "artifact store returned unexpected manifest id",
            ));
        }

        let head = self
            .env
            .stores
            .events
            .head_seq(child_run_id)
            .await
            .map_err(|_| {
                child_io_error(
                    CODE_CHILD_RUN_ENGINE_FAILED,
                    ErrorCategory::Storage,
                    "failed to query child run status",
                )
            })?;

        if head == 0 {
            let plan = self.child_engine.resolver.resolve(&manifest).map_err(|_| {
                child_io_error(
                    CODE_CHILD_RUN_ENGINE_FAILED,
                    ErrorCategory::Unknown,
                    "failed to resolve child run plan",
                )
            })?;

            let initial = if req.initial_context.is_null() {
                serde_json::json!({})
            } else {
                req.initial_context.clone()
            };
            let ctx =
                crate::context_runtime::JsonContext::from_snapshot(initial).map_err(|_| {
                    child_io_error(
                        CODE_CHILD_RUN_ENGINE_FAILED,
                        ErrorCategory::Context,
                        "invalid child initial_context snapshot",
                    )
                })?;

            let start = StartRun {
                manifest,
                manifest_id: stored_id.clone(),
                plan,
                run_config: req.run_config,
                initial_context: Box::new(ctx),
            };

            let engine = self.child_engine.clone();
            let stores = self.env.stores.clone();
            let _ = self
                .supervisor
                .spawn_if_absent(child_run_id, move || {
                    tokio::spawn(
                        async move { engine.start_with_id(stores, start, child_run_id).await },
                    )
                })
                .await;
        } else {
            let stream = self
                .env
                .stores
                .events
                .read_range(child_run_id, 1, None)
                .await
                .map_err(|_| {
                    child_io_error(
                        CODE_CHILD_RUN_ENGINE_FAILED,
                        ErrorCategory::Storage,
                        "failed to read child run event stream",
                    )
                })?;
            let history = read_run_history(child_run_id, &stream).map_err(|_| {
                child_io_error(
                    CODE_CHILD_RUN_ENGINE_FAILED,
                    ErrorCategory::Storage,
                    "invalid child run event stream",
                )
            })?;
            if history.started.manifest_id != stored_id {
                return Err(child_io_error(
                    "child_run_conflict",
                    ErrorCategory::Unknown,
                    "child run id already exists with a different manifest id",
                ));
            }

            if history.run_completed.is_none() {
                let engine = self.child_engine.clone();
                let stores = self.env.stores.clone();
                let _ = self
                    .supervisor
                    .spawn_if_absent(child_run_id, move || {
                        tokio::spawn(async move { engine.resume(stores, child_run_id).await })
                    })
                    .await;
            }
        }

        let resp = ChildRunSpawnResponseV1 {
            parent_run_id: self.env.run_id,
            child_run_id,
            child_manifest_id: stored_id,
        };
        serde_json::to_value(resp).map_err(|_| {
            child_io_error(
                CODE_CHILD_RUN_TASK_FAILED,
                ErrorCategory::Unknown,
                "failed to serialize child run spawn response",
            )
        })
    }

    async fn await_child(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        if call.fact_key.is_none() {
            return Err(Self::missing_fact_key());
        }
        let req = Self::parse_await_request(&call)?;

        let stores = self.env.stores.clone();

        let rr = if let Some(handle) = self.supervisor.take(req.child_run_id).await {
            match handle.await {
                Ok(res) => res.map_err(|_| {
                    child_io_error(
                        CODE_CHILD_RUN_TASK_FAILED,
                        ErrorCategory::Unknown,
                        "child run task failed",
                    )
                })?,
                Err(_) => {
                    return Err(child_io_error(
                        CODE_CHILD_RUN_TASK_FAILED,
                        ErrorCategory::Unknown,
                        "child run task panicked or was cancelled",
                    ));
                }
            }
        } else {
            match self
                .child_engine
                .resume(stores.clone(), req.child_run_id)
                .await
            {
                Ok(rr) => rr,
                Err(RunError::Storage(StorageError::NotFound(info)))
                    if info.code.0 == "run_not_found" =>
                {
                    let manifest = read_manifest(stores.artifacts.as_ref(), &req.child_manifest_id)
                        .await
                        .map_err(|_| {
                            child_io_error(
                                CODE_CHILD_RUN_ENGINE_FAILED,
                                ErrorCategory::Storage,
                                "failed to read child run manifest",
                            )
                        })?;
                    let plan = self.child_engine.resolver.resolve(&manifest).map_err(|_| {
                        child_io_error(
                            CODE_CHILD_RUN_ENGINE_FAILED,
                            ErrorCategory::Unknown,
                            "failed to resolve child run plan",
                        )
                    })?;

                    let run_config = manifest.run_config.clone();
                    let start = StartRun {
                        manifest,
                        manifest_id: req.child_manifest_id.clone(),
                        plan,
                        run_config,
                        initial_context: Box::new(crate::context_runtime::JsonContext::new()),
                    };
                    self.child_engine
                        .start_with_id(stores.clone(), start, req.child_run_id)
                        .await
                        .map_err(|_| {
                            child_io_error(
                                CODE_CHILD_RUN_ENGINE_FAILED,
                                ErrorCategory::Unknown,
                                "failed to start missing child run",
                            )
                        })?
                }
                Err(_) => {
                    return Err(child_io_error(
                        CODE_CHILD_RUN_ENGINE_FAILED,
                        ErrorCategory::Unknown,
                        "failed to resume child run",
                    ));
                }
            }
        };

        let status = match rr.phase {
            RunPhase::Completed => RunStatus::Completed,
            RunPhase::Failed => RunStatus::Failed,
            RunPhase::Cancelled => RunStatus::Cancelled,
            RunPhase::Running => RunStatus::Failed,
        };

        let final_snapshot = if let Some(id) = &rr.final_snapshot_id {
            let bytes = stores.artifacts.get(id).await.map_err(|_| {
                child_io_error(
                    CODE_CHILD_RUN_ENGINE_FAILED,
                    ErrorCategory::Storage,
                    "failed to read child final snapshot",
                )
            })?;
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                child_io_error(
                    CODE_CHILD_RUN_ENGINE_FAILED,
                    ErrorCategory::ParsingInput,
                    "child final snapshot was invalid JSON",
                )
            })?
        } else {
            serde_json::Value::Null
        };

        let resp = ChildRunAwaitResponseV1 {
            child_run_id: rr.run_id,
            status,
            final_snapshot_id: rr.final_snapshot_id,
            final_snapshot,
        };
        serde_json::to_value(resp).map_err(|_| {
            child_io_error(
                CODE_CHILD_RUN_TASK_FAILED,
                ErrorCategory::Unknown,
                "failed to serialize child run await response",
            )
        })
    }
}

#[async_trait]
impl LiveIoTransport for ChildRunLiveIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            NAMESPACE_CHILD_RUN_SPAWN => self.spawn_child(call).await,
            NAMESPACE_CHILD_RUN_AWAIT => self.await_child(call).await,
            _ => self.inner.call(call).await,
        }
    }
}
