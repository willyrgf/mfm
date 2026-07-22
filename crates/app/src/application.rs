use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use mfm_ids::{RunId, SchemaId};
use mfm_storage_postgres::PostgresStore;
use mfm_store::v1 as store;
use tokio::sync::OnceCell;

use crate::{
    assemble_dispatch_registry, export_setup_target, import_setup_toml, list_setup_targets,
    make_run_read_services, make_run_services, prepare_entry_point_run_launch,
    production_certification_registry, InvocationKey, ManualResolutionRecordRequest, PublicError,
    PublicFactDescriptorSummary, PublicFactExplain, PublicFactKindSummary, PublicFactQueryPage,
    PublicFactQueryRequest, PublicFactRef, PublicFactRefId, PublicOutputResponse, ReplayResponse,
    RunReadServices, RunResponse, RunServices, RunStartReport, RunStreamResponse,
    RuntimeConfigLoader, SetupConfigPublication, SharedLiveTransports,
};

/// Opaque application facade used by process transports.
///
/// The facade owns one shared store, lazily retains evidence-only services, and constructs a fresh
/// live dispatch for each execution-capable call. Store implementations, registries, live
/// transports, and runtime configuration remain private to application assembly.
#[derive(Clone)]
pub struct Application {
    backend: Arc<dyn ApplicationBackend>,
}

impl Application {
    fn new(backend: impl ApplicationBackend + 'static) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }

    /// Checks whether the shared run store is ready.
    pub async fn check_ready(&self) -> Result<(), PublicError> {
        self.backend.check_ready().await
    }

    /// Resolves, certifies, starts, and renders one configured entry-point run.
    pub async fn start_entry_point_run(
        &self,
        entry_point: &str,
        target: &str,
        invocation_key: Option<InvocationKey>,
    ) -> Result<RunStartReport, PublicError> {
        self.backend
            .start_entry_point_run(entry_point, target, invocation_key)
            .await
    }

    /// Resumes one certified stored run through live execution services.
    pub async fn resume_run(&self, run_id: &RunId) -> Result<RunResponse, PublicError> {
        self.backend.resume_run(run_id).await
    }

    /// Records one manual resolution and resumes eligible execution.
    pub async fn record_manual_resolution(
        &self,
        request: ManualResolutionRecordRequest,
    ) -> Result<RunResponse, PublicError> {
        self.backend.record_manual_resolution(request).await
    }

    /// Returns verified public status for one run.
    pub async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, PublicError> {
        self.backend.run_status(run_id).await
    }

    /// Returns the verified public event stream for one run.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, PublicError> {
        self.backend.run_stream(run_id).await
    }

    /// Reads one bounded observation page for run list/watch transports.
    pub async fn list_runs(
        &self,
        cursor: Option<String>,
        limit: u32,
        wait_ms: u64,
    ) -> Result<store::RunObservationPage, PublicError> {
        self.backend.list_runs(cursor, limit, wait_ms).await
    }

    /// Verifies replay authority for one stored run without live services.
    pub async fn verify_replay(&self, run_id: &RunId) -> Result<ReplayResponse, PublicError> {
        self.backend.verify_replay(run_id).await
    }

    /// Renders one certified public output.
    pub async fn public_output(
        &self,
        run_id: &RunId,
        schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, PublicError> {
        self.backend.public_output(run_id, schema_id).await
    }

    /// Lists public fact kinds from retained descriptor authority.
    pub async fn fact_kinds(&self) -> Result<Vec<PublicFactKindSummary>, PublicError> {
        self.backend.fact_kinds().await
    }

    /// Describes public fact descriptors for one kind.
    pub async fn describe_fact_kind(
        &self,
        fact_kind: &str,
    ) -> Result<Vec<PublicFactDescriptorSummary>, PublicError> {
        self.backend.describe_fact_kind(fact_kind).await
    }

    /// Explains query and return fields for one public fact kind.
    pub async fn explain_fact_kind(
        &self,
        fact_kind: &str,
    ) -> Result<PublicFactExplain, PublicError> {
        self.backend.explain_fact_kind(fact_kind).await
    }

    /// Resolves one opaque public fact reference.
    pub async fn resolve_public_fact_ref(
        &self,
        public_ref: &PublicFactRefId,
    ) -> Result<PublicFactRef, PublicError> {
        self.backend.resolve_public_fact_ref(public_ref).await
    }

    /// Executes one checked public fact query.
    pub async fn query_public_facts(
        &self,
        request: PublicFactQueryRequest,
    ) -> Result<PublicFactQueryPage, PublicError> {
        self.backend.query_public_facts(request).await
    }

    /// Imports one bounded setup document into the configured-value authority.
    pub async fn import_setup(
        &self,
        bytes: &[u8],
    ) -> Result<Vec<SetupConfigPublication>, PublicError> {
        self.backend.import_setup(bytes).await
    }

    /// Lists configured setup targets.
    pub async fn list_setup_targets(&self) -> Result<Vec<String>, PublicError> {
        self.backend.list_setup_targets().await
    }

    /// Exports canonical configuration bytes for one setup target.
    pub async fn export_setup_target(&self, target: &str) -> Result<Vec<u8>, PublicError> {
        self.backend.export_setup_target(target).await
    }
}

/// Connects one production application facade backed by the shared Postgres store.
pub async fn connect_production_application(
    database_url: Option<&str>,
    runtime_config_path: Option<&Path>,
) -> Result<Application, PublicError> {
    let store = Arc::new(crate::connect_production_store(database_url).await?);
    Ok(Application::new(StoreApplication::new(
        store.clone(),
        Some(store),
        runtime_config_path.map(Path::to_path_buf),
    )))
}

/// Builds an application facade over an explicit in-memory test store.
#[cfg(any(test, feature = "test-support"))]
pub fn in_memory_application_for_test(
    store: store::AsyncInMemoryRunStore,
    runtime_config_path: Option<&Path>,
) -> Application {
    let store = Arc::new(store);
    Application::new(StoreApplication::new(
        store,
        None,
        runtime_config_path.map(Path::to_path_buf),
    ))
}

/// Builds an in-memory facade whose live executable/config access panics.
///
/// Evidence-only tests use this boundary to prove replay and observation never initialize live
/// process authority.
#[cfg(any(test, feature = "test-support"))]
pub fn in_memory_application_with_panicking_live_io_for_test(
    store: store::AsyncInMemoryRunStore,
) -> Application {
    let store = Arc::new(store);
    Application::new(StoreApplication::new_with_live_authority(
        store,
        None,
        RuntimeConfigLoader::panicking_for_test(),
        panicking_executable_identity_resolver,
    ))
}

type ExecutableIdentityFuture = Pin<
    Box<
        dyn Future<Output = Result<mfm_runtime::ExecutableIdentityTemplate, PublicError>>
            + Send
            + 'static,
    >,
>;
type ExecutableIdentityResolver = fn() -> ExecutableIdentityFuture;

fn current_executable_identity_resolver() -> ExecutableIdentityFuture {
    Box::pin(crate::executable_identity::current_executable_identity_template())
}

#[cfg(any(test, feature = "test-support"))]
fn panicking_executable_identity_resolver() -> ExecutableIdentityFuture {
    panic!("evidence-only work accessed the current executable")
}

struct StoreApplication<S> {
    store: Arc<S>,
    configured_store: Option<Arc<PostgresStore>>,
    runtime_config: RuntimeConfigLoader,
    shared_live_transports: Arc<SharedLiveTransports>,
    executable_identity: OnceCell<Result<mfm_runtime::ExecutableIdentityTemplate, PublicError>>,
    executable_identity_resolver: ExecutableIdentityResolver,
    read_services: OnceLock<Result<RunReadServices<S>, PublicError>>,
}

impl<S> StoreApplication<S>
where
    S: ApplicationStore,
{
    fn new(
        store: Arc<S>,
        configured_store: Option<Arc<PostgresStore>>,
        runtime_config_path: Option<std::path::PathBuf>,
    ) -> Self {
        Self::new_with_live_authority(
            store,
            configured_store,
            RuntimeConfigLoader::from_path(runtime_config_path.as_deref()),
            current_executable_identity_resolver,
        )
    }

    fn new_with_live_authority(
        store: Arc<S>,
        configured_store: Option<Arc<PostgresStore>>,
        runtime_config: RuntimeConfigLoader,
        executable_identity_resolver: ExecutableIdentityResolver,
    ) -> Self {
        Self {
            store,
            configured_store,
            runtime_config,
            shared_live_transports: Arc::new(SharedLiveTransports::new()),
            executable_identity: OnceCell::new(),
            executable_identity_resolver,
            read_services: OnceLock::new(),
        }
    }

    fn read_services(&self) -> Result<&RunReadServices<S>, PublicError> {
        self.read_services
            .get_or_init(|| {
                let certification = production_certification_registry()?;
                Ok(make_run_read_services(self.store.clone(), certification))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    async fn dispatch_services(&self) -> Result<RunServices<S>, PublicError> {
        let executable_identity = self
            .executable_identity
            .get_or_init(|| (self.executable_identity_resolver)())
            .await
            .as_ref()
            .map_err(Clone::clone)?
            .clone();
        let routes = Arc::new(
            self.shared_live_transports
                .new_dispatch(self.runtime_config.clone()),
        );
        let runners = assemble_dispatch_registry(self.store.clone(), executable_identity, routes)?;
        let certification = production_certification_registry()?;
        Ok(make_run_services(
            runners,
            self.store.clone(),
            certification,
        ))
    }

    fn configured_store(&self) -> Result<&PostgresStore, PublicError> {
        self.configured_store.as_deref().ok_or_else(|| {
            PublicError::internal(
                "ConfiguredStoreUnavailable",
                "Configured-value authority is unavailable for run preparation",
            )
        })
    }
}

trait ApplicationStore:
    store::RunEventStore
    + store::StoreScopeStore
    + store::ExecutionClaimStore
    + store::RetainedArtifactReadProvider
    + store::FactQueryStore
    + store::RunObservationStore<Error = <Self as store::RunEventStore>::Error>
    + Send
    + Sync
    + 'static
{
}

impl<S> ApplicationStore for S where
    S: store::RunEventStore
        + store::StoreScopeStore
        + store::ExecutionClaimStore
        + store::RetainedArtifactReadProvider
        + store::FactQueryStore
        + store::RunObservationStore<Error = <S as store::RunEventStore>::Error>
        + Send
        + Sync
        + 'static
{
}

#[async_trait]
trait ApplicationBackend: Send + Sync {
    async fn check_ready(&self) -> Result<(), PublicError>;
    async fn start_entry_point_run(
        &self,
        entry_point: &str,
        target: &str,
        invocation_key: Option<InvocationKey>,
    ) -> Result<RunStartReport, PublicError>;
    async fn resume_run(&self, run_id: &RunId) -> Result<RunResponse, PublicError>;
    async fn record_manual_resolution(
        &self,
        request: ManualResolutionRecordRequest,
    ) -> Result<RunResponse, PublicError>;
    async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, PublicError>;
    async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, PublicError>;
    async fn list_runs(
        &self,
        cursor: Option<String>,
        limit: u32,
        wait_ms: u64,
    ) -> Result<store::RunObservationPage, PublicError>;
    async fn verify_replay(&self, run_id: &RunId) -> Result<ReplayResponse, PublicError>;
    async fn public_output(
        &self,
        run_id: &RunId,
        schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, PublicError>;
    async fn fact_kinds(&self) -> Result<Vec<PublicFactKindSummary>, PublicError>;
    async fn describe_fact_kind(
        &self,
        fact_kind: &str,
    ) -> Result<Vec<PublicFactDescriptorSummary>, PublicError>;
    async fn explain_fact_kind(&self, fact_kind: &str) -> Result<PublicFactExplain, PublicError>;
    async fn resolve_public_fact_ref(
        &self,
        public_ref: &PublicFactRefId,
    ) -> Result<PublicFactRef, PublicError>;
    async fn query_public_facts(
        &self,
        request: PublicFactQueryRequest,
    ) -> Result<PublicFactQueryPage, PublicError>;
    async fn import_setup(&self, bytes: &[u8]) -> Result<Vec<SetupConfigPublication>, PublicError>;
    async fn list_setup_targets(&self) -> Result<Vec<String>, PublicError>;
    async fn export_setup_target(&self, target: &str) -> Result<Vec<u8>, PublicError>;
}

#[async_trait]
impl<S> ApplicationBackend for StoreApplication<S>
where
    S: ApplicationStore,
{
    async fn check_ready(&self) -> Result<(), PublicError> {
        self.read_services()?
            .load_store_scope_id()
            .await
            .map(|_| ())
    }

    async fn start_entry_point_run(
        &self,
        entry_point: &str,
        target: &str,
        invocation_key: Option<InvocationKey>,
    ) -> Result<RunStartReport, PublicError> {
        let services = self.dispatch_services().await?;
        let store_scope_id = services.load_store_scope_id().await?;
        let request = prepare_entry_point_run_launch(
            self.configured_store()?,
            entry_point,
            target,
            services.certification_registry(),
            store_scope_id,
            invocation_key,
        )
        .await?;
        services.launch_run_and_render(request).await
    }

    async fn resume_run(&self, run_id: &RunId) -> Result<RunResponse, PublicError> {
        self.dispatch_services()
            .await?
            .resume_stored_run(run_id)
            .await
    }

    async fn record_manual_resolution(
        &self,
        request: ManualResolutionRecordRequest,
    ) -> Result<RunResponse, PublicError> {
        self.dispatch_services()
            .await?
            .record_manual_resolution(request)
            .await
    }

    async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, PublicError> {
        self.read_services()?.run_status(run_id).await
    }

    async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, PublicError> {
        self.read_services()?.run_stream(run_id).await
    }

    async fn list_runs(
        &self,
        cursor: Option<String>,
        limit: u32,
        wait_ms: u64,
    ) -> Result<store::RunObservationPage, PublicError> {
        self.read_services()?
            .read_run_observations(store::RunObservationQuery::new(cursor, limit, wait_ms))
            .await
    }

    async fn verify_replay(&self, run_id: &RunId) -> Result<ReplayResponse, PublicError> {
        self.read_services()?.verify_replay_for_run(run_id).await
    }

    async fn public_output(
        &self,
        run_id: &RunId,
        schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, PublicError> {
        self.read_services()?.public_output(run_id, schema_id).await
    }

    async fn fact_kinds(&self) -> Result<Vec<PublicFactKindSummary>, PublicError> {
        self.read_services()?.fact_kinds().await
    }

    async fn describe_fact_kind(
        &self,
        fact_kind: &str,
    ) -> Result<Vec<PublicFactDescriptorSummary>, PublicError> {
        self.read_services()?.describe_fact_kind(fact_kind).await
    }

    async fn explain_fact_kind(&self, fact_kind: &str) -> Result<PublicFactExplain, PublicError> {
        self.read_services()?.explain_fact_kind(fact_kind).await
    }

    async fn resolve_public_fact_ref(
        &self,
        public_ref: &PublicFactRefId,
    ) -> Result<PublicFactRef, PublicError> {
        self.read_services()?
            .resolve_public_fact_ref(public_ref)
            .await
    }

    async fn query_public_facts(
        &self,
        request: PublicFactQueryRequest,
    ) -> Result<PublicFactQueryPage, PublicError> {
        self.read_services()?.query_public_facts(request).await
    }

    async fn import_setup(&self, bytes: &[u8]) -> Result<Vec<SetupConfigPublication>, PublicError> {
        import_setup_toml(self.configured_store()?, bytes).await
    }

    async fn list_setup_targets(&self) -> Result<Vec<String>, PublicError> {
        list_setup_targets(self.configured_store()?).await
    }

    async fn export_setup_target(&self, target: &str) -> Result<Vec<u8>, PublicError> {
        export_setup_target(self.configured_store()?, target).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_live_facade_call_builds_a_fresh_dispatch() {
        let backend = StoreApplication::new(
            Arc::new(store::AsyncInMemoryRunStore::default()),
            None,
            None,
        );

        assert_eq!(backend.shared_live_transports.dispatch_count(), 0);
        drop(
            backend
                .dispatch_services()
                .await
                .expect("first live dispatch"),
        );
        assert_eq!(backend.shared_live_transports.dispatch_count(), 1);
        drop(
            backend
                .dispatch_services()
                .await
                .expect("second live dispatch"),
        );
        assert_eq!(backend.shared_live_transports.dispatch_count(), 2);
    }
}
