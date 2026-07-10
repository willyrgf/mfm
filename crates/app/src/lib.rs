#![warn(missing_docs)]
//! Typed application assembly for certified MFM runs.
//!
//! `mfm-app` is the typed boundary used by binaries and process adapters. Callers select a
//! registered entry-point operation and authored config; this crate resolves,
//! plans, certifies, stages launch material, and wires typed services for start, resume, replay,
//! and public-output rendering.
//!
//! Production binaries should construct run services through the Postgres-backed factory exported by
//! this crate, while tests can use explicit test-support stores.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mfm_artifact_capabilities::ArtifactReadProvider;
use mfm_authored_config::AuthoredConfig;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySpec, ProviderDiagnosticCode, ProviderDiagnosticValue};
use mfm_certify::{CertificationRegistry, CertifiedTypedSpec};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{EvmNetworkId, EvmSourcePolicyId, EvmSourceRef};
use mfm_evm_contract_model::{
    ContractArtifactConfig, EvmContractContext, LifecycleArtifactEvidenceRef,
};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, EventId, RunId, SchemaId, SeedId, SemanticTypeId,
    SpecHash, StoreScopeId,
};
use mfm_replay::v1::{
    ReplayBroker, ReplayError, ReplayReadAuthority, RetainedSourceFactReplayEvent,
};
use mfm_runtime::{
    CertifiedRuntimeSpec, ManualResolutionEvidenceArtifact, ManualResolutionRequest,
    RunLaunchArtifact, RunLaunchEvidence, RunLaunchSeedCell, RuntimeDiagnostic, SchedulerStatus,
    SerialTypedScheduler, VerifiedRunHistoryView,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_store::v1::RunEventStore;
use mfm_values::MfmConfig;
use serde::{Deserialize, Serialize};
use serde_json::map::Entry;
use serde_json::{Map, Value};
use zeroize::{Zeroize, Zeroizing};

pub use mfm_runtime::ErasedRunnerRegistry;
pub use mfm_stream_store_postgres::PostgresRunStore as ProductionRunStore;
pub use mfm_stream_store_postgres::PostgresSchema as ProductionPostgresSchema;

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
pub use public_facts::public_fact_ref_id_from_projection_entry_for_test;
#[cfg(any(test, feature = "test-support"))]
pub use public_facts::{
    assert_public_fact_json_redacts_private_tokens_for_test, PublicFactVisibilityFixtureForTest,
};

#[cfg(test)]
pub(crate) use public_facts::{public_ref_id, query_public_facts, AppFactQueryRow};

mod btc_collector;
mod entry_point;
mod entry_points;
mod evm_collector;
mod evm_contracts;
mod fact_index;
mod live_transports;
mod public_facts;
mod replay_verifiers;

use live_transports::{LiveTransportRuntime, RuntimeConfigLoader};

pub use entry_point::{
    EntryPointOpId, EntryPointOpPlan, EntryPointOpRegistry, EntryPointOpResolveError,
    EntryPointPlannerAdapter, LaunchableOp, OpLaunchError, OpVersion, PublicOpName,
};

/// Environment variable that selects the live runtime config file.
pub const MFM_RUNTIME_CONFIG_FILE: &str = "MFM_RUNTIME_CONFIG_FILE";

/// Environment variable that selects the Ed25519 fact-query receipt signing-key file.
pub const MFM_FACT_RECEIPT_SIGNING_KEY_FILE: &str = "MFM_FACT_RECEIPT_SIGNING_KEY_FILE";

/// Store identity used when provisioning the first production fact-receipt trust root.
pub const PRODUCTION_FACT_RECEIPT_STORE_IDENTITY: &str = "mfm.postgres.fact_receipt";

/// Key identity used when provisioning the first production fact-receipt trust root.
pub const PRODUCTION_FACT_RECEIPT_KEY_ID: &str = "mfm.postgres.fact_receipt.v1";

const MANAGED_FACT_RECORD_CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.runtime.managed-fact-record.v1";

/// Shared observability configuration used by typed binaries.
pub mod observability;

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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
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
            | mfm_runtime::RuntimeError::ExecutionClaim(_)
            | mfm_runtime::RuntimeError::InputMaterialization(_)
            | mfm_runtime::RuntimeError::InvalidRunnerOutput(_)
            | mfm_runtime::RuntimeError::InvalidRunnerOutputFailure { .. }
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
    fn from(error: store::StoreError) -> Self {
        match error {
            store::StoreError::MissingArtifact { artifact_id } => Self::not_found(
                "ArtifactNotFound",
                format!("typed artifact {artifact_id} was not found"),
            ),
            store::StoreError::ArtifactReadFailed { .. } => Self::backend(
                ErrorClass::Internal,
                "ArtifactReadFailed",
                "Typed artifact bytes could not be loaded",
            ),
            store::StoreError::ArtifactEvidenceMismatch { .. } => Self::backend(
                ErrorClass::Internal,
                "ArtifactEvidenceMismatch",
                "Typed artifact evidence did not match the requested authority",
            ),
            _ => Self::backend(
                ErrorClass::Conflict,
                "RunStoreRejected",
                "Run store rejected the requested operation",
            ),
        }
    }
}

impl From<mfm_stream_store_postgres::PostgresStoreError> for AppError {
    fn from(error: mfm_stream_store_postgres::PostgresStoreError) -> Self {
        match error {
            mfm_stream_store_postgres::PostgresStoreError::Authority(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreAuthorityInvalid",
                "Run store authority could not be validated",
            ),
            mfm_stream_store_postgres::PostgresStoreError::Store(_) => Self::backend(
                ErrorClass::Conflict,
                "RunStoreRejected",
                "Run store rejected the requested operation",
            ),
            mfm_stream_store_postgres::PostgresStoreError::Database(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreUnavailable",
                "Run store is unavailable",
            ),
            mfm_stream_store_postgres::PostgresStoreError::Corruption(_) => Self::backend(
                ErrorClass::Internal,
                "RunStoreCorruption",
                "Run store returned invalid data",
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
            "CertifiedSpecVerificationFailed",
            "Certified typed spec verification failed",
        )
    }
}

impl From<mfm_facts::FactError> for AppError {
    fn from(_error: mfm_facts::FactError) -> Self {
        Self::backend(
            ErrorClass::BadRequest,
            "FactQueryInvalid",
            "Fact query input is invalid",
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

/// Builds live typed async app services with explicit certification and fact-query authority.
///
/// Pass `fact_query_receipt_trust_root = None` and `fact_query_authority_ready = false` when the
/// process does not verify or sign fact-query receipts.
pub fn make_run_services<S, A>(
    runners: ErasedRunnerRegistry,
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
    fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
    fact_query_authority_ready: bool,
) -> RunServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    let runtime_artifacts = Arc::new(artifacts.clone());
    RunServices::new_with_certification_registry_and_fact_query_authority(
        SerialTypedScheduler::new(runners, runtime_artifacts),
        store,
        artifacts,
        certification_registry,
        fact_query_receipt_trust_root,
        fact_query_authority_ready,
    )
}

/// Builds evidence-only async app services with explicit certification and optional fact-query
/// trust root for replay verification.
///
/// Pass `fact_query_receipt_trust_root = None` when the process will not verify fact-query
/// receipts.
pub fn make_run_read_services<S, A>(
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
    fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
) -> RunReadServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    RunReadServices::new_with_certification_registry_and_fact_query_authority(
        store,
        artifacts,
        certification_registry,
        fact_query_receipt_trust_root,
    )
}

/// Production typed run services backed by the Postgres run store.
pub type ProductionRunServices = RunServices<ProductionRunStore, ProductionRunStore>;

/// Production evidence-only run services backed by the Postgres run store.
pub type ProductionRunReadServices = RunReadServices<ProductionRunStore, ProductionRunStore>;

/// Production public fact query service backed by the Postgres fact query executor.
pub type ProductionFactPublicQueryService = FactPublicQueryService<ProductionRunStore>;

/// Connects the production Postgres run store with its optional fact-query signer.
///
/// The signer is loaded only when [`MFM_FACT_RECEIPT_SIGNING_KEY_FILE`] is configured. Runs that
/// do not use the fact-index capability remain usable without it; a fact-reading graph is
/// rejected before admission when the signer/root pair is unavailable.
pub async fn connect_production_run_store(
    database_url: Option<&str>,
) -> Result<ProductionRunStore, AppError> {
    let database_url = production_database_url(database_url)?;
    if configured_fact_receipt_signing_key_file().is_none() {
        return Ok(ProductionRunStore::connect(&database_url).await?);
    } else {
        let mut signing_key = load_fact_receipt_signing_key_from_env()?;
        let store = ProductionRunStore::connect_with_fact_receipt_signing_key_bytes(
            &database_url,
            *signing_key,
        )
        .await
        .map_err(fact_receipt_signer_store_error)?;
        signing_key.zeroize();
        Ok(store)
    }
}

/// Connects production evidence-only services without loading a fact-receipt signing key.
pub async fn connect_production_run_read_store(
    database_url: Option<&str>,
) -> Result<ProductionRunStore, AppError> {
    let database_url = production_database_url(database_url)?;
    Ok(ProductionRunStore::connect(&database_url).await?)
}

/// Connects the production authenticated public-fact query service.
pub async fn connect_production_fact_public_query_service(
    database_url: Option<&str>,
) -> Result<ProductionFactPublicQueryService, AppError> {
    let store = connect_production_run_store(database_url).await?;
    let projection = store
        .fact_projection_snapshot()
        .await
        .map_err(async_app_store_error)?;
    let catalog = FactCatalogService::from_retained_public_projection(&store, &projection).await?;
    FactPublicQueryService::new(catalog, store)
}

/// Provisions the immutable production fact-receipt trust root from the configured signing key.
///
/// Only the public verifying key is inserted into Postgres. If a root already exists, the
/// supplied key must match it exactly; the authority table is never updated or replaced.
pub async fn provision_production_fact_receipt_authority(
    database_url: Option<&str>,
) -> Result<(), AppError> {
    let database_url = production_database_url(database_url)?;
    let mut signing_key = load_fact_receipt_signing_key_from_env()?;
    let store_identity = mfm_facts::StoreIdentity::new(PRODUCTION_FACT_RECEIPT_STORE_IDENTITY)
        .map_err(|_| fact_receipt_authority_provisioning_error())?;
    let key_id = mfm_facts::StoreKeyId::new(PRODUCTION_FACT_RECEIPT_KEY_ID)
        .map_err(|_| fact_receipt_authority_provisioning_error())?;
    ProductionPostgresSchema::provision_fact_receipt_trust_root(
        &database_url,
        store_identity,
        key_id,
        *signing_key,
    )
    .await
    .map_err(|_| fact_receipt_authority_provisioning_error())?;
    signing_key.zeroize();
    Ok(())
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

fn configured_fact_receipt_signing_key_file() -> Option<PathBuf> {
    env::var_os(MFM_FACT_RECEIPT_SIGNING_KEY_FILE)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn load_fact_receipt_signing_key_from_env() -> Result<Zeroizing<[u8; 32]>, AppError> {
    let path = configured_fact_receipt_signing_key_file().ok_or_else(|| {
        AppError::new(
            ErrorClass::BadRequest,
            "MissingFactReceiptSigningKey",
            format!("Missing {MFM_FACT_RECEIPT_SIGNING_KEY_FILE}"),
        )
    })?;
    load_fact_receipt_signing_key_file(&path)
}

fn load_fact_receipt_signing_key_file(path: &Path) -> Result<Zeroizing<[u8; 32]>, AppError> {
    let bytes = Zeroizing::new(std::fs::read(path).map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "FactReceiptSigningKeyReadFailed",
            "Failed to read fact receipt signing key",
        )
    })?);
    decode_fact_receipt_signing_key_bytes(&bytes)
}

fn decode_fact_receipt_signing_key_bytes(bytes: &[u8]) -> Result<Zeroizing<[u8; 32]>, AppError> {
    if bytes.len() == 32 {
        let mut signing_key = [0_u8; 32];
        signing_key.copy_from_slice(bytes);
        return Ok(Zeroizing::new(signing_key));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_fact_receipt_signing_key())?;
    let hex = text
        .trim()
        .strip_prefix("0x")
        .unwrap_or_else(|| text.trim());
    if hex.len() != 64 {
        return Err(invalid_fact_receipt_signing_key());
    }
    let mut signing_key = Zeroizing::new([0_u8; 32]);
    for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let high = fact_receipt_hex_nibble(chunk[0])?;
        let low = fact_receipt_hex_nibble(chunk[1])?;
        signing_key[index] = (high << 4) | low;
    }
    Ok(signing_key)
}

fn fact_receipt_hex_nibble(byte: u8) -> Result<u8, AppError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(invalid_fact_receipt_signing_key()),
    }
}

fn invalid_fact_receipt_signing_key() -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        "FactReceiptSigningKeyInvalid",
        "Fact receipt signing key must be raw 32-byte Ed25519 material or 64 hex characters",
    )
}

fn fact_receipt_signer_store_error(
    error: mfm_stream_store_postgres::PostgresStoreError,
) -> AppError {
    match error {
        mfm_stream_store_postgres::PostgresStoreError::Store(
            store::StoreError::ReceiptAuthentication { .. },
        ) => AppError::backend(
            ErrorClass::BadRequest,
            "FactReceiptSignerInvalid",
            "Fact receipt signer does not match the run store trust root",
        ),
        error => AppError::from(error),
    }
}

fn fact_receipt_authority_provisioning_error() -> AppError {
    AppError::backend(
        ErrorClass::BadRequest,
        "FactReceiptAuthorityProvisioningFailed",
        "Fact receipt authority could not be provisioned",
    )
}

pub(crate) fn fact_query_execution_store_error(
    error: mfm_stream_store_postgres::PostgresStoreError,
) -> AppError {
    match error {
        mfm_stream_store_postgres::PostgresStoreError::Store(
            store::StoreError::ReceiptAuthentication { .. },
        ) => fact_receipt_authority_unavailable_error(),
        error => AppError::from(error),
    }
}

fn fact_receipt_authority_unavailable_error() -> AppError {
    AppError::backend(
        ErrorClass::BadRequest,
        "FactReceiptAuthorityUnavailable",
        "Fact receipt authority is required for this fact-reading workflow",
    )
}

fn runtime_spec_requires_fact_index(runtime_spec: &CertifiedRuntimeSpec) -> Result<bool, AppError> {
    let capability_kind = mfm_fact_capabilities::FactIndexReadCapability::kind().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "CapabilityIdentityInvalid",
            "Fact-index capability identity is invalid",
        )
    })?;
    let capability_version =
        mfm_fact_capabilities::FactIndexReadCapability::version().map_err(|_| {
            AppError::backend(
                ErrorClass::Internal,
                "CapabilityIdentityInvalid",
                "Fact-index capability version is invalid",
            )
        })?;
    Ok(runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(runtime_spec.spec().remediations.values())
        .any(|node| {
            node.capability_bindings
                .capabilities
                .iter()
                .any(|capability| {
                    capability.kind == capability_kind && capability.version == capability_version
                })
        }))
}

/// Builds production typed run services backed by the Postgres run store.
pub async fn connect_production_run_services(
    database_url: Option<&str>,
    runtime_config_path: Option<&Path>,
) -> Result<ProductionRunServices, AppError> {
    let store = connect_production_run_store(database_url).await?;
    // Portfolio SelectHoldings and BTC collectors require the Postgres fact-index provider.
    let fact_index = production_fact_index_read_provider(store.clone());
    let runners =
        production_runner_registry(Arc::new(store.clone()), fact_index, runtime_config_path)?;
    let certification_registry = production_certification_registry()?;
    let fact_query_receipt_trust_root = store.store_authority().fact_receipt_trust_root().cloned();
    let fact_query_authority_ready = store.fact_receipt_queries_ready();
    Ok(make_run_services(
        runners,
        store.clone(),
        store,
        certification_registry,
        fact_query_receipt_trust_root,
        fact_query_authority_ready,
    ))
}

/// Builds production evidence-only run services backed by the Postgres run store.
pub async fn connect_production_run_read_services(
    database_url: Option<&str>,
) -> Result<ProductionRunReadServices, AppError> {
    let store = connect_production_run_read_store(database_url).await?;
    let certification_registry = production_certification_registry()?;
    let fact_query_receipt_trust_root = store.store_authority().fact_receipt_trust_root().cloned();
    Ok(make_run_read_services(
        store.clone(),
        store.clone(),
        certification_registry,
        fact_query_receipt_trust_root,
    ))
}

/// Builds the production typed runner registry for this process.
///
/// Framework public-output render nodes are resolved by `mfm-runtime` as built-ins. Domain runners
/// register here as certified typed descriptor bindings. Portfolio report and BTC collectors require
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
    let source_run_registry = production_certification_registry()?;
    evm_contracts::register_contract_lifecycle_runners(
        &mut registry,
        artifacts.clone(),
        runtime_config.clone(),
        source_run_registry,
    )?;
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
    /// Store-owned deployment scope.
    pub store_scope_id: StoreScopeId,
    /// Optional caller material identifying one intended invocation.
    pub invocation_key: Option<InvocationKey>,
}

/// App-level evidence for a prepared entry-point op launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPointLaunchEvidence {
    /// Typed entry-point op id resolved from the registry.
    pub resolved_op_id: EntryPointOpId,
    /// Canonical digest of the entry-point registry used for resolution.
    pub entry_point_registry_digest: ContentDigest,
}

/// Prepared entry-point op launch material accepted by app runtime services.
#[derive(Debug, Clone)]
pub struct PreparedEntryPointRunLaunch {
    /// Runtime run launch request.
    pub request: RunLaunchRequest,
    /// Entry-point launch evidence produced by app assembly.
    pub evidence: EntryPointLaunchEvidence,
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

/// Stable semantic run mode for typed run responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunModeStatus {
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

impl RunModeStatus {
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

impl From<store::RunMode> for RunModeStatus {
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

impl fmt::Display for RunModeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_store_run_mode().as_str())
    }
}

/// Public saga status derived from certified policy and stream evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaStatus {
    /// Certified saga policy variant and manual authorization requirements.
    pub policy: SagaPolicyStatus,
    /// Derived per-forward-ledger obligations.
    pub obligations: Vec<SagaObligationStatus>,
    /// Resource claims and recorded evidence for all projected side-effect ledgers.
    pub resource_ledgers: Vec<ResourceLedgerStatus>,
    /// Active exclusive resource lane holders referenced by this run's live side-effect ledgers.
    pub resource_lanes: Vec<ResourceLaneHolderStatus>,
    /// Manual-block reason when the derived run mode is `manual_blocked`.
    pub manual_block_reason: Option<String>,
    /// Required manual authorization when an authorized decision can resolve the current block.
    pub required_manual_authorization: Option<ManualAuthorizationRequirements>,
    /// Terminal completion evidence, when the run has resolved.
    pub terminal_resolution: Option<TerminalResolutionStatus>,
}

/// Public certified saga policy summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaPolicyStatus {
    /// Certified policy variant.
    pub variant: String,
    /// Manual authorization requirements certified directly by the policy, when present.
    pub manual_authorization: Option<ManualAuthorizationRequirements>,
    /// Unresolved-remediation directive under compensating policy.
    pub on_remediation_unresolved: Option<String>,
}

/// Public manual authorization requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualAuthorizationRequirements {
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
pub struct SagaObligationStatus {
    /// Forward ledger key.
    pub forward_ledger_key: String,
    /// Current forward side-effect phase.
    pub forward_phase: String,
    /// Derived forward classification.
    pub classification: String,
    /// Declared resource claim and recorded evidence for the forward ledger.
    pub resource: Option<ResourceLedgerStatus>,
    /// Linked remediation ledger, when one exists.
    pub remediation: Option<RemediationLedgerStatus>,
}

/// Public remediation state linked to a forward ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemediationLedgerStatus {
    /// Remediation ledger key.
    pub ledger_key: String,
    /// Forward ledger key this remediation closes.
    pub forward_ledger_key: String,
    /// Current remediation side-effect phase.
    pub phase: String,
    /// Declared resource claim and recorded evidence for the remediation ledger.
    pub resource: Option<ResourceLedgerStatus>,
    /// Whether remediation confirmation closed the obligation.
    pub closed: bool,
    /// Unresolved reason if remediation cannot close the obligation.
    pub unresolved: Option<String>,
}

/// Public resource-claim and evidence status for one side-effect ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLedgerStatus {
    /// Side-effect ledger key.
    pub ledger_key: String,
    /// Ledger purpose.
    pub ledger_purpose: String,
    /// Linked forward ledger key when this is a remediation ledger.
    pub forward_ledger_key: Option<String>,
    /// Current projected side-effect phase.
    pub phase: String,
    /// Certified resource claim declared by the side-effect node.
    pub claim: ResourceClaimStatus,
    /// Recorded exclusive key evidence, when the claim is `exclusive` and preparation occurred.
    pub key: Option<ResourceKeyStatus>,
    /// Recorded exact touched-set evidence, when the claim is `exact_touched_set`.
    pub touched_set: Option<ResourceTouchedSetStatus>,
    /// Active lane holder when this ledger currently owns its exclusive lane.
    pub active_lane: Option<ResourceLaneHolderStatus>,
    /// Active holder of the same recorded key when this ledger is not the holder.
    pub blocked_by_lane: Option<ResourceLaneHolderStatus>,
}

/// Public certified resource claim summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceClaimStatus {
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
pub struct ResourceKeyStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Key schema id.
    pub key_schema_id: String,
    /// Stable digest of the store-comparable resource key evidence.
    pub key_digest: String,
}

/// Public exact touched-set evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTouchedSetStatus {
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
pub struct ResourceLaneHolderStatus {
    /// Resource namespace.
    pub namespace: String,
    /// Key schema id.
    pub key_schema_id: String,
    /// Stable digest of the store-comparable resource lane key.
    pub key_digest: String,
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
pub struct AttemptDispositionStatus {
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
    /// Stable error code for failed attempts.
    pub error_code: Option<String>,
    /// Output cell id for completed attempts.
    pub output_cell_id: Option<String>,
}

/// Public terminal resolution summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalResolutionStatus {
    /// Terminal outcome.
    pub outcome: String,
    /// Public claim carried by the terminal outcome.
    pub claim: String,
}

/// Response returned after typed start or resume dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current semantic run mode.
    pub run_mode: RunModeStatus,
    /// Derived saga status.
    pub saga: SagaStatus,
    /// Attempt-level dispositions, distinct from semantic run mode.
    pub attempt_dispositions: Vec<AttemptDispositionStatus>,
    /// Last scheduler status observed by the app dispatch loop.
    ///
    /// Read-only status reports `observed`. Start/resume dispatch reports scheduler progress or
    /// execution-claim coordination, and already-terminal resume reports `observed` because no
    /// scheduler dispatch is needed.
    pub scheduler_status: String,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
}

impl fmt::Display for RunResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} run_mode={} spec_hash={} head_seq={} scheduler_status={}",
            self.run_id, self.run_mode, self.spec_hash, self.head_seq, self.scheduler_status
        )
    }
}

/// Public launch outcome kind for start responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLaunchOutcomeStatus {
    /// A new run was admitted by this launch request.
    Admitted,
    /// The launch request attached to an already admitted compatible run.
    Attached,
    /// The launch request found active equivalent work and did not admit a new run.
    AlreadyActive,
}

impl RunLaunchOutcomeStatus {
    /// Returns the stable public status string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Attached => "attached",
            Self::AlreadyActive => "already_active",
        }
    }
}

impl fmt::Display for RunLaunchOutcomeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// App-level result of a normal run launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunLaunchOutcome {
    /// This launch admitted a new run.
    Admitted {
        /// Current run status after driving until blocked or completed.
        run: RunResponse,
    },
    /// This launch attached to an already admitted compatible run.
    Attached {
        /// Current run status for the existing run.
        run: RunResponse,
    },
    /// This launch found active equivalent work before run admission.
    AlreadyActive {
        /// Active run id holding the equivalent execution lane.
        active_run_id: RunId,
    },
}

impl RunLaunchOutcome {
    /// Returns the stable public outcome kind.
    pub fn status(&self) -> RunLaunchOutcomeStatus {
        match self {
            Self::Admitted { .. } => RunLaunchOutcomeStatus::Admitted,
            Self::Attached { .. } => RunLaunchOutcomeStatus::Attached,
            Self::AlreadyActive { .. } => RunLaunchOutcomeStatus::AlreadyActive,
        }
    }

    /// Returns the run response carried by this outcome.
    pub fn run(&self) -> Option<&RunResponse> {
        match self {
            Self::Admitted { run } | Self::Attached { run } => Some(run),
            Self::AlreadyActive { .. } => None,
        }
    }

    /// Splits this outcome into the public kind, run response, and active run id.
    pub fn into_response_parts(
        self,
    ) -> (RunLaunchOutcomeStatus, Option<RunResponse>, Option<RunId>) {
        match self {
            Self::Admitted { run } => (RunLaunchOutcomeStatus::Admitted, Some(run), None),
            Self::Attached { run } => (RunLaunchOutcomeStatus::Attached, Some(run), None),
            Self::AlreadyActive { active_run_id } => (
                RunLaunchOutcomeStatus::AlreadyActive,
                None,
                Some(active_run_id),
            ),
        }
    }
}

/// Typed public-output rendering response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicOutputResponse {
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

impl fmt::Display for PublicOutputResponse {
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

/// App-level report for a start request after launch driving and optional public-output rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStartReport {
    /// Public launch outcome kind.
    pub outcome: RunLaunchOutcomeStatus,
    /// Current run status when a run was admitted or attached.
    pub run: Option<RunResponse>,
    /// Active equivalent run id when admission was skipped.
    pub active_run_id: Option<String>,
    /// Rendered public output when the run completed during launch.
    pub public_output: Option<PublicOutputResponse>,
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
pub struct RunStreamResponse {
    /// Run id.
    pub run_id: String,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
    /// Store-owned typed event references.
    pub events: Vec<RunEventRef>,
}

impl fmt::Display for RunStreamResponse {
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
pub struct RunEventRef {
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
    /// Stable error code when this event is a failed state attempt.
    pub error_code: Option<String>,
}

/// Response returned after verifying replay authority for a typed run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayResponse {
    /// Run id.
    pub run_id: String,
    /// Certified spec hash.
    pub spec_hash: String,
    /// Current semantic run mode.
    pub run_mode: RunModeStatus,
    /// Derived saga status.
    pub saga: SagaStatus,
    /// Attempt-level dispositions, distinct from semantic run mode.
    pub attempt_dispositions: Vec<AttemptDispositionStatus>,
    /// Current typed run-stream head sequence.
    pub head_seq: u64,
    /// Retained artifact evidence entries supplied to the replay broker.
    pub retained_artifacts: usize,
}

impl fmt::Display for ReplayResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "run {} replay verified spec_hash={} run_mode={} head_seq={} retained_artifacts={}",
            self.run_id, self.spec_hash, self.run_mode, self.head_seq, self.retained_artifacts
        )
    }
}

/// Evidence-only application facade for certified typed run reads.
#[derive(Clone)]
pub struct RunReadServices<S, A> {
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
    fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
}

impl<S, A> RunReadServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    /// Creates evidence-only app services with an explicit trusted certification registry.
    pub fn new_with_certification_registry(
        store: S,
        artifacts: A,
        certification_registry: CertificationRegistry,
    ) -> Self {
        Self::new_with_certification_registry_and_fact_query_authority(
            store,
            artifacts,
            certification_registry,
            None,
        )
    }

    /// Creates evidence-only app services with explicit fact-query receipt authority.
    pub fn new_with_certification_registry_and_fact_query_authority(
        store: S,
        artifacts: A,
        certification_registry: CertificationRegistry,
        fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
    ) -> Self {
        Self {
            store,
            artifacts,
            certification_registry,
            fact_query_receipt_trust_root,
        }
    }

    /// Returns the typed artifact store.
    pub fn artifacts(&self) -> &A {
        &self.artifacts
    }

    /// Returns the async typed run store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Returns the trusted certification registry used for stored spec verification.
    pub fn certification_registry(&self) -> &CertificationRegistry {
        &self.certification_registry
    }

    /// Loads the store-owned deployment scope used to verify run identities.
    pub async fn load_store_scope_id(&self) -> Result<StoreScopeId, AppError> {
        self.trusted_run_reader().load_store_scope_id().await
    }

    /// Returns typed run status by rebuilding projection from the authoritative run stream.
    pub async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, AppError> {
        self.trusted_run_reader().run_status(run_id).await
    }

    /// Returns the authoritative typed run stream.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, AppError> {
        self.trusted_run_reader().run_stream(run_id).await
    }

    /// Reads one observation-only run list/watch page.
    pub async fn read_run_observations(
        &self,
        query: store::RunObservationQuery,
    ) -> Result<store::RunObservationPage, AppError>
    where
        S: store::RunObservationStore,
        <S as store::RunObservationStore>::Error:
            store::StoreErrorInspection + fmt::Display + Send + Sync + 'static,
    {
        self.trusted_run_reader().read_run_observations(query).await
    }

    /// Verifies replay authority for a run using retained typed artifact evidence only.
    pub async fn verify_replay_for_run(&self, run_id: &RunId) -> Result<ReplayResponse, AppError> {
        self.trusted_run_reader()
            .verify_replay(self.fact_query_receipt_trust_root.clone(), run_id)
            .await
    }

    /// Renders typed public output from store-owned projection and typed artifact bytes.
    pub async fn public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, AppError> {
        self.trusted_run_reader()
            .public_output(run_id, public_schema_id)
            .await
    }

    /// Lists public fact kinds from retained descriptor artifacts and store-scoped fact projection authority.
    pub async fn fact_kinds(&self) -> Result<Vec<PublicFactKindSummary>, AppError> {
        Ok(self.public_fact_catalog().await?.list_kinds())
    }

    /// Describes public fact descriptors for one kind from store-scoped fact projection authority.
    pub async fn describe_fact_kind(
        &self,
        fact_kind: &str,
    ) -> Result<Vec<PublicFactDescriptorSummary>, AppError> {
        self.public_fact_catalog().await?.describe_kind(fact_kind)
    }

    /// Explains public query and return fields for one fact kind from store-scoped fact projection authority.
    pub async fn explain_fact_kind(&self, fact_kind: &str) -> Result<PublicFactExplain, AppError> {
        self.public_fact_catalog().await?.explain_kind(fact_kind)
    }

    /// Resolves an opaque public fact reference against store-scoped Platform facts.
    ///
    /// Unknown, `Control`, and `RunPrivate` facts all return the same redacted not-found class.
    pub async fn resolve_public_fact_ref(
        &self,
        public_ref: &PublicFactRefId,
    ) -> Result<PublicFactRef, AppError> {
        let projection = self.public_fact_projection().await?;
        let catalog =
            FactCatalogService::from_retained_public_projection(&self.artifacts, &projection)
                .await?;
        FactPublicRefResolver::new(catalog, projection).resolve(public_ref)
    }

    async fn public_fact_catalog(&self) -> Result<FactCatalogService, AppError> {
        let projection = self.public_fact_projection().await?;
        FactCatalogService::from_retained_public_projection(&self.artifacts, &projection).await
    }

    async fn public_fact_projection(&self) -> Result<store::ProjectionSnapshot, AppError> {
        self.store
            .fact_projection_snapshot()
            .await
            .map_err(async_app_store_error)
    }

    fn trusted_run_reader(&self) -> TrustedRunReader<'_, S, A> {
        TrustedRunReader::new(&self.store, &self.artifacts, &self.certification_registry)
    }
}

impl<S, A> RunReadServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + PublicFactQueryExecutor + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    /// Builds a production public fact query service from retained descriptor and store-scoped projection authority.
    pub async fn public_fact_query_service(&self) -> Result<FactPublicQueryService<S>, AppError> {
        let catalog = self.public_fact_catalog().await?;
        FactPublicQueryService::new(catalog, self.store.clone())
    }

    /// Executes a public Platform fact query against store-scoped public facts.
    pub async fn query_public_facts(
        &self,
        request: PublicFactQueryRequest,
    ) -> Result<PublicFactQueryPage, AppError> {
        self.public_fact_query_service().await?.query(request).await
    }
}

/// Application facade for certified typed runtime dispatch.
#[derive(Clone)]
pub struct RunServices<S, A> {
    scheduler: SerialTypedScheduler,
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
    fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
    fact_query_authority_ready: bool,
    execution_claim_heartbeat_interval: Duration,
}

impl<S, A> RunServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    /// Creates typed async app services with an explicit trusted certification registry.
    pub fn new_with_certification_registry(
        scheduler: SerialTypedScheduler,
        store: S,
        artifacts: A,
        certification_registry: CertificationRegistry,
    ) -> Self {
        Self::new_with_certification_registry_and_fact_query_authority(
            scheduler,
            store,
            artifacts,
            certification_registry,
            None,
            false,
        )
    }

    /// Creates typed async app services with explicit fact-query receipt authority.
    pub fn new_with_certification_registry_and_fact_query_authority(
        scheduler: SerialTypedScheduler,
        store: S,
        artifacts: A,
        certification_registry: CertificationRegistry,
        fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
        fact_query_authority_ready: bool,
    ) -> Self {
        Self {
            scheduler: scheduler
                .with_fact_query_receipt_trust_root(fact_query_receipt_trust_root.clone()),
            store,
            artifacts,
            certification_registry,
            fact_query_receipt_trust_root,
            fact_query_authority_ready,
            execution_claim_heartbeat_interval: default_execution_claim_heartbeat_interval(),
        }
    }

    /// Returns the typed artifact store.
    pub fn artifacts(&self) -> &A {
        &self.artifacts
    }

    /// Returns the async typed run store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Returns the trusted certification registry used for stored spec verification.
    pub fn certification_registry(&self) -> &CertificationRegistry {
        &self.certification_registry
    }

    /// Loads the store-owned deployment scope used to derive run identities.
    pub async fn load_store_scope_id(&self) -> Result<StoreScopeId, AppError> {
        self.trusted_run_reader().load_store_scope_id().await
    }

    /// Returns typed run status by rebuilding projection from the authoritative run stream.
    pub async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, AppError> {
        self.trusted_run_reader().run_status(run_id).await
    }

    async fn run_response_from_verified_status(
        &self,
        run_id: &RunId,
        status: DriveStatus,
    ) -> Result<RunResponse, AppError> {
        let context = self.load_verified_status_read_context(run_id).await?;
        run_response_from_projection(
            run_id,
            context.runtime_spec(),
            context.events(),
            context.projection(),
            status,
        )
    }

    /// Returns the authoritative typed run stream.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, AppError> {
        self.trusted_run_reader().run_stream(run_id).await
    }

    /// Reads one observation-only run list/watch page.
    pub async fn read_run_observations(
        &self,
        query: store::RunObservationQuery,
    ) -> Result<store::RunObservationPage, AppError>
    where
        S: store::RunObservationStore,
        <S as store::RunObservationStore>::Error:
            store::StoreErrorInspection + fmt::Display + Send + Sync + 'static,
    {
        self.trusted_run_reader().read_run_observations(query).await
    }

    /// Verifies replay authority for a run using retained typed artifact evidence only.
    pub async fn verify_replay_for_run(&self, run_id: &RunId) -> Result<ReplayResponse, AppError> {
        self.trusted_run_reader()
            .verify_replay(self.fact_query_receipt_trust_root.clone(), run_id)
            .await
    }

    /// Renders typed public output from store-owned projection and typed artifact bytes.
    pub async fn public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, AppError> {
        self.trusted_run_reader()
            .public_output(run_id, public_schema_id)
            .await
    }

    async fn load_verified_run_read_context(
        &self,
        run_id: &RunId,
    ) -> Result<VerifiedRunReadContext, AppError> {
        self.trusted_run_reader().load_run_context(run_id).await
    }

    async fn load_verified_status_read_context(
        &self,
        run_id: &RunId,
    ) -> Result<VerifiedStatusReadContext, AppError> {
        self.trusted_run_reader().load_status_context(run_id).await
    }

    async fn validate_identity_material_store_scope(
        &self,
        identity_material: &events::RunIdentityMaterialV1,
    ) -> Result<(), AppError> {
        self.trusted_run_reader()
            .validate_identity_material_store_scope(identity_material)
            .await
    }

    fn trusted_run_reader(&self) -> TrustedRunReader<'_, S, A> {
        TrustedRunReader::new(&self.store, &self.artifacts, &self.certification_registry)
    }
}

impl<S, A> RunServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + store::ExecutionClaimStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    /// Resumes a certified typed run from its stored spec artifact.
    pub async fn resume_stored_run(&self, run_id: &RunId) -> Result<RunResponse, AppError> {
        let runtime_spec = self
            .load_verified_run_read_context(run_id)
            .await?
            .runtime_spec()
            .clone();
        if runtime_spec_requires_fact_index(&runtime_spec)? && !self.fact_query_authority_ready {
            return Err(fact_receipt_authority_unavailable_error());
        }
        let status = self.drive_until_blocked(&runtime_spec, run_id).await?;
        self.run_response_from_verified_status(run_id, status).await
    }

    /// Records a signed manual resolution and optionally resumes typed scheduler execution.
    pub async fn record_manual_resolution(
        &self,
        req: ManualResolutionRecordRequest,
    ) -> Result<RunResponse, AppError> {
        let run_id = req.run_id.clone();
        let runtime_spec = self
            .load_verified_run_read_context(&run_id)
            .await?
            .runtime_spec()
            .clone();
        if runtime_spec_requires_fact_index(&runtime_spec)? && !self.fact_query_authority_ready {
            return Err(fact_receipt_authority_unavailable_error());
        }
        let manual_request = manual_resolution_runtime_request(req)?;
        self.scheduler
            .record_manual_resolution(&self.store, &runtime_spec, &run_id, manual_request)
            .await?;
        let status = self.drive_until_blocked(&runtime_spec, &run_id).await?;
        self.run_response_from_verified_status(&run_id, status)
            .await
    }

    /// Starts a certified typed run against a durable async typed store.
    pub async fn launch_run(&self, req: RunLaunchRequest) -> Result<RunLaunchOutcome, AppError> {
        self.validate_identity_material_store_scope(&req.identity_material)
            .await?;
        if req.identity_material.certified_spec_hash != *req.certified_spec.spec_hash() {
            return Err(run_identity_material_mismatch());
        }
        let run_id = req.identity_material.derive_run_id().map_err(|_| {
            AppError::backend(
                ErrorClass::Internal,
                "RunIdentityMaterialInvalid",
                "Run identity material is invalid",
            )
        })?;
        if run_id != req.run_id {
            return Err(run_identity_material_mismatch());
        }
        let identity_material = req.identity_material;
        let execution_scope =
            store::ExecutionClaimScope::from_run_identity_material(&identity_material);
        let runtime_spec = CertifiedRuntimeSpec::new(req.certified_spec)?;
        if runtime_spec_requires_fact_index(&runtime_spec)? && !self.fact_query_authority_ready {
            return Err(fact_receipt_authority_unavailable_error());
        }

        loop {
            let stream = self
                .store
                .load_run_stream(&run_id)
                .await
                .map_err(async_app_store_error)?;
            if !stream.is_empty() {
                return self
                    .attach_to_existing_run(&run_id, &identity_material)
                    .await;
            }
            let expected_next_seq = self
                .store
                .expected_next_seq(&run_id)
                .await
                .map_err(async_app_store_error)?;
            if expected_next_seq != store::StreamSeq::FIRST {
                return self
                    .attach_to_existing_run(&run_id, &identity_material)
                    .await;
            }
            let launch = self.scheduler.prepare_run_launch(
                &runtime_spec,
                identity_material.clone(),
                req.evidence.clone(),
                expected_next_seq,
            )?;
            let execution_claim_token = new_execution_claim_token()?;
            let execution_claim = store::PreparedExecutionClaim::new(
                execution_scope.clone(),
                run_id.clone(),
                execution_claim_token.clone(),
            );
            match self
                .scheduler
                .start_run_with_execution_claim(&self.store, launch, execution_claim)
                .await
            {
                Ok(store::CommitOutcome::Appended(_)) => {
                    let status = self
                        .drive_until_blocked_with_existing_claim(
                            &runtime_spec,
                            &run_id,
                            &execution_scope,
                            &execution_claim_token,
                        )
                        .await?;
                    let run = self
                        .run_response_from_verified_status(&run_id, status)
                        .await?;
                    return Ok(RunLaunchOutcome::Admitted { run });
                }
                Ok(store::CommitOutcome::Idempotent(_)) => {
                    return self
                        .attach_to_existing_run(&run_id, &identity_material)
                        .await;
                }
                Ok(store::CommitOutcome::ExecutionClaimBusy(_)) => {
                    match self
                        .store
                        .execution_claim_status(&execution_scope)
                        .await
                        .map_err(async_app_store_error)?
                    {
                        store::ExecutionClaimStatus::Live(lease) => {
                            return Ok(RunLaunchOutcome::AlreadyActive {
                                active_run_id: lease.holder_run_id,
                            });
                        }
                        store::ExecutionClaimStatus::Expired(lease) => {
                            self.store
                                .reap_expired_execution_claim(
                                    &execution_scope,
                                    &lease.holder_run_id,
                                    &lease.token,
                                )
                                .await
                                .map_err(async_app_store_error)?;
                        }
                        store::ExecutionClaimStatus::Unclaimed => {}
                    }
                }
                Ok(store::CommitOutcome::AdmissionBlocked(_)) => {
                    return Err(AppError::backend(
                        ErrorClass::Internal,
                        "RunAdmissionBlockedUnexpectedly",
                        "run admission was blocked by a resource lane",
                    ));
                }
                Err(error) => {
                    let stream = self
                        .store
                        .load_run_stream(&run_id)
                        .await
                        .map_err(async_app_store_error)?;
                    if !stream.is_empty() {
                        return self
                            .attach_to_existing_run(&run_id, &identity_material)
                            .await;
                    }
                    return Err(error.into());
                }
            }
        }
    }

    /// Starts a prepared entry-point run and renders public output if the launch completes.
    pub async fn launch_prepared_entry_point_run(
        &self,
        prepared: PreparedEntryPointRunLaunch,
    ) -> Result<RunStartReport, AppError> {
        let run_id = prepared.request.run_id.clone();
        let public_output_schema_id = prepared
            .request
            .certified_spec
            .envelope()
            .spec
            .public_outputs
            .public_schema_id
            .clone();
        let launch = self.launch_run(prepared.request).await?;
        let (outcome, run, active_run_id) = launch.into_response_parts();
        let public_output = if matches!(
            run.as_ref().map(|run| run.run_mode),
            Some(RunModeStatus::Completed)
        ) {
            Some(
                self.public_output(&run_id, &public_output_schema_id)
                    .await?,
            )
        } else {
            None
        };
        Ok(RunStartReport {
            outcome,
            run,
            active_run_id: active_run_id.map(|run_id| run_id.to_string()),
            public_output,
        })
    }

    async fn attach_to_existing_run(
        &self,
        run_id: &RunId,
        identity_material: &events::RunIdentityMaterialV1,
    ) -> Result<RunLaunchOutcome, AppError> {
        let context = self.load_verified_status_read_context(run_id).await?;
        let run_admitted = context.read.view().run_admitted();
        if &run_admitted.identity_material != identity_material {
            return Err(run_identity_material_mismatch());
        }
        self.scheduler
            .validate_admitted_run_binding(context.runtime_spec(), run_admitted)?;
        let run = run_status_from_projection(
            run_id,
            context.runtime_spec(),
            context.events(),
            context.projection(),
        )?;
        match self
            .store
            .execution_claim_status(&store::ExecutionClaimScope::from_run_identity_material(
                identity_material,
            ))
            .await
            .map_err(async_app_store_error)?
        {
            store::ExecutionClaimStatus::Live(lease) => Ok(RunLaunchOutcome::AlreadyActive {
                active_run_id: lease.holder_run_id,
            }),
            store::ExecutionClaimStatus::Unclaimed | store::ExecutionClaimStatus::Expired(_) => {
                Ok(RunLaunchOutcome::Attached { run })
            }
        }
    }

    async fn drive_until_blocked(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<DriveStatus, AppError> {
        let context = self.load_verified_status_read_context(run_id).await?;
        let execution_scope = store::ExecutionClaimScope::from_run_identity_material(
            &context.read.view().run_admitted().identity_material,
        );
        if run_projection_has_terminal_completion(
            run_id,
            context.runtime_spec(),
            context.projection(),
        )? {
            self.reap_expired_execution_claim_if_present(&execution_scope, run_id)
                .await?;
            return Ok(DriveStatus::Observed);
        }
        self.scheduler
            .validate_admitted_run_binding(runtime_spec, context.read.view().run_admitted())?;
        let launch_evidence = stored_launch_evidence_from_run_admitted(
            &self.artifacts,
            context.read.view().run_admitted(),
        )
        .await?;
        self.scheduler
            .validate_admitted_run_ingress(runtime_spec, &launch_evidence)?;
        let mut lease = match self
            .acquire_execution_claim_for_drive(&execution_scope, run_id)
            .await?
        {
            ExecutionClaimAcquire::Acquired(lease) => lease,
            ExecutionClaimAcquire::Busy => return Ok(DriveStatus::ExecutionClaimBusy),
        };
        self.drive_until_blocked_with_lease(runtime_spec, run_id, &execution_scope, &mut lease)
            .await
    }

    async fn drive_until_blocked_with_existing_claim(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        execution_scope: &store::ExecutionClaimScope,
        token: &store::AdmissionToken,
    ) -> Result<DriveStatus, AppError> {
        let mut lease = match self
            .store
            .execution_claim_status(execution_scope)
            .await
            .map_err(async_app_store_error)?
        {
            store::ExecutionClaimStatus::Live(lease)
                if &lease.holder_run_id == run_id && &lease.token == token =>
            {
                lease
            }
            store::ExecutionClaimStatus::Live(_) => return Ok(DriveStatus::ExecutionClaimBusy),
            store::ExecutionClaimStatus::Expired(_) => return Ok(DriveStatus::ExecutionClaimLost),
            store::ExecutionClaimStatus::Unclaimed => return Ok(DriveStatus::ExecutionClaimLost),
        };
        self.drive_until_blocked_with_lease(runtime_spec, run_id, execution_scope, &mut lease)
            .await
    }

    async fn drive_until_blocked_with_lease(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        execution_scope: &store::ExecutionClaimScope,
        lease: &mut store::AdmissionLease,
    ) -> Result<DriveStatus, AppError> {
        loop {
            if !self
                .renew_execution_claim_or_lost(execution_scope, run_id, lease)
                .await?
            {
                return Ok(DriveStatus::ExecutionClaimLost);
            }
            let step = self
                .drive_once_with_execution_claim(runtime_spec, run_id, execution_scope, lease)
                .await?;
            if step.claim_lost {
                return Ok(DriveStatus::ExecutionClaimLost);
            }
            let context = self.load_verified_status_read_context(run_id).await?;
            if run_projection_has_terminal_completion(
                run_id,
                context.runtime_spec(),
                context.projection(),
            )? || step.status == SchedulerStatus::PublicOutputProjected
            {
                self.release_execution_claim_if_holder(execution_scope, run_id, lease)
                    .await?;
                return Ok(step.status.into());
            }
            match step.status {
                SchedulerStatus::Advanced => {}
                SchedulerStatus::Blocked => return Ok(SchedulerStatus::Blocked.into()),
                SchedulerStatus::PublicOutputProjected => unreachable!("handled above"),
            }
        }
    }

    async fn acquire_execution_claim_for_drive(
        &self,
        execution_scope: &store::ExecutionClaimScope,
        run_id: &RunId,
    ) -> Result<ExecutionClaimAcquire, AppError> {
        loop {
            match self
                .store
                .execution_claim_status(execution_scope)
                .await
                .map_err(async_app_store_error)?
            {
                store::ExecutionClaimStatus::Live(_) => return Ok(ExecutionClaimAcquire::Busy),
                store::ExecutionClaimStatus::Expired(lease) => {
                    self.store
                        .reap_expired_execution_claim(
                            execution_scope,
                            &lease.holder_run_id,
                            &lease.token,
                        )
                        .await
                        .map_err(async_app_store_error)?;
                }
                store::ExecutionClaimStatus::Unclaimed => {
                    let token = new_execution_claim_token()?;
                    match self
                        .store
                        .acquire_execution_claim(execution_scope, run_id, token)
                        .await
                        .map_err(async_app_store_error)?
                    {
                        store::NowaitSkipAdmissionResult::Admitted(lease) => {
                            return Ok(ExecutionClaimAcquire::Acquired(lease));
                        }
                        store::NowaitSkipAdmissionResult::Busy(_) => {}
                    }
                }
            }
        }
    }

    async fn reap_expired_execution_claim_if_present(
        &self,
        execution_scope: &store::ExecutionClaimScope,
        _run_id: &RunId,
    ) -> Result<(), AppError> {
        if let store::ExecutionClaimStatus::Expired(lease) = self
            .store
            .execution_claim_status(execution_scope)
            .await
            .map_err(async_app_store_error)?
        {
            self.store
                .reap_expired_execution_claim(execution_scope, &lease.holder_run_id, &lease.token)
                .await
                .map_err(async_app_store_error)?;
        }
        Ok(())
    }

    async fn drive_once_with_execution_claim(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        execution_scope: &store::ExecutionClaimScope,
        lease: &mut store::AdmissionLease,
    ) -> Result<ClaimedDriveStep, AppError> {
        let scheduler = self.scheduler.clone();
        let step = scheduler.drive_once(
            &self.store,
            runtime_spec,
            run_id,
            execution_scope,
            lease.token.clone(),
        );
        tokio::pin!(step);
        loop {
            tokio::select! {
                status = &mut step => {
                    return match status {
                        Ok(status) => Ok(ClaimedDriveStep {
                            status,
                            claim_lost: false,
                        }),
                        Err(mfm_runtime::RuntimeError::ExecutionClaim(_)) => Ok(ClaimedDriveStep {
                            status: SchedulerStatus::Blocked,
                            claim_lost: true,
                        }),
                        Err(error) => Err(error.into()),
                    };
                }
                _ = tokio::time::sleep(self.execution_claim_heartbeat_interval) => {
                    if !self.renew_execution_claim_or_lost(execution_scope, run_id, lease).await? {
                        let status = match (&mut step).await {
                            Ok(status) => status,
                            Err(mfm_runtime::RuntimeError::ExecutionClaim(_)) => {
                                SchedulerStatus::Blocked
                            }
                            Err(error) => return Err(error.into()),
                        };
                        return Ok(ClaimedDriveStep { status, claim_lost: true });
                    }
                }
            }
        }
    }

    async fn renew_execution_claim_or_lost(
        &self,
        execution_scope: &store::ExecutionClaimScope,
        run_id: &RunId,
        lease: &mut store::AdmissionLease,
    ) -> Result<bool, AppError> {
        match self
            .store
            .renew_execution_claim(execution_scope, run_id, &lease.token)
            .await
            .map_err(async_app_store_error)?
        {
            Some(renewed) => {
                *lease = renewed;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn release_execution_claim_if_holder(
        &self,
        execution_scope: &store::ExecutionClaimScope,
        run_id: &RunId,
        lease: &store::AdmissionLease,
    ) -> Result<(), AppError> {
        self.store
            .release_execution_claim(execution_scope, run_id, &lease.token)
            .await
            .map_err(async_app_store_error)?;
        Ok(())
    }
}

fn run_projection_has_terminal_completion(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    projection: &store::ProjectionSnapshot,
) -> Result<bool, AppError> {
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    let saga =
        projection.derive_saga_projection(run_id, &runtime_spec.spec().saga, &terminal_policies)?;
    Ok(saga.run_completion.is_some())
}

enum ExecutionClaimAcquire {
    Acquired(store::AdmissionLease),
    Busy,
}

struct ClaimedDriveStep {
    status: SchedulerStatus,
    claim_lost: bool,
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
        Ok(status_projection_from_verified_view_with_resource_lanes(
            &self.view,
            global_projection,
        )?)
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

struct TrustedRunReader<'a, S, A: ?Sized> {
    store: &'a S,
    artifacts: &'a A,
    registry: &'a CertificationRegistry,
}

impl<'a, S, A: ?Sized> TrustedRunReader<'a, S, A> {
    fn new(store: &'a S, artifacts: &'a A, registry: &'a CertificationRegistry) -> Self {
        Self {
            store,
            artifacts,
            registry,
        }
    }
}

impl<S, A> TrustedRunReader<'_, S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    async fn load_store_scope_id(&self) -> Result<StoreScopeId, AppError> {
        self.store
            .load_store_scope_id()
            .await
            .map_err(async_app_store_error)
    }

    async fn validate_identity_material_store_scope(
        &self,
        identity_material: &events::RunIdentityMaterialV1,
    ) -> Result<(), AppError> {
        let store_scope_id = self.load_store_scope_id().await?;
        if store_scope_id != identity_material.store_scope_id {
            return Err(run_identity_material_mismatch());
        }
        Ok(())
    }

    async fn load_run_context(&self, run_id: &RunId) -> Result<VerifiedRunReadContext, AppError> {
        let context =
            load_async_verified_run_read_context(self.store, self.artifacts, self.registry, run_id)
                .await?;
        self.validate_identity_material_store_scope(
            &context.view().run_admitted().identity_material,
        )
        .await?;
        Ok(context)
    }

    async fn load_status_context(
        &self,
        run_id: &RunId,
    ) -> Result<VerifiedStatusReadContext, AppError> {
        let context = load_async_verified_status_read_context(
            self.store,
            self.artifacts,
            self.registry,
            run_id,
        )
        .await?;
        self.validate_identity_material_store_scope(
            &context.read.view().run_admitted().identity_material,
        )
        .await?;
        Ok(context)
    }

    async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, AppError> {
        let context = self.load_status_context(run_id).await?;
        run_status_from_projection(
            run_id,
            context.runtime_spec(),
            context.events(),
            context.projection(),
        )
    }

    async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, AppError> {
        let context = self.load_run_context(run_id).await?;
        Ok(run_stream_response_from_verified_context(&context))
    }

    async fn read_run_observations(
        &self,
        query: store::RunObservationQuery,
    ) -> Result<store::RunObservationPage, AppError>
    where
        S: store::RunObservationStore,
        <S as store::RunObservationStore>::Error:
            store::StoreErrorInspection + fmt::Display + Send + Sync + 'static,
    {
        self.store
            .read_run_observations(query)
            .await
            .map_err(observation_app_store_error)
    }

    async fn verify_replay(
        &self,
        fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
        run_id: &RunId,
    ) -> Result<ReplayResponse, AppError> {
        let context = self.load_run_context(run_id).await?;
        verify_replay_diagnostics_from_recorded_artifacts(self.artifacts, run_id, context.events())
            .await?;
        let authority = replay_read_authority_for_run_with_retained_source_facts(
            self.store,
            self.artifacts,
            context.runtime_spec(),
            context.view(),
            fact_query_receipt_trust_root,
        )
        .await?;
        let broker = ReplayBroker::from_read_authority(authority)?;
        let stream = context.events();
        replay_verifiers::ReplayVerifierRegistry::production().verify(&broker, self.registry)?;
        let projection = broker.projection_snapshot();
        let terminal_policies =
            store::SideEffectTerminalPolicies::from_spec(context.runtime_spec().spec())?;
        let saga = projection.derive_saga_projection(
            run_id,
            &context.runtime_spec().spec().saga,
            &terminal_policies,
        )?;
        let retained_artifacts = projection
            .retention(run_id)
            .map(|retention| retention.refs.len())
            .unwrap_or_default();
        Ok(ReplayResponse {
            run_id: run_id.as_str().to_owned(),
            spec_hash: broker.certified_spec().spec_hash.as_str().to_owned(),
            run_mode: run_mode_status(saga.run_mode),
            saga: saga_status_with_resources(context.runtime_spec().spec(), projection, &saga),
            attempt_dispositions: attempt_dispositions(projection),
            head_seq: stream_head(stream),
            retained_artifacts,
        })
    }

    async fn public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, AppError> {
        let context = self.load_run_context(run_id).await?;
        let authority = public_output_read_authority_for_run(
            self.artifacts,
            context.runtime_spec(),
            context.view(),
            public_schema_id,
        )
        .await?;
        render_public_output(self.artifacts, &authority).await
    }
}

async fn load_async_verified_run_read_context<S, A>(
    store: &S,
    artifacts: &A,
    registry: &CertificationRegistry,
    run_id: &RunId,
) -> Result<VerifiedRunReadContext, AppError>
where
    S: store::RunEventStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    let committed = store
        .load_committed_run_stream(run_id)
        .await
        .map_err(async_app_store_error)?;
    verified_run_read_context_from_committed_stream(artifacts, registry, committed).await
}

async fn load_async_verified_status_read_context<S, A>(
    store: &S,
    artifacts: &A,
    registry: &CertificationRegistry,
    run_id: &RunId,
) -> Result<VerifiedStatusReadContext, AppError>
where
    S: store::RunEventStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    let committed = store
        .load_committed_run_stream(run_id)
        .await
        .map_err(async_app_store_error)?;
    let projection = store
        .status_projection_snapshot(run_id)
        .await
        .map_err(async_app_store_error)?;
    verified_status_read_context_from_committed_stream(artifacts, registry, committed, &projection)
        .await
}

async fn verified_status_read_context_from_committed_stream(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    registry: &CertificationRegistry,
    committed: store::CommittedRunStream,
    global_projection: &store::ProjectionSnapshot,
) -> Result<VerifiedStatusReadContext, AppError> {
    let read =
        verified_run_read_context_from_committed_stream(artifacts, registry, committed).await?;
    let projection = read.status_projection_with_resource_lanes(global_projection)?;
    Ok(VerifiedStatusReadContext { read, projection })
}

async fn verified_run_read_context_from_committed_stream(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    registry: &CertificationRegistry,
    committed: store::CommittedRunStream,
) -> Result<VerifiedRunReadContext, AppError> {
    let run_id = committed.run_id().clone();
    let stream = committed.events();
    if stream.is_empty() {
        return Err(AppError::not_found(
            "RunNotFound",
            "typed run stream was not found",
        ));
    }
    let runtime_spec = load_runtime_spec_for_run(artifacts, registry, &run_id, stream).await?;
    let retained_artifacts =
        store::VerifiedRunArtifactStore::from_committed_stream(&committed, artifacts).await?;
    let view = VerifiedRunHistoryView::from_committed_stream(
        &runtime_spec,
        committed,
        retained_artifacts,
    )?;
    Ok(VerifiedRunReadContext { runtime_spec, view })
}

async fn verify_replay_diagnostics_from_recorded_artifacts(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<(), AppError> {
    let _ = run_admitted_payload(run_id, stream)?;
    for event in stream {
        let events::KernelEventPayload::StateAttemptFailed(payload) = event.payload() else {
            continue;
        };
        let Some(diagnostic_ref) = &payload.error.diagnostic_ref else {
            continue;
        };
        let requirement = diagnostic_artifact_requirement(diagnostic_ref);
        let artifact = artifacts
            .read_retained_artifact(&requirement)
            .await
            .map_err(async_app_store_error)?;
        let diagnostic_artifact = serde_json::from_slice::<serde_json::Value>(artifact.bytes())
            .map_err(|_| replay_diagnostic_error())?;
        let diagnostic = RuntimeDiagnostic::from_attempt_artifact_json(&diagnostic_artifact)
            .map_err(|_| replay_diagnostic_error())?;
        verify_replay_diagnostic(payload.error.public_details.as_ref(), diagnostic.as_ref())?;
    }
    Ok(())
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

/// Builds sealed replay read authority from a verified run-history view.
pub fn replay_read_authority_for_run(
    runtime_spec: &CertifiedRuntimeSpec,
    verified_view: &VerifiedRunHistoryView,
) -> Result<ReplayReadAuthority, AppError> {
    Ok(ReplayReadAuthority::from_verified_run_history_view(
        runtime_spec,
        verified_view,
    )?)
}

/// Builds sealed replay read authority with explicit fact-query receipt trust authority.
pub fn replay_read_authority_for_run_with_fact_query_trust_root(
    runtime_spec: &CertifiedRuntimeSpec,
    verified_view: &VerifiedRunHistoryView,
    fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
) -> Result<ReplayReadAuthority, AppError> {
    Ok(
        ReplayReadAuthority::from_verified_run_history_view_with_fact_query_receipt_trust_root(
            runtime_spec,
            verified_view,
            fact_query_receipt_trust_root,
        )?,
    )
}

async fn replay_read_authority_for_run_with_retained_source_facts<S, A>(
    store: &S,
    artifacts: &A,
    runtime_spec: &CertifiedRuntimeSpec,
    verified_view: &VerifiedRunHistoryView,
    fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
) -> Result<ReplayReadAuthority, AppError>
where
    S: store::RunEventStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    let source_fact_events =
        retained_source_fact_events_from_query_evidence(store, artifacts, verified_view.events())
            .await?;
    let context_artifacts =
        certified_evm_context_artifacts_for_replay(artifacts, runtime_spec).await?;
    Ok(ReplayReadAuthority::from_verified_run_history_view_with_fact_query_receipt_trust_root_source_facts_and_artifacts(
        runtime_spec,
        verified_view,
        fact_query_receipt_trust_root,
        source_fact_events,
        context_artifacts,
    )?)
}

async fn certified_evm_context_artifacts_for_replay(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<Vec<store::VerifiedRunArtifactBytes>, AppError> {
    let descriptor = evm_contract_context_descriptor()?;
    let mut retained = Vec::new();
    for context in &runtime_spec.envelope().spec.contexts {
        if context.context_descriptor_id != descriptor.context_descriptor_id
            || context.schema_id != descriptor.schema_id
            || context.semantic_type_id != descriptor.semantic_type_id
            || context.canonicalizer_identity != descriptor.canonicalizer_identity
        {
            continue;
        }
        let context =
            mfm_program::CertifiedContext::<EvmContractContext>::from_certified_spec(context)
                .map_err(|_| certified_evm_context_artifact_error())?;
        let Some(reference) = context.value().contract_profile.artifact_ref.as_ref() else {
            continue;
        };
        let requirement = evm_contract_profile_artifact_requirement(reference)?;
        retained.push(
            artifacts
                .read_retained_artifact(&requirement)
                .await
                .map_err(async_app_store_error)?,
        );
    }
    Ok(retained)
}

fn evm_contract_context_descriptor() -> Result<spec::StateContextDescriptorRequirementSpec, AppError>
{
    let spec::StateContextDescriptorSpec::Required(descriptor) =
        <EvmContractContext as mfm_program::StateContext>::descriptor()
            .map_err(|_| certified_evm_context_artifact_error())?
    else {
        return Err(certified_evm_context_artifact_error());
    };
    Ok(*descriptor)
}

fn evm_contract_profile_artifact_requirement(
    reference: &LifecycleArtifactEvidenceRef,
) -> Result<store::EventArtifactRequirement, AppError> {
    let schema_id = reference
        .schema_id()
        .map_err(|_| certified_evm_context_artifact_error())?
        .ok_or_else(certified_evm_context_artifact_error)?;
    let expected_schema = <ContractArtifactConfig as MfmConfig>::schema_id()
        .map_err(|_| certified_evm_context_artifact_error())?;
    if schema_id != expected_schema
        || reference
            .semantic_type_id()
            .map_err(|_| certified_evm_context_artifact_error())?
            .is_some()
    {
        return Err(certified_evm_context_artifact_error());
    }
    let artifact_id = reference
        .artifact_id()
        .map_err(|_| certified_evm_context_artifact_error())?;
    let digest = reference
        .content_digest()
        .map_err(|_| certified_evm_context_artifact_error())?;
    let media_type = spec::MediaType::new("application/json")
        .map_err(|_| certified_evm_context_artifact_error())?;
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: digest.clone(),
        byte_len: reference.byte_len(),
        media_type: media_type.clone(),
        schema_id: Some(schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    Ok(store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id,
        evidence_hash: evidence
            .evidence_hash()
            .map_err(|_| certified_evm_context_artifact_error())?,
        digest: Some(digest),
        byte_len: Some(reference.byte_len()),
        media_type: Some(media_type),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    })
}

fn certified_evm_context_artifact_error() -> AppError {
    AppError::backend(
        ErrorClass::Internal,
        "CertifiedEvmContextArtifactInvalid",
        "Certified EVM context artifact failed replay verification",
    )
}

async fn retained_source_fact_events_from_query_evidence<S, A>(
    store: &S,
    artifacts: &A,
    stream: &[store::KernelEventEnvelope],
) -> Result<Vec<RetainedSourceFactReplayEvent>, AppError>
where
    S: store::RunEventStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    let mut source_events = BTreeMap::new();
    for event in stream {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::FactQueryEvidence {
            continue;
        }
        let artifact = artifacts
            .read_retained_artifact(&artifact_referenced_artifact_requirement(payload))
            .await
            .map_err(async_app_store_error)?;
        let evidence = mfm_facts::parse_canonical_fact_query_evidence_bytes(artifact.bytes())
            .map_err(|_| {
                AppError::backend(
                    ErrorClass::Internal,
                    "FactQueryEvidenceInvalid",
                    "Fact query evidence artifact is invalid",
                )
            })?;
        for fact_ref in evidence.receipt().returned_refs() {
            let fact_claim_id = fact_ref.fact_claim_id().clone();
            if source_events.contains_key(&fact_claim_id) {
                continue;
            }
            let source_stream = store
                .load_run_stream(fact_claim_id.source_run_id())
                .await
                .map_err(async_app_store_error)?;
            let envelope = source_stream
                .into_iter()
                .find(|candidate| {
                    candidate.seq().as_u64() == fact_claim_id.source_seq()
                        && candidate.ordinal().as_u32() == fact_claim_id.source_ordinal()
                })
                .ok_or_else(|| {
                    AppError::backend(
                        ErrorClass::Internal,
                        "FactQuerySourceFactMissing",
                        "Fact query evidence source fact event is missing",
                    )
                })?;
            let source_event = RetainedSourceFactReplayEvent::new(fact_claim_id.clone(), envelope)?;
            source_events.insert(fact_claim_id, source_event);
        }
    }
    Ok(source_events.into_values().collect())
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
        Some(spec::typed_execution_spec_schema_id().map_err(|_| {
            AppError::backend(
                ErrorClass::Internal,
                "TypedSpecSchemaInvalid",
                "Typed execution spec schema identity is invalid",
            )
        })?),
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
        Some(
            mfm_certify::typed_spec_certificate_schema_id().map_err(|_| {
                AppError::backend(
                    ErrorClass::Internal,
                    "TypedSpecCertificateSchemaInvalid",
                    "Typed spec certificate schema identity is invalid",
                )
            })?,
        ),
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
            config_ref_artifact_requirement(config_ref)?,
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

fn fact_descriptor_launch_artifacts_for_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<RunLaunchArtifact>, AppError> {
    let schema_id = mfm_program::facts::fact_descriptor_schema_id().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "FactDescriptorSchemaInvalid",
            "Fact descriptor schema identity is invalid",
        )
    })?;
    let media_type = json_media_type()?;
    let required = runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(runtime_spec.spec().remediations.values())
        .flat_map(|node| {
            node.fact_descriptor_allowlist
                .iter()
                .map(|reference| reference.descriptor_hash.clone())
        })
        .collect::<BTreeSet<_>>();
    let mut artifacts = Vec::with_capacity(required.len());
    for descriptor_hash in required {
        let descriptor = registry
            .fact_descriptor_artifact(&descriptor_hash)
            .ok_or_else(|| {
                AppError::backend(
                    ErrorClass::Internal,
                    "FactDescriptorArtifactMissing",
                    "certified fact descriptor bytes are not available for launch",
                )
            })?;
        let artifact = launch_artifact(
            descriptor.bytes().to_vec(),
            media_type.clone(),
            Some(schema_id.clone()),
            None,
            None,
            events::ArtifactRole::FactDescriptor,
        );
        if artifact.evidence.digest != *descriptor.descriptor_hash()
            || artifact.evidence.digest != descriptor_hash
        {
            return Err(AppError::backend(
                ErrorClass::Internal,
                "FactDescriptorArtifactTampered",
                "certified fact descriptor bytes do not match their descriptor hash",
            ));
        }
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

fn framework_config_launch_artifacts_for_spec(
    execution_spec: &spec::TypedExecutionSpec,
) -> Result<Vec<RunLaunchConfigArtifact>, AppError> {
    let mut artifacts = Vec::new();
    for node in &execution_spec.nodes {
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
    execution_spec: &spec::TypedExecutionSpec,
    config_ref: &spec::ConfigRef,
    bytes: &[u8],
) -> Result<bool, AppError> {
    for node in &execution_spec.nodes {
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
            seed_artifact_requirement(seed_spec, &artifact.evidence)?,
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
                    content_digest: artifact.evidence.digest.clone(),
                    evidence_hash: artifact.evidence.evidence_hash()?,
                    byte_len: artifact.evidence.byte_len,
                    media_type: artifact.evidence.media_type.clone(),
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
        evidence_hash: expected.evidence_hash.clone(),
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

fn artifact_referenced_artifact_requirement(
    payload: &events::ArtifactReferenced,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: payload.artifact_ref.artifact_id.clone(),
        evidence_hash: payload.artifact_ref.evidence_hash.clone(),
        digest: Some(payload.artifact_ref.content_digest.clone()),
        byte_len: Some(payload.artifact_ref.byte_len),
        media_type: Some(payload.artifact_ref.media_type.clone()),
        schema_id: Some(payload.artifact_ref.schema_id.clone()),
        semantic_type_id: payload.artifact_ref.semantic_type_id.clone(),
        producer_node_id: payload.node_id.clone(),
        producer_seed_id: None,
        artifact_role: Some(payload.artifact_ref.role),
    }
}

fn config_ref_artifact_requirement(
    config_ref: &spec::ConfigRef,
) -> Result<store::EventArtifactRequirement, AppError> {
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: config_ref.artifact_id.clone(),
        digest: config_ref.digest.clone(),
        byte_len: config_ref.byte_len,
        media_type: config_ref.media_type.clone(),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    Ok(store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: config_ref.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash()?,
        digest: Some(config_ref.digest.clone()),
        byte_len: Some(config_ref.byte_len),
        media_type: Some(config_ref.media_type.clone()),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    })
}

fn seed_artifact_requirement(
    seed_spec: &spec::SeedSpec,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<store::EventArtifactRequirement, AppError> {
    Ok(store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::SeedCell,
        artifact_id: evidence.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash()?,
        digest: Some(evidence.digest.clone()),
        byte_len: Some(evidence.byte_len),
        media_type: Some(evidence.media_type.clone()),
        schema_id: Some(seed_spec.schema_id.clone()),
        semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed_spec.seed_id.clone()),
        artifact_role: Some(events::ArtifactRole::SeedInput),
    })
}

fn seed_cell_artifact_requirement(cell: &events::SeedCellRef) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::SeedCell,
        artifact_id: cell.seed_artifact.artifact_id.clone(),
        evidence_hash: cell.seed_artifact.evidence_hash.clone(),
        digest: Some(cell.seed_artifact.content_digest.clone()),
        byte_len: Some(cell.seed_artifact.byte_len),
        media_type: Some(cell.seed_artifact.media_type.clone()),
        schema_id: Some(cell.seed_artifact.schema_id.clone()),
        semantic_type_id: cell.seed_artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: Some(cell.seed_id.clone()),
        artifact_role: Some(cell.seed_artifact.role),
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
        evidence_hash: cell.evidence_hash.clone(),
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
) -> Result<store::EventArtifactRequirement, AppError> {
    let evidence_hash = payload
        .rendered_artifact_evidence_hash
        .clone()
        .ok_or_else(|| {
            AppError::new(
                ErrorClass::Internal,
                "RenderedArtifactEvidenceHashMissing",
                "public output rendered artifact is missing evidence hash",
            )
        })?;
    Ok(store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::PublicOutputRendered,
        artifact_id: artifact_id.clone(),
        evidence_hash,
        digest: Some(rendered_digest.clone()),
        byte_len: None,
        media_type: Some(media_type),
        schema_id: Some(payload.public_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(payload.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::PublicOutput),
    })
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

/// Resolves, plans, certifies, and prepares an entry-point op run launch.
pub fn prepare_entry_point_run_launch(
    input: EntryPointRunLaunchInput<'_>,
) -> Result<PreparedEntryPointRunLaunch, AppError> {
    let registry_digest = input.entry_point_registry.registry_digest()?;
    let op = input
        .entry_point_registry
        .resolve(&input.public_op_name, input.op_version)?;
    let resolved_op_id = op.op_id();
    let plan = op.plan(input.authored_config)?;

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
    let scoped_registry = input
        .certification_registry
        .scoped_for_spec(lowered.spec())
        .map_err(entry_point_certification_error)?;
    let certified_spec = mfm_certify::certify_typed_spec(lowered, &scoped_registry)
        .map_err(entry_point_certification_error)?;
    config_inputs.extend(framework_config_launch_artifacts_for_spec(
        &certified_spec.envelope().spec,
    )?);

    let evidence = EntryPointLaunchEvidence {
        resolved_op_id,
        entry_point_registry_digest: registry_digest,
    };
    let invocation_key_digest = invocation_key_digest_or_mint(input.invocation_key.as_ref())?;
    let runtime_entry_point_evidence = events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new(evidence.resolved_op_id.to_string()).map_err(
            |_| {
                entry_point_launch_internal_error(
                    "EntryPointLaunchEvidenceInvalid",
                    "entry-point launch evidence is invalid",
                )
            },
        )?,
        entry_point_registry_digest: evidence.entry_point_registry_digest.clone(),
    };
    let request = prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: &scoped_registry,
            store_scope_id: input.store_scope_id,
            invocation_key_digest,
            entry_point_evidence: runtime_entry_point_evidence,
        },
        config_inputs,
        seed_inputs,
    )?;
    Ok(PreparedEntryPointRunLaunch { request, evidence })
}

/// Prepares a certified typed run launch from a program draft for integration tests.
///
/// This helper is compiled only with `test-support`. It uses the same launch-material preparation
/// path as production app services after the caller has supplied an already-built typed program
/// draft and matching root seed material.
#[cfg(feature = "test-support")]
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
    let invocation_key_digest = invocation_key_digest_or_mint(invocation_key.as_ref())?;
    prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: &scoped_registry,
            store_scope_id,
            invocation_key_digest,
            entry_point_evidence: events::EntryPointLaunchEvidence {
                resolved_op_id: events::EntryPointOpId::new("mfm.test.typed_program_internal_test")
                    .map_err(|_| {
                        entry_point_launch_internal_error(
                            "EntryPointLaunchEvidenceInvalid",
                            "entry-point launch evidence is invalid",
                        )
                    })?,
                entry_point_registry_digest: content_digest_for_bytes(
                    b"mfm.app.test-support.typed-program-launch.v1",
                ),
            },
        },
        config_inputs,
        seed_inputs,
    )
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

fn run_status_from_projection(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
) -> Result<RunResponse, AppError> {
    let spec_hash = run_admitted_spec_hash(stream)?;
    run_status_from_projection_with_spec_hash(run_id, runtime_spec, stream, projection, &spec_hash)
}

fn run_status_from_projection_with_spec_hash(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
    spec_hash: &SpecHash,
) -> Result<RunResponse, AppError> {
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    let saga =
        projection.derive_saga_projection(run_id, &runtime_spec.spec().saga, &terminal_policies)?;
    Ok(RunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: spec_hash.as_str().to_owned(),
        run_mode: run_mode_status(saga.run_mode),
        saga: saga_status_with_resources(runtime_spec.spec(), projection, &saga),
        attempt_dispositions: attempt_dispositions(projection),
        scheduler_status: "observed".to_owned(),
        head_seq: stream_head(stream),
    })
}

fn run_stream_response_from_verified_context(
    context: &VerifiedRunReadContext,
) -> RunStreamResponse {
    let events = context.events();
    RunStreamResponse {
        run_id: context.view().run_id().as_str().to_owned(),
        head_seq: stream_head(events),
        events: events.iter().map(run_event_ref).collect(),
    }
}

/// Builds typed public-output read authority from certified runtime authority, verified run-history
/// view, rebuilt projection, and verified typed artifact evidence.
pub async fn public_output_read_authority_for_run(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
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
    let public_output = projection
        .public_output(verified_view.run_id(), public_schema_id)
        .ok_or_else(|| {
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
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    payload: &events::PublicOutputProduced,
    rendered_artifact_id: Option<&ArtifactId>,
    rendered_digest: &ContentDigest,
) -> Result<(), AppError> {
    for cell in &payload.cells {
        let artifact = artifacts
            .read_retained_artifact(&public_output_cell_artifact_requirement(cell))
            .await?;
        let evidence = artifact.evidence().clone();
        verify_public_output_cell_evidence(cell, &evidence)?;
    }
    if let Some(artifact_id) = rendered_artifact_id {
        let json_media_type = spec::MediaType::new("application/json").map_err(|error| {
            let _ = error;
            AppError::backend(
                ErrorClass::Internal,
                "PublicOutputMediaTypeInvalid",
                "Public-output JSON media type is invalid",
            )
        })?;
        let requirement = public_output_rendered_artifact_requirement(
            payload,
            artifact_id,
            rendered_digest,
            json_media_type,
        )?;
        let artifact = artifacts.read_retained_artifact(&requirement).await?;
        let evidence = artifact.evidence().clone();
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
pub async fn render_public_output(
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    authority: &PublicOutputReadAuthority,
) -> Result<PublicOutputResponse, AppError> {
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
    Ok(PublicOutputResponse {
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
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    authority: &PublicOutputReadAuthority,
) -> Result<Value, AppError> {
    let mut root = Map::new();
    for cell in &authority.payload.cells {
        let artifact = artifacts
            .read_retained_artifact(&public_output_cell_artifact_requirement(cell))
            .await?;
        let evidence = artifact.evidence().clone();
        verify_public_output_cell_evidence(cell, &evidence)?;
        let bytes = artifact.into_bytes();
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
    artifacts: &(impl store::RetainedArtifactReadProvider + ?Sized),
    artifact_id: &ArtifactId,
    rendered_digest: &mfm_ids::ContentDigest,
    payload: &events::PublicOutputProduced,
) -> Result<serde_json::Value, AppError> {
    let json_media_type = spec::MediaType::new("application/json").map_err(|error| {
        let _ = error;
        AppError::backend(
            ErrorClass::Internal,
            "PublicOutputMediaTypeInvalid",
            "Public-output JSON media type is invalid",
        )
    })?;
    let requirement = public_output_rendered_artifact_requirement(
        payload,
        artifact_id,
        rendered_digest,
        json_media_type,
    )?;
    let artifact = artifacts.read_retained_artifact(&requirement).await?;
    let evidence = artifact.evidence().clone();
    verify_public_output_rendered_artifact_evidence(
        &evidence,
        artifact_id,
        rendered_digest,
        payload,
    )?;
    let bytes = artifact.into_bytes();
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
        )?,
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

fn run_response_from_projection(
    run_id: &RunId,
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
    projection: &store::ProjectionSnapshot,
    status: DriveStatus,
) -> Result<RunResponse, AppError> {
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    let saga =
        projection.derive_saga_projection(run_id, &runtime_spec.spec().saga, &terminal_policies)?;
    Ok(RunResponse {
        run_id: run_id.as_str().to_owned(),
        spec_hash: runtime_spec.spec_hash().as_str().to_owned(),
        run_mode: run_mode_status(saga.run_mode),
        saga: saga_status_with_resources(runtime_spec.spec(), projection, &saga),
        attempt_dispositions: attempt_dispositions(projection),
        scheduler_status: status.as_str().to_owned(),
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
    let mut parts = store::ProjectionSnapshotParts::from_snapshot(snapshot);
    parts.resource_lanes = resource_lanes;
    store::ProjectionSnapshot::from_parts(parts)
}

fn stream_head(stream: &[store::KernelEventEnvelope]) -> u64 {
    stream.last().map_or(0, |event| event.seq().as_u64())
}

fn run_mode_status(mode: store::RunMode) -> RunModeStatus {
    mode.into()
}

fn attempt_dispositions(projection: &store::ProjectionSnapshot) -> Vec<AttemptDispositionStatus> {
    projection
        .attempts()
        .map(|(_key, attempt)| attempt_disposition(attempt))
        .collect()
}

pub(crate) fn attempt_disposition(attempt: &store::AttemptProjection) -> AttemptDispositionStatus {
    let (disposition, attempt_no, retryable, error_code, output_cell_id) = match &attempt.status {
        store::AttemptStatus::Started { attempt_no, .. } => {
            ("started", Some(*attempt_no), None, None, None)
        }
        store::AttemptStatus::Completed { output_cell_id } => (
            "completed",
            None,
            None,
            None,
            Some(output_cell_id.as_str().to_owned()),
        ),
        store::AttemptStatus::Failed { retryable, error } => (
            "failed",
            None,
            Some(*retryable),
            Some(error.code.as_str().to_owned()),
            None,
        ),
        store::AttemptStatus::Interrupted => ("interrupted", None, None, None, None),
    };
    AttemptDispositionStatus {
        node_id: attempt.node_id.as_str().to_owned(),
        attempt_id: attempt.attempt_id.as_str().to_owned(),
        disposition: disposition.to_owned(),
        attempt_no,
        retryable,
        error_code,
        output_cell_id,
    }
}

fn saga_status_with_resources(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    saga: &store::SagaProjection,
) -> SagaStatus {
    saga_status_inner(
        &certified_spec.saga,
        Some(certified_spec),
        Some(projection),
        saga,
    )
}

fn saga_status_inner(
    policy: &spec::SagaPolicySpec,
    certified_spec: Option<&spec::TypedExecutionSpec>,
    projection: Option<&store::ProjectionSnapshot>,
    saga: &store::SagaProjection,
) -> SagaStatus {
    SagaStatus {
        policy: saga_policy_status(policy),
        obligations: saga
            .obligations
            .values()
            .map(|obligation| {
                obligation_status(&saga.run_id, obligation, certified_spec, projection)
            })
            .collect(),
        resource_ledgers: match (certified_spec, projection) {
            (Some(certified_spec), Some(projection)) => {
                resource_ledgers_for_run(certified_spec, projection, &saga.run_id)
            }
            _ => Vec::new(),
        },
        resource_lanes: projection
            .map(|projection| resource_lanes_for_run(projection, &saga.run_id))
            .unwrap_or_default(),
        manual_block_reason: saga.manual_block_reason.map(manual_block_reason_str),
        required_manual_authorization: matches!(saga.run_mode, store::RunMode::ManualBlocked)
            .then(|| manual_authorization_for_policy(policy))
            .flatten(),
        terminal_resolution: saga
            .run_completion
            .as_ref()
            .map(|completion| terminal_resolution_status(&completion.outcome)),
    }
}

fn saga_policy_status(policy: &spec::SagaPolicySpec) -> SagaPolicyStatus {
    match policy {
        spec::SagaPolicySpec::NoSideEffects => SagaPolicyStatus {
            variant: "no_side_effects".to_owned(),
            manual_authorization: None,
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::FailWithoutAcdcClaim => SagaPolicyStatus {
            variant: "fail_without_acdc_claim".to_owned(),
            manual_authorization: None,
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::ManualResolution { manual } => SagaPolicyStatus {
            variant: "manual_resolution".to_owned(),
            manual_authorization: Some(manual_authorization_requirements(manual)),
            on_remediation_unresolved: None,
        },
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved,
        } => {
            let (directive, manual_authorization) = match on_remediation_unresolved {
                spec::RemediationUnresolvedSpec::ManualResolution { manual } => (
                    "manual_resolution",
                    Some(manual_authorization_requirements(manual)),
                ),
                spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim => {
                    ("fail_without_acdc_claim", None)
                }
            };
            SagaPolicyStatus {
                variant: "compensate_completed".to_owned(),
                manual_authorization,
                on_remediation_unresolved: Some(directive.to_owned()),
            }
        }
    }
}

fn manual_authorization_for_policy(
    policy: &spec::SagaPolicySpec,
) -> Option<ManualAuthorizationRequirements> {
    match policy {
        spec::SagaPolicySpec::ManualResolution { manual } => {
            Some(manual_authorization_requirements(manual))
        }
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => Some(manual_authorization_requirements(manual)),
        spec::SagaPolicySpec::NoSideEffects
        | spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim,
        } => None,
    }
}

fn manual_authorization_requirements(
    manual: &spec::ManualResolutionEvidenceSpec,
) -> ManualAuthorizationRequirements {
    ManualAuthorizationRequirements {
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

fn obligation_status(
    run_id: &RunId,
    obligation: &store::SagaObligationProjection,
    certified_spec: Option<&spec::TypedExecutionSpec>,
    projection: Option<&store::ProjectionSnapshot>,
) -> SagaObligationStatus {
    let forward_resource = projection
        .and_then(|projection| projection.side_effect_for_pair(run_id, &obligation.forward_pair_id))
        .and_then(|side_effect| resource_ledger_status(certified_spec?, projection?, side_effect));
    SagaObligationStatus {
        forward_ledger_key: obligation.forward_ledger_key.as_str().to_owned(),
        forward_phase: side_effect_phase_str(&obligation.forward_phase),
        classification: forward_classification_str(obligation.classification),
        resource: forward_resource,
        remediation: obligation.remediation.as_ref().map(|remediation| {
            let remediation_resource = projection
                .and_then(|projection| {
                    projection.side_effect_for_pair(run_id, &remediation.pair_id)
                })
                .and_then(|side_effect| {
                    resource_ledger_status(certified_spec?, projection?, side_effect)
                });
            RemediationLedgerStatus {
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

fn resource_ledger_status(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    side_effect: &store::SideEffectProjection,
) -> Option<ResourceLedgerStatus> {
    let node = certified_node(certified_spec, &side_effect.intent.node_id)?;
    let claim = &node.side_effect.as_ref()?.resource_claim;
    let key = side_effect.resource_key.as_ref().map(resource_key_status);
    let touched_set = side_effect
        .resource_touched_set
        .as_ref()
        .map(resource_touched_set_status);
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
                    let holder = resource_lane_holder_status(projection, &lane_key, lane);
                    let side_effect_ref = store::SideEffectPairLedgerRef::new(
                        side_effect.run_id.clone(),
                        side_effect.pair_id.clone(),
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
    let ledger_purpose = ledger_purpose_status(&side_effect.ledger_purpose);
    let forward_ledger_key =
        forward_ledger_key_status(projection, &side_effect.run_id, &side_effect.ledger_purpose);

    Some(ResourceLedgerStatus {
        ledger_key: side_effect.ledger_key.as_str().to_owned(),
        ledger_purpose,
        forward_ledger_key,
        phase: side_effect_phase_str(&side_effect.phase),
        claim: resource_claim_status(claim),
        key,
        touched_set,
        active_lane,
        blocked_by_lane,
    })
}

fn resource_ledgers_for_run(
    certified_spec: &spec::TypedExecutionSpec,
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
) -> Vec<ResourceLedgerStatus> {
    projection
        .side_effects()
        .filter_map(|(_, side_effect)| {
            if &side_effect.run_id == run_id {
                resource_ledger_status(certified_spec, projection, side_effect)
            } else {
                None
            }
        })
        .collect()
}

fn resource_lanes_for_run(
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
) -> Vec<ResourceLaneHolderStatus> {
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
        .map(|(lane_key, lane)| resource_lane_holder_status(projection, lane_key, lane))
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

fn resource_claim_status(claim: &spec::ResourceClaimSpec) -> ResourceClaimStatus {
    match claim {
        spec::ResourceClaimSpec::Exclusive {
            namespace,
            key_schema,
        } => ResourceClaimStatus {
            kind: "exclusive".to_owned(),
            namespace: Some(namespace.as_str().to_owned()),
            key_schema_id: Some(key_schema.as_str().to_owned()),
            evidence_schema_id: None,
        },
        spec::ResourceClaimSpec::ExactTouchedSet {
            namespace,
            evidence_schema,
        } => ResourceClaimStatus {
            kind: "exact_touched_set".to_owned(),
            namespace: Some(namespace.as_str().to_owned()),
            key_schema_id: None,
            evidence_schema_id: Some(evidence_schema.as_str().to_owned()),
        },
        spec::ResourceClaimSpec::ManualOnly => ResourceClaimStatus {
            kind: "manual_only".to_owned(),
            namespace: None,
            key_schema_id: None,
            evidence_schema_id: None,
        },
    }
}

fn resource_key_status(evidence: &events::ResourceKeyEvidence) -> ResourceKeyStatus {
    ResourceKeyStatus {
        namespace: evidence.namespace.as_str().to_owned(),
        key_schema_id: evidence.key_schema_id.as_str().to_owned(),
        key_digest: resource_key_evidence_digest(evidence),
    }
}

fn resource_touched_set_status(
    evidence: &events::ResourceTouchedSetEvidence,
) -> ResourceTouchedSetStatus {
    ResourceTouchedSetStatus {
        namespace: evidence.namespace.as_str().to_owned(),
        evidence_schema_id: evidence.evidence_schema_id.as_str().to_owned(),
        evidence_hash: evidence.evidence_hash.as_str().to_owned(),
        evidence_artifact_id: evidence.evidence_artifact_id.as_str().to_owned(),
    }
}

fn resource_lane_holder_status(
    projection: &store::ProjectionSnapshot,
    lane_key: &store::ResourceLaneKey,
    lane: &store::ResourceLaneProjection,
) -> ResourceLaneHolderStatus {
    let holding_ledger_purpose = ledger_purpose_status(&lane.ledger_purpose);
    let holding_forward_ledger_key =
        forward_ledger_key_status(projection, &lane.holder.run_id, &lane.ledger_purpose);
    ResourceLaneHolderStatus {
        namespace: lane_key.namespace.as_str().to_owned(),
        key_schema_id: lane_key.key_schema_id.as_str().to_owned(),
        key_digest: resource_lane_key_digest(lane_key),
        holding_run_id: lane.holder.run_id.as_str().to_owned(),
        holding_ledger_key: lane.ledger_key.as_str().to_owned(),
        holding_ledger_purpose,
        holding_forward_ledger_key,
        holding_node_id: lane.node_id.as_str().to_owned(),
        holding_attempt_id: lane.attempt_id.as_str().to_owned(),
        invocation_epoch: lane.invocation_epoch,
    }
}

#[derive(Serialize)]
struct ResourceKeyDigestMaterial<'a> {
    key: &'a str,
    key_schema_id: &'a str,
    namespace: &'a str,
}

fn resource_key_evidence_digest(evidence: &events::ResourceKeyEvidence) -> String {
    resource_key_digest_material(ResourceKeyDigestMaterial {
        key: evidence.key.as_str(),
        key_schema_id: evidence.key_schema_id.as_str(),
        namespace: evidence.namespace.as_str(),
    })
}

fn resource_lane_key_digest(lane_key: &store::ResourceLaneKey) -> String {
    resource_key_digest_material(ResourceKeyDigestMaterial {
        key: lane_key.key.as_str(),
        key_schema_id: lane_key.key_schema_id.as_str(),
        namespace: lane_key.namespace.as_str(),
    })
}

fn resource_key_digest_material(material: ResourceKeyDigestMaterial<'_>) -> String {
    let json = serde_json::to_string(&material).expect("resource key digest material serializes");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("resource key digest material is JSON");
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(canonical.as_bytes()),
    )
    .to_string()
}

fn ledger_purpose_status(purpose: &events::SideEffectLedgerPurpose) -> String {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => "forward".to_owned(),
        events::SideEffectLedgerPurpose::Remediation { .. } => "remediation".to_owned(),
    }
}

fn forward_ledger_key_status(
    projection: &store::ProjectionSnapshot,
    run_id: &RunId,
    purpose: &events::SideEffectLedgerPurpose,
) -> Option<String> {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => None,
        events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => projection
            .side_effect_for_pair(run_id, forward_pair_id)
            .map(|side_effect| side_effect.ledger_key.as_str().to_owned()),
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

fn terminal_resolution_status(outcome: &events::RunCompletionOutcome) -> TerminalResolutionStatus {
    TerminalResolutionStatus {
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

pub(crate) fn run_event_ref(event: &store::KernelEventEnvelope) -> RunEventRef {
    RunEventRef {
        event_id: event.event_id().as_str().to_owned(),
        event_schema_id: event.event_schema_id().as_str().to_owned(),
        seq: event.seq().as_u64(),
        ordinal: event.ordinal().as_u32(),
        commit_key: event.commit_key().as_str().to_owned(),
        logical_key: event.logical_key().as_str().to_owned(),
        payload_hash: event.payload_hash().as_str().to_owned(),
        error_code: match event.payload() {
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                Some(payload.error.code.as_str().to_owned())
            }
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests;
