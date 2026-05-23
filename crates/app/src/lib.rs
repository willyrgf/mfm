#![warn(missing_docs)]
//! Typed application assembly for certified MFM runs.
//!
//! `mfm-app` is the typed boundary used by binaries and process adapters. It does not plan old
//! dynamic DAGs, own workflow semantics, or expose `mfm-machine`/`mfm-sdk` execution authority.
//! Callers supply a certified typed spec, a runner registry, typed artifact evidence, and a typed
//! run store; this crate wires those parts into start, resume, replay, and public-output rendering
//! helpers.
//!
//! # Examples
//!
//! ```rust
//! use mfm_app::{make_in_memory_typed_services, ErasedRunnerRegistry};
//!
//! let runners = ErasedRunnerRegistry::new();
//! let _services = make_in_memory_typed_services(runners, "/tmp/mfm-typed-artifacts");
//! ```

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use mfm_artifact_store_fs::{FsTypedArtifactError, FsTypedArtifactStore, TypedArtifactDescriptor};
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, DigestAlgorithm, EventId, RunId, SchemaId, SeedId, SpecHash};
use mfm_replay::v1::{ReplayAuthority, ReplayBroker, ReplayError};
use mfm_runtime::{
    build_public_output_receipt_artifact, build_retention_manifest_artifact, validate_run_stream,
    CertifiedRuntimeSpec, RunStartEvidence, SchedulerStatus, SerialTypedScheduler,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use serde::Serialize;
use serde_json::map::Entry;
use serde_json::{Map, Value};
use tokio::sync::Mutex;

pub use mfm_runtime::ErasedRunnerRegistry;

/// Shared observability configuration used by typed binaries.
pub mod observability;

const ENV_TYPED_ARTIFACT_ROOT: &str = "MFM_TYPED_ARTIFACT_ROOT";
const DEFAULT_TYPED_ARTIFACT_SUBDIR: &str = "typed_run_artifacts";

/// High-level error classes used by typed application-facing APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// The caller provided invalid input.
    BadRequest,
    /// The requested run, artifact, or output was not found.
    NotFound,
    /// The request conflicted with current persisted state.
    Conflict,
    /// Runtime, storage, replay, or artifact evidence was internally invalid.
    Internal,
}

/// Stable error payload returned by typed application helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    /// High-level error classification for HTTP/CLI mapping.
    pub class: ErrorClass,
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable message safe to display to callers.
    pub message: String,
}

impl AppError {
    /// Creates an application error from the supplied classification, code, and message.
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into(),
        }
    }

    /// Returns a generic invalid request error.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::BadRequest, "InvalidRequest", message)
    }

    /// Returns the standard invalid UUID error payload.
    pub fn invalid_uuid() -> Self {
        Self::new(
            ErrorClass::BadRequest,
            "InvalidRunId",
            "Invalid UUID format",
        )
    }

    /// Returns a not-found error with an explicit code and message.
    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::NotFound, code, message)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<mfm_runtime::RuntimeError> for AppError {
    fn from(error: mfm_runtime::RuntimeError) -> Self {
        match error {
            mfm_runtime::RuntimeError::Store(message) => {
                Self::new(ErrorClass::Conflict, "TypedStoreRejected", message)
            }
            mfm_runtime::RuntimeError::RunnerBinding(message) => {
                Self::new(ErrorClass::BadRequest, "TypedRunnerUnavailable", message)
            }
            mfm_runtime::RuntimeError::SpecHash(message)
            | mfm_runtime::RuntimeError::InvalidSpec(message)
            | mfm_runtime::RuntimeError::InvalidRunStream(message)
            | mfm_runtime::RuntimeError::Blocked(message)
            | mfm_runtime::RuntimeError::InputMaterialization(message)
            | mfm_runtime::RuntimeError::InvalidRunnerOutput(message)
            | mfm_runtime::RuntimeError::Identity(message)
            | mfm_runtime::RuntimeError::Canonical(message) => {
                Self::new(ErrorClass::Internal, "TypedRuntimeError", message)
            }
        }
    }
}

impl From<store::StoreError> for AppError {
    fn from(error: store::StoreError) -> Self {
        Self::new(
            ErrorClass::Conflict,
            "TypedStoreRejected",
            error.to_string(),
        )
    }
}

impl From<FsTypedArtifactError> for AppError {
    fn from(error: FsTypedArtifactError) -> Self {
        match error {
            FsTypedArtifactError::NotFound { artifact_id } => Self::not_found(
                "TypedArtifactNotFound",
                format!("typed artifact {artifact_id} was not found"),
            ),
            FsTypedArtifactError::RetainedArtifactRefused { artifact_id } => Self::new(
                ErrorClass::Conflict,
                "TypedArtifactRetained",
                format!("typed artifact {artifact_id} is retained"),
            ),
            FsTypedArtifactError::Corruption { .. }
            | FsTypedArtifactError::EvidenceMismatch { .. }
            | FsTypedArtifactError::InvalidEvidence { .. }
            | FsTypedArtifactError::InvalidIdentity { .. }
            | FsTypedArtifactError::Io { .. } => Self::new(
                ErrorClass::Internal,
                "TypedArtifactError",
                error.to_string(),
            ),
        }
    }
}

impl From<ReplayError> for AppError {
    fn from(error: ReplayError) -> Self {
        Self::new(ErrorClass::Internal, error.code(), error.message)
    }
}

impl From<mfm_spec::SpecError> for AppError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::new(ErrorClass::Internal, "TypedSpecInvalid", error.to_string())
    }
}

/// Returns the default typed artifact root from `MFM_TYPED_ARTIFACT_ROOT` or
/// `$HOME/.mfm/typed_run_artifacts`.
#[allow(clippy::disallowed_methods)]
pub fn default_typed_artifact_root() -> PathBuf {
    std::env::var(ENV_TYPED_ARTIFACT_ROOT)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".mfm")
                .join(DEFAULT_TYPED_ARTIFACT_SUBDIR)
        })
}

/// Builds the default certified typed filesystem artifact store.
pub fn make_default_typed_artifact_store() -> FsTypedArtifactStore {
    FsTypedArtifactStore::new(default_typed_artifact_root())
}

/// Builds an in-memory typed run store for tests and local single-process tools.
pub fn make_in_memory_typed_run_store() -> store::InMemoryTypedRunStore {
    store::InMemoryTypedRunStore::default()
}

/// Builds typed app services backed by an in-memory typed run event store.
pub fn make_in_memory_typed_services(
    runners: ErasedRunnerRegistry,
    artifact_root: impl Into<PathBuf>,
) -> TypedAppServices<store::InMemoryTypedRunStore> {
    TypedAppServices::new(
        SerialTypedScheduler::new(runners),
        store::InMemoryTypedRunStore::default(),
        FsTypedArtifactStore::new(artifact_root),
    )
}

/// Builds typed app services backed by a durable async typed run event store.
pub fn make_async_typed_services<S>(
    runners: ErasedRunnerRegistry,
    store: S,
    artifacts: FsTypedArtifactStore,
) -> TypedAsyncAppServices<S>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
    TypedAsyncAppServices::new(SerialTypedScheduler::new(runners), store, artifacts)
}

/// Builds the production typed runner registry for this process.
///
/// Framework public-output render nodes are resolved by `mfm-runtime` as built-ins. Domain
/// state runners are added by their typed porting commits; until then specs that reference those
/// descriptors fail before `RunStarted` with `TypedRunnerUnavailable`.
pub fn production_typed_runner_registry() -> ErasedRunnerRegistry {
    ErasedRunnerRegistry::new()
}

/// Generates a digest-only typed run id from a random UUID.
pub fn new_run_id() -> RunId {
    let uuid = uuid::Uuid::new_v4();
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(uuid.as_bytes()),
    )
}

/// Whether typed start/resume should run scheduler steps after appending `RunStarted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveMode {
    /// Append only the requested lifecycle event.
    AppendOnly,
    /// Drive deterministic runnable nodes until the scheduler blocks or the run completes.
    UntilBlocked,
}

/// Request to start a certified typed run.
#[derive(Debug, Clone)]
pub struct TypedRunStartRequest {
    /// Certified typed spec envelope.
    pub envelope: spec::CertifiedSpecEnvelope,
    /// Store-owned run id to bind.
    pub run_id: RunId,
    /// Run-start evidence whose artifacts must already be persisted in the typed artifact store.
    pub evidence: RunStartEvidence,
    /// Scheduler drive policy after start.
    pub drive: DriveMode,
}

/// Seed bytes supplied to a typed run start request.
#[derive(Debug, Clone)]
pub struct TypedSeedInput {
    /// Seed id from the certified spec.
    pub seed_id: SeedId,
    /// Canonical seed value bytes.
    pub bytes: Vec<u8>,
    /// Persisted seed artifact media type.
    pub media_type: spec::MediaType,
}

/// Request to resume a certified typed run.
#[derive(Debug, Clone)]
pub struct TypedRunResumeRequest {
    /// Certified typed spec envelope bound to the run.
    pub envelope: spec::CertifiedSpecEnvelope,
    /// Run id to resume.
    pub run_id: RunId,
    /// Scheduler drive policy.
    pub drive: DriveMode,
}

/// Stable phase for typed run responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypedRunPhase {
    /// No typed run-start event exists for the requested run id.
    Absent,
    /// The run has started and may have more runnable nodes.
    Started,
    /// The run completed with public-output evidence.
    Completed,
}

impl fmt::Display for TypedRunPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Absent => "absent",
            Self::Started => "started",
            Self::Completed => "completed",
        };
        f.write_str(value)
    }
}

/// Response returned after typed start or resume dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedRunResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current run phase.
    pub phase: TypedRunPhase,
    /// Last scheduler status observed by the app dispatch loop.
    pub scheduler_status: String,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
}

impl fmt::Display for TypedRunResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} phase={} spec_hash={} head_seq={} scheduler_status={}",
            self.run_id, self.phase, self.spec_hash, self.head_seq, self.scheduler_status
        )
    }
}

/// Typed public-output rendering response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedPublicOutputResponse {
    /// Run id.
    pub run_id: String,
    /// Public output schema id.
    pub public_schema_id: String,
    /// Producing public-output event id.
    pub event_id: String,
    /// Canonical rendered output digest.
    pub rendered_digest: String,
    /// Rendered artifact id when the renderer persisted output bytes.
    pub rendered_artifact_id: Option<String>,
    /// Rendered JSON body loaded from the typed artifact store, when available and JSON encoded.
    pub json: Option<serde_json::Value>,
}

impl fmt::Display for TypedPublicOutputResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.json {
            Some(json) => write!(
                f,
                "{}",
                serde_json::to_string(json).map_err(|_| fmt::Error)?
            ),
            None => write!(
                f,
                "run {} public_schema_id={} event_id={} rendered_digest={}",
                self.run_id, self.public_schema_id, self.event_id, self.rendered_digest
            ),
        }
    }
}

/// Typed run event stream response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedRunStreamResponse {
    /// Run id.
    pub run_id: String,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
    /// Store-owned typed event references.
    pub events: Vec<TypedRunEventRef>,
}

impl fmt::Display for TypedRunStreamResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} head_seq={} events={}",
            self.run_id,
            self.head_seq,
            self.events.len()
        )
    }
}

/// Transport-safe reference to one store-owned typed event envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedRunEventRef {
    /// Store-derived event id.
    pub event_id: String,
    /// Event schema id.
    pub event_schema_id: String,
    /// Store-owned stream sequence.
    pub seq: u64,
    /// Store-owned ordinal within the atomic commit.
    pub ordinal: u32,
    /// Commit key that appended this event.
    pub commit_key: String,
    /// Store-derived logical key.
    pub logical_key: String,
    /// Canonical payload hash.
    pub payload_hash: String,
}

/// Response returned after verifying replay authority for a typed run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedReplayResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current run phase.
    pub phase: TypedRunPhase,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
    /// Retained artifact evidence entries supplied to the replay broker.
    pub retained_artifacts: usize,
}

impl fmt::Display for TypedReplayResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} replay verified spec_hash={} phase={} head_seq={} retained_artifacts={}",
            self.run_id, self.spec_hash, self.phase, self.head_seq, self.retained_artifacts
        )
    }
}

/// Application service facade for certified typed runtime dispatch.
#[derive(Clone)]
pub struct TypedAppServices<S> {
    scheduler: SerialTypedScheduler,
    store: Arc<Mutex<S>>,
    artifacts: FsTypedArtifactStore,
}

impl<S> TypedAppServices<S>
where
    S: store::TypedRunEventStore + Send,
{
    /// Creates typed app services from explicit scheduler, store, and artifact store choices.
    pub fn new(scheduler: SerialTypedScheduler, store: S, artifacts: FsTypedArtifactStore) -> Self {
        Self {
            scheduler,
            store: Arc::new(Mutex::new(store)),
            artifacts,
        }
    }

    /// Returns the shared typed run store handle.
    pub fn store(&self) -> Arc<Mutex<S>> {
        Arc::clone(&self.store)
    }

    /// Returns the typed artifact store.
    pub fn artifacts(&self) -> &FsTypedArtifactStore {
        &self.artifacts
    }

    /// Persists the certified spec artifact required before `RunStarted`.
    pub async fn persist_certified_spec(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
    ) -> Result<store::ArtifactEvidenceRef, AppError> {
        let canonical = runtime_spec.spec().canonical_json().map_err(|error| {
            AppError::new(
                ErrorClass::Internal,
                "TypedSpecCanonicalError",
                error.to_string(),
            )
        })?;
        self.artifacts
            .put_artifact(
                canonical.to_vec(),
                TypedArtifactDescriptor {
                    media_type: runtime_spec.spec().media_type.clone(),
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::TypedExecutionSpec,
                },
            )
            .await
            .map_err(Into::into)
    }

    /// Persists a certified config artifact and verifies it matches the spec config reference.
    pub async fn persist_config_artifact(
        &self,
        config_ref: &spec::ConfigRef,
        bytes: Vec<u8>,
    ) -> Result<store::ArtifactEvidenceRef, AppError> {
        let evidence = self
            .artifacts
            .put_artifact(
                bytes,
                TypedArtifactDescriptor {
                    media_type: config_ref.media_type.clone(),
                    schema_id: Some(config_ref.schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::TypedConfig,
                },
            )
            .await?;
        if evidence.artifact_id != config_ref.artifact_id
            || evidence.digest != config_ref.digest
            || evidence.byte_len != config_ref.byte_len
        {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "TypedConfigArtifactMismatch",
                "persisted config artifact does not match certified config ref",
            ));
        }
        Ok(evidence)
    }

    /// Starts a certified typed run, optionally driving runnable nodes.
    pub async fn start_certified_run(
        &self,
        req: TypedRunStartRequest,
    ) -> Result<TypedRunResponse, AppError> {
        let runtime_spec = CertifiedRuntimeSpec::new(req.envelope)?;
        self.validate_launch_artifacts(&runtime_spec, &req.evidence)
            .await?;
        let mut store = self.store.lock().await;
        self.scheduler
            .start_run(&mut *store, &runtime_spec, req.run_id.clone(), req.evidence)?;
        let status = self
            .drive_with_mode(&mut *store, &runtime_spec, &req.run_id, req.drive)
            .await?;
        typed_run_response(&*store, &runtime_spec, &req.run_id, status)
    }

    /// Resumes a certified typed run, optionally driving runnable nodes.
    pub async fn resume_certified_run(
        &self,
        req: TypedRunResumeRequest,
    ) -> Result<TypedRunResponse, AppError> {
        let runtime_spec = CertifiedRuntimeSpec::new(req.envelope)?;
        let mut store = self.store.lock().await;
        let stream = store.load_run_stream(&req.run_id);
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        validate_run_stream(&runtime_spec, &req.run_id, &stream)?;
        let status = self
            .drive_with_mode(&mut *store, &runtime_spec, &req.run_id, req.drive)
            .await?;
        typed_run_response(&*store, &runtime_spec, &req.run_id, status)
    }

    /// Returns typed run status by rebuilding projection from the authoritative run stream.
    pub async fn run_status(&self, run_id: &RunId) -> Result<TypedRunResponse, AppError> {
        let store = self.store.lock().await;
        let stream = store.load_run_stream(run_id);
        typed_run_status_from_stream(run_id, &stream)
    }

    /// Returns the authoritative typed run stream.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<TypedRunStreamResponse, AppError> {
        let store = self.store.lock().await;
        let events = store.load_run_stream(run_id);
        Ok(typed_run_stream_response_from_events(
            run_id,
            stream_head(&events),
            &events,
        ))
    }

    /// Builds an evidence-only replay broker from a certified spec and typed run stream.
    pub async fn replay_broker(
        &self,
        envelope: spec::CertifiedSpecEnvelope,
        run_id: &RunId,
        authority: ReplayAuthority,
    ) -> Result<ReplayBroker, AppError> {
        let store = self.store.lock().await;
        let stream = store.load_run_stream(run_id);
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        ReplayBroker::from_run_stream(envelope, &stream, authority).map_err(Into::into)
    }

    /// Renders typed public output from store-owned projection and typed artifact bytes.
    pub async fn typed_public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<TypedPublicOutputResponse, AppError> {
        let stream = {
            let store = self.store.lock().await;
            store.load_run_stream(run_id)
        };
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        let envelope = load_certified_spec_for_run(&self.artifacts, run_id, &stream).await?;
        let runtime_spec = CertifiedRuntimeSpec::new(envelope)?;
        validate_run_stream(&runtime_spec, run_id, &stream)?;
        typed_public_output_from_stream(&self.artifacts, run_id, public_schema_id, &stream).await
    }

    async fn drive_with_mode(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<SchedulerStatus, AppError> {
        match drive {
            DriveMode::AppendOnly => Ok(SchedulerStatus::Blocked),
            DriveMode::UntilBlocked => {
                self.drive_until_blocked_with_retention(store, runtime_spec, run_id)
                    .await
            }
        }
    }

    async fn validate_launch_artifacts(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        evidence: &RunStartEvidence,
    ) -> Result<(), AppError> {
        validate_launch_artifacts(&self.artifacts, runtime_spec, evidence).await
    }

    async fn drive_until_blocked_with_retention(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus, AppError> {
        let mut advanced = false;
        loop {
            match self
                .scheduler
                .drive_once(store, runtime_spec, run_id)
                .await?
            {
                SchedulerStatus::Advanced => {
                    advanced = true;
                    if self
                        .project_retention_manifest_if_ready(store, runtime_spec, run_id)
                        .await?
                    {
                        continue;
                    }
                }
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    async fn project_retention_manifest_if_ready(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<bool, AppError> {
        let stream = store.load_run_stream(run_id);
        if !retention_manifest_should_project(runtime_spec, run_id, &stream)? {
            return Ok(false);
        }
        persist_framework_public_output_receipts(&self.artifacts, runtime_spec, &stream).await?;
        let manifest = build_retention_manifest_artifact(runtime_spec, run_id, &stream)?;
        self.artifacts
            .put_verified_artifact(manifest.bytes.to_vec(), manifest.evidence.clone())
            .await?;
        self.scheduler.append_retention_manifest_projection(
            store,
            runtime_spec,
            run_id,
            manifest,
        )?;
        Ok(true)
    }
}

/// Application facade for durable async certified typed runtime dispatch.
#[derive(Clone)]
pub struct TypedAsyncAppServices<S> {
    scheduler: SerialTypedScheduler,
    store: S,
    artifacts: FsTypedArtifactStore,
}

impl<S> TypedAsyncAppServices<S>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
    /// Creates typed async app services from explicit scheduler, store, and artifact store choices.
    pub fn new(scheduler: SerialTypedScheduler, store: S, artifacts: FsTypedArtifactStore) -> Self {
        Self {
            scheduler,
            store,
            artifacts,
        }
    }

    /// Returns the typed artifact store.
    pub fn artifacts(&self) -> &FsTypedArtifactStore {
        &self.artifacts
    }

    /// Returns the async typed run store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Starts a certified typed run against a durable async typed store.
    pub async fn start_certified_run(
        &self,
        req: TypedRunStartRequest,
    ) -> Result<TypedRunResponse, AppError> {
        let runtime_spec = CertifiedRuntimeSpec::new(req.envelope)?;
        validate_launch_artifacts(&self.artifacts, &runtime_spec, &req.evidence).await?;
        self.scheduler
            .start_run_async(&self.store, &runtime_spec, req.run_id.clone(), req.evidence)
            .await?;
        let status = self
            .drive_with_mode(&runtime_spec, &req.run_id, req.drive)
            .await?;
        let stream = self
            .store
            .load_run_stream(&req.run_id)
            .await
            .map_err(async_app_store_error)?;
        typed_run_response_from_stream(&req.run_id, runtime_spec.spec_hash(), &stream, status)
    }

    /// Resumes a certified typed run from its stored spec artifact.
    pub async fn resume_stored_run(
        &self,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<TypedRunResponse, AppError> {
        let stream = self
            .store
            .load_run_stream(run_id)
            .await
            .map_err(async_app_store_error)?;
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        let envelope = load_certified_spec_for_run(&self.artifacts, run_id, &stream).await?;
        let runtime_spec = CertifiedRuntimeSpec::new(envelope)?;
        validate_run_stream(&runtime_spec, run_id, &stream)?;
        let status = self.drive_with_mode(&runtime_spec, run_id, drive).await?;
        let stream = self
            .store
            .load_run_stream(run_id)
            .await
            .map_err(async_app_store_error)?;
        typed_run_response_from_stream(run_id, runtime_spec.spec_hash(), &stream, status)
    }

    /// Returns typed run status by rebuilding projection from the authoritative run stream.
    pub async fn run_status(&self, run_id: &RunId) -> Result<TypedRunResponse, AppError> {
        let stream = self
            .store
            .load_run_stream(run_id)
            .await
            .map_err(async_app_store_error)?;
        typed_run_status_from_stream(run_id, &stream)
    }

    /// Returns the authoritative typed run stream.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<TypedRunStreamResponse, AppError> {
        let events = self
            .store
            .load_run_stream(run_id)
            .await
            .map_err(async_app_store_error)?;
        Ok(typed_run_stream_response_from_events(
            run_id,
            stream_head(&events),
            &events,
        ))
    }

    /// Verifies replay authority for a run using retained typed artifact evidence only.
    pub async fn verify_replay_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<TypedReplayResponse, AppError> {
        let stream = self
            .store
            .load_run_stream(run_id)
            .await
            .map_err(async_app_store_error)?;
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        let envelope = load_certified_spec_for_run(&self.artifacts, run_id, &stream).await?;
        let authority =
            replay_authority_for_run(&self.artifacts, &envelope, run_id, &stream).await?;
        let broker = ReplayBroker::from_run_stream(envelope, &stream, authority)?;
        let projection = broker.projection_snapshot();
        let retained_artifacts = projection
            .retention(run_id)
            .map(|retention| retention.refs.len())
            .unwrap_or_default();
        Ok(TypedReplayResponse {
            run_id: run_id.as_str().to_owned(),
            spec_hash: broker.certified_spec().spec_hash.as_str().to_owned(),
            phase: typed_phase(projection.run_state(run_id)),
            head_seq: stream_head(&stream),
            retained_artifacts,
        })
    }

    /// Renders typed public output from store-owned projection and typed artifact bytes.
    pub async fn typed_public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<TypedPublicOutputResponse, AppError> {
        let stream = self
            .store
            .load_run_stream(run_id)
            .await
            .map_err(async_app_store_error)?;
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        let envelope = load_certified_spec_for_run(&self.artifacts, run_id, &stream).await?;
        let runtime_spec = CertifiedRuntimeSpec::new(envelope)?;
        validate_run_stream(&runtime_spec, run_id, &stream)?;
        typed_public_output_from_stream(&self.artifacts, run_id, public_schema_id, &stream).await
    }

    async fn drive_with_mode(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<SchedulerStatus, AppError> {
        match drive {
            DriveMode::AppendOnly => Ok(SchedulerStatus::Blocked),
            DriveMode::UntilBlocked => {
                self.drive_until_blocked_with_retention(runtime_spec, run_id)
                    .await
            }
        }
    }

    async fn drive_until_blocked_with_retention(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus, AppError> {
        let mut advanced = false;
        loop {
            match self
                .scheduler
                .drive_once_async(&self.store, runtime_spec, run_id)
                .await?
            {
                SchedulerStatus::Advanced => {
                    advanced = true;
                    if self
                        .project_retention_manifest_if_ready(runtime_spec, run_id)
                        .await?
                    {
                        continue;
                    }
                }
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    async fn project_retention_manifest_if_ready(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<bool, AppError> {
        let stream = self
            .store
            .load_run_stream(run_id)
            .await
            .map_err(async_app_store_error)?;
        if !retention_manifest_should_project(runtime_spec, run_id, &stream)? {
            return Ok(false);
        }
        persist_framework_public_output_receipts(&self.artifacts, runtime_spec, &stream).await?;
        let manifest = build_retention_manifest_artifact(runtime_spec, run_id, &stream)?;
        self.artifacts
            .put_verified_artifact(manifest.bytes.to_vec(), manifest.evidence.clone())
            .await?;
        self.scheduler
            .append_retention_manifest_projection_async(&self.store, runtime_spec, run_id, manifest)
            .await?;
        Ok(true)
    }
}

/// Loads and verifies the certified spec artifact bound by a typed run stream.
pub async fn load_certified_spec_for_run(
    artifacts: &FsTypedArtifactStore,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<spec::CertifiedSpecEnvelope, AppError> {
    let run_started = run_started_payload(run_id, stream)?;
    let (bytes, evidence) = artifacts
        .get_artifact_by_id(&run_started.spec_artifact_id)
        .await?;
    validate_spec_artifact_evidence(run_started, &evidence)?;
    let spec = spec::TypedExecutionSpec::from_json_slice(&bytes)?;
    let envelope =
        spec::CertifiedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())?;
    envelope.verify_hash()?;
    validate_run_started_matches_spec(run_started, &envelope)?;
    Ok(envelope)
}

/// Builds replay authority from retained artifact evidence in the run stream.
pub async fn replay_authority_for_run(
    artifacts: &FsTypedArtifactStore,
    envelope: &spec::CertifiedSpecEnvelope,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<ReplayAuthority, AppError> {
    let run_started = run_started_payload(run_id, stream)?;
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
    let Some(retention) = projection.retention(run_id) else {
        return Err(AppError::new(
            ErrorClass::Internal,
            "TypedReplayRetentionMissing",
            "typed replay requires retained artifact evidence",
        ));
    };
    let mut artifact_evidence = Vec::with_capacity(retention.refs.len());
    for retained in retention.refs.values() {
        let (_, evidence) = artifacts.get_artifact_by_id(&retained.artifact_id).await?;
        if evidence.digest != retained.content_digest || evidence.artifact_role != retained.role {
            return Err(AppError::new(
                ErrorClass::Internal,
                "TypedReplayArtifactMismatch",
                "retained artifact metadata does not match retention evidence",
            ));
        }
        artifact_evidence.push(evidence);
    }
    Ok(ReplayAuthority::from_certified_spec(
        envelope,
        run_started.runner_executables.clone(),
        run_started.adapter_executables.clone(),
        artifact_evidence,
    ))
}

/// Persists the certified typed execution spec artifact for a runtime spec.
pub async fn persist_certified_spec_artifact(
    artifacts: &FsTypedArtifactStore,
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<store::ArtifactEvidenceRef, AppError> {
    let canonical = runtime_spec.spec().canonical_json().map_err(|error| {
        AppError::new(
            ErrorClass::Internal,
            "TypedSpecCanonicalError",
            error.to_string(),
        )
    })?;
    artifacts
        .put_artifact(
            canonical.to_vec(),
            TypedArtifactDescriptor {
                media_type: runtime_spec.spec().media_type.clone(),
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::TypedExecutionSpec,
            },
        )
        .await
        .map_err(Into::into)
}

/// Loads and verifies config artifact evidence required by a certified spec.
pub async fn load_config_artifacts_for_spec(
    artifacts: &FsTypedArtifactStore,
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<Vec<store::ArtifactEvidenceRef>, AppError> {
    let mut evidence = Vec::with_capacity(runtime_spec.spec().config_refs.len());
    for config_ref in &runtime_spec.spec().config_refs {
        let (_, artifact) = artifacts
            .get_artifact_by_id(&config_ref.artifact_id)
            .await?;
        if artifact.artifact_id != config_ref.artifact_id
            || artifact.digest != config_ref.digest
            || artifact.byte_len != config_ref.byte_len
            || artifact.media_type != config_ref.media_type
            || artifact.schema_id.as_ref() != Some(&config_ref.schema_id)
            || artifact.semantic_type_id.is_some()
            || artifact.producer_node_id.is_some()
            || artifact.producer_seed_id.is_some()
            || artifact.artifact_role != events::ArtifactRole::TypedConfig
        {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "TypedConfigArtifactMismatch",
                "typed config artifact metadata does not match the certified spec",
            ));
        }
        evidence.push(artifact);
    }
    Ok(evidence)
}

/// Persists and verifies launch seed artifacts for a certified spec.
pub async fn persist_seed_inputs_for_spec(
    artifacts: &FsTypedArtifactStore,
    runtime_spec: &CertifiedRuntimeSpec,
    seeds: Vec<TypedSeedInput>,
) -> Result<Vec<events::SeedCellRef>, AppError> {
    let mut supplied = std::collections::BTreeMap::new();
    for seed in seeds {
        if supplied.insert(seed.seed_id.clone(), seed).is_some() {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "DuplicateTypedSeedInput",
                "seed input was supplied more than once",
            ));
        }
    }

    let mut seed_refs = Vec::with_capacity(runtime_spec.spec().seeds.len());
    for seed_spec in &runtime_spec.spec().seeds {
        let input = supplied.remove(&seed_spec.seed_id).ok_or_else(|| {
            AppError::new(
                ErrorClass::BadRequest,
                "MissingTypedSeedInput",
                format!("missing seed input for {}", seed_spec.seed_id),
            )
        })?;
        let evidence = artifacts
            .put_artifact(
                input.bytes,
                TypedArtifactDescriptor {
                    media_type: input.media_type,
                    schema_id: Some(seed_spec.schema_id.clone()),
                    semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
                    producer_node_id: None,
                    producer_seed_id: Some(seed_spec.seed_id.clone()),
                    artifact_role: events::ArtifactRole::SeedInput,
                },
            )
            .await?;
        if let Some(required_digest) = &seed_spec.required_digest {
            if &evidence.digest != required_digest {
                return Err(AppError::new(
                    ErrorClass::BadRequest,
                    "TypedSeedDigestMismatch",
                    "seed input digest does not match the certified spec",
                ));
            }
        }
        seed_refs.push(events::SeedCellRef {
            seed_id: seed_spec.seed_id.clone(),
            cell_id: seed_spec.cell_id.clone(),
            scope_id: seed_spec.scope_id.clone(),
            semantic_type_id: seed_spec.semantic_type_id.clone(),
            schema_id: seed_spec.schema_id.clone(),
            digest: evidence.digest.clone(),
            seed_artifact: events::ArtifactEvidenceRef {
                artifact_id: evidence.artifact_id.clone(),
                role: evidence.artifact_role,
                schema_id: seed_spec.schema_id.clone(),
                semantic_type_id: evidence.semantic_type_id.clone(),
                content_digest: evidence.digest,
                byte_len: evidence.byte_len,
                media_type: evidence.media_type,
            },
        });
    }
    if !supplied.is_empty() {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "UnknownTypedSeedInput",
            "seed input was supplied for a seed not present in the certified spec",
        ));
    }
    Ok(seed_refs)
}

/// Returns the default JSON media type used by typed CLI seed inputs.
pub fn json_media_type() -> Result<spec::MediaType, AppError> {
    spec::MediaType::new("application/json").map_err(|error| {
        AppError::new(
            ErrorClass::Internal,
            "TypedJsonMediaTypeInvalid",
            error.to_string(),
        )
    })
}

/// Builds a typed run-start request from persisted spec JSON bytes and launch inputs.
pub async fn build_typed_run_start_request(
    artifacts: &FsTypedArtifactStore,
    spec_bytes: &[u8],
    run_id: RunId,
    framework_version: &str,
    source_revision: &str,
    seed_inputs: Vec<TypedSeedInput>,
    drive: DriveMode,
) -> Result<TypedRunStartRequest, AppError> {
    let spec = spec::TypedExecutionSpec::from_json_slice(spec_bytes).map_err(|error| {
        AppError::new(
            ErrorClass::BadRequest,
            "TypedSpecInvalid",
            error.to_string(),
        )
    })?;
    let envelope =
        spec::CertifiedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())?;
    let runtime_spec = CertifiedRuntimeSpec::new(envelope.clone())?;
    let spec_artifact = persist_certified_spec_artifact(artifacts, &runtime_spec).await?;
    let config_artifacts = load_config_artifacts_for_spec(artifacts, &runtime_spec).await?;
    let seed_cells = persist_seed_inputs_for_spec(artifacts, &runtime_spec, seed_inputs).await?;
    Ok(TypedRunStartRequest {
        envelope,
        run_id,
        evidence: RunStartEvidence {
            spec_artifact,
            config_artifacts,
            framework_version: events::FrameworkVersion::new(framework_version).map_err(
                |error| {
                    AppError::new(
                        ErrorClass::BadRequest,
                        "TypedFrameworkVersionInvalid",
                        error.to_string(),
                    )
                },
            )?,
            source_revision: events::SourceRevision::new(source_revision).map_err(|error| {
                AppError::new(
                    ErrorClass::BadRequest,
                    "TypedSourceRevisionInvalid",
                    error.to_string(),
                )
            })?,
            adapter_executables: Vec::new(),
            seed_cells,
        },
        drive,
    })
}

fn async_app_store_error(error: impl fmt::Display) -> AppError {
    AppError::new(
        ErrorClass::Conflict,
        "TypedStoreRejected",
        error.to_string(),
    )
}

async fn validate_launch_artifacts(
    artifacts: &FsTypedArtifactStore,
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: &RunStartEvidence,
) -> Result<(), AppError> {
    let spec_bytes = artifacts.get_artifact(&evidence.spec_artifact).await?;
    let canonical = runtime_spec.spec().canonical_json().map_err(|error| {
        AppError::new(
            ErrorClass::Internal,
            "TypedSpecCanonicalError",
            error.to_string(),
        )
    })?;
    if spec_bytes != canonical.as_bytes() {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "TypedSpecArtifactMismatch",
            "persisted typed execution spec artifact bytes do not match the certified spec",
        ));
    }

    for config_artifact in &evidence.config_artifacts {
        artifacts.get_artifact(config_artifact).await?;
    }
    for seed in &evidence.seed_cells {
        artifacts
            .get_artifact(&seed_artifact_evidence(seed))
            .await?;
    }
    Ok(())
}

async fn persist_framework_public_output_receipts(
    artifacts: &FsTypedArtifactStore,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<(), AppError> {
    for event in stream {
        if let events::KernelEventPayload::PublicOutputProduced(payload) = event.payload() {
            let (bytes, evidence) = build_public_output_receipt_artifact(runtime_spec, payload)?;
            artifacts
                .put_verified_artifact(bytes.to_vec(), evidence)
                .await?;
        }
    }
    Ok(())
}

fn retention_manifest_should_project(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<bool, AppError> {
    if stream.is_empty() {
        return Ok(false);
    }
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
    if projection.run_state(run_id) != store::RunState::Started
        || stream.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        })
    {
        return Ok(false);
    }
    Ok(matches!(
        projection.public_output(&runtime_spec.spec().public_outputs.public_schema_id),
        Some(store::PublicOutputProjection::Produced { .. })
    ))
}

/// Derives typed run status from an authoritative store-owned run stream.
pub fn typed_run_status_from_stream(
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<TypedRunResponse, AppError> {
    if stream.is_empty() {
        return Err(AppError::not_found(
            "TypedRunNotFound",
            "typed run stream was not found",
        ));
    }
    let spec_hash = run_started_spec_hash(stream)?;
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
    Ok(TypedRunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: spec_hash.as_str().to_owned(),
        phase: typed_phase(projection.run_state(run_id)),
        scheduler_status: "observed".to_owned(),
        head_seq: stream_head(stream),
    })
}

/// Renders store-owned typed event references for a run stream response.
pub fn typed_run_stream_response_from_events(
    run_id: &RunId,
    head_seq: u64,
    events: &[store::KernelEventEnvelope],
) -> TypedRunStreamResponse {
    TypedRunStreamResponse {
        run_id: run_id.as_str().to_owned(),
        head_seq,
        events: events.iter().map(typed_event_ref).collect(),
    }
}

/// Renders typed public output from a store-owned run stream and typed artifact store.
pub async fn typed_public_output_from_stream(
    artifacts: &FsTypedArtifactStore,
    run_id: &RunId,
    public_schema_id: &SchemaId,
    stream: &[store::KernelEventEnvelope],
) -> Result<TypedPublicOutputResponse, AppError> {
    if stream.is_empty() {
        return Err(AppError::not_found(
            "TypedRunNotFound",
            "typed run stream was not found",
        ));
    }
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
    let public_output = projection.public_output(public_schema_id).ok_or_else(|| {
        AppError::not_found(
            "TypedPublicOutputNotFound",
            "typed public output was not found for the requested schema",
        )
    })?;
    let store::PublicOutputProjection::Produced {
        event_id,
        rendered_digest,
        rendered_artifact_id,
    } = public_output
    else {
        return Err(AppError::new(
            ErrorClass::Conflict,
            "TypedPublicOutputRenderFailed",
            "typed public output render failed",
        ));
    };

    let payload = public_output_payload_from_stream(stream, event_id, public_schema_id)?;
    let json = match rendered_artifact_id {
        Some(artifact_id) => {
            Some(load_public_output_json(artifacts, artifact_id, rendered_digest, payload).await?)
        }
        None => Some(render_public_output_json_from_cells(artifacts, &payload.cells).await?),
    };
    Ok(TypedPublicOutputResponse {
        run_id: run_id.as_str().to_owned(),
        public_schema_id: public_schema_id.as_str().to_owned(),
        event_id: event_id.as_str().to_owned(),
        rendered_digest: rendered_digest.as_str().to_owned(),
        rendered_artifact_id: rendered_artifact_id
            .as_ref()
            .map(|artifact_id| artifact_id.as_str().to_owned()),
        json,
    })
}

fn public_output_payload_from_stream<'a>(
    stream: &'a [store::KernelEventEnvelope],
    event_id: &EventId,
    public_schema_id: &SchemaId,
) -> Result<&'a events::PublicOutputProduced, AppError> {
    stream
        .iter()
        .find_map(|event| {
            if event.event_id() != event_id {
                return None;
            }
            match event.payload() {
                events::KernelEventPayload::PublicOutputProduced(payload)
                    if &payload.public_schema_id == public_schema_id =>
                {
                    Some(payload)
                }
                _ => None,
            }
        })
        .ok_or_else(|| {
            AppError::new(
                ErrorClass::Internal,
                "TypedPublicOutputProjectionMismatch",
                "typed public-output projection does not match the authoritative run stream",
            )
        })
}

async fn render_public_output_json_from_cells(
    artifacts: &FsTypedArtifactStore,
    cells: &[events::NamedTypedCellRef],
) -> Result<Value, AppError> {
    let mut root = Map::new();
    for cell in cells {
        let (bytes, evidence) = artifacts.get_artifact_by_id(&cell.artifact_id).await?;
        verify_public_output_cell_evidence(cell, &evidence)?;
        let value = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::new(
                ErrorClass::Internal,
                "TypedPublicOutputDecodeFailed",
                format!("typed public-output cell artifact was not JSON: {error}"),
            )
        })?;
        insert_public_output_value(&mut root, cell.public_field_path.as_str(), value)?;
    }
    Ok(Value::Object(root))
}

fn verify_public_output_cell_evidence(
    cell: &events::NamedTypedCellRef,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), AppError> {
    if evidence.artifact_id != cell.artifact_id
        || evidence.digest != cell.content_digest
        || evidence.schema_id.as_ref() != Some(&cell.schema_id)
        || evidence.semantic_type_id.as_ref() != Some(&cell.semantic_type_id)
    {
        return Err(public_output_artifact_mismatch(
            "typed public-output cell artifact evidence does not match the event cell reference",
        ));
    }

    match &cell.producer {
        spec::CellProducer::Node(node_id) => {
            if evidence.artifact_role != events::ArtifactRole::StateOutput
                || evidence.producer_node_id.as_ref() != Some(node_id)
                || evidence.producer_seed_id.is_some()
            {
                return Err(public_output_artifact_mismatch(
                    "typed public-output node cell artifact evidence has the wrong producer or role",
                ));
            }
        }
        spec::CellProducer::Seed(seed_id) => {
            if evidence.artifact_role != events::ArtifactRole::SeedInput
                || evidence.producer_seed_id.as_ref() != Some(seed_id)
                || evidence.producer_node_id.is_some()
            {
                return Err(public_output_artifact_mismatch(
                    "typed public-output seed cell artifact evidence has the wrong producer or role",
                ));
            }
        }
    }

    Ok(())
}

fn insert_public_output_value(
    root: &mut Map<String, Value>,
    path: &str,
    value: Value,
) -> Result<(), AppError> {
    let mut parts = path.split('.').peekable();
    let mut current = root;
    let mut value = Some(value);

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            match current.entry(part.to_owned()) {
                Entry::Vacant(entry) => {
                    entry.insert(value.take().expect("public output value inserted once"));
                    return Ok(());
                }
                Entry::Occupied(_) => {
                    return Err(public_output_artifact_mismatch(
                        "typed public-output field paths collide",
                    ));
                }
            }
        }

        let entry = current
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(next) = entry else {
            return Err(public_output_artifact_mismatch(
                "typed public-output field path collides with a scalar value",
            ));
        };
        current = next;
    }

    Err(public_output_artifact_mismatch(
        "typed public-output field path was empty",
    ))
}

async fn load_public_output_json(
    artifacts: &FsTypedArtifactStore,
    artifact_id: &ArtifactId,
    rendered_digest: &mfm_ids::ContentDigest,
    payload: &events::PublicOutputProduced,
) -> Result<serde_json::Value, AppError> {
    let (bytes, evidence) = artifacts.get_artifact_by_id(artifact_id).await?;
    let json_media_type = spec::MediaType::new("application/json").map_err(|error| {
        AppError::new(
            ErrorClass::Internal,
            "TypedPublicOutputMediaTypeInvalid",
            error.to_string(),
        )
    })?;
    if evidence.artifact_id != *artifact_id
        || evidence.digest != *rendered_digest
        || evidence.media_type != json_media_type
        || evidence.schema_id.as_ref() != Some(&payload.public_schema_id)
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.as_ref() != Some(&payload.node_id)
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::PublicOutput
    {
        return Err(public_output_artifact_mismatch(
            "typed public-output cache artifact evidence does not match the produced event",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        AppError::new(
            ErrorClass::Internal,
            "TypedPublicOutputDecodeFailed",
            format!("typed public-output artifact was not JSON: {error}"),
        )
    })
}

fn public_output_artifact_mismatch(message: &'static str) -> AppError {
    AppError::new(
        ErrorClass::Internal,
        "TypedPublicOutputArtifactMismatch",
        message,
    )
}

fn run_started_payload<'a>(
    run_id: &RunId,
    stream: &'a [store::KernelEventEnvelope],
) -> Result<&'a events::RunStarted, AppError> {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .ok_or_else(|| {
            AppError::new(
                ErrorClass::Internal,
                "TypedRunStartedMissing",
                "typed run stream is missing RunStarted evidence",
            )
        })
        .and_then(|run_started| {
            if &run_started.run_id == run_id {
                Ok(run_started)
            } else {
                Err(AppError::new(
                    ErrorClass::Internal,
                    "TypedRunStartedMismatch",
                    "typed run stream RunStarted evidence is bound to a different run id",
                ))
            }
        })
}

fn validate_spec_artifact_evidence(
    run_started: &events::RunStarted,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), AppError> {
    let expected_spec_hash =
        SpecHash::from_digest(evidence.digest.algorithm(), *evidence.digest.digest());
    if evidence.artifact_id != run_started.spec_artifact_id
        || expected_spec_hash != run_started.spec_hash
        || evidence.media_type != run_started.spec_media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedExecutionSpec
    {
        return Err(AppError::new(
            ErrorClass::Internal,
            "TypedSpecArtifactMismatch",
            "typed execution spec artifact metadata does not match RunStarted evidence",
        ));
    }
    Ok(())
}

fn validate_run_started_matches_spec(
    run_started: &events::RunStarted,
    envelope: &spec::CertifiedSpecEnvelope,
) -> Result<(), AppError> {
    if envelope.spec_hash != run_started.spec_hash
        || envelope.spec.media_type != run_started.spec_media_type
        || envelope.spec.spec_version != run_started.spec_version
        || envelope.spec.lowering_version != run_started.lowering_version
        || envelope.spec.public_outputs.public_schema_id != run_started.public_output_schema_id
        || envelope.spec.descriptor_identities != run_started.descriptor_identities
        || envelope
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            != run_started.canonicalizer_identity
    {
        return Err(AppError::new(
            ErrorClass::Internal,
            "TypedRunStartedSpecMismatch",
            "RunStarted evidence does not match the stored certified spec artifact",
        ));
    }
    Ok(())
}

fn typed_run_response_from_stream(
    run_id: &RunId,
    spec_hash: &SpecHash,
    stream: &[store::KernelEventEnvelope],
    status: SchedulerStatus,
) -> Result<TypedRunResponse, AppError> {
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
    Ok(TypedRunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: spec_hash.as_str().to_owned(),
        phase: typed_phase(projection.run_state(run_id)),
        scheduler_status: scheduler_status_str(status).to_owned(),
        head_seq: stream_head(stream),
    })
}

fn typed_run_response<S: store::TypedRunEventStore + ?Sized>(
    store: &S,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    status: SchedulerStatus,
) -> Result<TypedRunResponse, AppError> {
    let stream = store.load_run_stream(run_id);
    typed_run_response_from_stream(run_id, runtime_spec.spec_hash(), &stream, status)
}

fn stream_head(stream: &[store::KernelEventEnvelope]) -> u64 {
    stream.last().map_or(0, |event| event.seq().as_u64())
}

fn run_started_spec_hash(stream: &[store::KernelEventEnvelope]) -> Result<SpecHash, AppError> {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload.spec_hash.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            AppError::new(
                ErrorClass::Internal,
                "TypedRunStartedMissing",
                "typed run stream is missing RunStarted evidence",
            )
        })
}

fn seed_artifact_evidence(seed: &events::SeedCellRef) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: seed.seed_artifact.artifact_id.clone(),
        digest: seed.seed_artifact.content_digest.clone(),
        byte_len: seed.seed_artifact.byte_len,
        media_type: seed.seed_artifact.media_type.clone(),
        schema_id: Some(seed.seed_artifact.schema_id.clone()),
        semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: Some(seed.seed_id.clone()),
        artifact_role: seed.seed_artifact.role,
    }
}

fn typed_phase(state: store::RunState) -> TypedRunPhase {
    match state {
        store::RunState::Absent => TypedRunPhase::Absent,
        store::RunState::Started => TypedRunPhase::Started,
        store::RunState::Completed => TypedRunPhase::Completed,
    }
}

fn scheduler_status_str(status: SchedulerStatus) -> &'static str {
    match status {
        SchedulerStatus::Advanced => "advanced",
        SchedulerStatus::Blocked => "blocked",
        SchedulerStatus::PublicOutputProjected => "public_output_projected",
    }
}

fn typed_event_ref(event: &store::KernelEventEnvelope) -> TypedRunEventRef {
    TypedRunEventRef {
        event_id: event.event_id().as_str().to_owned(),
        event_schema_id: event.event_schema_id().as_str().to_owned(),
        seq: event.seq().as_u64(),
        ordinal: event.ordinal().as_u32(),
        commit_key: event.commit_key().as_str().to_owned(),
        logical_key: event.logical_key().as_str().to_owned(),
        payload_hash: event.payload_hash().as_str().to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use mfm_capabilities::{CapabilitySetDescriptor, EffectSpec, ManagedPlatformWrite, Pure};
    use mfm_ids::{AttemptId, DescriptorId, LoweringVersion, SpecVersion, StateKind, StateVersion};
    use mfm_ids::{CellId, ContentDigest, DigestBytes, NodeId, ScopeId, SeedId, SemanticTypeId};
    use mfm_runtime::{
        ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding, ErasedRunnerFuture,
        ErasedRunnerOutput, StagedRetentionRefs,
    };
    use mfm_store::v1::{AsyncTypedRunEventStore, TypedRunEventStore};
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn generated_run_ids_are_typed_digest_ids() {
        let run_id = new_run_id();

        assert!(run_id.as_str().starts_with("run:sha256-jcs-v1:"));
    }

    #[test]
    fn typed_phase_names_are_stable() {
        assert_eq!(TypedRunPhase::Absent.to_string(), "absent");
        assert_eq!(TypedRunPhase::Started.to_string(), "started");
        assert_eq!(TypedRunPhase::Completed.to_string(), "completed");
    }

    #[tokio::test]
    async fn public_output_renders_json_from_cells_without_rendered_artifact_id() {
        let root =
            std::env::temp_dir().join(format!("mfm-app-public-output-{}", uuid::Uuid::new_v4()));
        let artifacts = FsTypedArtifactStore::new(&root);
        let node_id = node_id(0x20);
        let schema_id = schema_id("mfm.test.position", 0x21);
        let semantic_type_id = semantic_id("position", 0x22);
        let bytes = br#"{"total":"12.50"}"#.to_vec();

        let evidence = artifacts
            .put_artifact(
                bytes,
                TypedArtifactDescriptor {
                    media_type: spec::MediaType::new("application/json").expect("media type"),
                    schema_id: Some(schema_id.clone()),
                    semantic_type_id: Some(semantic_type_id.clone()),
                    producer_node_id: Some(node_id.clone()),
                    producer_seed_id: None::<SeedId>,
                    artifact_role: events::ArtifactRole::StateOutput,
                },
            )
            .await
            .expect("persist typed artifact");
        let cell = events::NamedTypedCellRef {
            public_field_path: spec::PublicFieldPath::new("result").expect("field path"),
            cell_id: cell_id(0x23),
            producer: spec::CellProducer::Node(node_id),
            scope_id: scope_id(0x24),
            semantic_type_id,
            schema_id,
            value_lineage: spec::ValueLineageRef {
                lineage_digest: content_digest(0x25),
            },
            content_digest: evidence.digest,
            artifact_id: evidence.artifact_id,
        };

        let rendered = render_public_output_json_from_cells(&artifacts, &[cell])
            .await
            .expect("render public output");

        assert_eq!(
            rendered,
            serde_json::json!({
                "result": {
                    "total": "12.50"
                }
            })
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn public_output_api_renders_from_authoritative_stream_cells() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-public-output-stream-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let mut store = store::InMemoryTypedRunStore::new();
        let run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0x10));
        let spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0x11));
        let public_schema_id = schema_id("mfm.test.public_output", 0x12);
        let source_node = node_id(0x20);
        let source_attempt = attempt_id(0x21);
        let source_cell = cell_id(0x22);
        let source_scope = scope_id(0x23);
        let source_schema = schema_id("mfm.test.position", 0x24);
        let source_semantic = semantic_id("position", 0x25);
        let source_lineage = spec::ValueLineageRef {
            lineage_digest: content_digest(0x26),
        };
        let source_evidence = artifacts
            .put_artifact(
                br#"{"total":"12.50"}"#.to_vec(),
                TypedArtifactDescriptor {
                    media_type: spec::MediaType::new("application/json").expect("media type"),
                    schema_id: Some(source_schema.clone()),
                    semantic_type_id: Some(source_semantic.clone()),
                    producer_node_id: Some(source_node.clone()),
                    producer_seed_id: None::<SeedId>,
                    artifact_role: events::ArtifactRole::StateOutput,
                },
            )
            .await
            .expect("persist source artifact");
        let spec_evidence = store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(spec_hash.algorithm(), *spec_hash.digest()),
            digest: ContentDigest::from_digest(spec_hash.algorithm(), *spec_hash.digest()),
            byte_len: 2,
            media_type: spec::MediaType::new(
                "application/vnd.mfm.typed-execution-spec+json;version=1",
            )
            .expect("media type"),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        };
        store
            .record_artifact_evidence(spec_evidence.clone())
            .expect("record spec evidence");
        store
            .record_artifact_evidence(source_evidence.clone())
            .expect("record source evidence");

        append_commit(
            &mut store,
            &run_id,
            "start",
            vec![events::KernelEventPayload::RunStarted(events::RunStarted {
                run_id: run_id.clone(),
                spec_hash: spec_hash.clone(),
                spec_artifact_id: spec_evidence.artifact_id.clone(),
                spec_media_type: spec_evidence.media_type.clone(),
                spec_version: SpecVersion::new("mfm.typed.execution_spec.v1")
                    .expect("spec version"),
                lowering_version: LoweringVersion::new("mfm.typed.lowering.v1")
                    .expect("lowering version"),
                public_output_schema_id: public_schema_id.clone(),
                descriptor_identities: Vec::new(),
                runner_executables: Vec::new(),
                adapter_executables: Vec::new(),
                canonicalizer_identity: spec::CanonicalizerIdentity::new("mfm.jcs.v1")
                    .expect("canonicalizer"),
                framework_version: events::FrameworkVersion::new("mfm.test.1")
                    .expect("framework version"),
                source_revision: events::SourceRevision::new("test-revision")
                    .expect("source revision"),
                seed_cells: Vec::new(),
            })],
            vec![spec_evidence],
            store::RequiredRunState::Absent,
        );

        append_commit(
            &mut store,
            &run_id,
            "source-terminal",
            vec![
                state_attempt_started(&spec_hash, &source_node, &source_attempt, 1),
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: spec_hash.clone(),
                    node_id: source_node.clone(),
                    cell_id: source_cell.clone(),
                    scope_id: source_scope.clone(),
                    attempt_id: source_attempt.clone(),
                    semantic_type_id: source_semantic.clone(),
                    schema_id: source_schema.clone(),
                    value_lineage: source_lineage.clone(),
                    artifact_id: source_evidence.artifact_id.clone(),
                    content_digest: source_evidence.digest.clone(),
                    producer_state_kind: None,
                    producer_state_version: None,
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: spec_hash.clone(),
                    node_id: source_node.clone(),
                    attempt_id: source_attempt.clone(),
                    output_cell_id: source_cell.clone(),
                }),
            ],
            Vec::new(),
            store::RequiredRunState::Started,
        );

        let render_node = node_id(0x30);
        let render_attempt = attempt_id(0x31);
        let receipt_cell = cell_id(0x32);
        let receipt_evidence = store::ArtifactEvidenceRef {
            artifact_id: artifact_id(0x33),
            digest: content_digest(0x34),
            byte_len: 128,
            media_type: spec::MediaType::new("application/json").expect("media type"),
            schema_id: Some(schema_id("mfm.test.public_output_receipt", 0x35)),
            semantic_type_id: Some(semantic_id("public-output-receipt", 0x36)),
            producer_node_id: Some(render_node.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        };
        store
            .record_artifact_evidence(receipt_evidence.clone())
            .expect("record receipt evidence");

        append_commit(
            &mut store,
            &run_id,
            "public-output",
            vec![
                state_attempt_started(&spec_hash, &render_node, &render_attempt, 2),
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: spec_hash.clone(),
                    node_id: render_node.clone(),
                    cell_id: receipt_cell.clone(),
                    scope_id: scope_id(0x37),
                    attempt_id: render_attempt.clone(),
                    semantic_type_id: receipt_evidence.semantic_type_id.clone().expect("semantic"),
                    schema_id: receipt_evidence.schema_id.clone().expect("schema"),
                    value_lineage: spec::ValueLineageRef {
                        lineage_digest: content_digest(0x38),
                    },
                    artifact_id: receipt_evidence.artifact_id.clone(),
                    content_digest: receipt_evidence.digest.clone(),
                    producer_state_kind: None,
                    producer_state_version: None,
                }),
                events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: spec_hash.clone(),
                    node_id: render_node.clone(),
                    attempt_id: render_attempt.clone(),
                    receipt_cell_id: receipt_cell.clone(),
                    public_schema_id: public_schema_id.clone(),
                    output_spec_digest: content_digest(0x39),
                    cells: vec![events::NamedTypedCellRef {
                        public_field_path: spec::PublicFieldPath::new("result")
                            .expect("field path"),
                        cell_id: source_cell.clone(),
                        producer: spec::CellProducer::Node(source_node.clone()),
                        scope_id: source_scope,
                        semantic_type_id: source_semantic,
                        schema_id: source_schema,
                        value_lineage: source_lineage,
                        content_digest: source_evidence.digest,
                        artifact_id: source_evidence.artifact_id,
                    }],
                    rendered_digest: content_digest(0x40),
                    rendered_artifact_id: None,
                    renderer_descriptor_id: descriptor_id(0x41),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: spec_hash.clone(),
                    node_id: render_node,
                    attempt_id: render_attempt,
                    output_cell_id: receipt_cell,
                }),
            ],
            Vec::new(),
            store::RequiredRunState::Started,
        );

        let stream = store.load_run_stream(&run_id);
        let response =
            typed_public_output_from_stream(&artifacts, &run_id, &public_schema_id, &stream)
                .await
                .expect("render public output from stream");

        assert_eq!(
            response.json,
            Some(serde_json::json!({
                "result": {
                    "total": "12.50"
                }
            }))
        );
        assert!(response.rendered_artifact_id.is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn async_typed_services_start_resume_replay_and_render_framework_public_output() {
        let (root, fixture, services, started) = start_framework_fixture_run().await;
        assert_eq!(started.phase, TypedRunPhase::Completed);
        assert_eq!(started.scheduler_status, "public_output_projected");

        let resumed = services
            .resume_stored_run(&fixture.run_id, DriveMode::UntilBlocked)
            .await
            .expect("resume completed typed run");
        assert_eq!(resumed.phase, TypedRunPhase::Completed);
        assert_eq!(resumed.scheduler_status, "public_output_projected");

        let replay = services
            .verify_replay_for_run(&fixture.run_id)
            .await
            .expect("verify replay authority");
        assert_eq!(replay.phase, TypedRunPhase::Completed);
        assert!(replay.retained_artifacts > 0);

        let output = services
            .typed_public_output(&fixture.run_id, &fixture.public_schema_id)
            .await
            .expect("render typed public output");
        assert_eq!(
            output.json,
            Some(serde_json::json!({
                "result": {
                    "total": "12.50"
                }
            }))
        );

        let stream = services
            .run_stream(&fixture.run_id)
            .await
            .expect("load stream");
        assert!(stream.events.iter().any(|event| {
            event.logical_key.starts_with("retention:")
                && event.logical_key.ends_with(":manifest:1")
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn public_output_and_append_only_resume_validate_certified_history() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let valid_stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load valid stream");
        let artifacts = services.artifacts().clone();
        let corrupt_stream = corrupt_public_output_history(&artifacts, &valid_stream).await;
        let corrupt_services = make_async_typed_services(
            production_typed_runner_registry(),
            StaticAsyncStore {
                run_id: fixture.run_id.clone(),
                stream: corrupt_stream,
            },
            artifacts,
        );

        let public_output_err = corrupt_services
            .typed_public_output(&fixture.run_id, &fixture.public_schema_id)
            .await
            .expect_err("public output rejects uncertified history");
        assert_eq!(public_output_err.code, "TypedRuntimeError");
        assert!(public_output_err.message.contains("public-output payload"));

        let resume_err = corrupt_services
            .resume_stored_run(&fixture.run_id, DriveMode::AppendOnly)
            .await
            .expect_err("append-only resume rejects uncertified history");
        assert_eq!(resume_err.code, "TypedRuntimeError");
        assert!(resume_err.message.contains("public-output payload"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn async_typed_start_rejects_missing_runner_before_run_started() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-async-missing-runner-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let fixture = framework_seed_public_output_fixture();
        artifacts
            .put_artifact(
                fixture.config_bytes.clone(),
                TypedArtifactDescriptor {
                    media_type: spec::MediaType::new("application/json").expect("media type"),
                    schema_id: Some(fixture.config_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::TypedConfig,
                },
            )
            .await
            .expect("persist config artifact");
        let canonical_spec = fixture.spec.canonical_json().expect("canonical spec");
        let request = build_typed_run_start_request(
            &artifacts,
            canonical_spec.as_bytes(),
            fixture.run_id.clone(),
            "mfm.test.framework",
            "test-source",
            vec![TypedSeedInput {
                seed_id: fixture.seed_id.clone(),
                bytes: fixture.seed_bytes.clone(),
                media_type: spec::MediaType::new("application/json").expect("media type"),
            }],
            DriveMode::UntilBlocked,
        )
        .await
        .expect("typed run request");
        let services = make_async_typed_services(
            production_typed_runner_registry(),
            AsyncInMemoryStore::default(),
            artifacts.clone(),
        );

        let err = services
            .start_certified_run(request)
            .await
            .expect_err("missing typed runner rejects");
        assert_eq!(err.code, "TypedRunnerUnavailable");
        assert!(matches!(err.class, ErrorClass::BadRequest));
        let status = services
            .run_status(&fixture.run_id)
            .await
            .expect_err("run was not started");
        assert_eq!(status.code, "TypedRunNotFound");

        let _ = std::fs::remove_dir_all(root);
    }

    async fn start_framework_fixture_run() -> (
        PathBuf,
        FrameworkSeedPublicOutputFixture,
        TypedAsyncAppServices<AsyncInMemoryStore>,
        TypedRunResponse,
    ) {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-async-framework-run-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let fixture = framework_seed_public_output_fixture();
        artifacts
            .put_artifact(
                fixture.config_bytes.clone(),
                TypedArtifactDescriptor {
                    media_type: spec::MediaType::new("application/json").expect("media type"),
                    schema_id: Some(fixture.config_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::TypedConfig,
                },
            )
            .await
            .expect("persist config artifact");
        artifacts
            .put_artifact(
                fixture.output_bytes.clone(),
                TypedArtifactDescriptor {
                    media_type: spec::MediaType::new("application/json").expect("media type"),
                    schema_id: Some(fixture.value_schema_id.clone()),
                    semantic_type_id: Some(fixture.semantic_type_id.clone()),
                    producer_node_id: Some(fixture.value_node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                },
            )
            .await
            .expect("persist runner output artifact");
        let canonical_spec = fixture.spec.canonical_json().expect("canonical spec");
        let request = build_typed_run_start_request(
            &artifacts,
            canonical_spec.as_bytes(),
            fixture.run_id.clone(),
            "mfm.test.framework",
            "test-source",
            vec![TypedSeedInput {
                seed_id: fixture.seed_id.clone(),
                bytes: fixture.seed_bytes.clone(),
                media_type: spec::MediaType::new("application/json").expect("media type"),
            }],
            DriveMode::UntilBlocked,
        )
        .await
        .expect("typed run request");
        let mut runners = ErasedRunnerRegistry::new();
        let factory_id = events::RunnerFactoryId::new("test-value").expect("factory id");
        runners
            .register(
                ErasedRunnerBinding::new(
                    fixture.value_descriptor_id.clone(),
                    factory_id.clone(),
                    test_executable(factory_id),
                    Arc::new(TestValueRunner {
                        output_bytes: fixture.output_bytes.clone(),
                    }),
                )
                .expect("runner binding"),
            )
            .expect("register runner");
        let services =
            make_async_typed_services(runners, AsyncInMemoryStore::default(), artifacts.clone());

        let started = services
            .start_certified_run(request)
            .await
            .expect("start typed run");
        (root, fixture, services, started)
    }

    async fn corrupt_public_output_history(
        artifacts: &FsTypedArtifactStore,
        valid_stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let run_id = valid_stream
            .first()
            .expect("valid stream is non-empty")
            .run_id()
            .clone();
        let mut corrupt_store = store::InMemoryTypedRunStore::new();
        let mut index = 0;
        while index < valid_stream.len() {
            let seq = valid_stream[index].seq();
            let start = index;
            while index < valid_stream.len() && valid_stream[index].seq() == seq {
                index += 1;
            }
            let group = &valid_stream[start..index];
            let mut payloads = group
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            let mut contains_public_output = false;
            for payload in &mut payloads {
                if let events::KernelEventPayload::PublicOutputProduced(public_output) = payload {
                    public_output.output_spec_digest = content_digest(0xee);
                    contains_public_output = true;
                }
            }
            let required_artifacts = required_artifacts_for_payloads(artifacts, &payloads).await;
            for evidence in &required_artifacts {
                corrupt_store
                    .record_artifact_evidence(evidence.clone())
                    .expect("record required artifact");
            }
            let contains_run_started = payloads
                .iter()
                .any(|payload| matches!(payload, events::KernelEventPayload::RunStarted(_)));
            corrupt_store
                .append_typed_run_commit(store::TypedCommitRequest {
                    run_id: run_id.clone(),
                    expected_next_seq: corrupt_store.expected_next_seq(&run_id),
                    commit_key: group[0].commit_key().clone(),
                    payloads,
                    required_artifacts,
                    preconditions: store::CommitPreconditions {
                        required_run_state: if contains_run_started {
                            store::RequiredRunState::Absent
                        } else {
                            store::RequiredRunState::NotCompleted
                        },
                        ..store::CommitPreconditions::default()
                    },
                })
                .expect("append corrupt-history test commit");
            if contains_public_output {
                break;
            }
        }
        corrupt_store.load_run_stream(&run_id)
    }

    async fn required_artifacts_for_payloads(
        artifacts: &FsTypedArtifactStore,
        payloads: &[events::KernelEventPayload],
    ) -> Vec<store::ArtifactEvidenceRef> {
        let mut artifact_ids = BTreeSet::new();
        for payload in payloads {
            match payload {
                events::KernelEventPayload::RunStarted(started) => {
                    artifact_ids.insert(started.spec_artifact_id.clone());
                    artifact_ids.extend(
                        started
                            .seed_cells
                            .iter()
                            .map(|seed| seed.seed_artifact.artifact_id.clone()),
                    );
                }
                events::KernelEventPayload::RetentionRefsAppended(retention) => {
                    artifact_ids.extend(
                        retention
                            .refs
                            .iter()
                            .map(|reference| reference.artifact_id.clone()),
                    );
                }
                events::KernelEventPayload::CellProduced(cell) => {
                    artifact_ids.insert(cell.artifact_id.clone());
                }
                events::KernelEventPayload::FactRecorded(fact) => {
                    artifact_ids.insert(fact.artifact_id.clone());
                }
                events::KernelEventPayload::PublicOutputProduced(public_output) => {
                    if let Some(artifact_id) = &public_output.rendered_artifact_id {
                        artifact_ids.insert(artifact_id.clone());
                    }
                }
                _ => {}
            }
        }

        let mut evidence_by_id = BTreeMap::new();
        for artifact_id in artifact_ids {
            let (_bytes, evidence) = artifacts
                .get_artifact_by_id(&artifact_id)
                .await
                .expect("required artifact exists in fixture store");
            evidence_by_id.insert(artifact_id, evidence);
        }
        evidence_by_id.into_values().collect()
    }

    struct StaticAsyncStore {
        run_id: RunId,
        stream: Vec<store::KernelEventEnvelope>,
    }

    impl store::AsyncTypedRunEventStore for StaticAsyncStore {
        type Error = store::StoreError;

        fn record_artifact_evidence<'a>(
            &'a self,
            _evidence: store::ArtifactEvidenceRef,
        ) -> store::AsyncStoreFuture<'a, (), Self::Error> {
            Box::pin(std::future::ready(Err(store::StoreError::Event(
                "static test store is read-only".to_owned(),
            ))))
        }

        fn append_typed_run_commit<'a>(
            &'a self,
            _request: store::TypedCommitRequest,
        ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
            Box::pin(std::future::ready(Err(store::StoreError::Event(
                "static test store is read-only".to_owned(),
            ))))
        }

        fn load_run_stream<'a>(
            &'a self,
            run_id: &'a RunId,
        ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
            let stream = if run_id == &self.run_id {
                self.stream.clone()
            } else {
                Vec::new()
            };
            Box::pin(std::future::ready(Ok(stream)))
        }

        fn expected_next_seq<'a>(
            &'a self,
            _run_id: &'a RunId,
        ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
            Box::pin(std::future::ready(Ok(store::StreamSeq::FIRST)))
        }
    }

    #[derive(Default)]
    struct AsyncInMemoryStore(StdMutex<store::InMemoryTypedRunStore>);

    impl store::AsyncTypedRunEventStore for AsyncInMemoryStore {
        type Error = store::StoreError;

        fn record_artifact_evidence<'a>(
            &'a self,
            evidence: store::ArtifactEvidenceRef,
        ) -> store::AsyncStoreFuture<'a, (), Self::Error> {
            let result = self
                .0
                .lock()
                .expect("store lock")
                .record_artifact_evidence(evidence);
            Box::pin(std::future::ready(result))
        }

        fn append_typed_run_commit<'a>(
            &'a self,
            request: store::TypedCommitRequest,
        ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
            let result = self
                .0
                .lock()
                .expect("store lock")
                .append_typed_run_commit(request);
            Box::pin(std::future::ready(result))
        }

        fn load_run_stream<'a>(
            &'a self,
            run_id: &'a RunId,
        ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
            let result = Ok(self.0.lock().expect("store lock").load_run_stream(run_id));
            Box::pin(std::future::ready(result))
        }

        fn expected_next_seq<'a>(
            &'a self,
            run_id: &'a RunId,
        ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
            let result = Ok(self.0.lock().expect("store lock").expected_next_seq(run_id));
            Box::pin(std::future::ready(result))
        }
    }

    struct FrameworkSeedPublicOutputFixture {
        spec: spec::TypedExecutionSpec,
        run_id: RunId,
        seed_id: SeedId,
        seed_bytes: Vec<u8>,
        output_bytes: Vec<u8>,
        config_bytes: Vec<u8>,
        config_schema_id: SchemaId,
        value_schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        value_node_id: NodeId,
        value_descriptor_id: DescriptorId,
        public_schema_id: SchemaId,
    }

    struct TestValueRunner {
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for TestValueRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let output_digest = content_digest_for_bytes(&self.output_bytes);
                let artifact_id = artifact_id_for_digest(&output_digest);
                let evidence = store::ArtifactEvidenceRef {
                    artifact_id: artifact_id.clone(),
                    digest: output_digest.clone(),
                    byte_len: self.output_bytes.len() as u64,
                    media_type: spec::MediaType::new("application/json")?,
                    schema_id: Some(ctx.output_cell.schema_id.clone()),
                    semantic_type_id: Some(ctx.output_cell.semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                Ok(ErasedRunnerOutput {
                    required_artifacts: vec![evidence],
                    staged_retention_refs: vec![StagedRetentionRefs {
                        refs: vec![events::RetentionRef {
                            artifact_id: artifact_id.clone(),
                            role: events::ArtifactRole::StateOutput,
                            content_digest: output_digest.clone(),
                        }],
                        reason: events::RetentionReason::PublicOutput,
                    }],
                    payloads: vec![
                        events::KernelEventPayload::CellProduced(events::CellProduced {
                            spec_hash: ctx.spec_hash.clone(),
                            node_id: ctx.node.node_id.clone(),
                            cell_id: ctx.node.output_cell.clone(),
                            scope_id: ctx.output_cell.scope_id.clone(),
                            attempt_id: ctx.attempt_id.clone(),
                            semantic_type_id: ctx.output_cell.semantic_type_id.clone(),
                            schema_id: ctx.output_cell.schema_id.clone(),
                            value_lineage: ctx.output_cell.value_lineage.clone(),
                            artifact_id,
                            content_digest: output_digest,
                            producer_state_kind: Some(ctx.node.state_kind.clone()),
                            producer_state_version: Some(ctx.node.state_version.clone()),
                        }),
                        events::KernelEventPayload::StateAttemptCompleted(
                            events::StateAttemptCompleted {
                                spec_hash: ctx.spec_hash.clone(),
                                node_id: ctx.node.node_id.clone(),
                                attempt_id: ctx.attempt_id.clone(),
                                output_cell_id: ctx.node.output_cell.clone(),
                            },
                        ),
                    ],
                })
            })
        }
    }

    fn test_executable(factory_id: events::RunnerFactoryId) -> events::ExecutableIdentity {
        events::ExecutableIdentity {
            factory_id,
            source_revision: events::SourceRevision::new("test-source").expect("source revision"),
            cargo_package_name: events::PackageName::new("mfm-app").expect("package name"),
            cargo_package_version: events::PackageVersion::new("0.0.0-test")
                .expect("package version"),
            cargo_package_digest: content_digest(0xd0),
            binary_digest: content_digest(0xd1),
            nix_derivation_hash: None,
            nix_output_hash: None,
        }
    }

    fn framework_seed_public_output_fixture() -> FrameworkSeedPublicOutputFixture {
        let run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0xa0));
        let scope_id = scope_id(0xa1);
        let seed_id = seed_id(0xa2);
        let seed_cell = cell_id(0xa3);
        let value_node = node_id(0xa4);
        let render_node = node_id(0xa5);
        let value_cell = cell_id(0xa6);
        let render_cell = cell_id(0xa7);
        let value_descriptor_id = descriptor_id(0xa8);
        let render_descriptor_id = descriptor_id(0xa9);
        let renderer_descriptor_id = descriptor_id(0xaa);
        let composition_descriptor_id = descriptor_id(0xab);
        let semantic_type_id = semantic_id("framework-seed-value", 0xac);
        let value_schema_id = schema_id("mfm.test.framework_seed_value", 0xad);
        let public_schema_id = schema_id("mfm.test.framework_seed_public", 0xae);
        let config_schema_id = schema_id("mfm.test.framework_seed_config", 0xaf);
        let seed_lineage = spec::ValueLineageRef {
            lineage_digest: content_digest(0xb0),
        };
        let value_lineage = spec::ValueLineageRef {
            lineage_digest: content_digest(0xb1),
        };
        let render_lineage = spec::ValueLineageRef {
            lineage_digest: content_digest(0xb2),
        };
        let planning_lineage = spec::PlanningLineage {
            active_operation_instances: Vec::new(),
            completed_operation_frames: Vec::new(),
            lineage_digest: content_digest(0xb3),
        };
        let config_bytes = b"{}".to_vec();
        let config_digest = content_digest_for_bytes(&config_bytes);
        let config_ref = spec::ConfigRef {
            schema_id: config_schema_id.clone(),
            artifact_id: artifact_id_for_digest(&config_digest),
            digest: config_digest.clone(),
            byte_len: config_bytes.len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media type"),
        };
        let seed_bytes = br#"{"input":"start"}"#.to_vec();
        let seed_digest = content_digest_for_bytes(&seed_bytes);
        let output_bytes = br#"{"total":"12.50"}"#.to_vec();
        let public_output_cell = spec::PublicOutputCell {
            public_field_path: spec::PublicFieldPath::new("result").expect("field path"),
            cell_id: value_cell.clone(),
            producer: spec::CellProducer::Node(value_node.clone()),
            scope_id: scope_id.clone(),
            semantic_type_id: semantic_type_id.clone(),
            schema_id: value_schema_id.clone(),
            value_lineage: value_lineage.clone(),
            required_terminal: spec::RequiredTerminal::ProducedOnly,
        };
        let renderer_descriptor = spec::RendererDescriptorIdentity {
            descriptor_id: renderer_descriptor_id,
            renderer_kind: spec::RendererKind::new("public-output/json").expect("renderer kind"),
            renderer_version: spec::RendererVersion::new("mfm.test.renderer.v1")
                .expect("renderer version"),
            public_schema_id: public_schema_id.clone(),
            canonicalizer_identity: spec::CanonicalizerIdentity::new("sha256-jcs-v1")
                .expect("canonicalizer"),
        };
        let public_outputs = spec::PublicOutputSpec {
            public_schema_id: public_schema_id.clone(),
            outputs: vec![public_output_cell.clone()],
            renderer_descriptor: renderer_descriptor.clone(),
        };
        let value_input_root =
            spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                field_path: spec::PublicFieldPath::new("input").expect("field path"),
                cell_id: seed_cell.clone(),
                semantic_type_id: semantic_type_id.clone(),
                schema_id: value_schema_id.clone(),
                required_terminal: spec::RequiredTerminal::ProducedOnly,
                value_lineage: seed_lineage.clone(),
            }));
        let render_input_root =
            spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                field_path: public_output_cell.public_field_path.clone(),
                node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                    field_path: public_output_cell.public_field_path.clone(),
                    cell_id: value_cell.clone(),
                    semantic_type_id: semantic_type_id.clone(),
                    schema_id: value_schema_id.clone(),
                    required_terminal: spec::RequiredTerminal::ProducedOnly,
                    value_lineage: value_lineage.clone(),
                })),
            }]);
        let value_state_kind = state_kind(0xb4);
        let render_state_kind = StateKind::new(
            "mfm.framework.state",
            "render_public_outputs",
            DigestAlgorithm::Sha256JcsV1,
            digest(0xb5),
        )
        .expect("state kind");
        let pure_effect = Pure::descriptor().expect("pure effect");
        let managed_effect = ManagedPlatformWrite::descriptor().expect("managed effect");
        let output_spec_digest = public_outputs.digest().expect("public output digest");
        let value_node_spec = spec::NodeSpec {
            node_id: value_node.clone(),
            stable_key: spec::StableAuthorKey::new("value").expect("stable key"),
            scope_id: scope_id.clone(),
            state_kind: value_state_kind.clone(),
            state_version: StateVersion::new("mfm.test.value.v1").expect("state version"),
            descriptor_id: value_descriptor_id.clone(),
            config_ref: config_ref.clone(),
            input_bindings: spec::InputBindingSpec {
                input_schema_id: value_schema_id.clone(),
                input_descriptor_id: descriptor_id(0xb6),
                root: value_input_root,
                digest: content_digest(0xb7),
            },
            output_cell: value_cell.clone(),
            effect_kind: pure_effect.kind.clone(),
            capability_bindings: CapabilitySetDescriptor::new(Vec::new()).expect("capability set"),
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: None,
            planning_lineage: planning_lineage.clone(),
            deterministic_predecessors: Vec::new(),
        };
        let render_node_spec = spec::NodeSpec {
            node_id: render_node.clone(),
            stable_key: spec::StableAuthorKey::new("public-output").expect("stable key"),
            scope_id: scope_id.clone(),
            state_kind: render_state_kind.clone(),
            state_version: StateVersion::new("mfm.framework.state.render_public_outputs.v1")
                .expect("state version"),
            descriptor_id: render_descriptor_id.clone(),
            config_ref: config_ref.clone(),
            input_bindings: spec::InputBindingSpec {
                input_schema_id: public_schema_id.clone(),
                input_descriptor_id: descriptor_id(0xb8),
                root: render_input_root,
                digest: content_digest(0xb9),
            },
            output_cell: render_cell.clone(),
            effect_kind: managed_effect.kind.clone(),
            capability_bindings: CapabilitySetDescriptor::new(Vec::new()).expect("capability set"),
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
                spec::PublicOutputRenderNodeSpec {
                    public_schema_id: public_schema_id.clone(),
                    output_spec_digest,
                    renderer_descriptor: renderer_descriptor.clone(),
                    required_cells: vec![public_output_cell],
                },
            )),
            planning_lineage: planning_lineage.clone(),
            deterministic_predecessors: vec![value_node.clone()],
        };
        let spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
            authoring: spec::AuthoringProvenance::StateComposition {
                descriptor: spec::CompositionDescriptor {
                    descriptor_id: composition_descriptor_id,
                    name: "mfm.test.framework_seed_public_output".to_owned(),
                    version: "mfm.test.framework_seed_public_output.v1".to_owned(),
                },
                config_hash: config_digest.clone(),
            },
            scopes: vec![spec::ScopeSpec {
                scope_id: scope_id.clone(),
                parent_scope_id: None,
                stable_key: spec::StableAuthorKey::new("root").expect("stable key"),
                planning_lineage: planning_lineage.clone(),
            }],
            seeds: vec![spec::SeedSpec {
                seed_id: seed_id.clone(),
                seed_key: spec::StableAuthorKey::new("launch").expect("stable key"),
                cell_id: seed_cell.clone(),
                scope_id: scope_id.clone(),
                semantic_type_id: semantic_type_id.clone(),
                schema_id: value_schema_id.clone(),
                required_digest: Some(seed_digest),
            }],
            descriptor_identities: vec![
                spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
                    descriptor_id: value_descriptor_id.clone(),
                    name: "mfm.test.value".to_owned(),
                    state_kind: value_state_kind,
                    state_version: StateVersion::new("mfm.test.value.v1").expect("state version"),
                    config_schema_id: config_schema_id.clone(),
                    input_schema_id: value_schema_id.clone(),
                    output_schema_id: value_schema_id.clone(),
                    output_semantic_type_id: semantic_type_id.clone(),
                    effect_kind: pure_effect.kind,
                    effect_class: pure_effect.class.as_str().to_owned(),
                    effect_name: pure_effect.name.to_owned(),
                    effect_version: pure_effect.version,
                    capabilities: CapabilitySetDescriptor::new(Vec::new()).expect("capability set"),
                    runner: "test-value".to_owned(),
                    side_effect_contract_digest: None,
                })),
                spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
                    descriptor_id: render_descriptor_id,
                    name: "mfm.framework.render_public_outputs".to_owned(),
                    state_kind: render_state_kind,
                    state_version: StateVersion::new(
                        "mfm.framework.state.render_public_outputs.v1",
                    )
                    .expect("state version"),
                    config_schema_id: config_schema_id.clone(),
                    input_schema_id: public_schema_id.clone(),
                    output_schema_id: spec::public_output_receipt_schema_id()
                        .expect("receipt schema"),
                    output_semantic_type_id: spec::public_output_receipt_semantic_type_id()
                        .expect("receipt semantic"),
                    effect_kind: managed_effect.kind,
                    effect_class: managed_effect.class.as_str().to_owned(),
                    effect_name: managed_effect.name.to_owned(),
                    effect_version: managed_effect.version,
                    capabilities: CapabilitySetDescriptor::new(Vec::new()).expect("capability set"),
                    runner: "managed_platform_write".to_owned(),
                    side_effect_contract_digest: None,
                })),
                spec::DescriptorIdentity::Renderer(Box::new(renderer_descriptor)),
            ],
            config_refs: vec![config_ref.clone()],
            nodes: vec![render_node_spec, value_node_spec],
            cells: vec![
                spec::CellSpec {
                    cell_id: seed_cell.clone(),
                    producer: spec::CellProducer::Seed(seed_id.clone()),
                    scope_id: scope_id.clone(),
                    semantic_type_id: semantic_type_id.clone(),
                    schema_id: value_schema_id.clone(),
                    value_lineage: seed_lineage.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
                spec::CellSpec {
                    cell_id: value_cell.clone(),
                    producer: spec::CellProducer::Node(value_node.clone()),
                    scope_id: scope_id.clone(),
                    semantic_type_id: semantic_type_id.clone(),
                    schema_id: value_schema_id.clone(),
                    value_lineage: value_lineage.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
                spec::CellSpec {
                    cell_id: render_cell,
                    producer: spec::CellProducer::Node(render_node),
                    scope_id: scope_id.clone(),
                    semantic_type_id: spec::public_output_receipt_semantic_type_id()
                        .expect("receipt semantic"),
                    schema_id: spec::public_output_receipt_schema_id().expect("receipt schema"),
                    value_lineage: render_lineage.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::PublicOutputArtifact,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
            ],
            value_lineages: vec![
                spec::ValueLineage {
                    lineage_ref: seed_lineage,
                    scope_id: scope_id.clone(),
                    producer: spec::CellProducer::Seed(seed_id.clone()),
                    input_cells: Vec::new(),
                    config_ref_digest: None,
                    planning_lineage: planning_lineage.clone(),
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::Source,
                },
                spec::ValueLineage {
                    lineage_ref: value_lineage,
                    scope_id: scope_id.clone(),
                    producer: spec::CellProducer::Node(value_node.clone()),
                    input_cells: vec![seed_cell],
                    config_ref_digest: Some(config_ref.digest.clone()),
                    planning_lineage: planning_lineage.clone(),
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::StateOutput,
                },
                spec::ValueLineage {
                    lineage_ref: render_lineage,
                    scope_id,
                    producer: spec::CellProducer::Node(node_id(0xa5)),
                    input_cells: vec![value_cell],
                    config_ref_digest: Some(config_ref.digest),
                    planning_lineage,
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::StateOutput,
                },
            ],
            planning_lineage: Vec::new(),
            public_outputs,
        })
        .expect("typed spec");

        FrameworkSeedPublicOutputFixture {
            spec,
            run_id,
            seed_id,
            seed_bytes,
            output_bytes,
            config_bytes,
            config_schema_id,
            value_schema_id,
            semantic_type_id,
            value_node_id: value_node,
            value_descriptor_id,
            public_schema_id,
        }
    }

    fn append_commit(
        store: &mut store::InMemoryTypedRunStore,
        run_id: &RunId,
        commit_key: &str,
        payloads: Vec<events::KernelEventPayload>,
        required_artifacts: Vec<store::ArtifactEvidenceRef>,
        required_run_state: store::RequiredRunState,
    ) {
        store
            .append_typed_run_commit(store::TypedCommitRequest {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(run_id),
                commit_key: store::CommitKey::new(commit_key).expect("commit key"),
                payloads,
                required_artifacts,
                preconditions: store::CommitPreconditions {
                    required_run_state,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append commit");
    }

    fn state_attempt_started(
        spec_hash: &SpecHash,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        attempt_no: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: spec_hash.clone(),
            node_id: node_id.clone(),
            attempt_id: attempt_id.clone(),
            attempt_no,
            state_kind: state_kind(0x42),
            state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
        })
    }

    fn digest(byte: u8) -> DigestBytes {
        DigestBytes::from_array([byte; 32])
    }

    fn content_digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn content_digest_for_bytes(bytes: &[u8]) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
    }

    fn artifact_id_for_digest(digest: &ContentDigest) -> ArtifactId {
        ArtifactId::from_digest(digest.algorithm(), *digest.digest())
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn artifact_id(byte: u8) -> ArtifactId {
        ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn attempt_id(byte: u8) -> AttemptId {
        AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn descriptor_id(byte: u8) -> DescriptorId {
        DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn cell_id(byte: u8) -> CellId {
        CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn scope_id(byte: u8) -> ScopeId {
        ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn seed_id(byte: u8) -> SeedId {
        SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
    }

    fn schema_id(name: &str, byte: u8) -> SchemaId {
        SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest(byte)).expect("schema id")
    }

    fn semantic_id(name: &str, byte: u8) -> SemanticTypeId {
        SemanticTypeId::new(
            "mfm.test",
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest(byte),
        )
        .expect("semantic id")
    }

    fn state_kind(byte: u8) -> StateKind {
        StateKind::new(
            "mfm.test",
            "state",
            DigestAlgorithm::Sha256JcsV1,
            digest(byte),
        )
        .expect("state kind")
    }
}
