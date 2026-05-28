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
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_certify::{CertificationRegistry, CertifiedSpecBundle, CertifiedTypedSpec};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, EventId, RunId, SchemaId, SeedId, SpecHash,
};
use mfm_replay::v1::{ReplayAuthority, ReplayBroker, ReplayError};
use mfm_runtime::{
    build_public_output_receipt_artifact, build_retention_manifest_artifact, validate_run_stream,
    CertifiedRuntimeSpec, RunStartEvidence, SchedulerStatus, SerialTypedScheduler,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use serde::{Deserialize, Serialize};
use serde_json::map::Entry;
use serde_json::{Map, Value};
use tokio::sync::Mutex;

pub use mfm_runtime::ErasedRunnerRegistry;

/// Shared observability configuration used by typed binaries.
pub mod observability;

const ENV_TYPED_ARTIFACT_ROOT: &str = "MFM_TYPED_ARTIFACT_ROOT";
const DEFAULT_TYPED_ARTIFACT_SUBDIR: &str = "typed_run_artifacts";
/// Transport kind for a certified typed spec bundle accepted by app frontends.
pub const CERTIFIED_SPEC_BUNDLE_KIND: &str = "certified_typed_spec_bundle_v1";

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

impl From<mfm_certify::CertifyError> for AppError {
    fn from(error: mfm_certify::CertifyError) -> Self {
        Self::new(
            ErrorClass::BadRequest,
            "TypedCertificationFailed",
            error.to_string(),
        )
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
    make_in_memory_typed_services_with_certification_registry(
        runners,
        artifact_root,
        CertificationRegistry::new(),
    )
}

/// Builds in-memory typed app services with an explicit trusted certification registry.
pub fn make_in_memory_typed_services_with_certification_registry(
    runners: ErasedRunnerRegistry,
    artifact_root: impl Into<PathBuf>,
    certification_registry: CertificationRegistry,
) -> TypedAppServices<store::InMemoryTypedRunStore> {
    TypedAppServices::new_with_certification_registry(
        SerialTypedScheduler::new(runners),
        store::InMemoryTypedRunStore::default(),
        FsTypedArtifactStore::new(artifact_root),
        certification_registry,
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
    make_async_typed_services_with_certification_registry(
        runners,
        store,
        artifacts,
        CertificationRegistry::new(),
    )
}

/// Builds typed async app services with an explicit trusted certification registry.
pub fn make_async_typed_services_with_certification_registry<S>(
    runners: ErasedRunnerRegistry,
    store: S,
    artifacts: FsTypedArtifactStore,
    certification_registry: CertificationRegistry,
) -> TypedAsyncAppServices<S>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
    TypedAsyncAppServices::new_with_certification_registry(
        SerialTypedScheduler::new(runners),
        store,
        artifacts,
        certification_registry,
    )
}

/// Builds the production typed runner registry for this process.
///
/// Framework public-output render nodes are resolved by `mfm-runtime` as built-ins. Enabled
/// domain runners register here as certified typed descriptor bindings.
pub fn production_typed_runner_registry(
    artifacts: FsTypedArtifactStore,
) -> Result<ErasedRunnerRegistry, AppError> {
    let mut registry = ErasedRunnerRegistry::new();
    let portfolio_artifacts: Arc<dyn mfm_transports_portfolio::PortfolioArtifactStore> =
        Arc::new(artifacts.clone());
    mfm_transports_portfolio::register_portfolio_runners(&mut registry, portfolio_artifacts)?;
    let evm_dcv_artifacts: Arc<dyn mfm_transports_evm_dcv::EvmDcvArtifactStore> =
        Arc::new(artifacts.clone());
    mfm_transports_evm_dcv::register_evm_dcv_runners(&mut registry, evm_dcv_artifacts)?;
    let proof_artifacts: Arc<dyn mfm_transports_proof::ProofArtifactSink> =
        Arc::new(FsProofArtifactSink { artifacts });
    mfm_transports_proof::register_deterministic_proof_runners(&mut registry, proof_artifacts)?;
    Ok(registry)
}

/// Builds the trusted production certification registry for bundled spec verification.
pub fn production_certification_registry() -> Result<CertificationRegistry, AppError> {
    let mut registry = CertificationRegistry::new();
    mfm_op_portfolio_tracker::register_portfolio_certification_descriptors(&mut registry)?;
    mfm_op_evm_deploy_configure_validate::register_dcv_certification_descriptors(&mut registry)?;
    mfm_op_proof::register_proof_certification_descriptors(&mut registry)?;
    Ok(registry)
}

#[derive(Clone)]
struct FsProofArtifactSink {
    artifacts: FsTypedArtifactStore,
}

impl mfm_transports_proof::ProofArtifactSink for FsProofArtifactSink {
    fn put_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> mfm_transports_proof::ProofArtifactSinkFuture<'a> {
        Box::pin(async move {
            self.artifacts
                .put_verified_artifact(bytes, evidence)
                .await
                .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
            Ok(())
        })
    }
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
    /// Drive at most one scheduler step, preserving a restart boundary after each durable commit.
    Once,
    /// Drive deterministic runnable nodes until the scheduler blocks or the run completes.
    UntilBlocked,
}

/// Request to start a certified typed run.
#[derive(Debug, Clone)]
pub struct TypedRunStartRequest {
    /// Certifier-backed typed spec authority.
    pub certified_spec: CertifiedTypedSpec,
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

/// Non-forgeable authority to render one typed public output.
///
/// This is minted only after the app verifies certified runtime authority, validates the
/// authoritative run stream, and rebuilds public-output projection from that stream. Rendered JSON
/// artifacts are caches only and cannot construct this authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputReadAuthority {
    run_id: RunId,
    public_schema_id: SchemaId,
    event_id: EventId,
    rendered_digest: ContentDigest,
    rendered_artifact_id: Option<ArtifactId>,
    payload: events::PublicOutputProduced,
}

impl PublicOutputReadAuthority {
    /// Returns the run id bound to this read authority.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the public-output schema id bound to this read authority.
    pub fn public_schema_id(&self) -> &SchemaId {
        &self.public_schema_id
    }

    /// Returns the store-owned event id that produced this public output.
    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    /// Returns the canonical digest of the rendered public output.
    pub fn rendered_digest(&self) -> &ContentDigest {
        &self.rendered_digest
    }

    /// Returns the persisted rendered artifact id, when the renderer wrote one.
    pub fn rendered_artifact_id(&self) -> Option<&ArtifactId> {
        self.rendered_artifact_id.as_ref()
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
    certification_registry: CertificationRegistry,
}

impl<S> TypedAppServices<S>
where
    S: store::TypedRunEventStore + Send,
{
    /// Creates typed app services from explicit scheduler, store, and artifact store choices.
    pub fn new(scheduler: SerialTypedScheduler, store: S, artifacts: FsTypedArtifactStore) -> Self {
        Self::new_with_certification_registry(
            scheduler,
            store,
            artifacts,
            CertificationRegistry::new(),
        )
    }

    /// Creates typed app services with an explicit trusted certification registry.
    pub fn new_with_certification_registry(
        scheduler: SerialTypedScheduler,
        store: S,
        artifacts: FsTypedArtifactStore,
        certification_registry: CertificationRegistry,
    ) -> Self {
        Self {
            scheduler,
            store: Arc::new(Mutex::new(store)),
            artifacts,
            certification_registry,
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

    /// Returns the trusted certification registry used for stored bundle verification.
    pub fn certification_registry(&self) -> &CertificationRegistry {
        &self.certification_registry
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
        let runtime_spec = CertifiedRuntimeSpec::new(req.certified_spec)?;
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

    /// Resumes a stored typed run after verifying its persisted certified authority.
    pub async fn resume_stored_run(
        &self,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<TypedRunResponse, AppError> {
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
        let runtime_spec = load_runtime_spec_for_run(
            &self.artifacts,
            &self.certification_registry,
            run_id,
            &stream,
        )
        .await?;
        let mut store = self.store.lock().await;
        let stream = store.load_run_stream(run_id);
        validate_run_stream(&runtime_spec, run_id, &stream)?;
        let status = self
            .drive_with_mode(&mut *store, &runtime_spec, run_id, drive)
            .await?;
        typed_run_response(&*store, &runtime_spec, run_id, status)
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

    /// Builds an evidence-only replay broker from stored certified authority and retained evidence.
    pub async fn replay_broker(&self, run_id: &RunId) -> Result<ReplayBroker, AppError> {
        let store = self.store.lock().await;
        let stream = store.load_run_stream(run_id);
        if stream.is_empty() {
            return Err(AppError::not_found(
                "TypedRunNotFound",
                "typed run stream was not found",
            ));
        }
        let certified = load_certified_spec_for_run(
            &self.artifacts,
            &self.certification_registry,
            run_id,
            &stream,
        )
        .await?;
        let authority =
            replay_authority_for_run(&self.artifacts, &certified, run_id, &stream).await?;
        let envelope = certified.envelope().clone();
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
        let runtime_spec = load_runtime_spec_for_run(
            &self.artifacts,
            &self.certification_registry,
            run_id,
            &stream,
        )
        .await?;
        let authority =
            verify_public_output_read_authority(&runtime_spec, run_id, public_schema_id, &stream)?;
        render_typed_public_output(&self.artifacts, &authority).await
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
            DriveMode::Once => {
                self.drive_once_with_retention(store, runtime_spec, run_id)
                    .await
            }
            DriveMode::UntilBlocked => {
                self.drive_until_blocked_with_retention(store, runtime_spec, run_id)
                    .await
            }
        }
    }

    async fn drive_once_with_retention(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus, AppError> {
        let status = self
            .scheduler
            .drive_once(store, runtime_spec, run_id)
            .await?;
        if status == SchedulerStatus::Advanced {
            self.project_retention_manifest_if_ready(store, runtime_spec, run_id)
                .await?;
        }
        Ok(status)
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
    certification_registry: CertificationRegistry,
}

impl<S> TypedAsyncAppServices<S>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
    /// Creates typed async app services from explicit scheduler, store, and artifact store choices.
    pub fn new(scheduler: SerialTypedScheduler, store: S, artifacts: FsTypedArtifactStore) -> Self {
        Self::new_with_certification_registry(
            scheduler,
            store,
            artifacts,
            CertificationRegistry::new(),
        )
    }

    /// Creates typed async app services with an explicit trusted certification registry.
    pub fn new_with_certification_registry(
        scheduler: SerialTypedScheduler,
        store: S,
        artifacts: FsTypedArtifactStore,
        certification_registry: CertificationRegistry,
    ) -> Self {
        Self {
            scheduler,
            store,
            artifacts,
            certification_registry,
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

    /// Returns the trusted certification registry used for stored bundle verification.
    pub fn certification_registry(&self) -> &CertificationRegistry {
        &self.certification_registry
    }

    /// Starts a certified typed run against a durable async typed store.
    pub async fn start_certified_run(
        &self,
        req: TypedRunStartRequest,
    ) -> Result<TypedRunResponse, AppError> {
        let runtime_spec = CertifiedRuntimeSpec::new(req.certified_spec)?;
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
        let runtime_spec = load_runtime_spec_for_run(
            &self.artifacts,
            &self.certification_registry,
            run_id,
            &stream,
        )
        .await?;
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
        let certified = load_certified_spec_for_run(
            &self.artifacts,
            &self.certification_registry,
            run_id,
            &stream,
        )
        .await?;
        let authority =
            replay_authority_for_run(&self.artifacts, &certified, run_id, &stream).await?;
        let envelope = certified.envelope().clone();
        let broker = ReplayBroker::from_run_stream(envelope, &stream, authority)?;
        mfm_transports_proof::verify_deterministic_proof_replay(&broker, &stream)?;
        mfm_transports_evm_dcv::verify_evm_dcv_replay(&broker, &stream, &self.artifacts).await?;
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
        let runtime_spec = load_runtime_spec_for_run(
            &self.artifacts,
            &self.certification_registry,
            run_id,
            &stream,
        )
        .await?;
        let authority =
            verify_public_output_read_authority(&runtime_spec, run_id, public_schema_id, &stream)?;
        render_typed_public_output(&self.artifacts, &authority).await
    }

    async fn drive_with_mode(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<SchedulerStatus, AppError> {
        match drive {
            DriveMode::AppendOnly => Ok(SchedulerStatus::Blocked),
            DriveMode::Once => self.drive_once_with_retention(runtime_spec, run_id).await,
            DriveMode::UntilBlocked => {
                self.drive_until_blocked_with_retention(runtime_spec, run_id)
                    .await
            }
        }
    }

    async fn drive_once_with_retention(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus, AppError> {
        let status = self
            .scheduler
            .drive_once_async(&self.store, runtime_spec, run_id)
            .await?;
        if status == SchedulerStatus::Advanced {
            self.project_retention_manifest_if_ready(runtime_spec, run_id)
                .await?;
        }
        Ok(status)
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
    registry: &CertificationRegistry,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<CertifiedTypedSpec, AppError> {
    let run_started = run_started_payload(run_id, stream)?;
    let (spec_bytes, spec_evidence) = artifacts
        .get_artifact_by_id(&run_started.spec_artifact_id)
        .await?;
    validate_spec_artifact_evidence(run_started, &spec_evidence)?;
    let (certificate_bytes, certificate_evidence) = artifacts
        .get_artifact_by_id(&run_started.certificate_artifact_id)
        .await?;
    validate_certificate_artifact_evidence(run_started, &certificate_evidence)?;
    let certified = mfm_certify::verify_certified_bundle_with_trusted_registry(
        &spec_bytes,
        &certificate_bytes,
        registry,
    )?;
    validate_run_started_matches_spec(run_started, certified.envelope())?;
    Ok(certified)
}

async fn load_runtime_spec_for_run(
    artifacts: &FsTypedArtifactStore,
    registry: &CertificationRegistry,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<CertifiedRuntimeSpec, AppError> {
    let certified = load_certified_spec_for_run(artifacts, registry, run_id, stream).await?;
    CertifiedRuntimeSpec::new(certified).map_err(Into::into)
}

/// Builds replay authority from retained artifact evidence in the run stream.
pub async fn replay_authority_for_run(
    artifacts: &FsTypedArtifactStore,
    certified: &CertifiedTypedSpec,
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
        certified.envelope(),
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

/// Persists the certified typed spec certificate artifact for a runtime spec.
pub async fn persist_certified_spec_certificate_artifact(
    artifacts: &FsTypedArtifactStore,
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<store::ArtifactEvidenceRef, AppError> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| {
            AppError::new(
                ErrorClass::Internal,
                "TypedCertificateCanonicalError",
                error.to_string(),
            )
        })?;
    let media_type =
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).map_err(|error| {
            AppError::new(
                ErrorClass::Internal,
                "TypedCertificateMediaTypeInvalid",
                error.to_string(),
            )
        })?;
    artifacts
        .put_artifact(
            canonical.to_vec(),
            TypedArtifactDescriptor {
                media_type,
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::TypedSpecCertificate,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CertifiedSpecBundleJson {
    kind: String,
    spec: Value,
    certificate: Value,
}

/// Parses a certified typed spec bundle transport JSON document as untrusted bytes.
///
/// The returned bundle is not runtime authority. Callers must pass its spec and certificate bytes
/// through the certifier verifier before starting, resuming, replaying, or rendering a run.
pub fn parse_certified_spec_bundle_json_bytes(
    bytes: &[u8],
) -> Result<CertifiedSpecBundle, AppError> {
    let parsed: CertifiedSpecBundleJson = serde_json::from_slice(bytes).map_err(|error| {
        AppError::new(
            ErrorClass::BadRequest,
            "TypedBundleInvalid",
            format!("invalid certified typed spec bundle JSON: {error}"),
        )
    })?;
    certified_spec_bundle_from_json(parsed)
}

/// Parses a certified typed spec bundle JSON value as untrusted bytes.
///
/// This helper exists for transports that already parsed the request envelope. It does not mint
/// certified authority; verification is still required before any runtime contract is constructed.
pub fn parse_certified_spec_bundle_json_value(
    value: &Value,
) -> Result<CertifiedSpecBundle, AppError> {
    let parsed: CertifiedSpecBundleJson =
        serde_json::from_value(value.clone()).map_err(|error| {
            AppError::new(
                ErrorClass::BadRequest,
                "TypedBundleInvalid",
                format!("invalid certified typed spec bundle: {error}"),
            )
        })?;
    certified_spec_bundle_from_json(parsed)
}

fn certified_spec_bundle_from_json(
    parsed: CertifiedSpecBundleJson,
) -> Result<CertifiedSpecBundle, AppError> {
    if parsed.kind != CERTIFIED_SPEC_BUNDLE_KIND {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "TypedBundleInvalid",
            format!("certified typed spec bundle kind must be {CERTIFIED_SPEC_BUNDLE_KIND:?}"),
        ));
    }
    let spec_bytes = canonical_json_value_bytes(&parsed.spec, "spec")?;
    let certificate_bytes = canonical_json_value_bytes(&parsed.certificate, "certificate")?;
    Ok(CertifiedSpecBundle::from_untrusted_bytes(
        spec_bytes,
        certificate_bytes,
    ))
}

fn canonical_json_value_bytes(value: &Value, field: &'static str) -> Result<Vec<u8>, AppError> {
    let json = serde_json::to_string(value).map_err(|error| {
        AppError::new(
            ErrorClass::Internal,
            "TypedBundleSerializationFailed",
            format!("failed to serialize {field} JSON value: {error}"),
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.to_vec())
        .map_err(|error| {
            AppError::new(
                ErrorClass::BadRequest,
                "TypedBundleInvalid",
                format!("invalid certified typed spec bundle {field}: {error}"),
            )
        })
}

/// Untrusted persisted certified bundle bytes plus launch inputs for a typed run start.
pub struct UntrustedCertifiedSpecBundleStartInput<'a> {
    /// Canonical JSON bytes for the persisted typed execution spec.
    pub spec_bytes: &'a [u8],
    /// Canonical JSON bytes for the persisted typed spec certificate.
    pub certificate_bytes: &'a [u8],
    /// Trusted registry used to verify descriptor evidence in the certificate.
    pub registry: &'a CertificationRegistry,
    /// Run id to record in the started run stream.
    pub run_id: RunId,
    /// Framework version evidence to bind to the run start event.
    pub framework_version: &'a str,
    /// Source revision evidence to bind to the run start event.
    pub source_revision: &'a str,
    /// Drive mode used for the initial scheduler invocation.
    pub drive: DriveMode,
}

/// Verifies untrusted persisted certified bundle bytes and builds a typed run-start request.
///
/// This helper is for transport and storage boundaries that receive serialized bundle data. It
/// verifies the spec and certificate against the trusted registry before producing a request that
/// can reach the runtime boundary.
pub async fn verify_certified_bundle_run_start_request(
    artifacts: &FsTypedArtifactStore,
    input: UntrustedCertifiedSpecBundleStartInput<'_>,
    seed_inputs: Vec<TypedSeedInput>,
) -> Result<TypedRunStartRequest, AppError> {
    let certified_spec = mfm_certify::verify_certified_bundle_with_trusted_registry(
        input.spec_bytes,
        input.certificate_bytes,
        input.registry,
    )?;
    build_certified_typed_run_start_request(
        artifacts,
        certified_spec,
        input.run_id,
        input.framework_version,
        input.source_revision,
        seed_inputs,
        input.drive,
    )
    .await
}

/// Builds a typed run-start request from certifier-backed typed spec authority and launch inputs.
pub async fn build_certified_typed_run_start_request(
    artifacts: &FsTypedArtifactStore,
    certified_spec: CertifiedTypedSpec,
    run_id: RunId,
    framework_version: &str,
    source_revision: &str,
    seed_inputs: Vec<TypedSeedInput>,
    drive: DriveMode,
) -> Result<TypedRunStartRequest, AppError> {
    let runtime_spec = CertifiedRuntimeSpec::new(certified_spec.clone())?;
    let spec_artifact = persist_certified_spec_artifact(artifacts, &runtime_spec).await?;
    let certificate_artifact =
        persist_certified_spec_certificate_artifact(artifacts, &runtime_spec).await?;
    let config_artifacts = load_config_artifacts_for_spec(artifacts, &runtime_spec).await?;
    let seed_cells = persist_seed_inputs_for_spec(artifacts, &runtime_spec, seed_inputs).await?;
    Ok(TypedRunStartRequest {
        certified_spec,
        run_id,
        evidence: RunStartEvidence {
            spec_artifact,
            certificate_artifact,
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
    let certificate_bytes = artifacts
        .get_artifact(&evidence.certificate_artifact)
        .await?;
    let certificate_canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| {
            AppError::new(
                ErrorClass::Internal,
                "TypedCertificateCanonicalError",
                error.to_string(),
            )
        })?;
    if certificate_bytes != certificate_canonical.as_bytes() {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "TypedCertificateArtifactMismatch",
            "persisted typed spec certificate artifact bytes do not match the certified spec",
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

/// Verifies a typed public-output read authority from certified runtime authority and stream data.
///
/// The supplied stream is treated as the authoritative event source. This verifier validates that
/// stream against the certified runtime spec and rebuilds the public-output projection from events
/// before minting render authority.
pub fn verify_public_output_read_authority(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    public_schema_id: &SchemaId,
    stream: &[store::KernelEventEnvelope],
) -> Result<PublicOutputReadAuthority, AppError> {
    if stream.is_empty() {
        return Err(AppError::not_found(
            "TypedRunNotFound",
            "typed run stream was not found",
        ));
    }
    validate_run_stream(runtime_spec, run_id, stream)?;
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
    if &payload.spec_hash != runtime_spec.spec_hash()
        || &payload.public_schema_id != public_schema_id
        || public_schema_id != &runtime_spec.spec().public_outputs.public_schema_id
    {
        return Err(AppError::new(
            ErrorClass::Internal,
            "TypedPublicOutputAuthorityMismatch",
            "typed public-output evidence does not match certified runtime authority",
        ));
    }

    Ok(PublicOutputReadAuthority {
        run_id: run_id.clone(),
        public_schema_id: public_schema_id.clone(),
        event_id: event_id.clone(),
        rendered_digest: rendered_digest.clone(),
        rendered_artifact_id: rendered_artifact_id.clone(),
        payload: payload.clone(),
    })
}

/// Renders typed public output from app-verified read authority and typed artifact bytes.
pub async fn render_typed_public_output(
    artifacts: &FsTypedArtifactStore,
    authority: &PublicOutputReadAuthority,
) -> Result<TypedPublicOutputResponse, AppError> {
    let json = match authority.rendered_artifact_id() {
        Some(artifact_id) => Some(
            load_public_output_json(
                artifacts,
                artifact_id,
                authority.rendered_digest(),
                &authority.payload,
            )
            .await?,
        ),
        None => Some(render_public_output_json_from_authority(artifacts, authority).await?),
    };
    Ok(TypedPublicOutputResponse {
        run_id: authority.run_id().as_str().to_owned(),
        public_schema_id: authority.public_schema_id().as_str().to_owned(),
        event_id: authority.event_id().as_str().to_owned(),
        rendered_digest: authority.rendered_digest().as_str().to_owned(),
        rendered_artifact_id: authority
            .rendered_artifact_id()
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

async fn render_public_output_json_from_authority(
    artifacts: &FsTypedArtifactStore,
    authority: &PublicOutputReadAuthority,
) -> Result<Value, AppError> {
    let mut root = Map::new();
    for cell in &authority.payload.cells {
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

fn validate_certificate_artifact_evidence(
    run_started: &events::RunStarted,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), AppError> {
    if evidence.artifact_id != run_started.certificate_artifact_id
        || evidence.digest != run_started.certificate_artifact_digest
        || evidence.media_type != run_started.certificate_media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedSpecCertificate
    {
        return Err(AppError::new(
            ErrorClass::Internal,
            "TypedCertificateArtifactMismatch",
            "typed spec certificate artifact metadata does not match RunStarted evidence",
        ));
    }
    Ok(())
}

fn validate_run_started_matches_spec(
    run_started: &events::RunStarted,
    envelope: &spec::HashedSpecEnvelope,
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
    use mfm_capabilities::{NoCaps, Pure};
    use mfm_ids::{AttemptId, DescriptorId, StateKind, StateVersion};
    use mfm_ids::{CellId, ContentDigest, DigestBytes, NodeId, ScopeId, SeedId, SemanticTypeId};
    use mfm_program::{
        build_root_with_registries, CanonicalSeed, PublicOutputKey, PureState, RootBuilder,
        ScopeKey, SeedKey, StateKey, StateRegistryBuilder, StateResult, StateSpec,
    };
    use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
    use mfm_runtime::{
        ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding, ErasedRunnerFuture,
        ErasedRunnerOutput, StagedRetentionRefs,
    };
    use mfm_store::v1::{AsyncTypedRunEventStore, TypedRunEventStore};
    use serde::{Deserialize, Serialize};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;
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

    #[test]
    fn certified_spec_bundle_parser_rejects_bare_spec_json() {
        let err = parse_certified_spec_bundle_json_bytes(br#"{"spec_version":"mfm.typed.v1"}"#)
            .expect_err("bare spec is not a transport bundle");

        assert_eq!(err.code, "TypedBundleInvalid");
    }

    #[test]
    fn certified_spec_bundle_parser_returns_untrusted_canonical_bytes() {
        let bundle = parse_certified_spec_bundle_json_bytes(
            br#"{
                "kind": "certified_typed_spec_bundle_v1",
                "spec": {"b": 2, "a": 1},
                "certificate": {}
            }"#,
        )
        .expect("bundle transport parses");

        assert_eq!(bundle.spec_bytes(), br#"{"a":1,"b":2}"#);
        assert_eq!(bundle.certificate_bytes(), br#"{}"#);
    }

    #[tokio::test]
    async fn public_output_renders_json_from_cells_without_rendered_artifact_id() {
        let root =
            std::env::temp_dir().join(format!("mfm-app-public-output-{}", uuid::Uuid::new_v4()));
        let artifacts = FsTypedArtifactStore::new(&root);
        let source_node_id = node_id(0x20);
        let source_schema_id = schema_id("mfm.test.position", 0x21);
        let semantic_type_id = semantic_id("position", 0x22);
        let bytes = br#"{"total":"12.50"}"#.to_vec();

        let evidence = artifacts
            .put_artifact(
                bytes,
                TypedArtifactDescriptor {
                    media_type: spec::MediaType::new("application/json").expect("media type"),
                    schema_id: Some(source_schema_id.clone()),
                    semantic_type_id: Some(semantic_type_id.clone()),
                    producer_node_id: Some(source_node_id.clone()),
                    producer_seed_id: None::<SeedId>,
                    artifact_role: events::ArtifactRole::StateOutput,
                },
            )
            .await
            .expect("persist typed artifact");
        let public_schema_id = schema_id("mfm.test.public_output", 0x26);
        let cell = events::NamedTypedCellRef {
            public_field_path: spec::PublicFieldPath::new("result").expect("field path"),
            cell_id: cell_id(0x23),
            producer: spec::CellProducer::Node(source_node_id),
            scope_id: scope_id(0x24),
            semantic_type_id,
            schema_id: source_schema_id,
            value_lineage: spec::ValueLineageRef {
                lineage_digest: content_digest(0x25),
            },
            content_digest: evidence.digest,
            artifact_id: evidence.artifact_id,
        };
        let rendered_digest = content_digest(0x27);
        let authority = PublicOutputReadAuthority {
            run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0x28)),
            public_schema_id: public_schema_id.clone(),
            event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0x29)),
            rendered_digest,
            rendered_artifact_id: None,
            payload: events::PublicOutputProduced {
                spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0x2a)),
                node_id: node_id(0x2b),
                attempt_id: attempt_id(0x2c),
                receipt_cell_id: cell_id(0x2d),
                public_schema_id,
                output_spec_digest: content_digest(0x2e),
                cells: vec![cell],
                rendered_digest: content_digest(0x27),
                rendered_artifact_id: None,
                renderer_descriptor_id: descriptor_id(0x2f),
            },
        };

        let rendered = render_typed_public_output(&artifacts, &authority)
            .await
            .expect("render public output");

        assert_eq!(
            rendered.json,
            Some(serde_json::json!({
                "result": {
                    "total": "12.50"
                }
            }))
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn public_output_api_renders_from_authoritative_stream_cells() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load stream");
        let runtime_spec = load_runtime_spec_for_run(
            services.artifacts(),
            services.certification_registry(),
            &fixture.run_id,
            &stream,
        )
        .await
        .expect("runtime spec");
        let authority = verify_public_output_read_authority(
            &runtime_spec,
            &fixture.run_id,
            &fixture.public_schema_id,
            &stream,
        )
        .expect("public-output read authority");
        let response = render_typed_public_output(services.artifacts(), &authority)
            .await
            .expect("render public output from authority");

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
        let registry =
            CertificationRegistry::from_program_draft(&fixture.draft).expect("fixture registry");
        let corrupt_services = make_async_typed_services_with_certification_registry(
            production_typed_runner_registry(artifacts.clone())
                .expect("production runner registry"),
            StaticAsyncStore {
                run_id: fixture.run_id.clone(),
                stream: corrupt_stream,
            },
            artifacts,
            registry,
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
    async fn public_output_read_authority_rejects_tampered_stream_projection() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let valid_stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load valid stream");
        let runtime_spec = load_runtime_spec_for_run(
            services.artifacts(),
            services.certification_registry(),
            &fixture.run_id,
            &valid_stream,
        )
        .await
        .expect("runtime spec");
        let corrupt_stream =
            corrupt_public_output_history(services.artifacts(), &valid_stream).await;

        let err = verify_public_output_read_authority(
            &runtime_spec,
            &fixture.run_id,
            &fixture.public_schema_id,
            &corrupt_stream,
        )
        .expect_err("tampered stream rejects before render authority");

        assert_eq!(err.code, "TypedRuntimeError");
        assert!(err.message.contains("public-output payload"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn completed_run_without_public_output_evidence_rejects_render_authority() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let valid_stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load valid stream");
        let runtime_spec = load_runtime_spec_for_run(
            services.artifacts(),
            services.certification_registry(),
            &fixture.run_id,
            &valid_stream,
        )
        .await
        .expect("runtime spec");
        let stream_without_public_output = valid_stream
            .iter()
            .filter(|event| {
                !matches!(
                    event.payload(),
                    events::KernelEventPayload::PublicOutputProduced(_)
                )
            })
            .cloned()
            .collect::<Vec<_>>();

        let err = verify_public_output_read_authority(
            &runtime_spec,
            &fixture.run_id,
            &fixture.public_schema_id,
            &stream_without_public_output,
        )
        .expect_err("completed run without public output evidence rejects");

        assert!(matches!(
            err.code.as_str(),
            "TypedRuntimeError" | "TypedStoreRejected"
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_rejects_invalid_bundle_before_run_started() {
        let root =
            std::env::temp_dir().join(format!("mfm-app-invalid-bundle-{}", uuid::Uuid::new_v4()));
        let artifacts = FsTypedArtifactStore::new(&root);
        let fixture = framework_seed_public_output_fixture();
        let bundle = fixture.certified_spec.bundle().expect("fixture bundle");
        let mut bad_spec = bundle.spec_bytes().to_vec();
        let last = bad_spec.last_mut().expect("non-empty spec bytes");
        *last = if *last == b'}' { b']' } else { b'}' };
        let registry =
            CertificationRegistry::from_program_draft(&fixture.draft).expect("fixture registry");
        let run_id = fixture.run_id.clone();
        let err = verify_certified_bundle_run_start_request(
            &artifacts,
            UntrustedCertifiedSpecBundleStartInput {
                spec_bytes: &bad_spec,
                certificate_bytes: bundle.certificate_bytes(),
                registry: &registry,
                run_id: run_id.clone(),
                framework_version: "mfm.test.framework",
                source_revision: "test-source",
                drive: DriveMode::AppendOnly,
            },
            Vec::new(),
        )
        .await
        .expect_err("invalid bundle must not build a start request");
        assert_eq!(err.code, "TypedCertificationFailed");

        let services = make_async_typed_services_with_certification_registry(
            ErasedRunnerRegistry::new(),
            AsyncInMemoryStore::default(),
            artifacts,
            registry,
        );
        let stream = services
            .store()
            .load_run_stream(&run_id)
            .await
            .expect("load stream");
        assert!(!stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunStarted(_))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn app_rejects_certifier_invalid_runtime_shape_valid_bundle_before_run_started() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-certifier-invalid-bundle-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let fixture = framework_seed_public_output_fixture();
        let mut invalid_spec = fixture.certified_spec.spec().clone();
        invalid_spec.config_refs.clear();
        let spec_bytes = invalid_spec
            .canonical_json()
            .expect("invalid spec remains parseable")
            .to_vec();
        let mut evidence = fixture.certified_spec.certificate().evidence.clone();
        evidence.spec_hash = invalid_spec.spec_hash().expect("invalid spec hash");
        let certificate =
            mfm_certify::CertifiedSpecCertificate::from_evidence(evidence).expect("certificate");
        let certificate_bytes = certificate
            .canonical_json()
            .expect("certificate json")
            .to_vec();
        let registry =
            CertificationRegistry::from_program_draft(&fixture.draft).expect("fixture registry");
        let run_id = fixture.run_id.clone();
        let err = verify_certified_bundle_run_start_request(
            &artifacts,
            UntrustedCertifiedSpecBundleStartInput {
                spec_bytes: &spec_bytes,
                certificate_bytes: &certificate_bytes,
                registry: &registry,
                run_id: run_id.clone(),
                framework_version: "mfm.test.framework",
                source_revision: "test-source",
                drive: DriveMode::AppendOnly,
            },
            Vec::new(),
        )
        .await
        .expect_err("certifier-invalid bundle must not build a start request");
        assert_eq!(err.code, "TypedCertificationFailed");

        let services = make_async_typed_services_with_certification_registry(
            ErasedRunnerRegistry::new(),
            AsyncInMemoryStore::default(),
            artifacts,
            registry,
        );
        let stream = services
            .store()
            .load_run_stream(&run_id)
            .await
            .expect("load stream");
        assert!(!stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunStarted(_))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn tampered_stored_spec_artifact_rejects_before_resume() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load stream");
        let started = run_started_payload(&fixture.run_id, &stream)
            .expect("run started")
            .clone();
        std::fs::write(artifact_blob_path(&root, &started.spec_artifact_id), b"{}")
            .expect("tamper spec artifact");

        let err = services
            .resume_stored_run(&fixture.run_id, DriveMode::AppendOnly)
            .await
            .expect_err("tampered spec rejects before resume");
        assert_eq!(err.code, "TypedArtifactError");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn tampered_stored_certificate_artifact_rejects_before_resume() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load stream");
        let started = run_started_payload(&fixture.run_id, &stream)
            .expect("run started")
            .clone();
        std::fs::write(
            artifact_blob_path(&root, &started.certificate_artifact_id),
            b"{}",
        )
        .expect("tamper certificate artifact");

        let err = services
            .resume_stored_run(&fixture.run_id, DriveMode::AppendOnly)
            .await
            .expect_err("tampered certificate rejects before resume");
        assert_eq!(err.code, "TypedArtifactError");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn sync_resume_rebuilds_stored_authority_before_runtime_resume() {
        let (root, fixture, services, _started) = start_sync_framework_fixture_run().await;
        let stream = {
            let store = services.store();
            let store = store.lock().await;
            store.load_run_stream(&fixture.run_id)
        };
        let started = run_started_payload(&fixture.run_id, &stream)
            .expect("run started")
            .clone();
        std::fs::write(
            artifact_blob_path(&root, &started.certificate_artifact_id),
            b"{}",
        )
        .expect("tamper certificate artifact");

        let err = services
            .resume_stored_run(&fixture.run_id, DriveMode::AppendOnly)
            .await
            .expect_err("sync resume rejects tampered stored certificate");
        assert_eq!(err.code, "TypedArtifactError");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn registry_mismatch_rejects_before_replay_broker_construction() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load stream");
        let mismatched_services = make_async_typed_services_with_certification_registry(
            ErasedRunnerRegistry::new(),
            StaticAsyncStore {
                run_id: fixture.run_id.clone(),
                stream,
            },
            services.artifacts().clone(),
            CertificationRegistry::new(),
        );

        let err = mismatched_services
            .verify_replay_for_run(&fixture.run_id)
            .await
            .expect_err("registry mismatch rejects before replay");
        assert_eq!(err.code, "TypedCertificationFailed");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn sync_replay_broker_rebuilds_stored_authority_before_construction() {
        let (root, fixture, services, _started) = start_sync_framework_fixture_run().await;
        let stream = {
            let store = services.store();
            let store = store.lock().await;
            store.load_run_stream(&fixture.run_id)
        };
        let started = run_started_payload(&fixture.run_id, &stream)
            .expect("run started")
            .clone();
        std::fs::write(
            artifact_blob_path(&root, &started.certificate_artifact_id),
            b"{}",
        )
        .expect("tamper certificate artifact");

        let err = services
            .replay_broker(&fixture.run_id)
            .await
            .expect_err("sync replay broker rejects tampered stored certificate");
        assert_eq!(err.code, "TypedArtifactError");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn public_output_rejects_tampered_stored_authority_before_rendering() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load stream");
        let started = run_started_payload(&fixture.run_id, &stream)
            .expect("run started")
            .clone();
        std::fs::write(
            artifact_blob_path(&root, &started.certificate_artifact_id),
            b"{}",
        )
        .expect("tamper certificate artifact");

        let err = services
            .typed_public_output(&fixture.run_id, &fixture.public_schema_id)
            .await
            .expect_err("tampered authority rejects before rendering");
        assert_eq!(err.code, "TypedArtifactError");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rendered_json_cannot_authorize_resume_or_replay() {
        let (root, fixture, services, _started) = start_framework_fixture_run().await;
        let rendered = services
            .typed_public_output(&fixture.run_id, &fixture.public_schema_id)
            .await
            .expect("render public output before tamper");
        assert!(rendered.json.is_some());

        let stream = services
            .store()
            .load_run_stream(&fixture.run_id)
            .await
            .expect("load stream");
        let started = run_started_payload(&fixture.run_id, &stream)
            .expect("run started")
            .clone();
        std::fs::write(
            artifact_blob_path(&root, &started.certificate_artifact_id),
            b"{}",
        )
        .expect("tamper certificate artifact");

        let resume_err = services
            .resume_stored_run(&fixture.run_id, DriveMode::AppendOnly)
            .await
            .expect_err("rendered JSON must not authorize resume");
        assert_eq!(resume_err.code, "TypedArtifactError");

        let replay_err = services
            .verify_replay_for_run(&fixture.run_id)
            .await
            .expect_err("rendered JSON must not authorize replay");
        assert_eq!(replay_err.code, "TypedArtifactError");
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
        persist_draft_config_artifacts(&artifacts, &fixture.draft).await;
        persist_framework_config_artifacts(&artifacts, &fixture.certified_spec.envelope().spec)
            .await;
        let request = build_certified_typed_run_start_request(
            &artifacts,
            fixture.certified_spec.clone(),
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
            production_typed_runner_registry(artifacts.clone())
                .expect("production runner registry"),
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

    #[tokio::test]
    async fn production_typed_runner_registry_executes_certified_proof_run() {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-production-proof-run-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let proof_config = mfm_op_proof::ProofWorkflowConfig::default();
        let draft = mfm_op_proof::proof_program_draft(proof_config.clone()).expect("proof draft");
        persist_draft_config_artifacts(&artifacts, &draft).await;
        let certified = mfm_op_proof::certified_proof_spec(proof_config).expect("proof spec");
        persist_framework_config_artifacts(&artifacts, &certified.envelope().spec).await;
        let bundle = certified.bundle().expect("proof bundle");
        let registry = production_certification_registry().expect("production registry");
        let request = verify_certified_bundle_run_start_request(
            &artifacts,
            UntrustedCertifiedSpecBundleStartInput {
                spec_bytes: bundle.spec_bytes(),
                certificate_bytes: bundle.certificate_bytes(),
                registry: &registry,
                run_id: new_run_id(),
                framework_version: "mfm.test.proof",
                source_revision: "test-source",
                drive: DriveMode::UntilBlocked,
            },
            Vec::new(),
        )
        .await
        .expect("typed proof run request");
        let services = make_async_typed_services_with_certification_registry(
            production_typed_runner_registry(artifacts.clone())
                .expect("production runner registry"),
            AsyncInMemoryStore::default(),
            artifacts.clone(),
            registry,
        );

        let response = services
            .start_certified_run(request)
            .await
            .expect("start proof run");

        assert_eq!(response.phase, TypedRunPhase::Completed);
        let replay = services
            .verify_replay_for_run(&RunId::parse(&response.run_id).expect("typed run id"))
            .await
            .expect("verify proof replay");
        assert_eq!(replay.phase, TypedRunPhase::Completed);
        let public_output = services
            .typed_public_output(
                &RunId::parse(&response.run_id).expect("typed run id"),
                &certified.envelope().spec.public_outputs.public_schema_id,
            )
            .await
            .expect("render proof public output");
        let public_output_json = public_output.json.expect("rendered proof json");
        assert_eq!(public_output_json["output"]["fact"]["n"], 1);
        assert_eq!(
            public_output_json["output"]["side_effect"]["status"],
            "confirmed"
        );
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
        persist_draft_config_artifacts(&artifacts, &fixture.draft).await;
        persist_framework_config_artifacts(&artifacts, &fixture.certified_spec.envelope().spec)
            .await;
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
        let request = build_certified_typed_run_start_request(
            &artifacts,
            fixture.certified_spec.clone(),
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
        let runners = framework_fixture_runner_registry(&fixture);
        let registry =
            CertificationRegistry::from_program_draft(&fixture.draft).expect("fixture registry");
        let services = make_async_typed_services_with_certification_registry(
            runners,
            AsyncInMemoryStore::default(),
            artifacts.clone(),
            registry,
        );

        let started = services
            .start_certified_run(request)
            .await
            .expect("start typed run");
        (root, fixture, services, started)
    }

    async fn start_sync_framework_fixture_run() -> (
        PathBuf,
        FrameworkSeedPublicOutputFixture,
        TypedAppServices<store::InMemoryTypedRunStore>,
        TypedRunResponse,
    ) {
        let root = std::env::temp_dir().join(format!(
            "mfm-app-sync-framework-run-{}",
            uuid::Uuid::new_v4()
        ));
        let artifacts = FsTypedArtifactStore::new(&root);
        let fixture = framework_seed_public_output_fixture();
        persist_draft_config_artifacts(&artifacts, &fixture.draft).await;
        persist_framework_config_artifacts(&artifacts, &fixture.certified_spec.envelope().spec)
            .await;
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
        let request = build_certified_typed_run_start_request(
            &artifacts,
            fixture.certified_spec.clone(),
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
        let runners = framework_fixture_runner_registry(&fixture);
        let registry =
            CertificationRegistry::from_program_draft(&fixture.draft).expect("fixture registry");
        let services =
            make_in_memory_typed_services_with_certification_registry(runners, &root, registry);

        let started = services
            .start_certified_run(request)
            .await
            .expect("start typed run");
        (root, fixture, services, started)
    }

    fn framework_fixture_runner_registry(
        fixture: &FrameworkSeedPublicOutputFixture,
    ) -> ErasedRunnerRegistry {
        let mut runners = ErasedRunnerRegistry::new();
        let factory_id = fixture.value_runner_factory_id.clone();
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
        runners
    }

    async fn persist_draft_config_artifacts(
        artifacts: &FsTypedArtifactStore,
        draft: &mfm_program::TypedProgramDraft,
    ) {
        for config in draft
            .state_nodes()
            .iter()
            .map(|node| &node.config)
            .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        {
            artifacts
                .put_artifact(
                    config.canonical_json.to_vec(),
                    TypedArtifactDescriptor {
                        media_type: spec::MediaType::new("application/json").expect("media type"),
                        schema_id: Some(config.schema_id.clone()),
                        semantic_type_id: None,
                        producer_node_id: None,
                        producer_seed_id: None,
                        artifact_role: events::ArtifactRole::TypedConfig,
                    },
                )
                .await
                .expect("persist draft config artifact");
        }
    }

    async fn persist_framework_config_artifacts(
        artifacts: &FsTypedArtifactStore,
        typed_spec: &spec::TypedExecutionSpec,
    ) {
        for node in &typed_spec.nodes {
            let Some(framework) = &node.framework else {
                continue;
            };
            let framework_kind = match framework {
                spec::FrameworkNodeSpec::Bridge(_) => "bridge_same_value",
                spec::FrameworkNodeSpec::PublicOutputRender(_) => "public_output_render",
            };
            let payload = serde_json::json!({
                "framework": framework_kind,
                "node_id": node.node_id.as_str(),
            });
            let json = serde_json::to_string(&payload).expect("framework config json");
            let bytes = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
                .expect("canonical framework config");
            assert_eq!(
                bytes.content_digest(),
                node.config_ref.digest,
                "framework config helper must match certified config ref"
            );
            artifacts
                .put_artifact(
                    bytes.to_vec(),
                    TypedArtifactDescriptor {
                        media_type: node.config_ref.media_type.clone(),
                        schema_id: Some(node.config_ref.schema_id.clone()),
                        semantic_type_id: None,
                        producer_node_id: None,
                        producer_seed_id: None,
                        artifact_role: events::ArtifactRole::TypedConfig,
                    },
                )
                .await
                .expect("persist framework config artifact");
        }
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
            let admitted_artifacts = required_artifacts.clone();
            let contains_run_started = payloads
                .iter()
                .any(|payload| matches!(payload, events::KernelEventPayload::RunStarted(_)));
            let request = store::TypedCommitRequest {
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
            };
            let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)
                .expect("prepare corrupt-history test commit");
            corrupt_store
                .append_prepared_typed_commit(commit)
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
                    artifact_ids.insert(started.certificate_artifact_id.clone());
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

    fn artifact_blob_path(root: &Path, artifact_id: &ArtifactId) -> PathBuf {
        let digest = artifact_id.digest().to_string();
        root.join("typed")
            .join("blobs")
            .join(&digest[0..2])
            .join(artifact_id.as_str())
    }

    struct StaticAsyncStore {
        run_id: RunId,
        stream: Vec<store::KernelEventEnvelope>,
    }

    impl store::AsyncTypedRunEventStore for StaticAsyncStore {
        type Error = store::StoreError;

        fn append_prepared_typed_commit<'a>(
            &'a self,
            _commit: store::PreparedTypedCommit,
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

        fn append_prepared_typed_commit<'a>(
            &'a self,
            commit: store::PreparedTypedCommit,
        ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
            let result = self
                .0
                .lock()
                .expect("store lock")
                .append_prepared_typed_commit(commit);
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
        draft: mfm_program::TypedProgramDraft,
        certified_spec: CertifiedTypedSpec,
        run_id: RunId,
        seed_id: SeedId,
        seed_bytes: Vec<u8>,
        output_bytes: Vec<u8>,
        value_schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        value_node_id: NodeId,
        value_descriptor_id: DescriptorId,
        value_runner_factory_id: events::RunnerFactoryId,
        public_schema_id: SchemaId,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
    #[mfm(
        namespace = "mfm.app.test",
        name = "framework_input",
        version = "1",
        schema = "mfm.app.test.framework_input"
    )]
    struct FrameworkInput {
        input: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
    #[mfm(
        namespace = "mfm.app.test",
        name = "framework_output",
        version = "1",
        schema = "mfm.app.test.framework_output"
    )]
    struct FrameworkOutput {
        total: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct FrameworkConfig {
        version: u64,
    }

    #[derive(PublicOutputs)]
    #[mfm(schema = "mfm.app.test.framework_public")]
    struct FrameworkPublicOutputs<'program, 'scope> {
        result: mfm_program::Handle<'program, 'scope, FrameworkOutput>,
    }

    struct FrameworkValueState {
        _config: FrameworkConfig,
    }

    impl StateSpec for FrameworkValueState {
        type Config = FrameworkConfig;
        type Input = FrameworkInput;
        type Output = FrameworkOutput;
        type Effect = Pure;
        type Caps = NoCaps;

        fn kind() -> mfm_program::Result<StateKind> {
            StateKind::new(
                "mfm.app.test",
                "framework_value",
                DigestAlgorithm::Sha256JcsV1,
                digest(0xbf),
            )
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
        }

        fn version() -> mfm_program::Result<StateVersion> {
            StateVersion::new("mfm.app.test.framework_value.v1")
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.app.test.framework_value"
        }

        fn new(config: Self::Config) -> mfm_program::Result<Self> {
            Ok(Self { _config: config })
        }
    }

    impl PureState for FrameworkValueState {
        fn run(&self, _input: Self::Input) -> StateResult<Self::Output> {
            Ok(FrameworkOutput {
                total: "12.50".to_owned(),
            })
        }
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
        let seed = CanonicalSeed::from_value(&FrameworkInput {
            input: "start".to_owned(),
        })
        .expect("canonical seed");
        let seed_bytes = seed.canonical_json().to_vec();
        let output_bytes = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&FrameworkOutput {
                total: "12.50".to_owned(),
            })
            .expect("output json"),
        )
        .expect("canonical output")
        .to_vec();
        let mut states = StateRegistryBuilder::new();
        states
            .register::<FrameworkValueState>()
            .expect("state registration");
        let draft = build_root_with_registries(
            ScopeKey::new("root").expect("root key"),
            states.snapshot(),
            mfm_program::OperationRegistryBuilder::new().snapshot(),
            |root: &mut RootBuilder<'_, '_>| {
                let seed = root.seed(SeedKey::new("launch")?, seed.clone())?;
                let result = root.scope().state::<FrameworkValueState, _>(
                    StateKey::new("value")?,
                    FrameworkConfig { version: 1 },
                    seed,
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("public-output")?,
                    &FrameworkPublicOutputs { result },
                )
            },
        )
        .expect("program draft");
        let certified_spec = mfm_certify::certify_program_draft(&draft).expect("certified spec");
        let spec = &certified_spec.envelope().spec;
        let public_output_cell = spec
            .public_outputs
            .outputs
            .first()
            .expect("public output cell");
        let value_node_id = match &public_output_cell.producer {
            spec::CellProducer::Node(node_id) => node_id.clone(),
            spec::CellProducer::Seed(_) => panic!("public output must be node-produced"),
        };
        let value_node = spec
            .nodes
            .iter()
            .find(|node| node.node_id == value_node_id)
            .expect("value node");
        let seed_id = spec.seeds.first().expect("seed").seed_id.clone();
        let value_schema_id = public_output_cell.schema_id.clone();
        let semantic_type_id = public_output_cell.semantic_type_id.clone();
        let value_descriptor_id = value_node.descriptor_id.clone();
        let value_runner_factory_id = spec
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(identity)
                    if identity.descriptor_id == value_descriptor_id =>
                {
                    Some(events::RunnerFactoryId::new(&identity.runner).expect("runner factory"))
                }
                _ => None,
            })
            .expect("value descriptor identity");
        let public_schema_id = spec.public_outputs.public_schema_id.clone();

        FrameworkSeedPublicOutputFixture {
            draft,
            certified_spec,
            run_id,
            seed_id,
            seed_bytes,
            output_bytes,
            value_schema_id,
            semantic_type_id,
            value_node_id,
            value_descriptor_id,
            value_runner_factory_id,
            public_schema_id,
        }
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
}
