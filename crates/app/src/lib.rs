#![warn(missing_docs)]
//! Typed transport-neutral client use cases over one checked live composition.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_config::{ConfigImportResult, ConfigRepository, ConfigRepositoryError};
use mfm_evm::EvmBalanceRoute;
use mfm_evm_live::client::portfolio::{self as native, EvmPortfolioConfig, PortfolioResources};
use mfm_evm_live::{EvmAdapterLocator, EvmReadProvider, JsonRpcEvmProvider};
pub use mfm_evm_live::{EvmBindingView as PublicBindingView, MAX_EVM_BINDINGS};
use mfm_ids::RunId;
use mfm_portfolio::{
    EnrichmentProvenance, PortfolioEnrichmentInput, PortfolioEnrichmentOutput,
    PortfolioSnapshotInput,
};
use mfm_runtime::{InvocationFailure, RunView, Runtime, RuntimeError};
use mfm_storage_postgres::{
    provision_postgres as provision_postgres_backend, AdminPostgresLocator, PostgresBackend,
    RuntimePostgresLocator,
};
use mfm_store::{RunIndex, RunIndexError, Store};
use serde::{Deserialize, Serialize, Serializer};

mod config;
mod deployment;
mod inspection;
mod reporting;
#[cfg(test)]
mod reporting_tests;
mod run_view;
pub use reporting::encode_response;
pub use run_view::SerializableRunView;

pub use config::{
    ConfigDocument, ConfigDocumentError, ConfigSummary, EntryPointSummary, ImportOutcome,
};
pub use deployment::{
    Deployment, EnvironmentName, EnvironmentNameError, MAX_DEPLOYMENT_DOCUMENT_BYTES,
};
pub use inspection::{ComponentKind, ComponentSummary};
pub use mfm_config::{ConfigDigest, MAX_CONFIG_DOCUMENT_BYTES};
pub use mfm_ids::ConfigName;
pub use mfm_store::{RunPage, RunPageLimit};

use config::ENTRY_POINTS;
use deployment::resolve_environment;

/// Redaction-safe live composition failure.
#[derive(Debug, thiserror::Error)]
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
    /// Typed native resource or immutable Program construction failed.
    #[error("application composition is invalid")]
    Assembly,
    /// Native resource admission retains its reviewed causal data.
    #[error("native resource admission failed")]
    Native(mfm_values::InvocationDiagnostic),
}

/// Stable redacted request classification; invocation failures retain their audit separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// The selected config name is absent.
    #[error("config is absent")]
    ConfigAbsent,
    /// The complete config cannot be planned.
    #[error("config document is invalid")]
    InvalidConfigDocument,
    /// Enrichment is incomplete, has the wrong schema, or disagrees with its dependent configuration.
    #[error("enrichment result is invalid for publication or admission")]
    InvalidEnrichment,
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
    /// This physical append inserted nothing and could not return a qualified candidate view.
    #[error("run append was not inserted")]
    RunAppendNotInserted,
    /// Retained run history is invalid.
    #[error("retained run history is invalid")]
    InvalidRunHistory,
    /// The immutable assembly cannot execute retained history.
    #[error("runtime assembly is incompatible")]
    IncompatibleAssembly,
    /// A measured run resource exceeded its explicit limit.
    #[error("size limit exceeded")]
    SizeLimitExceeded,
    /// Capacity arithmetic could not represent a result.
    #[error("capacity arithmetic overflow")]
    CapacityArithmeticOverflow,
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
            Self::InvalidEnrichment => "invalid_enrichment",
            Self::ConfigMutationIndeterminate => "config_mutation_indeterminate",
            Self::InvalidRetainedConfig => "invalid_retained_config",
            Self::RunAbsent => "run_absent",
            Self::RunAdmissionConflict => "run_admission_conflict",
            Self::RunAppendNotInserted => "run_append_not_inserted",
            Self::InvalidRunHistory => "invalid_run_history",
            Self::IncompatibleAssembly => "incompatible_assembly",
            Self::SizeLimitExceeded => "size_limit_exceeded",
            Self::CapacityArithmeticOverflow => "capacity_arithmetic_overflow",
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
    diagnostic: Option<&'a mfm_values::InvocationDiagnostic>,
}

enum ClientErrorDetail<'a> {
    Construction {
        run_id: &'a RunId,
        cause: &'a mfm_values::InvocationDiagnostic,
    },
    None,
    RunId(&'a RunId),
    Recovery {
        recovery: &'a RunRecovery,
        invocation: run_view::Invocation<'a>,
    },
    Invocation(run_view::Invocation<'a>),
    FailedView {
        view: &'a mfm_runtime::RunView,
        original_report: Option<&'a serde_json::value::RawValue>,
    },
    FailedRun {
        error: &'a RunRequestError,
        original_report: Option<&'a serde_json::value::RawValue>,
    },
}

impl<'a> SerializableClientError<'a> {
    /// Constructs an error with no identity or recovery detail.
    pub const fn new(code: &'a str, message: &'a str) -> Self {
        Self {
            code,
            message,
            detail: ClientErrorDetail::None,
            diagnostic: None,
        }
    }

    /// Constructs a start error carrying its selected run identity.
    pub const fn identified(code: &'a str, message: &'a str, run_id: &'a RunId) -> Self {
        Self {
            code,
            message,
            detail: ClientErrorDetail::RunId(run_id),
            diagnostic: None,
        }
    }

    /// Renders a run-call failure with its last observed head and any recovery identity.
    pub fn for_run(
        error: &'a RunRequestError,
        message: &'a str,
    ) -> Result<Self, mfm_values::InvocationDiagnostic> {
        Ok(Self {
            code: error.code(),
            message,
            diagnostic: None,
            detail: match error {
                RunRequestError::Construction { run_id, cause } => {
                    ClientErrorDetail::Construction { run_id, cause }
                }
                RunRequestError::Request(_) => ClientErrorDetail::None,
                RunRequestError::AppendIndeterminate {
                    recovery,
                    invocation,
                } => ClientErrorDetail::Recovery {
                    recovery,
                    invocation: run_view::Invocation::new(invocation)?,
                },
                RunRequestError::Invocation(failure) => {
                    ClientErrorDetail::Invocation(run_view::Invocation::new(failure)?)
                }
            },
        })
    }
}

impl Serialize for SerializableClientError<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut state = serializer.serialize_map(None)?;
        state.serialize_entry("code", self.code)?;
        state.serialize_entry("message", self.message)?;
        match &self.detail {
            ClientErrorDetail::Construction { run_id, cause } => {
                state.serialize_entry("run_id", run_id)?;
                state.serialize_entry("diagnostic", cause)?;
            }
            ClientErrorDetail::None => {}
            ClientErrorDetail::RunId(run_id) => state.serialize_entry("run_id", run_id)?,
            ClientErrorDetail::Recovery {
                recovery,
                invocation,
            } => {
                state.serialize_entry("recovery", recovery)?;
                state.serialize_entry("invocation", invocation)?;
            }
            ClientErrorDetail::Invocation(failure) => {
                state.serialize_entry("invocation", failure)?
            }
            ClientErrorDetail::FailedView {
                view,
                original_report,
            } => {
                reporting::view_fields(&mut state, view)?;
                state.serialize_entry("original_report", original_report)?;
            }
            ClientErrorDetail::FailedRun {
                error,
                original_report,
            } => {
                reporting::run_fields(&mut state, error)?;
                state.serialize_entry("original_report", original_report)?;
            }
        }
        if let Some(diagnostic) = self.diagnostic {
            state.serialize_entry("diagnostic", diagnostic)?;
        }
        state.end()
    }
}

/// Run mutation failure with exact recovery identity for ambiguous append acknowledgement.
#[derive(thiserror::Error)]
// The public recovery sum deliberately retains its checked fields inline; append ambiguity is an
// exceptional path and changing the variant to an allocation-shaped API would weaken that contract.
#[allow(clippy::large_enum_variant)]
pub enum RunRequestError {
    /// Construction or cold association failed before Runtime execution.
    #[error("program construction failed")]
    Construction {
        /// Requested run identity.
        run_id: RunId,
        /// Original reviewed owner diagnostics.
        cause: mfm_values::InvocationDiagnostic,
    },
    /// Ordinary shared request failure.
    #[error("{0}")]
    Request(RequestError),
    /// The append may have committed.
    #[error("run append outcome is indeterminate")]
    AppendIndeterminate {
        /// Checked public recovery identity.
        recovery: RunRecovery,
        /// Complete invocation custody, including its original and exact candidate when available.
        #[source]
        invocation: InvocationFailure,
    },
    /// A call stopped without claiming a durable terminal outcome.
    #[error("{0}")]
    Invocation(#[source] InvocationFailure),
}

impl fmt::Debug for RunRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunRequestError")
            .field("code", &self.code())
            .finish_non_exhaustive()
    }
}

impl RunRequestError {
    fn from_invocation(error: InvocationFailure, recovery: RunRecovery) -> Self {
        let indeterminate = matches!(
            &error,
            InvocationFailure::Execution {
                error: RuntimeError::Recording { failure, .. }, ..
            } if matches!(failure.as_ref(), mfm_runtime::RecordingFailure::Store {
                cause: mfm_store::StoreError::Indeterminate(_), ..
            })
        );
        if indeterminate {
            Self::AppendIndeterminate {
                recovery,
                invocation: error,
            }
        } else {
            Self::Invocation(error)
        }
    }

    /// Returns the stable machine-readable error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Construction { .. } => "incompatible_assembly",
            Self::Request(error) => error.code(),
            Self::AppendIndeterminate { .. } => "run_append_indeterminate",
            Self::Invocation(InvocationFailure::Execution { error, .. }) => {
                map_runtime_error(error).code()
            }
            Self::Invocation(InvocationFailure::RecoveryStopped { .. }) => "recovery_stopped",
        }
    }

    /// Returns the reviewed request category when the invocation has an execution fault.
    pub fn request_error(&self) -> Option<RequestError> {
        match self {
            Self::Construction { .. } => Some(RequestError::IncompatibleAssembly),
            Self::Request(error) => Some(*error),
            Self::Invocation(InvocationFailure::Execution { error, .. }) => {
                Some(map_runtime_error(error))
            }
            Self::AppendIndeterminate { .. }
            | Self::Invocation(InvocationFailure::RecoveryStopped { .. }) => None,
        }
    }

    /// Returns recovery identity only for an ambiguously acknowledged append.
    pub const fn recovery(&self) -> Option<&RunRecovery> {
        match self {
            Self::Construction { .. } | Self::Request(_) | Self::Invocation(_) => None,
            Self::AppendIndeterminate { recovery, .. } => Some(recovery),
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
    // Preserve the public inline recovery sum shared by both async start paths.
    #[allow(clippy::result_large_err)]
    fn from_runtime(
        run_id: RunId,
        config: ConfigSummary,
        result: Result<RunView, InvocationFailure>,
    ) -> Result<Self, RunRequestError> {
        match result {
            Ok(run) => Ok(StartRunResult { config, run }),
            Err(error) => Err(RunRequestError::from_invocation(
                error,
                RunRecovery::Start { run_id, config },
            )),
        }
    }

    /// Returns the exact selected config revision.
    pub const fn config(&self) -> &ConfigSummary {
        &self.config
    }

    /// Returns the resulting durable run view.
    pub const fn run(&self) -> &RunView {
        &self.run
    }
}

impl StartRunResult {
    /// Prepares the selected revision and its qualified run view for transport serialization.
    pub fn serializable(&self) -> Result<impl Serialize + '_, mfm_values::InvocationDiagnostic> {
        #[derive(Serialize)]
        struct Prepared<'a> {
            config: &'a ConfigSummary,
            run: SerializableRunView<'a>,
        }
        Ok(Prepared {
            config: &self.config,
            run: SerializableRunView::new(&self.run)?,
        })
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

fn construction_failure(
    run_id: &RunId,
    operation: &'static str,
    cause: &impl Serialize,
) -> RunRequestError {
    RunRequestError::Construction {
        run_id: run_id.clone(),
        cause: mfm_values::InvocationDiagnostic::from_fields(
            "program_construction",
            operation,
            cause,
            None,
        ),
    }
}

fn admitted_identity(run: &RunView) -> Result<mfm_portfolio::PortfolioAdmission, RequestError> {
    let identity = match run.entry_point().as_str() {
        mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID => run
            .admitted_context()
            .decode::<PortfolioSnapshotInput>()
            .map_err(|_| RequestError::RunAdmissionConflict)?
            .admission()
            .cloned(),
        mfm_portfolio::PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID => run
            .admitted_context()
            .decode::<PortfolioEnrichmentInput>()
            .map_err(|_| RequestError::RunAdmissionConflict)?
            .admission()
            .cloned(),
        _ => return Err(RequestError::RunAdmissionConflict),
    };
    identity.ok_or(RequestError::RunAdmissionConflict)
}

/// Transport-neutral application use-case surface.
pub struct Application {
    runtime: Runtime,
    resources: Arc<PortfolioResources>,
    run_index: Arc<dyn RunIndex>,
    configs: Arc<dyn ConfigRepository>,
}

impl Application {
    /// Coordinates explicit native resources, mechanical persistence, and configuration storage.
    pub fn from_parts<B>(
        backend: Arc<B>,
        resources: PortfolioResources,
        configs: Arc<dyn ConfigRepository>,
    ) -> Self
    where
        B: Store + RunIndex + 'static,
    {
        Self {
            runtime: Runtime::new(backend.clone()),
            run_index: backend,
            resources: Arc::new(resources),
            configs,
        }
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
            let provider: Arc<dyn EvmReadProvider> = Arc::new(
                JsonRpcEvmProvider::connect(&locator).map_err(|_| ComposeError::Provider)?,
            );
            routes.push((
                EvmBalanceRoute::new(
                    NonZeroU64::new(route.chain_id()).ok_or(ComposeError::Assembly)?,
                    route.endpoint().clone(),
                ),
                provider,
            ));
        }
        let postgres = Arc::new(
            PostgresBackend::connect(&postgres_locator)
                .await
                .map_err(|_| ComposeError::Postgres)?,
        );
        let configs: Arc<dyn ConfigRepository> = postgres.clone();
        let resources = PortfolioResources::new(routes, vec![]).map_err(ComposeError::Native)?;
        Ok(Self::from_parts(postgres, resources, configs))
    }

    /// Returns the strictly ordered compiled entry points.
    pub const fn entry_points() -> &'static [EntryPointSummary] {
        &ENTRY_POINTS
    }

    /// Returns every definition admitted by the compiled product composition.
    pub fn components() -> mfm_program::Result<Vec<ComponentSummary>> {
        inspection::components()
    }

    /// Returns stable public bindings derived from the exact composed adapter targets.
    pub fn bindings(&self) -> Result<Vec<PublicBindingView>, mfm_values::InvocationDiagnostic> {
        self.resources.bindings()
    }

    /// Validates and conditionally imports one complete config revision.
    pub async fn import_config(
        &self,
        name: ConfigName,
        document: ConfigDocument,
    ) -> Result<ImportOutcome, RequestError> {
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

    /// Publishes one exact successful enrichment result as an immutable snapshot configuration.
    pub async fn publish_enrichment(
        &self,
        destination: ConfigName,
        run_id: &RunId,
    ) -> Result<ImportOutcome, RunRequestError> {
        let observed = self.read_run(run_id).await?;
        let document = tokio::task::spawn_blocking(move || {
            let mfm_runtime::RunViewState::Succeeded(value) = observed.state() else {
                return Err(RequestError::InvalidEnrichment);
            };
            let output = value
                .decode::<PortfolioEnrichmentOutput>()
                .map_err(|_| RequestError::InvalidEnrichment)?;
            let provenance = EnrichmentProvenance::new(
                observed.run_id().clone(),
                observed.head_digest().clone(),
                value.value_ref().clone(),
            )
            .map_err(|_| RequestError::InvalidEnrichment)?;
            ConfigDocument::from_enrichment(output, provenance)
                .map_err(|_| RequestError::InvalidEnrichment)
        })
        .await
        .map_err(|_| RequestError::Internal)??;
        self.import_config(destination, document)
            .await
            .map_err(Into::into)
    }

    /// Selects a stored config, plans it, admits the exact RunId, and progresses the run.
    pub async fn start_run(
        &self,
        run_id: RunId,
        selection: &ConfigSelection,
    ) -> Result<StartRunResult, RunRequestError> {
        match self.read_run(&run_id).await {
            Ok(run) => {
                let config = tokio::task::spawn_blocking(move || {
                    let identity = admitted_identity(&run)?;
                    if identity.entry_point() != run.entry_point() {
                        return Err(RequestError::RunAdmissionConflict);
                    }
                    let config = ConfigSummary::from_admission(&identity);
                    Ok::<_, RequestError>(config)
                })
                .await
                .map_err(|_| RequestError::Internal)??;
                if config.name() != selection.name() || config.digest() != selection.digest() {
                    return Err(RequestError::RunAdmissionConflict.into());
                }
                let program = self.retained_program(&run_id).await?;
                let result = self.runtime.resume(&run_id, &program).await;
                return StartRunResult::from_runtime(run_id, config, result);
            }
            Err(RunRequestError::Invocation(InvocationFailure::Execution {
                error: RuntimeError::Absent,
                ..
            })) => {}
            Err(error) => return Err(error),
        }
        let entry = self
            .configs
            .load_config(selection.name(), selection.digest())
            .await
            .map_err(map_config_repository_error)?
            .ok_or(RequestError::ConfigAbsent)?;
        let (name, digest, canonical) = entry.into_parts();
        let document = tokio::task::spawn_blocking(move || {
            ConfigDocument::parse_retained(canonical, &digest)
                .map_err(|_| RequestError::InvalidRetainedConfig)
        })
        .await
        .map_err(|_| RequestError::Internal)??;
        let observed = if let Some(provenance) = document.enrichment() {
            Some(self.read_run(provenance.run_id()).await?)
        } else {
            None
        };
        if let (Some(provenance), Some(observed)) = (document.enrichment(), observed) {
            let value = observed.success().ok_or(RequestError::InvalidEnrichment)?;
            let output = value
                .decode::<PortfolioEnrichmentOutput>()
                .map_err(|cause| construction_failure(&run_id, "decode_enrichment", &cause))?;
            if observed.head_digest() != provenance.head()
                || value.value_ref() != provenance.output()
                || !document
                    .matches_enrichment(&output)
                    .map_err(|cause| construction_failure(&run_id, "match_enrichment", &cause))?
            {
                return Err(RequestError::InvalidEnrichment.into());
            }
        }
        let admission = document.admission(&name);
        let config = document.summary(name);
        let result = match document.native() {
            EvmPortfolioConfig::Snapshot(_) => {
                let input = native::admit_snapshot(document.native(), Some(admission))
                    .map_err(|cause| construction_failure(&run_id, "admit_snapshot", &cause))?;
                let program = mfm_program::compile(
                    document.entry_point(),
                    &mfm_portfolio::PortfolioSnapshotOperation::default(),
                    &input,
                    self.resources.as_ref(),
                    mfm_program::ProgramLimits::new(0),
                )
                .map_err(|cause| construction_failure(&run_id, "compile_snapshot", &cause))?;
                self.runtime.start(run_id.clone(), &program, &input).await
            }
            EvmPortfolioConfig::Enrichment(_) => {
                let input = native::admit_enrichment(document.native(), Some(admission))
                    .map_err(|cause| construction_failure(&run_id, "admit_enrichment", &cause))?;
                let program = mfm_program::compile(
                    document.entry_point(),
                    &mfm_portfolio::PortfolioEnrichmentOperation::default(),
                    &input,
                    self.resources.as_ref(),
                    mfm_program::ProgramLimits::new(0),
                )
                .map_err(|cause| construction_failure(&run_id, "compile_enrichment", &cause))?;
                self.runtime.start(run_id.clone(), &program, &input).await
            }
        };
        StartRunResult::from_runtime(run_id, config, result)
    }

    /// Progresses one retained run under its exact immutable assembly.
    pub async fn progress_run(&self, run_id: &RunId) -> Result<RunView, RunRequestError> {
        let program = self.retained_program(run_id).await?;
        self.runtime
            .resume(run_id, &program)
            .await
            .map_err(|error| {
                RunRequestError::from_invocation(
                    error,
                    RunRecovery::Progress {
                        run_id: run_id.clone(),
                    },
                )
            })
    }

    /// Reads one retained run without progression.
    pub async fn read_run(&self, run_id: &RunId) -> Result<RunView, RunRequestError> {
        let program = self.retained_program(run_id).await?;
        self.runtime
            .read(run_id, &program)
            .await
            .map_err(RunRequestError::Invocation)
    }

    async fn retained_program(
        &self,
        run_id: &RunId,
    ) -> Result<mfm_program::Program, RunRequestError> {
        let document = self
            .runtime
            .program_document(run_id)
            .await
            .map_err(RunRequestError::Invocation)?;
        mfm_program::load(document.canonical_bytes(), self.resources.as_ref())
            .map_err(|cause| construction_failure(run_id, "load_program", &cause))
    }

    /// Lists one mechanical keyset page of current run heads.
    pub async fn list_runs(
        &self,
        after: Option<&RunId>,
        limit: RunPageLimit,
    ) -> Result<RunPage, RequestError> {
        self.run_index
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

fn map_runtime_error(error: &RuntimeError) -> RequestError {
    if error.size_limit().is_some() {
        return RequestError::SizeLimitExceeded;
    }
    match error {
        RuntimeError::Absent => RequestError::RunAbsent,
        RuntimeError::AdmissionConflict => RequestError::RunAdmissionConflict,
        RuntimeError::Store(error) => map_store_error(error),
        RuntimeError::InvalidHistory => RequestError::InvalidRunHistory,
        RuntimeError::SizeLimit { .. } => RequestError::SizeLimitExceeded,
        RuntimeError::ArithmeticOverflow => RequestError::CapacityArithmeticOverflow,
        RuntimeError::Recording { failure, .. } => match failure.as_ref() {
            mfm_runtime::RecordingFailure::BeforeAppend { cause, .. } => map_runtime_error(cause),
            mfm_runtime::RecordingFailure::NotInserted { .. } => RequestError::RunAppendNotInserted,
            mfm_runtime::RecordingFailure::Store { cause, .. } => map_store_error(cause),
        },
        RuntimeError::Native { .. } | RuntimeError::Projection { .. } => RequestError::Internal,
    }
}

fn map_store_error(error: &mfm_store::StoreError) -> RequestError {
    match error {
        mfm_store::StoreError::Unavailable(_) => RequestError::DependencyUnavailable,
        mfm_store::StoreError::FrameSize(_)
        | mfm_store::StoreError::HistorySize(_)
        | mfm_store::StoreError::FrameCount(_) => RequestError::SizeLimitExceeded,
        mfm_store::StoreError::ArithmeticOverflow => RequestError::CapacityArithmeticOverflow,
        mfm_store::StoreError::CorruptPhysicalState(_) => RequestError::InvalidRunHistory,
        mfm_store::StoreError::Indeterminate(_) => RequestError::Internal,
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
        let progress = RunRecovery::Progress {
            run_id: run_id.clone(),
        };
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
                serde_json::to_value(
                    SerializableClientError::for_run(
                        &RunRequestError::AppendIndeterminate {
                            recovery: recovery.clone(),
                            invocation: InvocationFailure::Execution {
                                run_id: run_id.clone(),
                                last_observed: None,
                                error: RuntimeError::Recording {
                                    operation: mfm_runtime::Operation::Record,
                                    failure: Box::new(mfm_runtime::RecordingFailure::Store {
                                        original: None,
                                        candidate: mfm_journal::seal_frame(
                                            &run_id,
                                            1,
                                            None,
                                            &mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
                                                "{}"
                                            )
                                            .unwrap()
                                        )
                                        .unwrap(),
                                        cause: mfm_store::StoreError::Indeterminate(mfm_values::DiagnosticEvidence::from_value(serde_json::json!({"operation": "test.store", "injected": "Indeterminate"})))
                                    }),
                                },
                            }
                        },
                        "run append outcome is indeterminate",
                    )
                    .unwrap()
                )
                .expect("recovery error JSON"),
                serde_json::from_str::<serde_json::Value>(fixture).expect("recovery fixture")
            );
        }
    }

    #[test]
    fn compiled_entry_points_and_components_are_the_shipping_set() {
        assert_eq!(
            serde_json::to_value(ItemList::new(Application::entry_points()))
                .expect("entry-point JSON"),
            serde_json::json!({
                "items": [{"entry_point": "mfm.portfolio/enrich@1"}, {"entry_point": "mfm.portfolio/snapshot@1"}]
            })
        );
        let components = Application::components().unwrap();
        let ids = components
            .iter()
            .map(|component| (component.kind(), component.id()))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                (ComponentKind::EntryPoint, "mfm.portfolio/enrich@1"),
                (ComponentKind::EntryPoint, "mfm.portfolio/snapshot@1"),
                (
                    ComponentKind::Operation,
                    "mfm.portfolio.operation.enrichment@1"
                ),
                (
                    ComponentKind::Operation,
                    "mfm.portfolio.operation.snapshot@1"
                ),
                (ComponentKind::PureState, "mfm.chain.consolidate-balances@1"),
                (
                    ComponentKind::PureState,
                    "mfm.portfolio.state.consolidate@1"
                ),
                (
                    ComponentKind::PureState,
                    "mfm.portfolio.state.enter-collection@1"
                ),
                (
                    ComponentKind::PureState,
                    "mfm.portfolio.state.enter-enrichment-collection@1"
                ),
                (
                    ComponentKind::PureState,
                    "mfm.portfolio.state.initialize-enrichment@1"
                ),
                (ComponentKind::PureState, "mfm.portfolio.state.initialize@1"),
                (
                    ComponentKind::PureState,
                    "mfm.portfolio.state.resolve-assets@1"
                ),
                (
                    ComponentKind::PureState,
                    "mfm.portfolio.state.resume-collection@1"
                ),
                (
                    ComponentKind::PureState,
                    "mfm.portfolio.state.resume-enrichment-collection@1"
                ),
                (ComponentKind::ReadState, "mfm.chain.observe-balance@1"),
                (ComponentKind::ReadState, "mfm.evm.check-balance-chain@1"),
                (ComponentKind::ReadState, "mfm.evm.confirm-balance-anchor@1"),
                (
                    ComponentKind::ReadState,
                    "mfm.evm.initial-native-balance-anchor@1"
                ),
                (
                    ComponentKind::ReadState,
                    "mfm.evm.initial-token-balance-anchor@1"
                ),
                (ComponentKind::ReadState, "mfm.evm.read-token-decimals@1"),
            ]
        );
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
                RequestError::InvalidEnrichment,
                "invalid_enrichment",
                "enrichment result is invalid for publication or admission",
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
                RequestError::SizeLimitExceeded,
                "size_limit_exceeded",
                "size limit exceeded",
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
    #[test]
    fn size_limit_client_errors_preserve_safe_numeric_details() {
        let size = mfm_values::SizeLimitExceeded::check(35_651_584, 33_554_432).unwrap_err();
        let failure = RunRequestError::Invocation(InvocationFailure::Execution {
            run_id: RunId::from_digest(mfm_ids::DigestBytes::from_array([90; 32])),
            error: RuntimeError::SizeLimit {
                resource: mfm_values::SizeResource::FailureReport,
                size,
            },
            last_observed: None,
        });
        assert_eq!(failure.code(), "size_limit_exceeded");
        let message = failure.to_string();
        let wire =
            serde_json::to_value(SerializableClientError::for_run(&failure, &message).unwrap())
                .unwrap();
        assert_eq!(
            wire["invocation"]["size_limit"],
            serde_json::json!({
                "resource": "failure_report", "actual": 35_651_584, "limit": 33_554_432
            })
        );
        assert_eq!(wire["invocation"]["last_observed"], serde_json::Value::Null);
    }
}

#[cfg(test)]
mod construction_tests;
