use super::*;

#[path = "services_read.rs"]
mod services_read;
#[path = "services_run.rs"]
mod services_run;

pub use self::services_read::RunReadServices;
pub(super) use self::services_read::{
    TrustedRunReader, VerifiedRunReadContext, VerifiedStatusReadContext,
};

/// Application facade for certified typed runtime dispatch.
#[derive(Clone)]
pub struct RunServices<S, A> {
    pub(super) scheduler: SerialTypedScheduler,
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
    pub(super) execution_claim_heartbeat_interval: Duration,
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
        Self {
            scheduler,
            store,
            artifacts,
            certification_registry,
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
        self.trusted_run_reader().verify_replay(run_id).await
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
