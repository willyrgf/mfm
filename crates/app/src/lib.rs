#![warn(missing_docs)]
//! Typed transport-neutral client use cases over one checked live composition.

use std::fmt;
use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_config::{ConfigImportResult, ConfigRepository, ConfigRepositoryError};
use mfm_evm::{EvmEndpoint, EvmPhysicalTarget};
use mfm_evm_live::{register_evm_reads, EvmAdapterLocator, EvmProvider, JsonRpcEvmProvider};
use mfm_ids::{ContentRef, RunId};
use mfm_portfolio::PortfolioError;
use mfm_runtime::{
    RetainedValueView, RunView, RunViewState, Runtime, RuntimeAssemblyBuilder, RuntimeError,
};
use mfm_storage_postgres::{
    provision_postgres as provision_postgres_backend, AdminPostgresLocator, PostgresBackend,
    RuntimePostgresLocator,
};
use mfm_store::{RunIndex, RunIndexError, Store};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

mod config;
mod deployment;
mod inspection;

pub use config::{
    ConfigDocument, ConfigDocumentError, ConfigSummary, EntryPointSummary, ImportOutcome,
};
pub use deployment::{
    Deployment, EnvironmentName, EnvironmentNameError, MAX_DEPLOYMENT_DOCUMENT_BYTES,
};
pub use inspection::{ComponentKind, ComponentSummary};
pub use mfm_config::{ConfigDigest, ConfigName, MAX_CONFIG_DOCUMENT_BYTES};
pub use mfm_store::{RunPage, RunPageLimit};

use config::ENTRY_POINTS;
use deployment::resolve_environment;

/// Maximum number of EVM capability bindings in one composed Runtime.
pub const MAX_EVM_BINDINGS: usize = 256;

/// Registers every Portfolio and EVM State implementation the snapshot Program declares.
///
/// [`ComposedRuntime`] is the composition trusted callers want. This entry stays public only for
/// adapterless tests that prove association rejects a Program before Store IO.
pub fn register_portfolio_states(builder: &mut RuntimeAssemblyBuilder) -> mfm_runtime::Result<()> {
    inspection::register_states(builder)
}

/// Redaction-safe live composition failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    /// Deployment path, bytes, or strict TOML were invalid or unavailable.
    #[error("deployment bootstrap is invalid or unavailable")]
    Deployment,
    /// A checked environment resolver had no usable value.
    #[error("environment variable {0} is not set")]
    Environment(EnvironmentName),
    /// The runtime PostgreSQL locator was invalid.
    #[error("postgres locator is invalid or unavailable")]
    PostgresLocator,
    /// PostgreSQL provisioning failed.
    #[error("postgres provisioning failed")]
    Provision,
    /// The PostgreSQL persistence backend could not be opened.
    #[error("postgres backend is invalid or unavailable")]
    Postgres,
    /// An EVM locator or HTTP client could not be constructed.
    #[error("evm provider transport could not be constructed")]
    Provider,
    /// Typed binding or immutable Runtime assembly construction failed.
    #[error("application composition is invalid")]
    Assembly,
}

/// One public capability binding derived from the exact typed live binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicBindingView {
    /// One EVM observational route.
    Evm {
        /// Public chain identity.
        chain_id: u64,
        /// Stable public endpoint name.
        endpoint_id: String,
        /// Exact adapter binding reference derived from the physical target.
        binding_ref: ContentRef,
    },
}

struct BoundEvmRoute {
    target: EvmPhysicalTarget,
    endpoint_id: String,
    provider: Arc<dyn EvmProvider>,
}

/// Checked typed capability bindings consumed by one [`ComposedRuntime`].
pub struct BoundCapabilitySet {
    evm: Vec<BoundEvmRoute>,
}

impl BoundCapabilitySet {
    /// Checks one stable, strictly ordered EVM route set.
    ///
    /// Empty sets are valid; at most 256 routes are accepted.
    pub fn new(
        routes: Vec<(u64, EvmEndpoint, Arc<dyn EvmProvider>)>,
    ) -> Result<Self, ComposeError> {
        if routes.len() > MAX_EVM_BINDINGS
            || routes.windows(2).any(|pair| {
                (pair[0].0, pair[0].1.endpoint_id()) >= (pair[1].0, pair[1].1.endpoint_id())
            })
        {
            return Err(ComposeError::Assembly);
        }
        let mut evm = Vec::new();
        evm.try_reserve_exact(routes.len())
            .map_err(|_| ComposeError::Assembly)?;
        for (chain_id, endpoint, provider) in routes {
            let endpoint_ref = endpoint
                .endpoint_ref()
                .map_err(|_| ComposeError::Assembly)?;
            let target = EvmPhysicalTarget::new(chain_id, endpoint_ref)
                .map_err(|_| ComposeError::Assembly)?;
            evm.push(BoundEvmRoute {
                target,
                endpoint_id: endpoint.endpoint_id().to_owned(),
                provider,
            });
        }
        Ok(Self { evm })
    }
}

/// One Runtime, RunIndex, typed planning targets, and exact public binding views built together.
pub struct ComposedRuntime {
    runtime: Runtime,
    run_index: Arc<dyn RunIndex>,
    targets: Vec<EvmPhysicalTarget>,
    bindings: Vec<PublicBindingView>,
}

impl ComposedRuntime {
    /// Builds the complete Portfolio assembly from one backend and checked binding set.
    pub fn compose<B>(backend: Arc<B>, bindings: BoundCapabilitySet) -> Result<Self, ComposeError>
    where
        B: Store + RunIndex + 'static,
    {
        let mut builder = RuntimeAssemblyBuilder::new();
        register_portfolio_states(&mut builder).map_err(|_| ComposeError::Assembly)?;
        let mut targets = Vec::new();
        let mut views = Vec::new();
        targets
            .try_reserve_exact(bindings.evm.len())
            .map_err(|_| ComposeError::Assembly)?;
        views
            .try_reserve_exact(bindings.evm.len())
            .map_err(|_| ComposeError::Assembly)?;
        for binding in bindings.evm {
            let binding_ref = binding
                .target
                .binding_ref()
                .map_err(|_| ComposeError::Assembly)?;
            views.push(PublicBindingView::Evm {
                chain_id: binding.target.chain_id(),
                endpoint_id: binding.endpoint_id,
                binding_ref,
            });
            targets.push(binding.target.clone());
            register_evm_reads(&mut builder, binding.target, binding.provider)
                .map_err(|_| ComposeError::Assembly)?;
        }
        let assembly = builder.finish().map_err(|_| ComposeError::Assembly)?;
        let store: Arc<dyn Store> = backend.clone();
        let run_index: Arc<dyn RunIndex> = backend;
        Ok(Self {
            runtime: Runtime::new(assembly, store),
            run_index,
            targets,
            bindings: views,
        })
    }

    fn has_targets(&self, targets: &[EvmPhysicalTarget]) -> bool {
        targets.iter().all(|target| self.targets.contains(target))
    }
}

/// Stable request-level application failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// The selected config name is absent.
    #[error("config is absent")]
    ConfigAbsent,
    /// The complete config cannot be planned.
    #[error("config document is invalid")]
    InvalidConfigDocument,
    /// A configuration mutation may have committed.
    #[error("config mutation outcome is indeterminate")]
    ConfigMutationIndeterminate,
    /// Retained configuration bytes or metadata are invalid.
    #[error("retained config is invalid")]
    InvalidRetainedConfig,
    /// The run is absent.
    #[error("run is absent")]
    RunAbsent,
    /// Admission differs from retained genesis.
    #[error("run admission conflicts with retained history")]
    RunAdmissionConflict,
    /// Retained run history is invalid.
    #[error("retained run history is invalid")]
    InvalidRunHistory,
    /// The immutable assembly cannot execute retained history.
    #[error("runtime assembly is incompatible")]
    IncompatibleAssembly,
    /// A run capacity was exceeded.
    #[error("run capacity exceeded")]
    RunCapacity,
    /// A config selects a capability not present in the composition.
    #[error("required capability binding is unavailable")]
    BindingUnbound,
    /// Retained mechanical run heads are invalid.
    #[error("retained run index is invalid")]
    InvalidRunIndex,
    /// A required store, configuration repository, or provider is unavailable.
    #[error("application dependency is unavailable")]
    DependencyUnavailable,
    /// A trusted local invariant failed.
    #[error("application internal failure")]
    Internal,
}

impl RequestError {
    /// Returns the stable machine-readable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ConfigAbsent => "config_absent",
            Self::InvalidConfigDocument => "invalid_config_document",
            Self::ConfigMutationIndeterminate => "config_mutation_indeterminate",
            Self::InvalidRetainedConfig => "invalid_retained_config",
            Self::RunAbsent => "run_absent",
            Self::RunAdmissionConflict => "run_admission_conflict",
            Self::InvalidRunHistory => "invalid_run_history",
            Self::IncompatibleAssembly => "incompatible_assembly",
            Self::RunCapacity => "run_capacity",
            Self::BindingUnbound => "binding_unbound",
            Self::InvalidRunIndex => "invalid_run_index",
            Self::DependencyUnavailable => "dependency_unavailable",
            Self::Internal => "internal",
        }
    }
}

/// Exact stored-configuration selection for one run start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSelection {
    name: ConfigName,
    digest: ConfigDigest,
}

impl ConfigSelection {
    /// Constructs an exact retained revision selection.
    pub const fn new(name: ConfigName, digest: ConfigDigest) -> Self {
        Self { name, digest }
    }

    /// Returns the selected configuration name.
    pub const fn name(&self) -> &ConfigName {
        &self.name
    }

    /// Returns the selected configuration digest.
    pub const fn digest(&self) -> &ConfigDigest {
        &self.digest
    }
}

/// Public recovery identity for an ambiguously acknowledged run append.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunRecovery {
    /// Recover or retry one start using the exact selected revision.
    Start {
        /// Explicit caller-owned run identity.
        run_id: RunId,
        /// Exact config revision selected before the append.
        config: ConfigSummary,
    },
    /// Read or retry progression for an already admitted run.
    Progress {
        /// Explicit run identity.
        run_id: RunId,
    },
}

/// Borrowed client-error JSON model shared by transport renderers.
pub struct SerializableClientError<'a> {
    code: &'a str,
    message: &'a str,
    detail: ClientErrorDetail<'a>,
}

#[derive(Clone, Copy)]
enum ClientErrorDetail<'a> {
    None,
    RunId(&'a RunId),
    Recovery(&'a RunRecovery),
}

impl<'a> SerializableClientError<'a> {
    /// Constructs an error with no identity or recovery detail.
    pub const fn new(code: &'a str, message: &'a str) -> Self {
        Self {
            code,
            message,
            detail: ClientErrorDetail::None,
        }
    }

    /// Constructs a start error carrying its selected run identity.
    pub const fn identified(code: &'a str, message: &'a str, run_id: &'a RunId) -> Self {
        Self {
            code,
            message,
            detail: ClientErrorDetail::RunId(run_id),
        }
    }

    /// Constructs an append error carrying its exact recovery instruction.
    pub const fn recoverable(code: &'a str, message: &'a str, recovery: &'a RunRecovery) -> Self {
        Self {
            code,
            message,
            detail: ClientErrorDetail::Recovery(recovery),
        }
    }
}

impl Serialize for SerializableClientError<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct(
            "ClientError",
            2 + usize::from(!matches!(self.detail, ClientErrorDetail::None)),
        )?;
        state.serialize_field("code", self.code)?;
        state.serialize_field("message", self.message)?;
        match self.detail {
            ClientErrorDetail::None => {}
            ClientErrorDetail::RunId(run_id) => state.serialize_field("run_id", run_id)?,
            ClientErrorDetail::Recovery(recovery) => state.serialize_field("recovery", recovery)?,
        }
        state.end()
    }
}

/// Run mutation failure with exact recovery identity for ambiguous append acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
// The public recovery sum deliberately retains its checked fields inline; append ambiguity is an
// exceptional path and changing the variant to an allocation-shaped API would weaken that contract.
#[allow(clippy::large_enum_variant)]
pub enum RunRequestError {
    /// Ordinary shared request failure.
    #[error("{0}")]
    Request(RequestError),
    /// The append may have committed.
    #[error("run append outcome is indeterminate")]
    AppendIndeterminate {
        /// Checked public recovery identity.
        recovery: RunRecovery,
    },
}

impl RunRequestError {
    /// Returns the stable machine-readable error code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Request(error) => error.code(),
            Self::AppendIndeterminate { .. } => "run_append_indeterminate",
        }
    }

    /// Returns recovery identity only for an ambiguously acknowledged append.
    pub const fn recovery(&self) -> Option<&RunRecovery> {
        match self {
            Self::Request(_) => None,
            Self::AppendIndeterminate { recovery } => Some(recovery),
        }
    }
}

impl From<RequestError> for RunRequestError {
    fn from(error: RequestError) -> Self {
        Self::Request(error)
    }
}

/// One selected config revision and its resulting run view.
pub struct StartRunResult {
    config: ConfigSummary,
    run: RunView,
}

impl StartRunResult {
    /// Returns the exact selected config revision.
    pub const fn config(&self) -> &ConfigSummary {
        &self.config
    }

    /// Returns the resulting durable run view.
    pub const fn run(&self) -> &RunView {
        &self.run
    }
}

impl Serialize for StartRunResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("StartRunResult", 2)?;
        state.serialize_field("config", &self.config)?;
        state.serialize_field("run", &SerializableRunView::new(&self.run))?;
        state.end()
    }
}

/// Borrowed exact JSON serializer for one [`RunView`].
pub struct SerializableRunView<'a> {
    view: &'a RunView,
}

impl<'a> SerializableRunView<'a> {
    /// Wraps one run view without changing its retained bytes.
    pub const fn new(view: &'a RunView) -> Self {
        Self { view }
    }
}

impl Serialize for SerializableRunView<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("RunView", 4)?;
        state.serialize_field("run_id", self.view.run_id())?;
        state.serialize_field("head_sequence", &self.view.head_sequence())?;
        state.serialize_field("head_digest", self.view.head_digest())?;
        match self.view.state() {
            RunViewState::Runnable => {
                state.serialize_field("state", &RunnableState { kind: "runnable" })?
            }
            RunViewState::Succeeded(value) => {
                state.serialize_field("state", &TerminalState::new("succeeded", value))?
            }
            RunViewState::Failed(value) => {
                state.serialize_field("state", &TerminalState::new("failed", value))?
            }
        }
        state.end()
    }
}

#[derive(Serialize)]
struct RunnableState {
    kind: &'static str,
}

struct TerminalState<'a> {
    kind: &'static str,
    value: &'a RetainedValueView,
}

impl<'a> TerminalState<'a> {
    const fn new(kind: &'static str, value: &'a RetainedValueView) -> Self {
        Self { kind, value }
    }
}

impl Serialize for TerminalState<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let raw = std::str::from_utf8(self.value.canonical_bytes())
            .ok()
            .and_then(|value| RawValue::from_string(value.to_owned()).ok())
            .ok_or_else(|| serde::ser::Error::custom("retained canonical value is invalid"))?;
        let mut state = serializer.serialize_struct("RunViewState", 4)?;
        state.serialize_field("kind", self.kind)?;
        state.serialize_field("contract_ref", self.value.contract_ref())?;
        state.serialize_field("value_ref", self.value.value_ref())?;
        state.serialize_field("value", &raw)?;
        state.end()
    }
}

/// Borrowed list JSON model shared by client transports.
#[derive(Serialize)]
pub struct ItemList<'a, T> {
    items: &'a [T],
}

impl<'a, T> ItemList<'a, T> {
    /// Wraps one ordered item slice.
    pub const fn new(items: &'a [T]) -> Self {
        Self { items }
    }
}

/// Transport-neutral application use-case surface.
pub struct Application {
    composed: ComposedRuntime,
    configs: Arc<dyn ConfigRepository>,
}

impl Application {
    /// Constructs an Application from one checked Runtime/index composition and config repository.
    pub fn from_parts(composed: ComposedRuntime, configs: Arc<dyn ConfigRepository>) -> Self {
        Self { composed, configs }
    }

    /// Resolves private locators once and constructs the production PostgreSQL/EVM Application.
    pub async fn open(deployment: &Deployment) -> Result<Self, ComposeError> {
        let postgres_value = resolve_environment(deployment.runtime_postgres_locator_env())?;
        let postgres_locator = RuntimePostgresLocator::parse(postgres_value)
            .map_err(|_| ComposeError::PostgresLocator)?;
        let mut routes = Vec::new();
        routes
            .try_reserve_exact(deployment.evm_routes().len())
            .map_err(|_| ComposeError::Assembly)?;
        for route in deployment.evm_routes() {
            let locator_value = resolve_environment(route.adapter_locator_env())?;
            let locator =
                EvmAdapterLocator::parse(locator_value).map_err(|_| ComposeError::Provider)?;
            let provider: Arc<dyn EvmProvider> = Arc::new(
                JsonRpcEvmProvider::connect(&locator).map_err(|_| ComposeError::Provider)?,
            );
            routes.push((route.chain_id(), route.endpoint().clone(), provider));
        }
        let postgres = Arc::new(
            PostgresBackend::connect(&postgres_locator)
                .await
                .map_err(|_| ComposeError::Postgres)?,
        );
        let configs: Arc<dyn ConfigRepository> = postgres.clone();
        let bindings = BoundCapabilitySet::new(routes)?;
        let composed = ComposedRuntime::compose(postgres, bindings)?;
        Ok(Self::from_parts(composed, configs))
    }

    /// Returns the strictly ordered compiled entry points.
    pub const fn entry_points() -> &'static [EntryPointSummary] {
        &ENTRY_POINTS
    }

    /// Returns every definition admitted by the compiled product composition.
    pub fn components() -> Vec<ComponentSummary> {
        inspection::components()
    }

    /// Returns stable public bindings derived from the exact composed adapter targets.
    pub fn bindings(&self) -> &[PublicBindingView] {
        &self.composed.bindings
    }

    /// Validates and conditionally imports one complete config revision.
    pub async fn import_config(
        &self,
        name: ConfigName,
        document: ConfigDocument,
    ) -> Result<ImportOutcome, RequestError> {
        let document = tokio::task::spawn_blocking(move || match document.plan() {
            Ok(_) => Ok(document),
            Err(PortfolioError::InvalidValue) => Err(RequestError::InvalidConfigDocument),
            Err(PortfolioError::InvalidContinuation | PortfolioError::Program) => {
                Err(RequestError::Internal)
            }
        })
        .await
        .map_err(|_| RequestError::Internal)??;
        let summary = document.summary(name.clone());
        let entry = document
            .revision(name)
            .map_err(|_| RequestError::Internal)?;
        match self
            .configs
            .import_config(&entry)
            .await
            .map_err(map_config_repository_error)?
        {
            ConfigImportResult::Created => Ok(ImportOutcome::Created { config: summary }),
            ConfigImportResult::Unchanged => Ok(ImportOutcome::Unchanged { config: summary }),
        }
    }

    /// Lists and revalidates every retained configuration revision.
    pub async fn list_configs(&self) -> Result<Vec<ConfigSummary>, RequestError> {
        let entries = self
            .configs
            .list_configs()
            .await
            .map_err(map_config_repository_error)?;
        tokio::task::spawn_blocking(move || {
            let items = entries
                .into_iter()
                .map(|entry| {
                    let (name, digest, canonical) = entry.into_parts();
                    ConfigDocument::parse_retained(canonical, &digest)
                        .map(|document| document.summary(name))
                        .map_err(|_| RequestError::InvalidRetainedConfig)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })
        .await
        .map_err(|_| RequestError::Internal)?
    }

    /// Idempotently deletes one exact retained configuration revision.
    pub async fn delete_config(
        &self,
        name: &ConfigName,
        digest: &ConfigDigest,
    ) -> Result<(), RequestError> {
        self.configs
            .delete_config(name, digest)
            .await
            .map_err(map_config_repository_error)
    }

    /// Selects a stored config, plans it, admits the exact RunId, and progresses the run.
    pub async fn start_run(
        &self,
        run_id: RunId,
        selection: &ConfigSelection,
    ) -> Result<StartRunResult, RunRequestError> {
        let entry = self
            .configs
            .load_config(selection.name(), selection.digest())
            .await
            .map_err(map_config_repository_error)?
            .ok_or(RequestError::ConfigAbsent)?;
        let (name, digest, canonical) = entry.into_parts();
        let (document, program, c0) = tokio::task::spawn_blocking(move || {
            let document = ConfigDocument::parse_retained(canonical, &digest)
                .map_err(|_| RequestError::InvalidRetainedConfig)?;
            let (program, c0) = match document.plan() {
                Ok(planned) => planned,
                Err(PortfolioError::InvalidValue) => {
                    return Err(RequestError::InvalidRetainedConfig)
                }
                Err(PortfolioError::InvalidContinuation | PortfolioError::Program) => {
                    return Err(RequestError::Internal);
                }
            };
            Ok((document, program, c0))
        })
        .await
        .map_err(|_| RequestError::Internal)??;
        let config = document.summary(name);
        if !self.composed.has_targets(document.targets()) {
            return Err(RequestError::BindingUnbound.into());
        }
        match self
            .composed
            .runtime
            .start(run_id.clone(), program, c0)
            .await
        {
            Ok(run) => Ok(StartRunResult { config, run }),
            Err(RuntimeError::Indeterminate) => Err(RunRequestError::AppendIndeterminate {
                recovery: RunRecovery::Start { run_id, config },
            }),
            Err(error) => Err(map_runtime_error(error).into()),
        }
    }

    /// Progresses one retained run under its exact immutable assembly.
    pub async fn progress_run(&self, run_id: &RunId) -> Result<RunView, RunRequestError> {
        match self.composed.runtime.resume(run_id).await {
            Ok(view) => Ok(view),
            Err(RuntimeError::Indeterminate) => Err(RunRequestError::AppendIndeterminate {
                recovery: RunRecovery::Progress {
                    run_id: run_id.clone(),
                },
            }),
            Err(error) => Err(map_runtime_error(error).into()),
        }
    }

    /// Reads one retained run without progression.
    pub async fn read_run(&self, run_id: &RunId) -> Result<RunView, RequestError> {
        self.composed
            .runtime
            .read(run_id)
            .await
            .map_err(map_runtime_error)
    }

    /// Lists one mechanical keyset page of current run heads.
    pub async fn list_runs(
        &self,
        after: Option<&RunId>,
        limit: RunPageLimit,
    ) -> Result<RunPage, RequestError> {
        self.composed
            .run_index
            .list_runs(after, limit)
            .await
            .map_err(map_run_index_error)
    }
}

/// Provisions both PostgreSQL schemas through short-lived CLI-only administrative authority.
pub async fn provision_postgres(
    deployment: &Deployment,
    admin_locator_env: &EnvironmentName,
) -> Result<(), ComposeError> {
    let runtime_value = resolve_environment(deployment.runtime_postgres_locator_env())?;
    let admin_value = resolve_environment(admin_locator_env)?;
    let runtime =
        RuntimePostgresLocator::parse(runtime_value).map_err(|_| ComposeError::PostgresLocator)?;
    let admin =
        AdminPostgresLocator::parse(admin_value).map_err(|_| ComposeError::PostgresLocator)?;
    provision_postgres_backend(&admin, &runtime)
        .await
        .map_err(|_| ComposeError::Provision)
}

/// Failure to obtain OS entropy for a fresh [`RunId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("run id generation failed")]
pub struct RunIdGenerationError;

impl RunIdGenerationError {
    /// Returns the stable machine-readable error code.
    pub const fn code(self) -> &'static str {
        "run_id_generation_failed"
    }
}

/// Generates `mfm.run-id.random.v1` from exactly 32 bytes of OS cryptographic entropy.
pub fn generate_run_id() -> Result<RunId, RunIdGenerationError> {
    generate_run_id_with(|entropy| getrandom::fill(entropy)).map_err(|_| RunIdGenerationError)
}

fn generate_run_id_with<E>(fill: impl FnOnce(&mut [u8; 32]) -> Result<(), E>) -> Result<RunId, E> {
    let mut entropy = [0; 32];
    fill(&mut entropy)?;
    Ok(derive_run_id(entropy))
}

fn derive_run_id(entropy: [u8; 32]) -> RunId {
    use fmt::Write as _;

    let mut hex = String::with_capacity(64);
    for byte in entropy {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    let preimage = format!("{{\"entropy\":\"{hex}\",\"purpose\":\"mfm.run-id.random\",\"v\":1}}");
    RunId::from_digest(sha256_digest_bytes(preimage.as_bytes()))
}

const fn map_config_repository_error(error: ConfigRepositoryError) -> RequestError {
    match error {
        ConfigRepositoryError::Corrupt => RequestError::InvalidRetainedConfig,
        ConfigRepositoryError::Unavailable => RequestError::DependencyUnavailable,
        ConfigRepositoryError::Indeterminate => RequestError::ConfigMutationIndeterminate,
    }
}

const fn map_run_index_error(error: RunIndexError) -> RequestError {
    match error {
        RunIndexError::Corrupt => RequestError::InvalidRunIndex,
        RunIndexError::Unavailable => RequestError::DependencyUnavailable,
    }
}

const fn map_runtime_error(error: RuntimeError) -> RequestError {
    match error {
        RuntimeError::Absent => RequestError::RunAbsent,
        RuntimeError::AdmissionConflict => RequestError::RunAdmissionConflict,
        RuntimeError::Indeterminate => RequestError::Internal,
        RuntimeError::InvalidHistory => RequestError::InvalidRunHistory,
        RuntimeError::IncompatibleAssembly => RequestError::IncompatibleAssembly,
        RuntimeError::Capacity => RequestError::RunCapacity,
        RuntimeError::Unavailable => RequestError::DependencyUnavailable,
        RuntimeError::Internal => RequestError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_ERROR_DOCUMENT: &[u8] = br#"{
      "input":{"portfolio":{"quotes":["usd"],"portfolio_id":"portfolio-example","collections":[{"request":{"sources":[{"token":null,"source_id":"wallet-0.native","chain_id":1,"address":"0x1111111111111111111111111111111111111111"}],"decimals":18},"correlation":"native-0"}]},"selector":{"quote":"usd","target":"portfolio-example"},"routes":[{"endpoint_id":"alpha","chain_id":1}]},
      "entry_point":"mfm.portfolio/snapshot@1"}"#;

    #[test]
    fn run_id_generation_consumes_exact_entropy_and_matches_interoperable_vectors() {
        assert_eq!(
            generate_run_id_with(|entropy| {
                entropy.fill(0);
                Ok::<_, ()>(())
            })
            .expect("zero entropy")
            .as_str(),
            "run:sha256-jcs-v1:1ffc529adb99fb0f91d2c1712affb64a6d57fcdc9ac78c31c9681449e9c9f79a"
        );
        assert_eq!(
            generate_run_id_with(|entropy| {
                for (value, byte) in entropy.iter_mut().zip(0_u8..) {
                    *value = byte;
                }
                Ok::<_, ()>(())
            })
            .expect("ascending entropy")
            .as_str(),
            "run:sha256-jcs-v1:4412ec646bc21fee7eb806a97dccf2ec412ce38b33f9c3d7ea25e83fac13339e"
        );
        assert!(generate_run_id_with(|entropy| {
            entropy.fill(9);
            Err::<(), _>(())
        })
        .is_err());
        assert_eq!(RunIdGenerationError.code(), "run_id_generation_failed");
        assert_eq!(RunIdGenerationError.to_string(), "run id generation failed");
    }

    #[tokio::test]
    async fn client_error_json_owns_plain_identified_and_recovery_shapes() {
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .expect("run id");
        assert_eq!(
            serde_json::to_value(SerializableClientError::new("internal", "failure"))
                .expect("plain error JSON"),
            serde_json::json!({"code": "internal", "message": "failure"})
        );
        assert_eq!(
            serde_json::to_value(SerializableClientError::identified(
                "dependency_unavailable",
                "application dependency is unavailable",
                &run_id,
            ))
            .expect("identified error JSON"),
            serde_json::json!({
                "code": "dependency_unavailable",
                "message": "application dependency is unavailable",
                "run_id": run_id
            })
        );

        let document = ConfigDocument::new(CLIENT_ERROR_DOCUMENT.to_vec())
            .await
            .expect("config document");
        let start = RunRecovery::Start {
            run_id: run_id.clone(),
            config: document.summary(ConfigName::new("daily").expect("config name")),
        };
        let progress = RunRecovery::Progress { run_id };
        for (recovery, fixture) in [
            (
                &start,
                include_str!("../../../docs/contracts/client-surface/run-recovery-start.json"),
            ),
            (
                &progress,
                include_str!("../../../docs/contracts/client-surface/run-recovery-progress.json"),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(SerializableClientError::recoverable(
                    "run_append_indeterminate",
                    "run append outcome is indeterminate",
                    recovery,
                ))
                .expect("recovery error JSON"),
                serde_json::from_str::<serde_json::Value>(fixture).expect("recovery fixture")
            );
        }
    }

    #[test]
    fn compiled_entry_points_are_checked_and_sorted() {
        assert!(Application::entry_points()
            .windows(2)
            .all(|pair| pair[0].entry_point() < pair[1].entry_point()));
        for entry in Application::entry_points() {
            assert!(mfm_ids::EntryPointId::new(entry.entry_point()).is_ok());
        }
        assert_eq!(
            serde_json::to_value(ItemList::new(Application::entry_points()))
                .expect("entry-point JSON"),
            serde_json::json!({
                "items": [{"entry_point": "mfm.portfolio/snapshot@1"}]
            })
        );
    }

    #[test]
    fn compiled_components_are_checked_complete_and_ordered() {
        let components = Application::components();
        let expected = [
            (ComponentKind::EntryPoint, "mfm.portfolio/snapshot@1"),
            (
                ComponentKind::Operation,
                "mfm.evm.operation.collect-balances@1",
            ),
            (
                ComponentKind::PureState,
                "mfm.evm.state.consolidate-balance-collection@1",
            ),
            (ComponentKind::PureState, "mfm.evm.state.select-asset@1"),
            (
                ComponentKind::PureState,
                "mfm.portfolio.state.consolidate@1",
            ),
            (
                ComponentKind::PureState,
                "mfm.portfolio.state.enter-collection@1",
            ),
            (ComponentKind::PureState, "mfm.portfolio.state.initialize@1"),
            (
                ComponentKind::PureState,
                "mfm.portfolio.state.map-evm-failure@1",
            ),
            (
                ComponentKind::PureState,
                "mfm.portfolio.state.resume-collection@1",
            ),
            (
                ComponentKind::ReadState,
                "mfm.evm.state.check-chain-identity@1",
            ),
            (
                ComponentKind::ReadState,
                "mfm.evm.state.confirm-balance-anchor@1",
            ),
            (
                ComponentKind::ReadState,
                "mfm.evm.state.read-initial-anchor@1",
            ),
            (
                ComponentKind::ReadState,
                "mfm.evm.state.read-native-balance@1",
            ),
            (
                ComponentKind::ReadState,
                "mfm.evm.state.read-token-balance@1",
            ),
            (
                ComponentKind::ReadState,
                "mfm.evm.state.read-token-decimals@1",
            ),
        ];
        assert_eq!(components.len(), expected.len());
        for (component, (kind, id)) in components.iter().zip(expected) {
            assert_eq!(component.kind(), kind);
            assert_eq!(component.id(), id);
            assert!(component.description().len() <= 512);
            assert_eq!(component.description(), component.description().trim());
            assert!(!component.description().is_empty());
            assert!(!component.description().chars().any(char::is_control));
            match kind {
                ComponentKind::EntryPoint => {
                    assert!(mfm_ids::EntryPointId::new(component.id()).is_ok());
                }
                ComponentKind::Operation | ComponentKind::PureState | ComponentKind::ReadState => {
                    assert!(mfm_ids::StableId::new(component.id()).is_ok());
                }
            }
        }
        assert!(components
            .windows(2)
            .all(|pair| (pair[0].kind(), pair[0].id()) < (pair[1].kind(), pair[1].id())));

        let json = serde_json::to_value(ItemList::new(&components)).expect("component JSON");
        assert_eq!(json["items"][0]["kind"], "entry_point");
        assert_eq!(json["items"][1]["kind"], "operation");
        assert_eq!(json["items"][2]["kind"], "pure_state");
        assert_eq!(json["items"][9]["kind"], "read_state");
    }

    #[test]
    fn request_error_codes_and_messages_are_frozen() {
        let cases = [
            (
                RequestError::ConfigAbsent,
                "config_absent",
                "config is absent",
            ),
            (
                RequestError::InvalidConfigDocument,
                "invalid_config_document",
                "config document is invalid",
            ),
            (
                RequestError::ConfigMutationIndeterminate,
                "config_mutation_indeterminate",
                "config mutation outcome is indeterminate",
            ),
            (
                RequestError::InvalidRetainedConfig,
                "invalid_retained_config",
                "retained config is invalid",
            ),
            (RequestError::RunAbsent, "run_absent", "run is absent"),
            (
                RequestError::RunAdmissionConflict,
                "run_admission_conflict",
                "run admission conflicts with retained history",
            ),
            (
                RequestError::InvalidRunHistory,
                "invalid_run_history",
                "retained run history is invalid",
            ),
            (
                RequestError::IncompatibleAssembly,
                "incompatible_assembly",
                "runtime assembly is incompatible",
            ),
            (
                RequestError::RunCapacity,
                "run_capacity",
                "run capacity exceeded",
            ),
            (
                RequestError::BindingUnbound,
                "binding_unbound",
                "required capability binding is unavailable",
            ),
            (
                RequestError::InvalidRunIndex,
                "invalid_run_index",
                "retained run index is invalid",
            ),
            (
                RequestError::DependencyUnavailable,
                "dependency_unavailable",
                "application dependency is unavailable",
            ),
            (
                RequestError::Internal,
                "internal",
                "application internal failure",
            ),
        ];
        for (error, code, message) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), message);
        }
    }
}
