#![warn(missing_docs)]
//! Typed application assembly for certified MFM runs.
//!
//! `mfm-app` is the typed boundary used by binaries and process adapters. It does not plan old
//! dynamic DAGs, own workflow semantics, or expose `mfm-machine`/`mfm-sdk` execution authority.
//! Callers supply a certified typed spec, a runner registry, staged launch material, and a typed
//! run store; this crate wires those parts into start, resume, replay, and public-output rendering
//! helpers.
//!
//! # Examples
//!
//! ```rust
//! use mfm_app::{make_async_typed_services, ErasedRunnerRegistry};
//! use mfm_artifact_store_fs::FsTypedArtifactStore;
//! use mfm_store::v1::AsyncInMemoryTypedRunStore;
//!
//! let runners = ErasedRunnerRegistry::new();
//! let artifacts = FsTypedArtifactStore::new("/tmp/mfm-typed-artifacts");
//! let store = AsyncInMemoryTypedRunStore::default();
//! let _services = make_async_typed_services(runners, store, artifacts);
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use mfm_artifact_store_fs::{FsTypedArtifactError, FsTypedArtifactStore};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_certify::{CertificationRegistry, CertifiedSpecBundle, CertifiedTypedSpec};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, EventId, RunId, SchemaId, SeedId, SemanticTypeId,
    SpecHash,
};
use mfm_replay::v1::{ReplayBroker, ReplayError, ReplayReadAuthority};
use mfm_runtime::{
    CertifiedRuntimeSpec, ManualResolutionEvidenceArtifact, ManualResolutionRequest,
    RunLaunchArtifact, RunLaunchEvidence, RunLaunchSeedCell, RuntimeArtifactStageFuture,
    RuntimeArtifactStager, SchedulerStatus, SerialTypedScheduler, VerifiedRunHistoryView,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use serde::{Deserialize, Serialize};
use serde_json::map::Entry;
use serde_json::{Map, Value};

pub use mfm_runtime::ErasedRunnerRegistry;

mod entry_point;
mod entry_points;
mod evm_contracts;

pub use entry_point::{
    AuthoredConfig, CanonicalConfigMaterial, CanonicalSeedMaterial, CanonicalizerIdentity,
    ConfigFormat, EntryPointOpId, EntryPointOpPlan, EntryPointOpRegistry, EntryPointOpResolveError,
    LaunchableOp, LoweringIdentity, NormalizedAuthoredConfig, OpLaunchError, OpVersion,
    PublicOpName, DEFAULT_AUTHORED_CONFIG_FORMAT, DEFAULT_AUTHORED_CONFIG_MAX_BYTES,
};

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

/// Message text that has been selected for public CLI/REST/app surfaces.
///
/// This type marks the boundary where lower-level diagnostics are either intentionally exposed as
/// caller input validation messages or replaced by fixed safe text. It does not authorize callers
/// to forward backend `Display` output from storage, runtime, transport, signer, or provider
/// errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicSafeMessage(String);

impl PublicSafeMessage {
    /// Creates public-safe message text from an already reviewed string.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// Creates fixed public-safe text for a lower-level backend failure.
    pub fn backend(message: &'static str) -> Self {
        Self(message.to_owned())
    }

    /// Consumes the reviewed message into owned text for existing public response structs.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<&'static str> for PublicSafeMessage {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PublicSafeMessage {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl AppError {
    /// Creates an application error from the supplied classification, code, and message.
    pub fn new(
        class: ErrorClass,
        code: impl Into<String>,
        message: impl Into<PublicSafeMessage>,
    ) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into().into_string(),
        }
    }

    /// Creates an application error for a lower-level failure without exposing backend details.
    pub fn backend(class: ErrorClass, code: impl Into<String>, message: &'static str) -> Self {
        Self::new(class, code, PublicSafeMessage::backend(message))
    }

    /// Returns a not-found error with an explicit code and message.
    pub fn not_found(code: impl Into<String>, message: impl Into<PublicSafeMessage>) -> Self {
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
            mfm_runtime::RuntimeError::Store(_) => Self::backend(
                ErrorClass::Conflict,
                "RunStoreRejected",
                "Run store rejected the requested operation",
            ),
            mfm_runtime::RuntimeError::RunnerBinding(_) => Self::backend(
                ErrorClass::BadRequest,
                "LaunchRunnerUnavailable",
                "A required typed runner is unavailable",
            ),
            mfm_runtime::RuntimeError::SpecHash(_)
            | mfm_runtime::RuntimeError::InvalidSpec(_)
            | mfm_runtime::RuntimeError::InvalidRunStream(_)
            | mfm_runtime::RuntimeError::Blocked(_)
            | mfm_runtime::RuntimeError::InputMaterialization(_)
            | mfm_runtime::RuntimeError::InvalidRunnerOutput(_)
            | mfm_runtime::RuntimeError::RuntimeValidation(_)
            | mfm_runtime::RuntimeError::Identity(_)
            | mfm_runtime::RuntimeError::Canonical(_) => Self::backend(
                ErrorClass::Internal,
                "LaunchRuntimeError",
                "Typed runtime rejected the requested operation",
            ),
        }
    }
}

impl From<store::StoreError> for AppError {
    fn from(_error: store::StoreError) -> Self {
        Self::backend(
            ErrorClass::Conflict,
            "RunStoreRejected",
            "Run store rejected the requested operation",
        )
    }
}

impl From<FsTypedArtifactError> for AppError {
    fn from(error: FsTypedArtifactError) -> Self {
        match error {
            FsTypedArtifactError::NotFound { artifact_id } => Self::not_found(
                "ArtifactNotFound",
                format!("typed artifact {artifact_id} was not found"),
            ),
            FsTypedArtifactError::RetainedArtifactRefused { artifact_id } => Self::new(
                ErrorClass::Conflict,
                "ArtifactRetained",
                format!("typed artifact {artifact_id} is retained"),
            ),
            FsTypedArtifactError::Corruption { .. }
            | FsTypedArtifactError::EvidenceMismatch { .. }
            | FsTypedArtifactError::InvalidEvidence { .. }
            | FsTypedArtifactError::InvalidIdentity { .. }
            | FsTypedArtifactError::Io { .. } => Self::backend(
                ErrorClass::Internal,
                "ArtifactError",
                "Typed artifact store rejected the requested operation",
            ),
        }
    }
}

impl From<ReplayError> for AppError {
    fn from(error: ReplayError) -> Self {
        Self::backend(
            ErrorClass::Internal,
            error.code(),
            "Replay verification failed",
        )
    }
}

impl From<mfm_spec::SpecError> for AppError {
    fn from(_error: mfm_spec::SpecError) -> Self {
        Self::backend(
            ErrorClass::Internal,
            "CertifiedSpecInvalid",
            "Certified typed spec is invalid",
        )
    }
}

impl From<mfm_certify::CertifyError> for AppError {
    fn from(_error: mfm_certify::CertifyError) -> Self {
        Self::backend(
            ErrorClass::BadRequest,
            "CertifiedBundleVerificationFailed",
            "Certified typed spec bundle verification failed",
        )
    }
}

impl From<EntryPointOpResolveError> for AppError {
    fn from(error: EntryPointOpResolveError) -> Self {
        Self::new(
            ErrorClass::BadRequest,
            error.code().to_owned(),
            error.message().to_owned(),
        )
    }
}

impl From<OpLaunchError> for AppError {
    fn from(error: OpLaunchError) -> Self {
        Self::new(
            ErrorClass::BadRequest,
            error.code().to_owned(),
            error.message().to_owned(),
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

/// Builds typed app services backed by a durable async typed run event store.
pub fn make_async_typed_services<S>(
    runners: ErasedRunnerRegistry,
    store: S,
    artifacts: FsTypedArtifactStore,
) -> RunServices<S>
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
) -> RunServices<S>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
    let artifact_stager = Arc::new(FsRuntimeArtifactStager {
        artifacts: artifacts.clone(),
    });
    RunServices::new_with_certification_registry(
        SerialTypedScheduler::new(runners, artifact_stager),
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
    let portfolio_artifacts: Arc<dyn mfm_artifact_capabilities::ArtifactReadProvider> =
        Arc::new(artifacts.clone());
    let portfolio_evm: Arc<dyn mfm_adapters_portfolio::PortfolioEvmProvider> =
        match mfm_transports_evm::EvmJsonRpcClient::from_env() {
            Ok(client) => Arc::new(client),
            Err(_) => Arc::new(mfm_adapters_portfolio::UnavailablePortfolioEvmProvider),
        };
    let portfolio_capabilities = mfm_adapters_portfolio::PortfolioRunnerCapabilities::new(
        portfolio_artifacts,
        portfolio_evm,
    );
    mfm_adapters_portfolio::register_portfolio_runners(&mut registry, portfolio_capabilities)?;
    evm_contracts::register_contract_lifecycle_runners(&mut registry, artifacts)?;
    mfm_transports_proof::register_deterministic_proof_runners(&mut registry)?;
    Ok(registry)
}

/// Builds the trusted production certification registry for bundled spec verification.
pub fn production_certification_registry() -> Result<CertificationRegistry, AppError> {
    let mut registry = CertificationRegistry::new();
    mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
        &mut registry,
    )?;
    mfm_op_portfolio_tracker::register_portfolio_certification_descriptors(&mut registry)?;
    mfm_op_proof::register_proof_certification_descriptors(&mut registry)?;
    Ok(registry)
}

/// Builds the production entry-point operation registry for this process.
pub fn production_entry_point_op_registry() -> Result<EntryPointOpRegistry, AppError> {
    entry_points::production_entry_point_op_registry()
}

#[derive(Clone)]
struct FsRuntimeArtifactStager {
    artifacts: FsTypedArtifactStore,
}

impl RuntimeArtifactStager for FsRuntimeArtifactStager {
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a> {
        Box::pin(async move {
            self.artifacts
                .put_verified_artifact(bytes, evidence)
                .await
                .map_err(|error| mfm_runtime::RuntimeError::Store(error.to_string()))?;
            Ok(())
        })
    }
}

impl store::RetainedArtifactReadProvider for FsRuntimeArtifactStager {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        self.artifacts.read_retained_artifact(requirement)
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

/// Whether typed start/resume should run scheduler steps after appending `RunAdmitted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveMode {
    /// Append only the requested lifecycle event.
    AppendOnly,
    /// Drive at most one scheduler step, preserving a restart boundary after each durable commit.
    Once,
    /// Drive deterministic runnable nodes until the scheduler blocks or the run completes.
    UntilBlocked,
}

/// Operator-selected manual resolution decision accepted by app ingress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManualResolutionDecision {
    /// Confirm that unresolved obligations were remediated externally.
    ConfirmRemediated,
    /// Close the run without a compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
}

impl ManualResolutionDecision {
    fn into_event(self) -> events::ManualResolutionOutcome {
        match self {
            Self::ConfirmRemediated => events::ManualResolutionOutcome::ConfirmRemediated,
            Self::FailWithoutAcdcClaim => events::ManualResolutionOutcome::FailWithoutAcdcClaim,
        }
    }
}

impl fmt::Display for ManualResolutionDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.into_event().as_str())
    }
}

/// Request to start a certified typed run.
#[derive(Debug, Clone)]
pub struct RunLaunchRequest {
    /// Certifier-backed typed spec authority.
    pub certified_spec: CertifiedTypedSpec,
    /// Store-owned run id to bind.
    pub run_id: RunId,
    /// Launch material that runtime middleware stages and admits with the admission commit.
    pub evidence: RunLaunchEvidence,
    /// Scheduler drive policy after start.
    pub drive: DriveMode,
}

/// Request to prepare an entry-point op launch.
pub struct EntryPointRunLaunchInput<'a> {
    /// Registry containing public entry-point operation registrations.
    pub entry_point_registry: &'a EntryPointOpRegistry,
    /// Public operation name submitted by the caller.
    pub public_op_name: PublicOpName,
    /// Optional explicit public operation version.
    pub op_version: Option<OpVersion>,
    /// Authored operation config submitted by the caller.
    pub authored_config: AuthoredConfig,
    /// Trusted certification registry used to certify the planned typed spec.
    pub certification_registry: &'a CertificationRegistry,
    /// Store-owned run id to bind.
    pub run_id: RunId,
    /// Framework version evidence to bind to the run start event.
    pub framework_version: &'a str,
    /// Source revision evidence to bind to the run start event.
    pub source_revision: &'a str,
    /// Caller-supplied launch time in Unix milliseconds.
    pub launched_at_unix_ms: u64,
    /// Scheduler drive policy after start.
    pub drive: DriveMode,
}

/// App-level evidence for a prepared entry-point op launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPointLaunchEvidence {
    /// Public operation name submitted by the caller.
    pub submitted_public_op_name: PublicOpName,
    /// Typed entry-point op id resolved from the registry.
    pub resolved_op_id: EntryPointOpId,
    /// Public operation version selected from the registry.
    pub resolved_op_version: OpVersion,
    /// Canonical digest of the entry-point registry used for resolution.
    pub entry_point_registry_digest: ContentDigest,
    /// Deterministic lowering identity used by the selected op.
    pub lowering_identity: LoweringIdentity,
    /// Config canonicalizer identity used by the selected op.
    pub canonicalizer_identity: CanonicalizerIdentity,
    /// Authored config format accepted by the selected op.
    pub config_format: ConfigFormat,
    /// Digest of the submitted authored config bytes.
    pub authored_config_digest: ContentDigest,
    /// Digest of the canonical op config bytes.
    pub canonical_config_digest: ContentDigest,
}

/// Prepared entry-point op launch material accepted by app runtime services.
#[derive(Debug, Clone)]
pub struct PreparedEntryPointRunLaunch {
    /// Runtime run launch request.
    pub request: RunLaunchRequest,
    /// Entry-point launch evidence produced by app assembly.
    pub evidence: EntryPointLaunchEvidence,
    /// Public output schema id exposed by the selected op, when available.
    pub public_output_schema_id: Option<SchemaId>,
}

/// Request to append a signed manual resolution for a manually blocked typed run.
#[derive(Debug, Clone)]
pub struct ManualResolutionRecordRequest {
    /// Store-owned run id to resolve.
    pub run_id: RunId,
    /// Authorized manual decision to record.
    pub outcome: ManualResolutionDecision,
    /// Operator evidence artifact bytes bound by the signed proof claim.
    pub evidence_bytes: Vec<u8>,
    /// Evidence artifact media type.
    pub evidence_media_type: String,
    /// Canonical signed manual authorization proof bytes.
    pub authorization_proof_bytes: Vec<u8>,
    /// Optional redaction-safe operator note.
    pub note: Option<String>,
    /// Scheduler drive policy after the manual-resolution append.
    pub drive: DriveMode,
}

/// Config bytes supplied to a typed run start request.
#[derive(Debug, Clone)]
pub struct RunLaunchConfigArtifact {
    /// Certified config schema id.
    pub schema_id: SchemaId,
    /// Canonical config artifact bytes.
    pub bytes: Vec<u8>,
    /// Config artifact media type.
    pub media_type: spec::MediaType,
}

/// Seed bytes supplied to a typed run start request.
#[derive(Debug, Clone)]
pub struct RunLaunchSeedArtifact {
    /// Seed id from the certified spec.
    pub seed_id: SeedId,
    /// Canonical seed value bytes.
    pub bytes: Vec<u8>,
    /// Persisted seed artifact media type.
    pub media_type: spec::MediaType,
}

/// Stable semantic run mode for typed run responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypedRunMode {
    /// Forward graph execution is active.
    Forward,
    /// Remediation graph execution is active.
    Remediating,
    /// The run is blocked for certified manual evidence.
    ManualBlocked,
    /// The run completed with public-output evidence.
    Completed,
    /// Confirmed forward obligations were compensated.
    Compensated,
    /// Certified manual evidence resolved the run.
    ManuallyResolved,
    /// The run ended without a compensation or AC/DC-equivalence claim.
    FailedWithoutAcdcClaim,
}

impl TypedRunMode {
    fn as_store_run_mode(self) -> store::RunMode {
        match self {
            Self::Forward => store::RunMode::Forward,
            Self::Remediating => store::RunMode::Remediating,
            Self::ManualBlocked => store::RunMode::ManualBlocked,
            Self::Completed => store::RunMode::Completed,
            Self::Compensated => store::RunMode::Compensated,
            Self::ManuallyResolved => store::RunMode::ManuallyResolved,
            Self::FailedWithoutAcdcClaim => store::RunMode::FailedWithoutAcdcClaim,
        }
    }
}

impl From<store::RunMode> for TypedRunMode {
    fn from(mode: store::RunMode) -> Self {
        match mode {
            store::RunMode::Forward => Self::Forward,
            store::RunMode::Remediating => Self::Remediating,
            store::RunMode::ManualBlocked => Self::ManualBlocked,
            store::RunMode::Completed => Self::Completed,
            store::RunMode::Compensated => Self::Compensated,
            store::RunMode::ManuallyResolved => Self::ManuallyResolved,
            store::RunMode::FailedWithoutAcdcClaim => Self::FailedWithoutAcdcClaim,
        }
    }
}

impl fmt::Display for TypedRunMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_store_run_mode().as_str())
    }
}

/// Public saga status derived from certified policy and stream evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedSagaStatus {
    /// Certified saga policy variant and manual authorization requirements.
    pub policy: TypedSagaPolicyStatus,
    /// Derived per-forward-ledger obligations.
    pub obligations: Vec<TypedSagaObligationStatus>,
    /// Resource claims and recorded evidence for all projected side-effect ledgers.
    pub resource_ledgers: Vec<TypedResourceLedgerStatus>,
    /// Active exclusive resource lane holders referenced by this run's live side-effect ledgers.
    pub resource_lanes: Vec<TypedResourceLaneHolderStatus>,
    /// Manual-block reason when the derived run mode is `manual_blocked`.
    pub manual_block_reason: Option<String>,
    /// Required manual authorization when an authorized decision can resolve the current block.
    pub required_manual_authorization: Option<TypedManualAuthorizationRequirements>,
    /// Terminal completion evidence, when the run has resolved.
    pub terminal_resolution: Option<TypedTerminalResolutionStatus>,
}

/// Public certified saga policy summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedSagaPolicyStatus {
    /// Certified policy variant.
    pub variant: String,
    /// Manual authorization requirements certified directly by the policy, when present.
    pub manual_authorization: Option<TypedManualAuthorizationRequirements>,
    /// Unresolved-remediation directive under compensating policy.
    pub on_remediation_unresolved: Option<String>,
}

/// Public manual authorization requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedManualAuthorizationRequirements {
    /// Schema id for the operator evidence artifact.
    pub evidence_schema_id: String,
    /// Manual authorization verifier id.
    pub verifier_id: String,
    /// Required manual signing scheme.
    pub signing_scheme: String,
    /// Certified operator authority id.
    pub authority_id: String,
    /// Allowed operator public identities from the certified authority snapshot.
    pub operator_public_identities: Vec<String>,
    /// Required number of operator signatures.
    pub quorum_required_signatures: u32,
}

/// Public obligation state for one forward ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedSagaObligationStatus {
    /// Forward ledger key.
    pub forward_ledger_key: String,
    /// Current forward side-effect phase.
    pub forward_phase: String,
    /// Derived forward classification.
    pub classification: String,
    /// Declared resource claim and recorded evidence for the forward ledger.
    pub resource: Option<TypedResourceLedgerStatus>,
    /// Linked remediation ledger, when one exists.
    pub remediation: Option<TypedRemediationLedgerStatus>,
}

/// Public remediation state linked to a forward ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedRemediationLedgerStatus {
    /// Remediation ledger key.
    pub ledger_key: String,
    /// Forward ledger key this remediation closes.
    pub forward_ledger_key: String,
    /// Current remediation side-effect phase.
    pub phase: String,
    /// Declared resource claim and recorded evidence for the remediation ledger.
    pub resource: Option<TypedResourceLedgerStatus>,
    /// Whether remediation confirmation closed the obligation.
    pub closed: bool,
    /// Unresolved reason if remediation cannot close the obligation.
    pub unresolved: Option<String>,
}

/// Public resource-claim and evidence status for one side-effect ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedResourceLedgerStatus {
    /// Side-effect ledger key.
    pub ledger_key: String,
    /// Ledger purpose.
    pub ledger_purpose: String,
    /// Linked forward ledger key when this is a remediation ledger.
    pub forward_ledger_key: Option<String>,
    /// Current projected side-effect phase.
    pub phase: String,
    /// Certified resource claim declared by the side-effect node.
    pub claim: TypedResourceClaimStatus,
    /// Recorded exclusive key evidence, when the claim is `exclusive` and preparation occurred.
    pub key: Option<TypedResourceKeyStatus>,
    /// Recorded exact touched-set evidence, when the claim is `exact_touched_set`.
    pub touched_set: Option<TypedResourceTouchedSetStatus>,
    /// Active lane holder when this ledger currently owns its exclusive lane.
    pub active_lane: Option<TypedResourceLaneHolderStatus>,
    /// Active holder of the same recorded key when this ledger is not the holder.
    pub blocked_by_lane: Option<TypedResourceLaneHolderStatus>,
}

/// Public certified resource claim summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedResourceClaimStatus {
    /// Claim variant: `exclusive`, `exact_touched_set`, or `manual_only`.
    pub kind: String,
    /// Resource namespace for `exclusive` and `exact_touched_set` claims.
    pub namespace: Option<String>,
    /// Key schema id for `exclusive` claims.
    pub key_schema_id: Option<String>,
    /// Evidence schema id for `exact_touched_set` claims.
    pub evidence_schema_id: Option<String>,
}

/// Public exclusive resource key evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedResourceKeyStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Key schema id.
    pub key_schema_id: String,
    /// Store-comparable resource key.
    pub key: String,
}

/// Public exact touched-set evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedResourceTouchedSetStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Evidence schema id.
    pub evidence_schema_id: String,
    /// Canonical touched-set evidence hash.
    pub evidence_hash: String,
    /// Touched-set evidence artifact id.
    pub evidence_artifact_id: String,
}

/// Public active exclusive resource lane holder referenced by the target run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedResourceLaneHolderStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Store-comparable resource key.
    pub key: String,
    /// Run id holding the lane.
    pub holding_run_id: String,
    /// Ledger key holding the lane.
    pub holding_ledger_key: String,
    /// Ledger purpose for the holder.
    pub holding_ledger_purpose: String,
    /// Forward ledger key when the holder is a remediation ledger.
    pub holding_forward_ledger_key: Option<String>,
    /// Node id that prepared the invocation.
    pub holding_node_id: String,
    /// Attempt id that prepared the invocation.
    pub holding_attempt_id: String,
    /// Invocation epoch that prepared the invocation.
    pub invocation_epoch: u32,
}

/// Public attempt lifecycle disposition derived from committed attempt projections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedAttemptDispositionStatus {
    /// Node id that owns the attempt.
    pub node_id: String,
    /// Attempt id.
    pub attempt_id: String,
    /// Attempt disposition: `started`, `completed`, `failed`, or `interrupted`.
    pub disposition: String,
    /// Attempt number when the attempt is still open.
    pub attempt_no: Option<u32>,
    /// Retryability for failed attempts.
    pub retryable: Option<bool>,
    /// Output cell id for completed attempts.
    pub output_cell_id: Option<String>,
}

/// Public terminal resolution summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedTerminalResolutionStatus {
    /// Terminal outcome.
    pub outcome: String,
    /// Public claim carried by the terminal outcome.
    pub claim: String,
}

/// Response returned after typed start or resume dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedRunResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current semantic run mode.
    pub run_mode: TypedRunMode,
    /// Derived saga status.
    pub saga: TypedSagaStatus,
    /// Attempt-level dispositions, distinct from semantic run mode.
    pub attempt_dispositions: Vec<TypedAttemptDispositionStatus>,
    /// Last scheduler status observed by the app dispatch loop.
    ///
    /// Read-only status reports `observed`; start/resume dispatch reports `advanced`, `blocked`, or
    /// `public_output_projected`.
    pub scheduler_status: String,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
}

impl fmt::Display for TypedRunResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} run_mode={} spec_hash={} head_seq={} scheduler_status={}",
            self.run_id, self.run_mode, self.spec_hash, self.head_seq, self.scheduler_status
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedReplayResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current semantic run mode.
    pub run_mode: TypedRunMode,
    /// Derived saga status.
    pub saga: TypedSagaStatus,
    /// Attempt-level dispositions, distinct from semantic run mode.
    pub attempt_dispositions: Vec<TypedAttemptDispositionStatus>,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
    /// Retained artifact evidence entries supplied to the replay broker.
    pub retained_artifacts: usize,
}

impl fmt::Display for TypedReplayResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} replay verified spec_hash={} run_mode={} head_seq={} retained_artifacts={}",
            self.run_id, self.spec_hash, self.run_mode, self.head_seq, self.retained_artifacts
        )
    }
}

/// Application facade for certified typed runtime dispatch.
#[derive(Clone)]
pub struct RunServices<S> {
    scheduler: SerialTypedScheduler,
    store: S,
    artifacts: FsTypedArtifactStore,
    certification_registry: CertificationRegistry,
}

impl<S> RunServices<S>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
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
    pub async fn launch_run(&self, req: RunLaunchRequest) -> Result<TypedRunResponse, AppError> {
        let runtime_spec = CertifiedRuntimeSpec::new(req.certified_spec)?;
        let expected_next_seq = self
            .store
            .expected_next_seq(&req.run_id)
            .await
            .map_err(async_app_store_error)?;
        let launch = self.scheduler.prepare_run_launch(
            &runtime_spec,
            req.run_id.clone(),
            req.evidence,
            expected_next_seq,
        )?;
        self.scheduler.start_run(&self.store, launch).await?;
        let status = self
            .drive_with_mode(&runtime_spec, &req.run_id, req.drive)
            .await?;
        self.run_response_from_verified_status(&req.run_id, status)
            .await
    }

    /// Resumes a certified typed run from its stored spec artifact.
    pub async fn resume_stored_run(
        &self,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<TypedRunResponse, AppError> {
        let runtime_spec = self
            .load_verified_run_read_context(run_id)
            .await?
            .runtime_spec()
            .clone();
        let status = self.drive_with_mode(&runtime_spec, run_id, drive).await?;
        self.run_response_from_verified_status(run_id, status).await
    }

    /// Records a signed manual resolution and optionally resumes typed scheduler execution.
    pub async fn record_manual_resolution(
        &self,
        req: ManualResolutionRecordRequest,
    ) -> Result<TypedRunResponse, AppError> {
        let run_id = req.run_id.clone();
        let drive = req.drive;
        let runtime_spec = self
            .load_verified_run_read_context(&run_id)
            .await?
            .runtime_spec()
            .clone();
        let manual_request = manual_resolution_runtime_request(req)?;
        self.scheduler
            .record_manual_resolution(&self.store, &runtime_spec, &run_id, manual_request)
            .await?;
        let status = self.drive_with_mode(&runtime_spec, &run_id, drive).await?;
        self.run_response_from_verified_status(&run_id, status)
            .await
    }

    /// Returns typed run status by rebuilding projection from the authoritative run stream.
    pub async fn run_status(&self, run_id: &RunId) -> Result<TypedRunResponse, AppError> {
        let context = load_async_verified_status_read_context(
            &self.store,
            &self.artifacts,
            &self.certification_registry,
            run_id,
        )
        .await?;
        typed_run_status_from_projection(
            run_id,
            context.runtime_spec(),
            context.events(),
            context.projection(),
        )
    }

    async fn run_response_from_verified_status(
        &self,
        run_id: &RunId,
        status: SchedulerStatus,
    ) -> Result<TypedRunResponse, AppError> {
        let context = load_async_verified_status_read_context(
            &self.store,
            &self.artifacts,
            &self.certification_registry,
            run_id,
        )
        .await?;
        typed_run_response_from_projection(
            run_id,
            context.runtime_spec(),
            context.events(),
            context.projection(),
            status,
        )
    }

    /// Returns the authoritative typed run stream.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<TypedRunStreamResponse, AppError> {
        let context = self.load_verified_run_read_context(run_id).await?;
        Ok(typed_run_stream_response_from_verified_context(&context))
    }

    /// Verifies replay authority for a run using retained typed artifact evidence only.
    pub async fn verify_replay_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<TypedReplayResponse, AppError> {
        let context = self.load_verified_run_read_context(run_id).await?;
        let authority = replay_read_authority_for_run(context.runtime_spec(), context.view())?;
        let broker = ReplayBroker::from_read_authority(authority)?;
        let stream = context.events();
        mfm_adapters_evm_contracts::verify_contract_lifecycle_replay(&broker)?;
        mfm_transports_proof::verify_deterministic_proof_replay(&broker)?;
        let projection = broker.projection_snapshot();
        let saga = projection.derive_saga_projection(run_id, &context.runtime_spec().spec().saga);
        let retained_artifacts = projection
            .retention(run_id)
            .map(|retention| retention.refs.len())
            .unwrap_or_default();
        Ok(TypedReplayResponse {
            run_id: run_id.as_str().to_owned(),
            spec_hash: broker.certified_spec().spec_hash.as_str().to_owned(),
            run_mode: typed_run_mode(saga.run_mode),
            saga: typed_saga_status_with_resources(
                context.runtime_spec().spec(),
                projection,
                &saga,
            ),
            attempt_dispositions: typed_attempt_dispositions(projection),
            head_seq: stream_head(stream),
            retained_artifacts,
        })
    }

    /// Renders typed public output from store-owned projection and typed artifact bytes.
    pub async fn typed_public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<TypedPublicOutputResponse, AppError> {
        let context = self.load_verified_run_read_context(run_id).await?;
        let authority = public_output_read_authority_for_run(
            &self.artifacts,
            context.runtime_spec(),
            context.view(),
            public_schema_id,
        )
        .await?;
        render_typed_public_output(&self.artifacts, &authority).await
    }

    async fn load_verified_run_read_context(
        &self,
        run_id: &RunId,
    ) -> Result<VerifiedRunReadContext, AppError> {
        load_async_verified_run_read_context(
            &self.store,
            &self.artifacts,
            &self.certification_registry,
            run_id,
        )
        .await
    }

    async fn drive_with_mode(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        drive: DriveMode,
    ) -> Result<SchedulerStatus, AppError> {
        match drive {
            DriveMode::AppendOnly => Ok(SchedulerStatus::Blocked),
            DriveMode::Once => Ok(self
                .scheduler
                .drive_once(&self.store, runtime_spec, run_id)
                .await?),
            DriveMode::UntilBlocked => Ok(self
                .scheduler
                .drive_until_blocked(&self.store, runtime_spec, run_id)
                .await?),
        }
    }
}

#[derive(Debug, Clone)]
struct VerifiedRunReadContext {
    runtime_spec: CertifiedRuntimeSpec,
    view: VerifiedRunHistoryView,
}

impl VerifiedRunReadContext {
    fn runtime_spec(&self) -> &CertifiedRuntimeSpec {
        &self.runtime_spec
    }

    fn view(&self) -> &VerifiedRunHistoryView {
        &self.view
    }

    fn events(&self) -> &[store::KernelEventEnvelope] {
        self.view.events()
    }

    fn status_projection_with_resource_lanes(
        &self,
        global_projection: &store::ProjectionSnapshot,
    ) -> Result<store::ProjectionSnapshot, AppError> {
        status_projection_from_verified_view_with_resource_lanes(&self.view, global_projection)
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone)]
struct VerifiedStatusReadContext {
    read: VerifiedRunReadContext,
    projection: store::ProjectionSnapshot,
}

impl VerifiedStatusReadContext {
    fn runtime_spec(&self) -> &CertifiedRuntimeSpec {
        self.read.runtime_spec()
    }

    fn events(&self) -> &[store::KernelEventEnvelope] {
        self.read.events()
    }

    fn projection(&self) -> &store::ProjectionSnapshot {
        &self.projection
    }
}

async fn load_async_verified_run_read_context<S>(
    store: &S,
    artifacts: &FsTypedArtifactStore,
    registry: &CertificationRegistry,
    run_id: &RunId,
) -> Result<VerifiedRunReadContext, AppError>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
    let stream = store
        .load_run_stream(run_id)
        .await
        .map_err(async_app_store_error)?;
    verified_run_read_context_from_events(artifacts, registry, run_id, stream).await
}

async fn load_async_verified_status_read_context<S>(
    store: &S,
    artifacts: &FsTypedArtifactStore,
    registry: &CertificationRegistry,
    run_id: &RunId,
) -> Result<VerifiedStatusReadContext, AppError>
where
    S: store::AsyncTypedRunEventStore + Send + Sync,
{
    let stream = store
        .load_run_stream(run_id)
        .await
        .map_err(async_app_store_error)?;
    let projection = store
        .status_projection_snapshot(run_id)
        .await
        .map_err(async_app_store_error)?;
    verified_status_read_context_from_events(artifacts, registry, run_id, stream, &projection).await
}

async fn verified_status_read_context_from_events(
    artifacts: &FsTypedArtifactStore,
    registry: &CertificationRegistry,
    run_id: &RunId,
    stream: Vec<store::KernelEventEnvelope>,
    global_projection: &store::ProjectionSnapshot,
) -> Result<VerifiedStatusReadContext, AppError> {
    let read = verified_run_read_context_from_events(artifacts, registry, run_id, stream).await?;
    let projection = read.status_projection_with_resource_lanes(global_projection)?;
    Ok(VerifiedStatusReadContext { read, projection })
}

async fn verified_run_read_context_from_events(
    artifacts: &FsTypedArtifactStore,
    registry: &CertificationRegistry,
    run_id: &RunId,
    stream: Vec<store::KernelEventEnvelope>,
) -> Result<VerifiedRunReadContext, AppError> {
    if stream.is_empty() {
        return Err(AppError::not_found(
            "RunNotFound",
            "typed run stream was not found",
        ));
    }
    let runtime_spec = load_runtime_spec_for_run(artifacts, registry, run_id, &stream).await?;
    let committed = store::CommittedRunStream::from_events(run_id.clone(), stream)?;
    let retained_artifacts =
        store::VerifiedRunArtifactStore::from_committed_stream(&committed, artifacts).await?;
    let view = VerifiedRunHistoryView::from_committed_stream(
        &runtime_spec,
        committed,
        retained_artifacts,
    )?;
    Ok(VerifiedRunReadContext { runtime_spec, view })
}

/// Loads and verifies the certified spec artifact bound by a typed run stream.
pub async fn load_certified_spec_for_run(
    artifacts: &FsTypedArtifactStore,
    registry: &CertificationRegistry,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<CertifiedTypedSpec, AppError> {
    let run_admitted = run_admitted_payload(run_id, stream)?;
    let (spec_bytes, spec_evidence) = artifacts
        .get_artifact_by_id(&run_admitted.spec_artifact.artifact_id)
        .await?;
    validate_spec_artifact_evidence(run_admitted, &spec_evidence)?;
    let (certificate_bytes, certificate_evidence) = artifacts
        .get_artifact_by_id(&run_admitted.certificate_artifact.artifact_id)
        .await?;
    validate_certificate_artifact_evidence(run_admitted, &certificate_evidence)?;
    let certified = mfm_certify::verify_certified_bundle_with_trusted_registry(
        &spec_bytes,
        &certificate_bytes,
        registry,
    )?;
    validate_run_admitted_matches_spec(run_admitted, certified.envelope())?;
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

/// Builds sealed replay read authority from a verified run-history view.
pub fn replay_read_authority_for_run(
    runtime_spec: &CertifiedRuntimeSpec,
    verified_view: &VerifiedRunHistoryView,
) -> Result<ReplayReadAuthority, AppError> {
    ReplayReadAuthority::from_verified_run_history_view(runtime_spec, verified_view)
        .map_err(Into::into)
}

fn certified_spec_launch_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<RunLaunchArtifact, AppError> {
    let canonical = runtime_spec.spec().canonical_json().map_err(|error| {
        let _ = error;
        AppError::backend(
            ErrorClass::Internal,
            "CertifiedSpecCanonicalError",
            "Certified typed spec canonicalization failed",
        )
    })?;
    Ok(launch_artifact(
        canonical.to_vec(),
        runtime_spec.spec().media_type.clone(),
        None,
        None,
        None,
        events::ArtifactRole::TypedExecutionSpec,
    ))
}

fn certified_spec_certificate_launch_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<RunLaunchArtifact, AppError> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| {
            let _ = error;
            AppError::backend(
                ErrorClass::Internal,
                "CertifiedCertificateCanonicalError",
                "Certified typed spec certificate canonicalization failed",
            )
        })?;
    let media_type =
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).map_err(|error| {
            let _ = error;
            AppError::backend(
                ErrorClass::Internal,
                "CertifiedCertificateMediaTypeInvalid",
                "Certified typed spec certificate media type is invalid",
            )
        })?;
    Ok(launch_artifact(
        canonical.to_vec(),
        media_type,
        None,
        None,
        None,
        events::ArtifactRole::TypedSpecCertificate,
    ))
}

fn config_launch_artifacts_for_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    registry: &CertificationRegistry,
    configs: Vec<RunLaunchConfigArtifact>,
) -> Result<Vec<RunLaunchArtifact>, AppError> {
    let mut supplied = BTreeMap::new();
    for config in configs {
        let artifact = launch_artifact(
            config.bytes,
            config.media_type,
            Some(config.schema_id.clone()),
            None,
            None,
            events::ArtifactRole::TypedConfig,
        );
        let key = config_input_key(&config.schema_id, &artifact.evidence.digest);
        if let Some(existing) = supplied.get(&key) {
            if existing != &artifact {
                return Err(AppError::new(
                    ErrorClass::BadRequest,
                    "DuplicateLaunchConfigArtifact",
                    "config input was supplied more than once with conflicting bytes",
                ));
            }
            continue;
        }
        if supplied.insert(key, artifact).is_some() {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "DuplicateLaunchConfigArtifact",
                "config input was supplied more than once",
            ));
        }
    }

    let mut validated = Vec::with_capacity(runtime_spec.spec().config_refs.len());
    for config_ref in &runtime_spec.spec().config_refs {
        let key = config_input_key(&config_ref.schema_id, &config_ref.digest);
        let artifact = supplied.remove(&key).ok_or_else(|| {
            AppError::new(
                ErrorClass::BadRequest,
                "MissingLaunchConfigArtifact",
                format!("missing config input for {}", config_ref.schema_id),
            )
        })?;
        validate_artifact_requirement_for_app(
            config_ref_artifact_requirement(config_ref),
            &artifact.evidence,
            ErrorClass::BadRequest,
            "LaunchConfigArtifactMismatch",
            "typed config input does not match the certified spec",
        )?;
        if registry
            .validate_config_ref_bytes(config_ref, &artifact.bytes)?
            .is_none()
            && !framework_config_matches_ref(runtime_spec.spec(), config_ref, &artifact.bytes)?
        {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "LaunchConfigValidatorMissing",
                format!(
                    "no trusted typed config validator was registered for {}",
                    config_ref.schema_id
                ),
            ));
        }
        validated.push(artifact);
    }
    if !supplied.is_empty() {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "UnknownLaunchConfigArtifact",
            "config input was supplied for a config not present in the certified spec",
        ));
    }
    Ok(validated)
}

fn framework_config_launch_artifacts_for_spec(
    typed_spec: &spec::TypedExecutionSpec,
) -> Result<Vec<RunLaunchConfigArtifact>, AppError> {
    let mut artifacts = Vec::new();
    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes =
            match spec::framework_config_canonical_json(framework.config_kind(), &node.node_id) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let _ = error;
                    return Err(AppError::backend(
                        ErrorClass::Internal,
                        "LaunchFrameworkConfigInvalid",
                        "Framework config canonicalization failed",
                    ));
                }
            };
        artifacts.push(RunLaunchConfigArtifact {
            schema_id: node.config_ref.schema_id.clone(),
            bytes: bytes.to_vec(),
            media_type: json_media_type()?,
        });
    }
    Ok(artifacts)
}

fn framework_config_matches_ref(
    typed_spec: &spec::TypedExecutionSpec,
    config_ref: &spec::ConfigRef,
    bytes: &[u8],
) -> Result<bool, AppError> {
    for node in &typed_spec.nodes {
        if &node.config_ref != config_ref {
            continue;
        }
        let Some(framework) = &node.framework else {
            continue;
        };
        let expected =
            spec::framework_config_canonical_json(framework.config_kind(), &node.node_id).map_err(
                |error| {
                    let _ = error;
                    AppError::backend(
                        ErrorClass::Internal,
                        "LaunchFrameworkConfigInvalid",
                        "Framework config canonicalization failed",
                    )
                },
            )?;
        return Ok(expected.as_bytes() == bytes);
    }
    Ok(false)
}

fn seed_launch_cells_for_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    seeds: Vec<RunLaunchSeedArtifact>,
) -> Result<Vec<RunLaunchSeedCell>, AppError> {
    let mut supplied = std::collections::BTreeMap::new();
    for seed in seeds {
        if supplied.insert(seed.seed_id.clone(), seed).is_some() {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "DuplicateLaunchSeedArtifact",
                "seed input was supplied more than once",
            ));
        }
    }

    let mut seed_refs = Vec::with_capacity(runtime_spec.spec().seeds.len());
    for seed_spec in &runtime_spec.spec().seeds {
        let input = supplied.remove(&seed_spec.seed_id).ok_or_else(|| {
            AppError::new(
                ErrorClass::BadRequest,
                "MissingLaunchSeedArtifact",
                format!("missing seed input for {}", seed_spec.seed_id),
            )
        })?;
        let artifact = launch_artifact(
            input.bytes,
            input.media_type,
            Some(seed_spec.schema_id.clone()),
            Some(seed_spec.semantic_type_id.clone()),
            Some(seed_spec.seed_id.clone()),
            events::ArtifactRole::SeedInput,
        );
        validate_artifact_requirement_for_app(
            seed_artifact_requirement(seed_spec, &artifact.evidence),
            &artifact.evidence,
            ErrorClass::BadRequest,
            "LaunchSeedArtifactMismatch",
            "seed input artifact metadata does not match the certified spec",
        )?;
        if let Some(required_digest) = &seed_spec.required_digest {
            if &artifact.evidence.digest != required_digest {
                return Err(AppError::new(
                    ErrorClass::BadRequest,
                    "LaunchSeedDigestMismatch",
                    "seed input digest does not match the certified spec",
                ));
            }
        }
        seed_refs.push(RunLaunchSeedCell {
            bytes: artifact.bytes,
            cell: events::SeedCellRef {
                seed_id: seed_spec.seed_id.clone(),
                cell_id: seed_spec.cell_id.clone(),
                scope_id: seed_spec.scope_id.clone(),
                semantic_type_id: seed_spec.semantic_type_id.clone(),
                schema_id: seed_spec.schema_id.clone(),
                digest: artifact.evidence.digest.clone(),
                seed_artifact: events::ArtifactEvidenceRef {
                    artifact_id: artifact.evidence.artifact_id.clone(),
                    role: artifact.evidence.artifact_role,
                    schema_id: seed_spec.schema_id.clone(),
                    semantic_type_id: artifact.evidence.semantic_type_id.clone(),
                    content_digest: artifact.evidence.digest,
                    byte_len: artifact.evidence.byte_len,
                    media_type: artifact.evidence.media_type,
                },
            },
        });
    }
    if !supplied.is_empty() {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "UnknownLaunchSeedArtifact",
            "seed input was supplied for a seed not present in the certified spec",
        ));
    }
    Ok(seed_refs)
}

fn launch_artifact(
    bytes: Vec<u8>,
    media_type: spec::MediaType,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    producer_seed_id: Option<SeedId>,
    artifact_role: events::ArtifactRole,
) -> RunLaunchArtifact {
    let byte_len = bytes.len() as u64;
    let digest = content_digest_for_bytes(&bytes);
    RunLaunchArtifact {
        bytes,
        evidence: store::ArtifactEvidenceRef {
            artifact_id: artifact_id_for_digest(&digest),
            digest,
            byte_len,
            media_type,
            schema_id,
            semantic_type_id,
            producer_node_id: None,
            producer_seed_id,
            artifact_role,
        },
    }
}

fn content_digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn artifact_id_for_digest(digest: &ContentDigest) -> ArtifactId {
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
}

fn config_input_key(schema_id: &SchemaId, digest: &ContentDigest) -> String {
    format!("{schema_id}:{digest}")
}

fn validate_artifact_requirement_for_app(
    requirement: store::EventArtifactRequirement,
    evidence: &store::ArtifactEvidenceRef,
    class: ErrorClass,
    code: &'static str,
    message: &'static str,
) -> Result<(), AppError> {
    store::validate_artifact_requirement_against_evidence(&requirement, evidence).map_err(|error| {
        match error {
            store::StoreError::ArtifactEvidenceMismatch { .. } => {
                AppError::new(class, code, message)
            }
            error => error.into(),
        }
    })
}

fn run_artifact_requirement(
    source: store::EventArtifactReferenceSource,
    expected: &events::RunArtifactEvidenceRef,
    role: events::ArtifactRole,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source,
        artifact_id: expected.artifact_id.clone(),
        digest: Some(expected.content_digest.clone()),
        byte_len: Some(expected.byte_len),
        media_type: Some(expected.media_type.clone()),
        schema_id: expected.schema_id.clone(),
        semantic_type_id: expected.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(role),
    }
}

fn config_ref_artifact_requirement(
    config_ref: &spec::ConfigRef,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: config_ref.artifact_id.clone(),
        digest: Some(config_ref.digest.clone()),
        byte_len: Some(config_ref.byte_len),
        media_type: Some(config_ref.media_type.clone()),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    }
}

fn seed_artifact_requirement(
    seed_spec: &spec::SeedSpec,
    evidence: &store::ArtifactEvidenceRef,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::SeedCell,
        artifact_id: evidence.artifact_id.clone(),
        digest: Some(evidence.digest.clone()),
        byte_len: Some(evidence.byte_len),
        media_type: Some(evidence.media_type.clone()),
        schema_id: Some(seed_spec.schema_id.clone()),
        semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed_spec.seed_id.clone()),
        artifact_role: Some(events::ArtifactRole::SeedInput),
    }
}

fn public_output_cell_artifact_requirement(
    cell: &events::NamedTypedCellRef,
) -> store::EventArtifactRequirement {
    let (artifact_role, producer_node_id, producer_seed_id) = match &cell.producer {
        spec::CellProducer::Node(node_id) => (
            events::ArtifactRole::StateOutput,
            Some(node_id.clone()),
            None,
        ),
        spec::CellProducer::Seed(seed_id) => {
            (events::ArtifactRole::SeedInput, None, Some(seed_id.clone()))
        }
    };
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::PublicOutputCell,
        artifact_id: cell.artifact_id.clone(),
        digest: Some(cell.content_digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: Some(cell.schema_id.clone()),
        semantic_type_id: Some(cell.semantic_type_id.clone()),
        producer_node_id,
        producer_seed_id,
        artifact_role: Some(artifact_role),
    }
}

fn public_output_rendered_artifact_requirement(
    payload: &events::PublicOutputProduced,
    artifact_id: &ArtifactId,
    rendered_digest: &ContentDigest,
    media_type: spec::MediaType,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::PublicOutputRendered,
        artifact_id: artifact_id.clone(),
        digest: Some(rendered_digest.clone()),
        byte_len: None,
        media_type: Some(media_type),
        schema_id: Some(payload.public_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(payload.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::PublicOutput),
    }
}

/// Returns the default JSON media type used by typed CLI seed inputs.
pub fn json_media_type() -> Result<spec::MediaType, AppError> {
    spec::MediaType::new("application/json").map_err(|error| {
        let _ = error;
        AppError::backend(
            ErrorClass::Internal,
            "JsonMediaTypeInvalid",
            "JSON media type is invalid",
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
        let _ = error;
        AppError::new(
            ErrorClass::BadRequest,
            "CertifiedBundleInvalid",
            "Certified typed spec bundle JSON is invalid",
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
            let _ = error;
            AppError::new(
                ErrorClass::BadRequest,
                "CertifiedBundleInvalid",
                "Certified typed spec bundle is invalid",
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
            "CertifiedBundleInvalid",
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
        let _ = (field, error);
        AppError::backend(
            ErrorClass::Internal,
            "CertifiedBundleSerializationFailed",
            "Certified typed spec bundle serialization failed",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.to_vec())
        .map_err(|error| {
            let _ = (field, error);
            AppError::new(
                ErrorClass::BadRequest,
                "CertifiedBundleInvalid",
                "Certified typed spec bundle field is not canonical JSON",
            )
        })
}

/// Resolves, plans, certifies, and prepares an entry-point op run launch.
pub fn prepare_entry_point_run_launch(
    input: EntryPointRunLaunchInput<'_>,
) -> Result<PreparedEntryPointRunLaunch, AppError> {
    let registry_digest = input.entry_point_registry.registry_digest()?;
    let submitted_public_op_name = input.public_op_name.clone();
    let config_format = input.authored_config.format();
    let authored_config_digest = input.authored_config.authored_digest().clone();
    let op = input
        .entry_point_registry
        .resolve(&input.public_op_name, input.op_version)?;
    let resolved_op_id = op.op_id();
    let resolved_op_version = op.version();
    let plan = op.plan(input.authored_config)?;
    if plan.authored_config_digest != authored_config_digest {
        return Err(entry_point_launch_internal_error(
            "EntryPointOpAuthoredDigestMismatch",
            "entry-point op produced mismatched authored config evidence",
        ));
    }

    let public_output_schema_id = plan.public_output_schema_id.clone();
    let mut config_inputs = plan
        .config_material
        .iter()
        .map(|artifact| RunLaunchConfigArtifact {
            schema_id: artifact.schema_id.clone(),
            bytes: artifact.bytes.to_vec(),
            media_type: artifact.media_type.clone(),
        })
        .collect::<Vec<_>>();
    let seed_inputs = plan
        .seed_material
        .iter()
        .map(|artifact| RunLaunchSeedArtifact {
            seed_id: artifact.seed_id.clone(),
            bytes: artifact.bytes.to_vec(),
            media_type: artifact.media_type.clone(),
        })
        .collect::<Vec<_>>();
    let lowered =
        mfm_certify::lower_program_draft(&plan.draft).map_err(entry_point_certification_error)?;
    let certified_spec = mfm_certify::certify_typed_spec(lowered, input.certification_registry)
        .map_err(entry_point_certification_error)?;
    config_inputs.extend(framework_config_launch_artifacts_for_spec(
        &certified_spec.envelope().spec,
    )?);
    if let Some(expected_public_schema_id) = &public_output_schema_id {
        let actual_public_schema_id = &certified_spec
            .envelope()
            .spec
            .public_outputs
            .public_schema_id;
        if actual_public_schema_id != expected_public_schema_id {
            return Err(entry_point_launch_internal_error(
                "EntryPointOpPublicOutputMismatch",
                "entry-point op public output schema does not match the certified spec",
            ));
        }
    }

    let evidence = EntryPointLaunchEvidence {
        submitted_public_op_name,
        resolved_op_id,
        resolved_op_version,
        entry_point_registry_digest: registry_digest,
        lowering_identity: plan.lowering_identity,
        canonicalizer_identity: plan.canonicalizer_identity,
        config_format,
        authored_config_digest,
        canonical_config_digest: plan.canonical_config_digest,
    };
    let request = prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: input.certification_registry,
            run_id: input.run_id,
            framework_version: input.framework_version,
            source_revision: input.source_revision,
            launched_at_unix_ms: input.launched_at_unix_ms,
            drive: input.drive,
        },
        config_inputs,
        seed_inputs,
    )?;
    Ok(PreparedEntryPointRunLaunch {
        request,
        evidence,
        public_output_schema_id,
    })
}

fn entry_point_certification_error(_error: mfm_certify::CertifyError) -> AppError {
    AppError::backend(
        ErrorClass::BadRequest,
        "EntryPointOpCertificationFailed",
        "Entry-point op planned spec failed certification",
    )
}

fn entry_point_launch_internal_error(code: &'static str, message: &'static str) -> AppError {
    AppError::backend(ErrorClass::Internal, code, message)
}

/// Untrusted persisted certified bundle bytes plus launch inputs for a typed run start.
pub struct UntrustedCertifiedBundleLaunchInput<'a> {
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
    /// Caller-supplied launch time in Unix milliseconds.
    pub launched_at_unix_ms: u64,
    /// Drive mode used for the initial scheduler invocation.
    pub drive: DriveMode,
}

/// Certifier-backed typed spec authority plus launch metadata for a typed run start.
pub struct CertifiedRunLaunchInput<'a> {
    /// Certifier-backed typed spec authority.
    pub certified_spec: CertifiedTypedSpec,
    /// Trusted registry used to validate launch config artifacts.
    pub registry: &'a CertificationRegistry,
    /// Run id to record in the started run stream.
    pub run_id: RunId,
    /// Framework version evidence to bind to the run start event.
    pub framework_version: &'a str,
    /// Source revision evidence to bind to the run start event.
    pub source_revision: &'a str,
    /// Caller-supplied launch time in Unix milliseconds.
    pub launched_at_unix_ms: u64,
    /// Drive mode used for the initial scheduler invocation.
    pub drive: DriveMode,
}

/// Verifies untrusted persisted certified bundle bytes and builds a typed run-start request.
///
/// This helper is for transport and storage boundaries that receive serialized bundle data. It
/// verifies the spec and certificate against the trusted registry before producing a request that
/// can reach the runtime boundary.
pub fn prepare_verified_bundle_launch(
    input: UntrustedCertifiedBundleLaunchInput<'_>,
    config_inputs: Vec<RunLaunchConfigArtifact>,
    seed_inputs: Vec<RunLaunchSeedArtifact>,
) -> Result<RunLaunchRequest, AppError> {
    let certified_spec = mfm_certify::verify_certified_bundle_with_trusted_registry(
        input.spec_bytes,
        input.certificate_bytes,
        input.registry,
    )?;
    prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: input.registry,
            run_id: input.run_id,
            framework_version: input.framework_version,
            source_revision: input.source_revision,
            launched_at_unix_ms: input.launched_at_unix_ms,
            drive: input.drive,
        },
        config_inputs,
        seed_inputs,
    )
}

/// Builds a typed run-start request from certifier-backed typed spec authority and launch inputs.
pub fn prepare_certified_run_launch(
    input: CertifiedRunLaunchInput<'_>,
    config_inputs: Vec<RunLaunchConfigArtifact>,
    seed_inputs: Vec<RunLaunchSeedArtifact>,
) -> Result<RunLaunchRequest, AppError> {
    let runtime_spec = CertifiedRuntimeSpec::new(input.certified_spec.clone())?;
    let spec_artifact = certified_spec_launch_artifact(&runtime_spec)?;
    let certificate_artifact = certified_spec_certificate_launch_artifact(&runtime_spec)?;
    let config_artifacts =
        config_launch_artifacts_for_spec(&runtime_spec, input.registry, config_inputs)?;
    let seed_cells = seed_launch_cells_for_spec(&runtime_spec, seed_inputs)?;
    Ok(RunLaunchRequest {
        certified_spec: input.certified_spec,
        run_id: input.run_id,
        evidence: RunLaunchEvidence {
            spec_artifact,
            certificate_artifact,
            config_artifacts,
            framework_version: events::FrameworkVersion::new(input.framework_version).map_err(
                |error| {
                    let _ = error;
                    AppError::new(
                        ErrorClass::BadRequest,
                        "FrameworkVersionInvalid",
                        "Framework version is invalid",
                    )
                },
            )?,
            source_revision: events::SourceRevision::new(input.source_revision).map_err(
                |error| {
                    let _ = error;
                    AppError::new(
                        ErrorClass::BadRequest,
                        "SourceRevisionInvalid",
                        "Source revision is invalid",
                    )
                },
            )?,
            launched_at_unix_ms: input.launched_at_unix_ms,
            adapter_executables: Vec::new(),
            seed_cells,
        },
        drive: input.drive,
    })
}

fn async_app_store_error(_error: impl fmt::Display) -> AppError {
    AppError::backend(
        ErrorClass::Conflict,
        "RunStoreRejected",
        "Run store rejected the requested operation",
    )
}

fn manual_resolution_runtime_request(
    req: ManualResolutionRecordRequest,
) -> Result<ManualResolutionRequest, AppError> {
    let media_type = spec::MediaType::new(&req.evidence_media_type).map_err(|error| {
        let _ = error;
        AppError::new(
            ErrorClass::BadRequest,
            "ManualResolutionEvidenceMediaTypeInvalid",
            "Manual resolution evidence media type is invalid",
        )
    })?;
    let note = req
        .note
        .map(events::ManualResolutionNote::new)
        .transpose()
        .map_err(|error| {
            let _ = error;
            AppError::new(
                ErrorClass::BadRequest,
                "ManualResolutionNoteInvalid",
                "Manual resolution note is invalid",
            )
        })?;
    Ok(ManualResolutionRequest {
        outcome: req.outcome.into_event(),
        evidence_artifact: ManualResolutionEvidenceArtifact {
            bytes: req.evidence_bytes,
            media_type,
        },
        proof_bytes: req.authorization_proof_bytes,
        note,
    })
}

fn typed_run_status_from_projection(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
) -> Result<TypedRunResponse, AppError> {
    let spec_hash = run_admitted_spec_hash(stream)?;
    typed_run_status_from_projection_with_spec_hash(
        run_id,
        runtime_spec,
        stream,
        projection,
        &spec_hash,
    )
}

fn typed_run_status_from_projection_with_spec_hash(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
    spec_hash: &SpecHash,
) -> Result<TypedRunResponse, AppError> {
    let saga = projection.derive_saga_projection(run_id, &runtime_spec.spec().saga);
    Ok(TypedRunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: spec_hash.as_str().to_owned(),
        run_mode: typed_run_mode(saga.run_mode),
        saga: typed_saga_status_with_resources(runtime_spec.spec(), projection, &saga),
        attempt_dispositions: typed_attempt_dispositions(projection),
        scheduler_status: "observed".to_owned(),
        head_seq: stream_head(stream),
    })
}

fn typed_run_stream_response_from_verified_context(
    context: &VerifiedRunReadContext,
) -> TypedRunStreamResponse {
    let events = context.events();
    TypedRunStreamResponse {
        run_id: context.view().run_id().as_str().to_owned(),
        head_seq: stream_head(events),
        events: events.iter().map(typed_event_ref).collect(),
    }
}

/// Builds typed public-output read authority from certified runtime authority, verified run-history
/// view, rebuilt projection, and verified typed artifact evidence.
pub async fn public_output_read_authority_for_run(
    artifacts: &FsTypedArtifactStore,
    runtime_spec: &CertifiedRuntimeSpec,
    verified_view: &VerifiedRunHistoryView,
    public_schema_id: &SchemaId,
) -> Result<PublicOutputReadAuthority, AppError> {
    if runtime_spec.spec_hash() != verified_view.spec_hash() {
        return Err(AppError::new(
            ErrorClass::Internal,
            "PublicOutputAuthorityMismatch",
            "verified history spec hash does not match certified runtime authority",
        ));
    }
    let projection = verified_view.projection_snapshot();
    let public_output = projection.public_output(public_schema_id).ok_or_else(|| {
        AppError::not_found(
            "PublicOutputNotFound",
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
            "PublicOutputRenderFailed",
            "typed public output render failed",
        ));
    };

    let payload =
        public_output_payload_from_stream(verified_view.events(), event_id, public_schema_id)?;
    if &payload.spec_hash != runtime_spec.spec_hash()
        || &payload.public_schema_id != public_schema_id
        || public_schema_id != &runtime_spec.spec().public_outputs.public_schema_id
    {
        return Err(AppError::new(
            ErrorClass::Internal,
            "PublicOutputAuthorityMismatch",
            "typed public-output evidence does not match certified runtime authority",
        ));
    }
    verify_public_output_authority_artifacts(
        artifacts,
        payload,
        rendered_artifact_id.as_ref(),
        rendered_digest,
    )
    .await?;

    Ok(PublicOutputReadAuthority {
        run_id: verified_view.run_id().clone(),
        public_schema_id: public_schema_id.clone(),
        event_id: event_id.clone(),
        rendered_digest: rendered_digest.clone(),
        rendered_artifact_id: rendered_artifact_id.clone(),
        payload: payload.clone(),
    })
}

async fn verify_public_output_authority_artifacts(
    artifacts: &FsTypedArtifactStore,
    payload: &events::PublicOutputProduced,
    rendered_artifact_id: Option<&ArtifactId>,
    rendered_digest: &ContentDigest,
) -> Result<(), AppError> {
    for cell in &payload.cells {
        let (_, evidence) = artifacts.get_artifact_by_id(&cell.artifact_id).await?;
        verify_public_output_cell_evidence(cell, &evidence)?;
    }
    if let Some(artifact_id) = rendered_artifact_id {
        let (_, evidence) = artifacts.get_artifact_by_id(artifact_id).await?;
        verify_public_output_rendered_artifact_evidence(
            &evidence,
            artifact_id,
            rendered_digest,
            payload,
        )?;
    }
    Ok(())
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
                "PublicOutputProjectionMismatch",
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
            let _ = error;
            AppError::backend(
                ErrorClass::Internal,
                "PublicOutputDecodeFailed",
                "Typed public-output cell artifact was not JSON",
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
    validate_artifact_requirement_for_app(
        public_output_cell_artifact_requirement(cell),
        evidence,
        ErrorClass::Internal,
        "PublicOutputArtifactMismatch",
        "typed public-output cell artifact evidence does not match the event cell reference",
    )
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
    verify_public_output_rendered_artifact_evidence(
        &evidence,
        artifact_id,
        rendered_digest,
        payload,
    )?;
    serde_json::from_slice(&bytes).map_err(|error| {
        let _ = error;
        AppError::backend(
            ErrorClass::Internal,
            "PublicOutputDecodeFailed",
            "Typed public-output artifact was not JSON",
        )
    })
}

fn verify_public_output_rendered_artifact_evidence(
    evidence: &store::ArtifactEvidenceRef,
    artifact_id: &ArtifactId,
    rendered_digest: &ContentDigest,
    payload: &events::PublicOutputProduced,
) -> Result<(), AppError> {
    let json_media_type = spec::MediaType::new("application/json").map_err(|error| {
        let _ = error;
        AppError::backend(
            ErrorClass::Internal,
            "PublicOutputMediaTypeInvalid",
            "Public-output JSON media type is invalid",
        )
    })?;
    validate_artifact_requirement_for_app(
        public_output_rendered_artifact_requirement(
            payload,
            artifact_id,
            rendered_digest,
            json_media_type,
        ),
        evidence,
        ErrorClass::Internal,
        "PublicOutputArtifactMismatch",
        "typed public-output cache artifact evidence does not match the produced event",
    )
}

fn public_output_artifact_mismatch(message: &'static str) -> AppError {
    AppError::new(
        ErrorClass::Internal,
        "PublicOutputArtifactMismatch",
        message,
    )
}

fn run_admitted_payload<'a>(
    run_id: &RunId,
    stream: &'a [store::KernelEventEnvelope],
) -> Result<&'a events::RunAdmitted, AppError> {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload.as_ref()),
            _ => None,
        })
        .ok_or_else(|| {
            AppError::new(
                ErrorClass::Internal,
                "RunAdmittedMissing",
                "typed run stream is missing RunAdmitted evidence",
            )
        })
        .and_then(|run_admitted| {
            if &run_admitted.run_id == run_id {
                Ok(run_admitted)
            } else {
                Err(AppError::new(
                    ErrorClass::Internal,
                    "RunAdmittedMismatch",
                    "typed run stream RunAdmitted evidence is bound to a different run id",
                ))
            }
        })
}

fn validate_spec_artifact_evidence(
    run_admitted: &events::RunAdmitted,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), AppError> {
    if run_admitted.spec_artifact.role != events::ArtifactRole::TypedExecutionSpec {
        return Err(AppError::new(
            ErrorClass::Internal,
            "CertifiedSpecArtifactMismatch",
            "typed execution spec artifact metadata does not match RunAdmitted evidence",
        ));
    }
    validate_artifact_requirement_for_app(
        run_artifact_requirement(
            store::EventArtifactReferenceSource::RunSpec,
            &run_admitted.spec_artifact,
            events::ArtifactRole::TypedExecutionSpec,
        ),
        evidence,
        ErrorClass::Internal,
        "CertifiedSpecArtifactMismatch",
        "typed execution spec artifact metadata does not match RunAdmitted evidence",
    )?;
    let expected_spec_hash =
        SpecHash::from_digest(evidence.digest.algorithm(), *evidence.digest.digest());
    if expected_spec_hash != run_admitted.spec_hash {
        return Err(AppError::new(
            ErrorClass::Internal,
            "CertifiedSpecArtifactMismatch",
            "typed execution spec artifact metadata does not match RunAdmitted evidence",
        ));
    }
    Ok(())
}

fn validate_certificate_artifact_evidence(
    run_admitted: &events::RunAdmitted,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<(), AppError> {
    if run_admitted.certificate_artifact.role != events::ArtifactRole::TypedSpecCertificate {
        return Err(AppError::new(
            ErrorClass::Internal,
            "CertifiedCertificateArtifactMismatch",
            "typed spec certificate artifact metadata does not match RunAdmitted evidence",
        ));
    }
    validate_artifact_requirement_for_app(
        run_artifact_requirement(
            store::EventArtifactReferenceSource::RunCertificate,
            &run_admitted.certificate_artifact,
            events::ArtifactRole::TypedSpecCertificate,
        ),
        evidence,
        ErrorClass::Internal,
        "CertifiedCertificateArtifactMismatch",
        "typed spec certificate artifact metadata does not match RunAdmitted evidence",
    )?;
    Ok(())
}

fn validate_run_admitted_matches_spec(
    run_admitted: &events::RunAdmitted,
    envelope: &spec::HashedSpecEnvelope,
) -> Result<(), AppError> {
    if envelope.spec_hash != run_admitted.spec_hash
        || envelope.spec.media_type != run_admitted.spec_artifact.media_type
        || envelope.spec.spec_version != run_admitted.spec_version
        || envelope.spec.lowering_version != run_admitted.lowering_version
        || envelope.spec.public_outputs.public_schema_id != run_admitted.public_output_schema_id
        || envelope.spec.descriptor_identities != run_admitted.descriptor_identities
        || envelope
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            != run_admitted.canonicalizer_identity
    {
        return Err(AppError::new(
            ErrorClass::Internal,
            "RunAdmittedSpecMismatch",
            "RunAdmitted evidence does not match the stored certified spec artifact",
        ));
    }
    Ok(())
}

fn typed_run_response_from_projection(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
    status: SchedulerStatus,
) -> Result<TypedRunResponse, AppError> {
    let saga = projection.derive_saga_projection(run_id, &runtime_spec.spec().saga);
    Ok(TypedRunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: runtime_spec.spec_hash().as_str().to_owned(),
        run_mode: typed_run_mode(saga.run_mode),
        saga: typed_saga_status_with_resources(runtime_spec.spec(), projection, &saga),
        attempt_dispositions: typed_attempt_dispositions(projection),
        scheduler_status: scheduler_status_str(status).to_owned(),
        head_seq: stream_head(stream),
    })
}

fn status_projection_from_verified_view_with_resource_lanes(
    view: &VerifiedRunHistoryView,
    global_projection: &store::ProjectionSnapshot,
) -> Result<store::ProjectionSnapshot, store::StoreError> {
    projection_with_resource_lanes(
        view.projection_snapshot(),
        global_projection
            .resource_lanes()
            .map(|(lane_key, projection)| (lane_key.clone(), projection.clone()))
            .collect(),
    )
}

fn projection_with_resource_lanes(
    snapshot: &store::ProjectionSnapshot,
    resource_lanes: BTreeMap<store::ResourceLaneKey, store::ResourceLaneProjection>,
) -> Result<store::ProjectionSnapshot, store::StoreError> {
    store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
        run_states: snapshot
            .run_states()
            .map(|(run_id, state)| (run_id.clone(), *state))
            .collect(),
        saga_policy_digests: snapshot
            .saga_policy_digests()
            .map(|(run_id, digest)| (run_id.clone(), digest.clone()))
            .collect(),
        run_completions: snapshot
            .run_completions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        saga_engagements: snapshot
            .saga_engagements()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        manual_resolutions: snapshot
            .manual_resolutions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        attempts: snapshot
            .attempts()
            .map(|(key, projection)| (key.clone(), projection.clone()))
            .collect(),
        cells: snapshot
            .cells()
            .map(|(cell_id, projection)| (cell_id.clone(), projection.clone()))
            .collect(),
        facts: snapshot
            .facts()
            .map(|(key, projection)| (key.clone(), projection.clone()))
            .collect(),
        side_effects: snapshot
            .side_effects()
            .map(|(ledger_ref, projection)| (ledger_ref.clone(), projection.clone()))
            .collect(),
        resource_lanes,
        public_outputs: snapshot
            .public_outputs()
            .map(|(schema_id, projection)| (schema_id.clone(), projection.clone()))
            .collect(),
        retentions: snapshot
            .retentions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
    })
}

fn stream_head(stream: &[store::KernelEventEnvelope]) -> u64 {
    stream.last().map_or(0, |event| event.seq().as_u64())
}

fn typed_run_mode(mode: store::RunMode) -> TypedRunMode {
    mode.into()
}

fn typed_attempt_dispositions(
    projection: &store::ProjectionSnapshot,
) -> Vec<TypedAttemptDispositionStatus> {
    projection
        .attempts()
        .map(|(_key, attempt)| typed_attempt_disposition(attempt))
        .collect()
}

fn typed_attempt_disposition(attempt: &store::AttemptProjection) -> TypedAttemptDispositionStatus {
    let (disposition, attempt_no, retryable, output_cell_id) = match &attempt.status {
        store::AttemptStatus::Started { attempt_no, .. } => {
            ("started", Some(*attempt_no), None, None)
        }
        store::AttemptStatus::Completed { output_cell_id } => (
            "completed",
            None,
            None,
            Some(output_cell_id.as_str().to_owned()),
        ),
        store::AttemptStatus::Failed { retryable, .. } => ("failed", None, Some(*retryable), None),
        store::AttemptStatus::Interrupted => ("interrupted", None, None, None),
    };
    TypedAttemptDispositionStatus {
        node_id: attempt.node_id.as_str().to_owned(),
        attempt_id: attempt.attempt_id.as_str().to_owned(),
        disposition: disposition.to_owned(),
        attempt_no,
        retryable,
        output_cell_id,
    }
}

fn typed_saga_status_with_resources(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    saga: &store::SagaProjection,
) -> TypedSagaStatus {
    typed_saga_status_inner(
        &certified_spec.saga,
        Some(certified_spec),
        Some(projection),
        saga,
    )
}

fn typed_saga_status_inner(
    policy: &spec::SagaPolicySpec,
    certified_spec: Option<&spec::TypedExecutionSpec>,
    projection: Option<&store::ProjectionSnapshot>,
    saga: &store::SagaProjection,
) -> TypedSagaStatus {
    TypedSagaStatus {
        policy: typed_saga_policy_status(policy),
        obligations: saga
            .obligations
            .values()
            .map(|obligation| {
                typed_obligation_status(&saga.run_id, obligation, certified_spec, projection)
            })
            .collect(),
        resource_ledgers: match (certified_spec, projection) {
            (Some(certified_spec), Some(projection)) => {
                typed_resource_ledgers_for_run(certified_spec, projection, &saga.run_id)
            }
            _ => Vec::new(),
        },
        resource_lanes: projection
            .map(|projection| typed_resource_lanes_for_run(projection, &saga.run_id))
            .unwrap_or_default(),
        manual_block_reason: saga.manual_block_reason.map(manual_block_reason_str),
        required_manual_authorization: matches!(saga.run_mode, store::RunMode::ManualBlocked)
            .then(|| manual_authorization_for_policy(policy))
            .flatten(),
        terminal_resolution: saga
            .run_completion
            .as_ref()
            .map(|completion| typed_terminal_resolution(&completion.outcome)),
    }
}

fn typed_saga_policy_status(policy: &spec::SagaPolicySpec) -> TypedSagaPolicyStatus {
    match policy {
        spec::SagaPolicySpec::NoSideEffects => TypedSagaPolicyStatus {
            variant: "no_side_effects".to_owned(),
            manual_authorization: None,
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::FailWithoutAcdcClaim => TypedSagaPolicyStatus {
            variant: "fail_without_acdc_claim".to_owned(),
            manual_authorization: None,
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::ManualResolution { manual } => TypedSagaPolicyStatus {
            variant: "manual_resolution".to_owned(),
            manual_authorization: Some(typed_manual_authorization_requirements(manual)),
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved,
        } => {
            let (directive, manual_authorization) = match on_remediation_unresolved {
                spec::RemediationUnresolvedSpec::ManualResolution { manual } => (
                    "manual_resolution",
                    Some(typed_manual_authorization_requirements(manual)),
                ),
                spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim => {
                    ("fail_without_acdc_claim", None)
                }
            };
            TypedSagaPolicyStatus {
                variant: "compensate_completed".to_owned(),
                manual_authorization,
                on_remediation_unresolved: Some(directive.to_owned()),
            }
        }
    }
}

fn manual_authorization_for_policy(
    policy: &spec::SagaPolicySpec,
) -> Option<TypedManualAuthorizationRequirements> {
    match policy {
        spec::SagaPolicySpec::ManualResolution { manual } => {
            Some(typed_manual_authorization_requirements(manual))
        }
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => Some(typed_manual_authorization_requirements(manual)),
        spec::SagaPolicySpec::NoSideEffects
        | spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim,
        } => None,
    }
}

fn typed_manual_authorization_requirements(
    manual: &spec::ManualResolutionEvidenceSpec,
) -> TypedManualAuthorizationRequirements {
    TypedManualAuthorizationRequirements {
        evidence_schema_id: manual.evidence_schema.as_str().to_owned(),
        verifier_id: manual.authorization.verifier_id.as_str().to_owned(),
        signing_scheme: manual.authorization.signing_scheme.as_str().to_owned(),
        authority_id: manual
            .authorization
            .authority
            .authority_id
            .as_str()
            .to_owned(),
        operator_public_identities: manual
            .authorization
            .authority
            .operators
            .iter()
            .map(|operator| operator.public_identity.as_str().to_owned())
            .collect(),
        quorum_required_signatures: manual.authorization.quorum.required_signatures(),
    }
}

fn typed_obligation_status(
    run_id: &RunId,
    obligation: &store::SagaObligationProjection,
    certified_spec: Option<&spec::TypedExecutionSpec>,
    projection: Option<&store::ProjectionSnapshot>,
) -> TypedSagaObligationStatus {
    let forward_resource = projection
        .and_then(|projection| {
            projection.side_effect_for_run(run_id, &obligation.forward_ledger_key)
        })
        .and_then(|side_effect| {
            typed_resource_ledger_status(certified_spec?, projection?, side_effect)
        });
    TypedSagaObligationStatus {
        forward_ledger_key: obligation.forward_ledger_key.as_str().to_owned(),
        forward_phase: side_effect_phase_str(&obligation.forward_phase),
        classification: forward_classification_str(obligation.classification),
        resource: forward_resource,
        remediation: obligation.remediation.as_ref().map(|remediation| {
            let remediation_resource = projection
                .and_then(|projection| {
                    projection.side_effect_for_run(run_id, &remediation.ledger_key)
                })
                .and_then(|side_effect| {
                    typed_resource_ledger_status(certified_spec?, projection?, side_effect)
                });
            TypedRemediationLedgerStatus {
                ledger_key: remediation.ledger_key.as_str().to_owned(),
                forward_ledger_key: obligation.forward_ledger_key.as_str().to_owned(),
                phase: side_effect_phase_str(&remediation.phase),
                resource: remediation_resource,
                closed: remediation.closed,
                unresolved: remediation.unresolved.map(manual_block_reason_str),
            }
        }),
    }
}

fn typed_resource_ledger_status(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    side_effect: &store::SideEffectProjection,
) -> Option<TypedResourceLedgerStatus> {
    let node = certified_node(certified_spec, &side_effect.intent.node_id)?;
    let claim = &node.side_effect.as_ref()?.resource_claim;
    let key = side_effect
        .resource_key
        .as_ref()
        .map(typed_resource_key_status);
    let touched_set = side_effect
        .resource_touched_set
        .as_ref()
        .map(typed_resource_touched_set_status);
    let (active_lane, blocked_by_lane) =
        if side_effect_phase_has_live_resource_lane_interest(&side_effect.phase) {
            side_effect
                .resource_key
                .as_ref()
                .and_then(|resource_key| {
                    let lane_key = store::ResourceLaneKey::from_evidence(resource_key);
                    projection
                        .resource_lane(&lane_key)
                        .map(|lane| (lane_key, lane))
                })
                .map(|(lane_key, lane)| {
                    let holder = typed_resource_lane_holder(&lane_key, lane);
                    let side_effect_ref = store::SideEffectLedgerRef::new(
                        side_effect.run_id.clone(),
                        side_effect.ledger_key.clone(),
                    );
                    if lane.holder == side_effect_ref {
                        (Some(holder), None)
                    } else {
                        (None, Some(holder))
                    }
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
    let (ledger_purpose, forward_ledger_key) =
        typed_ledger_purpose_status(&side_effect.ledger_purpose);

    Some(TypedResourceLedgerStatus {
        ledger_key: side_effect.ledger_key.as_str().to_owned(),
        ledger_purpose,
        forward_ledger_key,
        phase: side_effect_phase_str(&side_effect.phase),
        claim: typed_resource_claim_status(claim),
        key,
        touched_set,
        active_lane,
        blocked_by_lane,
    })
}

fn typed_resource_ledgers_for_run(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
) -> Vec<TypedResourceLedgerStatus> {
    projection
        .side_effects()
        .filter_map(|(_, side_effect)| {
            if &side_effect.run_id == run_id {
                typed_resource_ledger_status(certified_spec, projection, side_effect)
            } else {
                None
            }
        })
        .collect()
}

fn typed_resource_lanes_for_run(
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
) -> Vec<TypedResourceLaneHolderStatus> {
    let referenced_lane_keys = projection
        .side_effects()
        .filter_map(|(_, side_effect)| {
            if &side_effect.run_id != run_id {
                return None;
            }
            if !side_effect_phase_has_live_resource_lane_interest(&side_effect.phase) {
                return None;
            }
            side_effect
                .resource_key
                .as_ref()
                .map(store::ResourceLaneKey::from_evidence)
        })
        .collect::<BTreeSet<_>>();

    projection
        .resource_lanes()
        .filter(|(lane_key, _lane)| referenced_lane_keys.contains(*lane_key))
        .map(|(lane_key, lane)| typed_resource_lane_holder(lane_key, lane))
        .collect()
}

fn side_effect_phase_has_live_resource_lane_interest(phase: &store::SideEffectPhase) -> bool {
    matches!(
        phase,
        store::SideEffectPhase::InvocationPrepared { .. }
            | store::SideEffectPhase::InvocationStarted { .. }
            | store::SideEffectPhase::SubmissionObserved { .. }
            | store::SideEffectPhase::SubmissionUnknown { .. }
            | store::SideEffectPhase::ReceiptObserved { .. }
    )
}

fn typed_resource_claim_status(claim: &spec::ResourceClaimSpec) -> TypedResourceClaimStatus {
    match claim {
        spec::ResourceClaimSpec::Exclusive {
            namespace,
            key_schema,
        } => TypedResourceClaimStatus {
            kind: "exclusive".to_owned(),
            namespace: Some(namespace.as_str().to_owned()),
            key_schema_id: Some(key_schema.as_str().to_owned()),
            evidence_schema_id: None,
        },
        spec::ResourceClaimSpec::ExactTouchedSet {
            namespace,
            evidence_schema,
        } => TypedResourceClaimStatus {
            kind: "exact_touched_set".to_owned(),
            namespace: Some(namespace.as_str().to_owned()),
            key_schema_id: None,
            evidence_schema_id: Some(evidence_schema.as_str().to_owned()),
        },
        spec::ResourceClaimSpec::ManualOnly => TypedResourceClaimStatus {
            kind: "manual_only".to_owned(),
            namespace: None,
            key_schema_id: None,
            evidence_schema_id: None,
        },
    }
}

fn typed_resource_key_status(evidence: &events::ResourceKeyEvidence) -> TypedResourceKeyStatus {
    TypedResourceKeyStatus {
        namespace: evidence.namespace.as_str().to_owned(),
        key_schema_id: evidence.key_schema_id.as_str().to_owned(),
        key: evidence.key.as_str().to_owned(),
    }
}

fn typed_resource_touched_set_status(
    evidence: &events::ResourceTouchedSetEvidence,
) -> TypedResourceTouchedSetStatus {
    TypedResourceTouchedSetStatus {
        namespace: evidence.namespace.as_str().to_owned(),
        evidence_schema_id: evidence.evidence_schema_id.as_str().to_owned(),
        evidence_hash: evidence.evidence_hash.as_str().to_owned(),
        evidence_artifact_id: evidence.evidence_artifact_id.as_str().to_owned(),
    }
}

fn typed_resource_lane_holder(
    lane_key: &store::ResourceLaneKey,
    lane: &store::ResourceLaneProjection,
) -> TypedResourceLaneHolderStatus {
    let (holding_ledger_purpose, holding_forward_ledger_key) =
        typed_ledger_purpose_status(&lane.ledger_purpose);
    TypedResourceLaneHolderStatus {
        namespace: lane_key.namespace.as_str().to_owned(),
        key: lane_key.key.as_str().to_owned(),
        holding_run_id: lane.holder.run_id.as_str().to_owned(),
        holding_ledger_key: lane.holder.ledger_key.as_str().to_owned(),
        holding_ledger_purpose,
        holding_forward_ledger_key,
        holding_node_id: lane.node_id.as_str().to_owned(),
        holding_attempt_id: lane.attempt_id.as_str().to_owned(),
        invocation_epoch: lane.invocation_epoch,
    }
}

fn typed_ledger_purpose_status(
    purpose: &events::SideEffectLedgerPurpose,
) -> (String, Option<String>) {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => ("forward".to_owned(), None),
        events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } => (
            "remediation".to_owned(),
            Some(forward_ledger_key.as_str().to_owned()),
        ),
    }
}

fn certified_node<'a>(
    certified_spec: &'a spec::TypedExecutionSpec,
    node_id: &mfm_ids::NodeId,
) -> Option<&'a spec::NodeSpec> {
    certified_spec
        .nodes
        .iter()
        .chain(certified_spec.remediations.values())
        .find(|node| &node.node_id == node_id)
}

fn typed_terminal_resolution(
    outcome: &events::RunCompletionOutcome,
) -> TypedTerminalResolutionStatus {
    TypedTerminalResolutionStatus {
        outcome: store::codec::run_completion_outcome_str(outcome).to_owned(),
        claim: store::codec::run_completion_claim_str(outcome).to_owned(),
    }
}

fn manual_block_reason_str(reason: store::ManualBlockReason) -> String {
    match reason {
        store::ManualBlockReason::PolicyManualResolution => "policy_manual_resolution",
        store::ManualBlockReason::ForwardAmbiguous => "forward_ambiguous",
        store::ManualBlockReason::RemediationFailed => "remediation_failed",
        store::ManualBlockReason::RemediationAmbiguous => "remediation_ambiguous",
    }
    .to_owned()
}

fn forward_classification_str(classification: store::ForwardLedgerClassification) -> String {
    match classification {
        store::ForwardLedgerClassification::Pending => "pending",
        store::ForwardLedgerClassification::NothingOwed => "nothing_owed",
        store::ForwardLedgerClassification::Owed => "owed",
        store::ForwardLedgerClassification::Unresolvable => "unresolvable",
    }
    .to_owned()
}

fn side_effect_phase_str(phase: &store::SideEffectPhase) -> String {
    phase.as_str().to_owned()
}

fn run_admitted_spec_hash(stream: &[store::KernelEventEnvelope]) -> Result<SpecHash, AppError> {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload.spec_hash.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            AppError::new(
                ErrorClass::Internal,
                "RunAdmittedMissing",
                "typed run stream is missing RunAdmitted evidence",
            )
        })
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
