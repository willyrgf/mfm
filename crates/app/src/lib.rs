#![warn(missing_docs)]
//! Typed application assembly for certified MFM runs.
//!
//! `mfm-app` is the typed boundary used by binaries and process adapters. Its published
//! objective uses one exact entry-point id and one target-keyed portfolio selection; this crate
//! resolves the current target, plans, certifies, stages launch material, and wires typed services
//! for start, resume, replay, and public-output rendering.
//!
//! Production binaries should construct run services through the Postgres-backed factory exported by
//! this crate, while tests can use explicit test-support stores.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mfm_artifact_capabilities::ArtifactReadProvider;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{ProviderDiagnosticCode, ProviderDiagnosticValue};
use mfm_certify::{CertificationRegistry, CertifiedTypedSpec};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{EvmNetworkId, EvmSourcePolicyId, EvmSourceRef};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, EventId, RunId, SchemaId, SeedId, SemanticTypeId,
    SpecHash, StoreScopeId,
};
use mfm_replay::v1::{ReplayBroker, ReplayReadAuthority, RetainedSourceFactReplayEvent};
use mfm_runtime::{
    CertifiedRuntimeSpec, ManualResolutionEvidenceArtifact, ManualResolutionRequest,
    RunLaunchArtifact, RunLaunchEvidence, RunLaunchSeedCell, RuntimeDiagnostic, SchedulerStatus,
    SerialTypedScheduler, VerifiedRunHistoryView,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_store::v1::RunEventStore;
use serde::{Deserialize, Serialize};
use serde_json::map::Entry;
use serde_json::{Map, Value};

pub use mfm_runtime::ErasedRunnerRegistry;
pub use mfm_storage_postgres::{PostgresSchema, PostgresStore};

pub use public_facts::{
    is_public_fact_query_parameter_error, parse_public_fact_predicates, FactCatalogService,
    FactPublicQueryService, FactPublicRefResolver, PublicFactDescriptorRef,
    PublicFactDescriptorSummary, PublicFactExplain, PublicFactFieldSummary, PublicFactFieldValue,
    PublicFactKindSummary, PublicFactOrderingSummary, PublicFactOrderingTermSummary,
    PublicFactPredicate, PublicFactQueryExecution, PublicFactQueryExecutor, PublicFactQueryFuture,
    PublicFactQueryPage, PublicFactQueryRequest, PublicFactQuerySelector, PublicFactRef,
    PublicFactRefId, PublicFactScalarValue, PublicFactShapeSelector,
};

#[cfg(any(test, feature = "test-support"))]
pub use public_facts::{
    assert_public_fact_json_redacts_private_tokens_for_test, PublicFactVisibilityFixtureForTest,
};

#[cfg(test)]
pub(crate) use public_facts::{public_ref_id, query_public_facts, AppFactQueryRow};

mod btc_collector;
mod config_setup;
mod entry_point;
mod evm_collector;
mod fact_index;
mod live_transports;
mod portfolio_snapshot;
mod public_facts;
mod replay_verifiers;
#[path = "responses.rs"]
mod responses;
pub use self::responses::*;
#[path = "status.rs"]
mod status;
pub use self::status::*;
#[path = "launch_artifacts.rs"]
mod launch_artifacts;
use self::launch_artifacts::*;
#[path = "services.rs"]
mod services;

use self::services::VerifiedRunReadContext;
pub use self::services::{RunReadServices, RunServices};

use live_transports::{LiveTransportRuntime, RuntimeConfigLoader};

pub use config_setup::{
    export_setup_target, import_setup_toml, list_setup_targets, SetupConfigPublication,
    SetupConfigPublicationStatus, MAX_SETUP_FILE_BYTES,
};
pub use entry_point::{entry_point_ids, prepare_entry_point_run_launch};

#[path = "errors.rs"]
mod errors;
pub use self::errors::{AppError, ErrorClass, PublicSafeMessage};

/// Environment variable that selects the live runtime config file.
pub const MFM_RUNTIME_CONFIG_FILE: &str = "MFM_RUNTIME_CONFIG_FILE";

const MANAGED_FACT_RECORD_CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.runtime.managed-fact-record.v1";

/// Shared observability configuration used by typed binaries.
pub mod observability;

/// Builds live typed async app services with explicit certification.
pub fn make_run_services<S, A>(
    runners: ErasedRunnerRegistry,
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
) -> RunServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    let runtime_artifacts = Arc::new(artifacts.clone());
    RunServices::new_with_certification_registry(
        SerialTypedScheduler::new(runners, runtime_artifacts),
        store,
        artifacts,
        certification_registry,
    )
}

/// Builds evidence-only async app services with explicit certification.
pub fn make_run_read_services<S, A>(
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
) -> RunReadServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    RunReadServices::new_with_certification_registry(store, artifacts, certification_registry)
}

/// Production typed run services backed by the Postgres run store.
pub type ProductionRunServices = RunServices<PostgresStore, PostgresStore>;

/// Production evidence-only run services backed by the Postgres run store.
pub type ProductionRunReadServices = RunReadServices<PostgresStore, PostgresStore>;

/// Production public fact query service backed by the Postgres fact query executor.
pub type ProductionFactPublicQueryService = FactPublicQueryService<PostgresStore>;

/// Connects and validates the production Postgres run store.
pub async fn connect_production_store(
    database_url: Option<&str>,
) -> Result<PostgresStore, AppError> {
    let database_url = production_database_url(database_url)?;
    Ok(PostgresStore::connect(&database_url).await?)
}

/// Connects the production store-backed public-fact query service.
pub async fn connect_production_fact_public_query_service(
    database_url: Option<&str>,
) -> Result<ProductionFactPublicQueryService, AppError> {
    let store = connect_production_store(database_url).await?;
    let projection = store
        .fact_projection_snapshot()
        .await
        .map_err(async_app_store_error)?;
    let catalog = FactCatalogService::from_retained_public_projection(&store, &projection).await?;
    FactPublicQueryService::new(catalog, store)
}

fn production_database_url(database_url: Option<&str>) -> Result<String, AppError> {
    match database_url {
        Some(database_url) => Ok(database_url.to_owned()),
        None => std::env::var("DATABASE_URL").map_err(|_| {
            AppError::new(
                ErrorClass::BadRequest,
                "MissingDatabaseUrl",
                "Missing DATABASE_URL (or pass --database-url)",
            )
        }),
    }
}

/// Builds production typed run services backed by the Postgres run store.
pub async fn connect_production_run_services(
    database_url: Option<&str>,
    runtime_config_path: Option<&Path>,
) -> Result<ProductionRunServices, AppError> {
    let store = connect_production_store(database_url).await?;
    // Portfolio SelectHoldings and BTC collectors require the Postgres fact-index provider.
    let fact_index = production_fact_index_read_provider(store.clone());
    let runners =
        production_runner_registry(Arc::new(store.clone()), fact_index, runtime_config_path)?;
    let certification_registry = production_certification_registry()?;
    Ok(make_run_services(
        runners,
        store.clone(),
        store,
        certification_registry,
    ))
}

/// Builds production evidence-only run services backed by the Postgres run store.
pub async fn connect_production_run_read_services(
    database_url: Option<&str>,
) -> Result<ProductionRunReadServices, AppError> {
    let store = connect_production_store(database_url).await?;
    let certification_registry = production_certification_registry()?;
    Ok(make_run_read_services(
        store.clone(),
        store.clone(),
        certification_registry,
    ))
}

/// Builds the production typed runner registry for this process.
///
/// Framework public-output render nodes are resolved by `mfm-runtime` as built-ins. Domain runners
/// register here as certified typed descriptor bindings. Portfolio snapshots and BTC collectors require
/// an explicit Platform/Control [`mfm_fact_capabilities::FactIndexReadProvider`] — production wiring
/// must supply the Postgres implementation from [`production_fact_index_read_provider`].
pub fn production_runner_registry(
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn mfm_fact_capabilities::FactIndexReadProvider>,
    runtime_config_path: Option<&Path>,
) -> Result<ErasedRunnerRegistry, AppError> {
    let runtime_config = Arc::new(LiveTransportRuntime::new(
        RuntimeConfigLoader::from_path_or_env(runtime_config_path),
    ));
    let mut registry = ErasedRunnerRegistry::new();
    registry.register_capability_spec::<mfm_fact_capabilities::FactIndexReadCapability>(
        mfm_runtime::CapabilityImplementationId::new(fact_index.implementation_id())?,
    )?;
    registry.register_capability_spec::<mfm_fact_capabilities::FactRecordCapability>(
        mfm_runtime::CapabilityImplementationId::new(
            MANAGED_FACT_RECORD_CAPABILITY_IMPLEMENTATION_ID,
        )?,
    )?;
    let portfolio_capabilities = mfm_adapters_portfolio::PortfolioRunnerCapabilities::new(
        artifacts.clone(),
        fact_index.clone(),
    );
    mfm_adapters_portfolio::register_portfolio_runners(&mut registry, portfolio_capabilities)?;
    portfolio_snapshot::register_portfolio_snapshot_runners(&mut registry, artifacts.clone())?;
    btc_collector::register_btc_collector_runners(
        &mut registry,
        artifacts.clone(),
        fact_index,
        runtime_config.clone(),
    )?;
    evm_collector::register_evm_collector_runners(&mut registry, artifacts, runtime_config)?;
    mfm_transports_proof::register_deterministic_proof_runners(&mut registry)?;
    Ok(registry)
}

/// Builds the production Postgres Platform/Control fact-index provider.
pub use fact_index::production_fact_index_read_provider;
/// Platform/Control fact-index capability used by portfolio and BTC collector runners.
pub use mfm_fact_capabilities::FactIndexReadProvider;

/// Projection-backed in-memory fact-index for store-backed tests and process assembly fixtures.
#[cfg(any(test, feature = "test-support"))]
pub use fact_index::ProjectionFactIndexProvider;

/// Builds an adapter-facing artifact read provider from a retained artifact reader.
pub fn artifact_read_provider_from_retained<A>(artifacts: A) -> Arc<dyn ArtifactReadProvider>
where
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    Arc::new(RetainedArtifactReadAdapter { artifacts })
}

#[derive(Clone)]
struct RetainedArtifactReadAdapter<A> {
    artifacts: A,
}

impl<A> ArtifactReadProvider for RetainedArtifactReadAdapter<A>
where
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    fn read_artifact<'a>(
        &'a self,
        request: &'a mfm_artifact_capabilities::ArtifactReadRequest,
    ) -> mfm_artifact_capabilities::ArtifactReadFuture<'a> {
        Box::pin(async move {
            let requirement = store::EventArtifactRequirement {
                source: store::EventArtifactReferenceSource::ArtifactReferenced,
                artifact_id: request.artifact_id().clone(),
                evidence_hash: request.evidence_hash().clone(),
                digest: request.digest().cloned(),
                byte_len: request.byte_len(),
                media_type: request.media_type().cloned(),
                schema_id: request.schema_id().cloned(),
                semantic_type_id: request.semantic_type_id().cloned(),
                producer_node_id: request.producer_node_id().cloned(),
                producer_seed_id: request.producer_seed_id().cloned(),
                artifact_role: request.artifact_role(),
            };
            let artifact = self
                .artifacts
                .read_retained_artifact(&requirement)
                .await
                .map_err(capability_artifact_error_from_store)?;
            let evidence =
                mfm_artifact_capabilities::ArtifactEvidenceRef::from(artifact.evidence().clone());
            mfm_artifact_capabilities::VerifiedArtifactBytes::new(
                artifact.into_bytes(),
                evidence,
                request,
            )
        })
    }
}

fn capability_artifact_error_from_store(
    error: store::StoreError,
) -> mfm_artifact_capabilities::ArtifactReadError {
    match error {
        store::StoreError::MissingArtifact { artifact_id } => {
            mfm_artifact_capabilities::ArtifactReadError::NotFound {
                artifact_id: Box::new(artifact_id),
            }
        }
        store::StoreError::ArtifactEvidenceMismatch { artifact_id, field } => {
            mfm_artifact_capabilities::ArtifactReadError::EvidenceMismatch {
                artifact_id: Box::new(artifact_id),
                field,
            }
        }
        error => mfm_artifact_capabilities::ArtifactReadError::redacted_backend_failure(error),
    }
}

/// Builds the trusted production certification registry for typed spec certification and replay verification.
pub fn production_certification_registry() -> Result<CertificationRegistry, AppError> {
    let mut registry = CertificationRegistry::new();
    mfm_op_btc_collectors::register_btc_collectors_certification_descriptors(&mut registry)?;
    registry.register_fact_type::<mfm_op_btc_collectors::BtcChainHeadFact>()?;
    registry.register_fact_type::<mfm_op_btc_collectors::CollectorCheckpointFact>()?;
    registry.register_fact_type::<mfm_op_btc_collectors::BtcAddressBalanceSnapshotFact>()?;
    mfm_op_evm_collectors::register_evm_collectors_certification_descriptors(&mut registry)?;
    registry.register_fact_type::<mfm_op_evm_collectors::EvmAddressNativeBalanceSnapshotFact>()?;
    registry.register_fact_type::<mfm_op_evm_collectors::EvmAddressErc20BalanceSnapshotFact>()?;
    mfm_op_portfolio_snapshot::register_portfolio_snapshot_certification_descriptors(
        &mut registry,
    )?;
    mfm_op_proof::register_proof_certification_descriptors(&mut registry)?;
    Ok(registry)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriveStatus {
    Observed,
    Scheduler(SchedulerStatus),
    ExecutionClaimBusy,
    ExecutionClaimLost,
}

impl DriveStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Scheduler(status) => scheduler_status_str(status),
            Self::ExecutionClaimBusy => "execution_claim_busy",
            Self::ExecutionClaimLost => "execution_claim_lost",
        }
    }
}

impl From<SchedulerStatus> for DriveStatus {
    fn from(status: SchedulerStatus) -> Self {
        Self::Scheduler(status)
    }
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

/// Raw caller material identifying one intended invocation of certified work.
///
/// The raw key is intentionally not exposed after construction. Only its digest may be recorded in
/// run identity material.
#[derive(Clone, PartialEq, Eq)]
pub struct InvocationKey(String);

impl InvocationKey {
    /// Creates a checked invocation key from the exact caller-supplied UTF-8 string.
    pub fn new(value: impl Into<String>) -> Result<Self, AppError> {
        let value = value.into();
        events::InvocationKeyMaterialV1::new(&value).map_err(invocation_key_error)?;
        Ok(Self(value))
    }

    /// Returns the domain-separated digest used by `RunIdentityMaterialV1`.
    pub fn digest(&self) -> Result<ContentDigest, AppError> {
        events::InvocationKeyMaterialV1::new(&self.0)
            .and_then(|material| material.digest())
            .map_err(invocation_key_error)
    }

    fn mint() -> Result<Self, AppError> {
        Self::new(format!("mfm.invocation_key.v1:{}", uuid::Uuid::new_v4()))
    }
}

impl fmt::Debug for InvocationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("InvocationKey").field(&"<redacted>").finish()
    }
}

fn invocation_key_error(_error: mfm_events::EventError) -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        "InvocationKeyInvalid",
        "Invocation key must be non-empty and at most 1024 bytes",
    )
}

/// Request to start a certified typed run.
#[derive(Debug, Clone)]
pub struct RunLaunchRequest {
    /// Certifier-backed typed spec authority.
    pub certified_spec: CertifiedTypedSpec,
    /// Derived run id to bind.
    pub run_id: RunId,
    /// Store-owned and certified identity material used to derive `run_id`.
    pub identity_material: events::RunIdentityMaterialV1,
    /// Launch material that runtime middleware stages and admits with the admission commit.
    pub evidence: RunLaunchEvidence,
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
}

/// Config bytes supplied to a typed run start request.
#[derive(Debug, Clone)]
pub(crate) struct RunLaunchConfigArtifact {
    /// Certified config schema id.
    pub(crate) schema_id: SchemaId,
    /// Canonical config artifact bytes.
    pub(crate) bytes: Vec<u8>,
    /// Config artifact media type.
    pub(crate) media_type: spec::MediaType,
}

/// Seed bytes supplied to a typed run start request.
#[derive(Debug, Clone)]
pub(crate) struct RunLaunchSeedArtifact {
    /// Seed id from the certified spec.
    pub(crate) seed_id: SeedId,
    /// Canonical seed value bytes.
    pub(crate) bytes: Vec<u8>,
    /// Persisted seed artifact media type.
    pub(crate) media_type: spec::MediaType,
}

fn verify_replay_diagnostic(
    expected: Option<&events::RedactedJson>,
    diagnostic: Option<&RuntimeDiagnostic>,
) -> Result<(), AppError> {
    match (expected, diagnostic) {
        (Some(_), None) => Err(replay_diagnostic_error()),
        (Some(expected), diagnostic) => {
            let diagnostic = diagnostic.ok_or_else(replay_diagnostic_error)?;
            let digest = canonical_value_digest(&diagnostic.public_details_json())?;
            if digest != expected.content_digest {
                return Err(replay_diagnostic_error());
            }
            validate_replay_diagnostic(diagnostic)?;
            Ok(())
        }
        (None, None) => Ok(()),
        (None, Some(_)) => Err(replay_diagnostic_error()),
    }
}

#[cfg(test)]
fn verify_replay_diagnostic_json(
    expected: Option<&events::RedactedJson>,
    value: &Value,
) -> Result<(), AppError> {
    let diagnostic = match value {
        Value::Null => None,
        value => Some(RuntimeDiagnostic::from_json(value).map_err(|_| replay_diagnostic_error())?),
    };
    verify_replay_diagnostic(expected, diagnostic.as_ref())
}

fn validate_replay_diagnostic(diagnostic: &RuntimeDiagnostic) -> Result<(), AppError> {
    let RuntimeDiagnostic::Provider(diagnostic) = diagnostic;
    if diagnostic.provider_family().as_str() == "evm"
        && diagnostic.code() == ProviderDiagnosticCode::SourceMismatch
    {
        validate_evm_source_mismatch_diagnostic(diagnostic)?;
    }
    Ok(())
}

fn diagnostic_artifact_requirement(
    reference: &events::ArtifactEvidenceRef,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: reference.artifact_id.clone(),
        evidence_hash: reference.evidence_hash.clone(),
        digest: Some(reference.content_digest.clone()),
        byte_len: Some(reference.byte_len),
        media_type: Some(reference.media_type.clone()),
        schema_id: Some(reference.schema_id.clone()),
        semantic_type_id: reference.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(reference.role),
    }
}

fn validate_evm_source_mismatch_diagnostic(
    diagnostic: &mfm_capabilities::RedactedProviderDiagnostic,
) -> Result<(), AppError> {
    const FIELDS: [&str; 5] = [
        "network_id",
        "expected_chain_id",
        "observed_chain_id",
        "source_ref",
        "policy_id",
    ];
    if diagnostic.fields().len() != FIELDS.len()
        || FIELDS
            .iter()
            .any(|field| !diagnostic.fields().keys().any(|key| key.as_str() == *field))
    {
        return Err(replay_diagnostic_error());
    }
    let field = |name: &str| {
        diagnostic
            .fields()
            .iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, value)| value)
            .ok_or_else(replay_diagnostic_error)
    };
    let network_id = match field("network_id")? {
        ProviderDiagnosticValue::Id(value) => value.as_str(),
        _ => return Err(replay_diagnostic_error()),
    };
    let expected_chain_id = match field("expected_chain_id")? {
        ProviderDiagnosticValue::U64(value) => *value,
        _ => return Err(replay_diagnostic_error()),
    };
    let observed_chain_id = match field("observed_chain_id")? {
        ProviderDiagnosticValue::U64(value) => *value,
        _ => return Err(replay_diagnostic_error()),
    };
    let source_ref = match field("source_ref")? {
        ProviderDiagnosticValue::Id(value) => value.as_str(),
        _ => return Err(replay_diagnostic_error()),
    };
    let policy_id = match field("policy_id")? {
        ProviderDiagnosticValue::Id(value) => value.as_str(),
        _ => return Err(replay_diagnostic_error()),
    };
    EvmNetworkId::new(network_id).map_err(|_| replay_diagnostic_error())?;
    EvmSourceRef::new(source_ref).map_err(|_| replay_diagnostic_error())?;
    EvmSourcePolicyId::new(policy_id).map_err(|_| replay_diagnostic_error())?;
    if expected_chain_id == 0 || expected_chain_id == observed_chain_id {
        return Err(replay_diagnostic_error());
    }
    Ok(())
}

fn canonical_value_digest(value: &serde_json::Value) -> Result<ContentDigest, AppError> {
    let json = serde_json::to_string(value).map_err(|_| replay_diagnostic_error())?;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| replay_diagnostic_error())?;
    Ok(canonical.content_digest())
}

fn replay_diagnostic_error() -> AppError {
    AppError::backend(
        ErrorClass::Internal,
        "ReplayDiagnosticInvalid",
        "Replay diagnostic evidence failed verification",
    )
}

/// Loads and verifies the certified spec artifact bound by a typed run stream.
pub async fn load_certified_spec_for_run(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    registry: &CertificationRegistry,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<CertifiedTypedSpec, AppError> {
    let run_admitted = run_admitted_payload(run_id, stream)?;
    let spec_artifact = artifacts
        .read_retained_artifact(&run_artifact_requirement(
            store::EventArtifactReferenceSource::RunSpec,
            &run_admitted.spec_artifact,
            events::ArtifactRole::TypedExecutionSpec,
        ))
        .await?;
    let spec_evidence = spec_artifact.evidence().clone();
    validate_spec_artifact_evidence(run_admitted, &spec_evidence)?;
    let spec_bytes = spec_artifact.into_bytes();
    let certificate_artifact = artifacts
        .read_retained_artifact(&run_artifact_requirement(
            store::EventArtifactReferenceSource::RunCertificate,
            &run_admitted.certificate_artifact,
            events::ArtifactRole::TypedSpecCertificate,
        ))
        .await?;
    let certificate_evidence = certificate_artifact.evidence().clone();
    validate_certificate_artifact_evidence(run_admitted, &certificate_evidence)?;
    let certificate_bytes = certificate_artifact.into_bytes();
    let certified = mfm_certify::verify_persisted_spec_certificate_with_trusted_registry(
        &spec_bytes,
        &certificate_bytes,
        registry,
    )?;
    validate_run_admitted_matches_spec(run_admitted, certified.envelope())?;
    Ok(certified)
}

async fn stored_launch_evidence_from_run_admitted(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    run_admitted: &events::RunAdmitted,
) -> Result<RunLaunchEvidence, AppError> {
    let spec_artifact = stored_run_launch_artifact(
        artifacts,
        run_artifact_requirement(
            store::EventArtifactReferenceSource::RunSpec,
            &run_admitted.spec_artifact,
            events::ArtifactRole::TypedExecutionSpec,
        ),
        |evidence| validate_spec_artifact_evidence(run_admitted, evidence),
    )
    .await?;
    let certificate_artifact = stored_run_launch_artifact(
        artifacts,
        run_artifact_requirement(
            store::EventArtifactReferenceSource::RunCertificate,
            &run_admitted.certificate_artifact,
            events::ArtifactRole::TypedSpecCertificate,
        ),
        |evidence| validate_certificate_artifact_evidence(run_admitted, evidence),
    )
    .await?;
    let mut config_artifacts = Vec::with_capacity(run_admitted.config_artifacts.len());
    for config in &run_admitted.config_artifacts {
        config_artifacts.push(
            stored_run_launch_artifact(
                artifacts,
                run_artifact_requirement(
                    store::EventArtifactReferenceSource::RunConfig,
                    config,
                    events::ArtifactRole::TypedConfig,
                ),
                |_| Ok(()),
            )
            .await?,
        );
    }
    let mut fact_descriptor_artifacts =
        Vec::with_capacity(run_admitted.fact_descriptor_artifacts.len());
    for descriptor in &run_admitted.fact_descriptor_artifacts {
        fact_descriptor_artifacts.push(
            stored_run_launch_artifact(
                artifacts,
                run_artifact_requirement(
                    store::EventArtifactReferenceSource::FactDescriptor,
                    descriptor,
                    events::ArtifactRole::FactDescriptor,
                ),
                |_| Ok(()),
            )
            .await?,
        );
    }
    let mut seed_cells = Vec::with_capacity(run_admitted.seed_cells.len());
    for cell in &run_admitted.seed_cells {
        let artifact = artifacts
            .read_retained_artifact(&seed_cell_artifact_requirement(cell))
            .await?;
        let evidence = artifact.evidence().clone();
        validate_artifact_requirement_for_app(
            seed_cell_artifact_requirement(cell),
            &evidence,
            ErrorClass::Internal,
            "RunAdmittedSeedArtifactMismatch",
            "RunAdmitted seed artifact evidence does not match retained bytes",
        )?;
        seed_cells.push(RunLaunchSeedCell {
            bytes: artifact.into_bytes(),
            cell: cell.clone(),
        });
    }
    Ok(RunLaunchEvidence {
        entry_point: run_admitted.entry_point.clone(),
        spec_artifact,
        certificate_artifact,
        config_artifacts,
        fact_descriptor_artifacts,
        seed_cells,
    })
}

async fn stored_run_launch_artifact(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    requirement: store::EventArtifactRequirement,
    validate: impl FnOnce(&store::ArtifactEvidenceRef) -> Result<(), AppError>,
) -> Result<RunLaunchArtifact, AppError> {
    let artifact = artifacts.read_retained_artifact(&requirement).await?;
    let evidence = artifact.evidence().clone();
    validate_artifact_requirement_for_app(
        requirement,
        &evidence,
        ErrorClass::Internal,
        "RunAdmittedArtifactMismatch",
        "RunAdmitted artifact evidence does not match retained bytes",
    )?;
    validate(&evidence)?;
    Ok(RunLaunchArtifact {
        bytes: artifact.into_bytes(),
        evidence,
    })
}

async fn load_runtime_spec_for_run(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    registry: &CertificationRegistry,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<CertifiedRuntimeSpec, AppError> {
    let certified = load_certified_spec_for_run(artifacts, registry, run_id, stream).await?;
    Ok(CertifiedRuntimeSpec::new(certified)?)
}

async fn replay_read_authority_for_run_with_retained_source_facts<S, A>(
    store: &S,
    artifacts: &A,
    runtime_spec: &CertifiedRuntimeSpec,
    verified_view: &VerifiedRunHistoryView,
) -> Result<ReplayReadAuthority, AppError>
where
    S: store::RunEventStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    let source_fact_events =
        retained_source_fact_events_from_query_evidence(store, artifacts, verified_view.events())
            .await?;
    Ok(
        ReplayReadAuthority::from_verified_run_history_view_with_source_facts_and_artifacts(
            runtime_spec,
            verified_view,
            source_fact_events,
            Vec::new(),
        )?,
    )
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

/// Prepares a certified typed run launch from a program draft for integration tests.
///
/// This helper is compiled only with `test-support`. It uses the same launch-material preparation
/// path as production app services after the caller has supplied an already-built typed program
/// draft and matching root seed material.
#[cfg(any(test, feature = "test-support"))]
pub fn prepare_typed_program_run_launch_for_test(
    draft: mfm_program::TypedProgramDraft,
    seed_material: BTreeMap<SeedId, PlainCanonicalJsonBytes>,
    certification_registry: &CertificationRegistry,
    store_scope_id: StoreScopeId,
    invocation_key: Option<InvocationKey>,
) -> Result<RunLaunchRequest, AppError> {
    let plan =
        mfm_program::TypedProgramLaunchPlan::from_draft_and_seed_material(draft, seed_material)
            .map_err(|_| {
                AppError::backend(
                    ErrorClass::BadRequest,
                    "TypedProgramLaunchPlanInvalid",
                    "typed program launch material is invalid",
                )
            })?;
    let (certified_spec, scoped_registry, config_inputs, seed_inputs) =
        certify_launch_plan(&plan, certification_registry)?;
    let invocation_key_digest = invocation_key_digest_or_mint(invocation_key.as_ref())?;
    prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: &scoped_registry,
            store_scope_id,
            invocation_key_digest,
            entry_point_evidence: events::EntryPointLaunchEvidence::new(
                "mfm.test/typed_program_internal_test@1",
                Vec::new(),
            )
            .map_err(|_| {
                entry_point_launch_internal_error(
                    "EntryPointLaunchEvidenceInvalid",
                    "entry-point launch evidence is invalid",
                )
            })?,
        },
        config_inputs,
        seed_inputs,
    )
}

fn certify_launch_plan(
    plan: &mfm_program::TypedProgramLaunchPlan,
    certification_registry: &CertificationRegistry,
) -> Result<
    (
        CertifiedTypedSpec,
        CertificationRegistry,
        Vec<RunLaunchConfigArtifact>,
        Vec<RunLaunchSeedArtifact>,
    ),
    AppError,
> {
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
    let scoped_registry = certification_registry
        .scoped_for_spec(lowered.spec())
        .map_err(entry_point_certification_error)?;
    let certified_spec = mfm_certify::certify_typed_spec(lowered, &scoped_registry)
        .map_err(entry_point_certification_error)?;
    config_inputs.extend(framework_config_launch_artifacts_for_spec(
        &certified_spec.envelope().spec,
    )?);
    Ok((certified_spec, scoped_registry, config_inputs, seed_inputs))
}

fn entry_point_certification_error(_error: mfm_certify::CertifyError) -> AppError {
    AppError::backend(
        ErrorClass::BadRequest,
        "EntryPointCertificationFailed",
        "Entry-point planned spec failed certification",
    )
}

fn entry_point_launch_internal_error(code: &'static str, message: &'static str) -> AppError {
    AppError::backend(ErrorClass::Internal, code, message)
}

/// Certifier-backed typed spec authority plus launch metadata for a typed run start.
pub(crate) struct CertifiedRunLaunchInput<'a> {
    /// Certifier-backed typed spec authority.
    pub(crate) certified_spec: CertifiedTypedSpec,
    /// Trusted registry used to validate launch config artifacts.
    pub(crate) registry: &'a CertificationRegistry,
    /// Store-owned deployment scope.
    pub(crate) store_scope_id: StoreScopeId,
    /// Required invocation key digest.
    pub(crate) invocation_key_digest: ContentDigest,
    /// Public entry-point operation evidence selected by app assembly.
    pub(crate) entry_point_evidence: events::EntryPointLaunchEvidence,
}

/// Builds a typed run-start request from certifier-backed typed spec authority and launch inputs.
fn prepare_certified_run_launch(
    input: CertifiedRunLaunchInput<'_>,
    config_inputs: Vec<RunLaunchConfigArtifact>,
    seed_inputs: Vec<RunLaunchSeedArtifact>,
) -> Result<RunLaunchRequest, AppError> {
    let runtime_spec = CertifiedRuntimeSpec::new(input.certified_spec.clone())?;
    let spec_artifact = certified_spec_launch_artifact(&runtime_spec)?;
    let certificate_artifact = certified_spec_certificate_launch_artifact(&runtime_spec)?;
    let config_artifacts =
        config_launch_artifacts_for_spec(&runtime_spec, input.registry, config_inputs)?;
    let fact_descriptor_artifacts =
        fact_descriptor_launch_artifacts_for_spec(&runtime_spec, input.registry)?;
    let seed_cells = seed_launch_cells_for_spec(&runtime_spec, seed_inputs)?;
    let identity_material = events::RunIdentityMaterialV1 {
        certified_spec_hash: runtime_spec.spec_hash().clone(),
        store_scope_id: input.store_scope_id,
        invocation_key_digest: input.invocation_key_digest,
    };
    let run_id = identity_material.derive_run_id().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "RunIdentityMaterialInvalid",
            "Run identity material is invalid",
        )
    })?;
    Ok(RunLaunchRequest {
        certified_spec: input.certified_spec,
        run_id,
        identity_material,
        evidence: RunLaunchEvidence {
            entry_point: input.entry_point_evidence,
            spec_artifact,
            certificate_artifact,
            config_artifacts,
            fact_descriptor_artifacts,
            seed_cells,
        },
    })
}

fn invocation_key_digest_or_mint(key: Option<&InvocationKey>) -> Result<ContentDigest, AppError> {
    match key {
        Some(key) => key.digest(),
        None => InvocationKey::mint()?.digest(),
    }
}

fn run_identity_material_mismatch() -> AppError {
    AppError::backend(
        ErrorClass::Internal,
        "RunIdentityMaterialMismatch",
        "Run identity material does not match the store or certified spec",
    )
}

fn new_execution_claim_token() -> Result<store::AdmissionToken, AppError> {
    store::AdmissionToken::new(format!("mfm.execution_claim.v1:{}", uuid::Uuid::new_v4()))
        .map_err(AppError::from)
}

fn default_execution_claim_heartbeat_interval() -> Duration {
    Duration::from_secs(store::EXECUTION_CLAIM_HEARTBEAT_INTERVAL_SECS)
}

fn async_app_store_error(_error: impl fmt::Display) -> AppError {
    AppError::backend(
        ErrorClass::Conflict,
        "RunStoreRejected",
        "Run store rejected the requested operation",
    )
}

fn observation_app_store_error<E>(error: E) -> AppError
where
    E: store::StoreErrorInspection + fmt::Display,
{
    match error.as_store_error() {
        Some(store::StoreError::InvalidCursor { .. }) => AppError::new(
            ErrorClass::BadRequest,
            "InvalidCursor",
            "Run observation cursor is invalid",
        ),
        Some(store::StoreError::CursorExpired) => AppError::new(
            ErrorClass::BadRequest,
            "CursorExpired",
            "Run observation cursor has expired",
        ),
        Some(store::StoreError::LimitOutOfRange { .. }) => AppError::new(
            ErrorClass::BadRequest,
            "LimitOutOfRange",
            "Run observation limit is out of range",
        ),
        Some(store::StoreError::ObservationUnavailable { .. }) => AppError::backend(
            ErrorClass::Internal,
            "ObservationUnavailable",
            "Run observations are unavailable",
        ),
        _ => async_app_store_error(error),
    }
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

#[cfg(test)]
mod tests;
