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
use mfm_ids::{ArtifactId, DigestAlgorithm, RunId, SchemaId, SpecHash};
use mfm_replay::v1::{ReplayAuthority, ReplayBroker, ReplayError};
use mfm_runtime::{CertifiedRuntimeSpec, RunStartEvidence, SchedulerStatus, SerialTypedScheduler};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use serde::Serialize;
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
            mfm_runtime::RuntimeError::SpecHash(message)
            | mfm_runtime::RuntimeError::InvalidSpec(message)
            | mfm_runtime::RuntimeError::InvalidRunStream(message)
            | mfm_runtime::RuntimeError::RunnerBinding(message)
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
        let status = self
            .drive_with_mode(&mut *store, &runtime_spec, &req.run_id, req.drive)
            .await?;
        typed_run_response(&*store, &runtime_spec, &req.run_id, status)
    }

    /// Returns typed run status by rebuilding projection from the authoritative run stream.
    pub async fn run_status(&self, run_id: &RunId) -> Result<TypedRunResponse, AppError> {
        let store = self.store.lock().await;
        let stream = store.load_run_stream(run_id);
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        let spec_hash = run_started_spec_hash(&stream)?;
        let projection = store::ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
        Ok(TypedRunResponse {
            run_id: run_id.as_str().to_owned(),
            spec_hash: spec_hash.as_str().to_owned(),
            phase: typed_phase(projection.run_state(run_id)),
            scheduler_status: "observed".to_owned(),
            head_seq: stream_head(&stream),
        })
    }

    /// Returns the authoritative typed run stream.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<TypedRunStreamResponse, AppError> {
        let store = self.store.lock().await;
        let events = store.load_run_stream(run_id);
        let refs = events.iter().map(typed_event_ref).collect();
        Ok(TypedRunStreamResponse {
            run_id: run_id.as_str().to_owned(),
            head_seq: stream_head(&events),
            events: refs,
        })
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
        let projection = {
            let store = self.store.lock().await;
            let stream = store.load_run_stream(run_id);
            if stream.is_empty() {
                return Err(AppError::not_found(
                    "TypedRunNotFound",
                    "typed run stream was not found",
                ));
            }
            store::ProjectionSnapshot::rebuild_from_run_stream(&stream)?
        };
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
        let json = match rendered_artifact_id {
            Some(artifact_id) => Some(load_public_output_json(&self.artifacts, artifact_id).await?),
            None => None,
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

    async fn drive_with_mode(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<SchedulerStatus, AppError> {
        match drive {
            DriveMode::AppendOnly => Ok(SchedulerStatus::Blocked),
            DriveMode::UntilBlocked => self
                .scheduler
                .drive_until_blocked(store, runtime_spec, run_id)
                .await
                .map_err(Into::into),
        }
    }

    async fn validate_launch_artifacts(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        evidence: &RunStartEvidence,
    ) -> Result<(), AppError> {
        let spec_bytes = self.artifacts.get_artifact(&evidence.spec_artifact).await?;
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
            self.artifacts.get_artifact(config_artifact).await?;
        }
        for seed in &evidence.seed_cells {
            self.artifacts
                .get_artifact(&seed_artifact_evidence(seed))
                .await?;
        }
        Ok(())
    }
}

async fn load_public_output_json(
    artifacts: &FsTypedArtifactStore,
    artifact_id: &ArtifactId,
) -> Result<serde_json::Value, AppError> {
    let (bytes, evidence) = artifacts.get_artifact_by_id(artifact_id).await?;
    if evidence.artifact_role != events::ArtifactRole::PublicOutput {
        return Err(AppError::new(
            ErrorClass::Internal,
            "TypedPublicOutputArtifactMismatch",
            "typed public-output projection points at a non-public-output artifact",
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

fn typed_run_response<S: store::TypedRunEventStore + ?Sized>(
    store: &S,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    status: SchedulerStatus,
) -> Result<TypedRunResponse, AppError> {
    let stream = store.load_run_stream(run_id);
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
    Ok(TypedRunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: runtime_spec.spec_hash().as_str().to_owned(),
        phase: typed_phase(projection.run_state(run_id)),
        scheduler_status: scheduler_status_str(status).to_owned(),
        head_seq: stream_head(&stream),
    })
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
mod tests {
    use super::*;

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
}
