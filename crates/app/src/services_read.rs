use super::*;

/// Evidence-only application facade for certified typed run reads.
#[derive(Clone)]
pub struct RunReadServices<S, A> {
    store: S,
    artifacts: A,
    certification_registry: CertificationRegistry,
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
        Self {
            store,
            artifacts,
            certification_registry,
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

#[derive(Debug, Clone)]
pub(crate) struct VerifiedRunReadContext {
    runtime_spec: CertifiedRuntimeSpec,
    view: VerifiedRunHistoryView,
}

impl VerifiedRunReadContext {
    pub(crate) fn runtime_spec(&self) -> &CertifiedRuntimeSpec {
        &self.runtime_spec
    }

    pub(crate) fn view(&self) -> &VerifiedRunHistoryView {
        &self.view
    }

    pub(crate) fn events(&self) -> &[store::KernelEventEnvelope] {
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
pub(crate) struct VerifiedStatusReadContext {
    pub(super) read: VerifiedRunReadContext,
    projection: store::ProjectionSnapshot,
}

impl VerifiedStatusReadContext {
    pub(super) fn runtime_spec(&self) -> &CertifiedRuntimeSpec {
        self.read.runtime_spec()
    }

    pub(super) fn events(&self) -> &[store::KernelEventEnvelope] {
        self.read.events()
    }

    pub(super) fn projection(&self) -> &store::ProjectionSnapshot {
        &self.projection
    }
}

pub(crate) struct TrustedRunReader<'a, S, A: ?Sized> {
    store: &'a S,
    artifacts: &'a A,
    registry: &'a CertificationRegistry,
}

impl<'a, S, A: ?Sized> TrustedRunReader<'a, S, A> {
    pub(super) fn new(store: &'a S, artifacts: &'a A, registry: &'a CertificationRegistry) -> Self {
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
    pub(super) async fn load_store_scope_id(&self) -> Result<StoreScopeId, AppError> {
        self.store
            .load_store_scope_id()
            .await
            .map_err(async_app_store_error)
    }

    pub(super) async fn validate_identity_material_store_scope(
        &self,
        identity_material: &events::RunIdentityMaterialV1,
    ) -> Result<(), AppError> {
        let store_scope_id = self.load_store_scope_id().await?;
        if store_scope_id != identity_material.store_scope_id {
            return Err(run_identity_material_mismatch());
        }
        Ok(())
    }

    pub(super) async fn load_run_context(
        &self,
        run_id: &RunId,
    ) -> Result<VerifiedRunReadContext, AppError> {
        let context =
            load_async_verified_run_read_context(self.store, self.artifacts, self.registry, run_id)
                .await?;
        self.validate_identity_material_store_scope(
            &context.view().run_admitted().identity_material,
        )
        .await?;
        Ok(context)
    }

    pub(super) async fn load_status_context(
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

    pub(super) async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, AppError> {
        let context = self.load_status_context(run_id).await?;
        run_status_from_projection(
            run_id,
            context.runtime_spec(),
            context.events(),
            context.projection(),
        )
    }

    pub(super) async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, AppError> {
        let context = self.load_run_context(run_id).await?;
        Ok(run_stream_response_from_verified_context(&context))
    }

    pub(super) async fn read_run_observations(
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

    pub(super) async fn verify_replay(&self, run_id: &RunId) -> Result<ReplayResponse, AppError> {
        let context = self.load_run_context(run_id).await?;
        verify_replay_diagnostics_from_recorded_artifacts(self.artifacts, run_id, context.events())
            .await?;
        let authority = replay_read_authority_for_run_with_retained_source_facts(
            self.store,
            self.artifacts,
            context.runtime_spec(),
            context.view(),
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

    pub(super) async fn public_output(
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
