use super::*;

/// Evidence-only application facade for certified typed run reads.
#[derive(Clone)]
pub struct RunReadServices<S> {
    store: Arc<S>,
    certification_registry: CertificationRegistry,
}

impl<S> RunReadServices<S>
where
    S: store::RunJournalStore + store::StoreScopeStore + Send + Sync + 'static,
{
    /// Creates evidence-only app services with an explicit trusted certification registry.
    pub fn new_with_certification_registry(
        store: Arc<S>,
        certification_registry: CertificationRegistry,
    ) -> Self {
        Self {
            store,
            certification_registry,
        }
    }

    /// Returns the async typed run store.
    pub fn store(&self) -> &S {
        self.store.as_ref()
    }

    /// Returns the trusted certification registry used for stored spec verification.
    pub fn certification_registry(&self) -> &CertificationRegistry {
        &self.certification_registry
    }

    /// Loads the store-owned deployment scope used to verify run identities.
    pub async fn load_store_scope_id(&self) -> Result<StoreScopeId, PublicError> {
        self.trusted_run_reader().load_store_scope_id().await
    }

    /// Returns typed run status from the store-owned verified run view.
    pub async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, PublicError>
    where
        S: store::CurrentProjectionStore,
    {
        self.trusted_run_reader().run_status(run_id).await
    }

    /// Returns store-owned references for the verified committed journal.
    pub async fn run_stream(&self, run_id: &RunId) -> Result<RunStreamResponse, PublicError> {
        self.trusted_run_reader().run_stream(run_id).await
    }

    /// Reads one observation-only run list/watch page.
    pub async fn read_run_observations(
        &self,
        query: store::RunObservationQuery,
    ) -> Result<store::RunObservationPage, PublicError>
    where
        S: store::RunObservationStore,
        <S as store::RunObservationStore>::Error:
            store::StoreErrorInspection + fmt::Display + Send + Sync + 'static,
    {
        self.trusted_run_reader().read_run_observations(query).await
    }

    /// Verifies replay by borrowing the store-owned verified run view.
    pub async fn verify_replay_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<ReplayResponse, PublicError> {
        self.trusted_run_reader().verify_replay(run_id).await
    }

    #[cfg(test)]
    pub(crate) async fn inspect_replay_broker_for_test<T>(
        &self,
        run_id: &RunId,
        inspect: impl FnOnce(&ReplayBroker<'_>) -> T,
    ) -> Result<T, PublicError> {
        let context = self.trusted_run_reader().load_run_context(run_id).await?;
        let authority = replay_read_authority_for_run_with_retained_source_facts(
            self.store.as_ref(),
            &self.certification_registry,
            context.view(),
        )
        .await?;
        let broker = ReplayBroker::from_read_authority(authority)?;
        Ok(inspect(&broker))
    }

    /// Renders typed public output from the verified view's exact retained objects.
    pub async fn public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, PublicError> {
        self.trusted_run_reader()
            .public_output(run_id, public_schema_id)
            .await
    }

    fn trusted_run_reader(&self) -> TrustedRunReader<'_, S> {
        TrustedRunReader::new(self.store.as_ref(), &self.certification_registry)
    }
}

impl<S> RunReadServices<S>
where
    S: store::RunJournalStore
        + store::CurrentProjectionStore
        + store::StoreScopeStore
        + store::RetainedArtifactReadProvider
        + Send
        + Sync
        + 'static,
{
    /// Lists public fact kinds from retained descriptor artifacts and store-scoped fact projection authority.
    pub async fn fact_kinds(&self) -> Result<Vec<PublicFactKindSummary>, PublicError> {
        Ok(self.public_fact_catalog().await?.list_kinds())
    }

    /// Describes public fact descriptors for one kind from store-scoped fact projection authority.
    pub async fn describe_fact_kind(
        &self,
        fact_kind: &str,
    ) -> Result<Vec<PublicFactDescriptorSummary>, PublicError> {
        self.public_fact_catalog().await?.describe_kind(fact_kind)
    }

    /// Explains public query and return fields for one fact kind from store-scoped fact projection authority.
    pub async fn explain_fact_kind(
        &self,
        fact_kind: &str,
    ) -> Result<PublicFactExplain, PublicError> {
        self.public_fact_catalog().await?.explain_kind(fact_kind)
    }

    /// Resolves an opaque public fact reference against store-scoped committed facts.
    ///
    /// Unknown facts return the same redacted not-found class.
    pub async fn resolve_public_fact_ref(
        &self,
        public_ref: &PublicFactRefId,
    ) -> Result<PublicFactRef, PublicError> {
        let projection = self.public_fact_projection().await?;
        let catalog =
            FactCatalogService::from_retained_public_projection(self.store.as_ref(), &projection)
                .await?;
        FactPublicRefResolver::new(catalog, projection).resolve(public_ref)
    }

    async fn public_fact_catalog(&self) -> Result<FactCatalogService, PublicError> {
        let projection = self.public_fact_projection().await?;
        FactCatalogService::from_retained_public_projection(self.store.as_ref(), &projection).await
    }

    async fn public_fact_projection(&self) -> Result<store::ProjectionSnapshot, PublicError> {
        self.store
            .fact_projection_snapshot()
            .await
            .map_err(async_app_store_error)
    }
}

impl<S> RunReadServices<S>
where
    S: store::RunJournalStore
        + store::CurrentProjectionStore
        + store::StoreScopeStore
        + store::RetainedArtifactReadProvider
        + store::FactQueryStore
        + Send
        + Sync
        + 'static,
{
    /// Builds a production public fact query service from retained descriptor and store-scoped projection authority.
    pub async fn public_fact_query_service(
        &self,
    ) -> Result<FactPublicQueryService<S>, PublicError> {
        let catalog = self.public_fact_catalog().await?;
        FactPublicQueryService::new(catalog, self.store.clone())
    }

    /// Executes a public Platform fact query against store-scoped public facts.
    pub async fn query_public_facts(
        &self,
        request: PublicFactQueryRequest,
    ) -> Result<PublicFactQueryPage, PublicError> {
        self.public_fact_query_service().await?.query(request).await
    }
}

#[derive(Debug)]
pub(crate) struct VerifiedRunReadContext {
    run: VerifiedCurrentRun,
}

impl VerifiedRunReadContext {
    pub(crate) fn run(&self) -> &VerifiedCurrentRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> VerifiedCurrentRun {
        self.run
    }

    pub(crate) fn view(&self) -> &store::VerifiedRunView {
        self.run.view()
    }

    pub(crate) fn lifecycle(&self) -> store::current_lifecycle::CurrentLifecycleReader<'_> {
        store::current_lifecycle::read(self.run.view())
    }
}

#[derive(Debug)]
pub(crate) struct VerifiedStatusReadContext {
    read: VerifiedRunReadContext,
    resource_lane_projection: store::ProjectionSnapshot,
}

impl VerifiedStatusReadContext {
    pub(crate) fn run(&self) -> &VerifiedCurrentRun {
        self.read.run()
    }

    pub(crate) fn resource_lane_projection(&self) -> &store::ProjectionSnapshot {
        &self.resource_lane_projection
    }
}

pub(crate) struct TrustedRunReader<'a, S> {
    store: &'a S,
    registry: &'a CertificationRegistry,
}

impl<'a, S> TrustedRunReader<'a, S> {
    pub(super) fn new(store: &'a S, registry: &'a CertificationRegistry) -> Self {
        Self { store, registry }
    }
}

impl<S> TrustedRunReader<'_, S>
where
    S: store::RunJournalStore + store::StoreScopeStore + Send + Sync,
{
    pub(super) async fn load_store_scope_id(&self) -> Result<StoreScopeId, PublicError> {
        self.store
            .load_store_scope_id()
            .await
            .map_err(async_app_store_error)
    }

    pub(super) async fn validate_identity_material_store_scope(
        &self,
        identity_material: &events::RunIdentityMaterialV1,
    ) -> Result<(), PublicError> {
        let store_scope_id = self.load_store_scope_id().await?;
        if store_scope_id != identity_material.store_scope_id {
            return Err(run_identity_material_mismatch());
        }
        Ok(())
    }

    pub(super) async fn load_run_context(
        &self,
        run_id: &RunId,
    ) -> Result<VerifiedRunReadContext, PublicError> {
        let context =
            load_async_verified_run_read_context(self.store, self.registry, run_id).await?;
        let admission = context.lifecycle().admission()?;
        self.validate_identity_material_store_scope(admission.identity_material())
            .await?;
        Ok(context)
    }

    pub(super) async fn load_status_context(
        &self,
        run_id: &RunId,
    ) -> Result<VerifiedStatusReadContext, PublicError>
    where
        S: store::CurrentProjectionStore,
    {
        let context =
            load_async_verified_status_read_context(self.store, self.registry, run_id).await?;
        let admission = context.read.lifecycle().admission()?;
        self.validate_identity_material_store_scope(admission.identity_material())
            .await?;
        Ok(context)
    }

    pub(super) async fn run_status(&self, run_id: &RunId) -> Result<RunResponse, PublicError>
    where
        S: store::CurrentProjectionStore,
    {
        let context = self.load_status_context(run_id).await?;
        run_response_from_verified_status(&context, "observed")
    }

    pub(super) async fn run_stream(
        &self,
        run_id: &RunId,
    ) -> Result<RunStreamResponse, PublicError> {
        let context = self.load_run_context(run_id).await?;
        Ok(run_stream_response_from_verified_context(&context))
    }

    pub(super) async fn read_run_observations(
        &self,
        query: store::RunObservationQuery,
    ) -> Result<store::RunObservationPage, PublicError>
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

    pub(super) async fn verify_replay(
        &self,
        run_id: &RunId,
    ) -> Result<ReplayResponse, PublicError> {
        let context = self.load_run_context(run_id).await?;
        verify_replay_diagnostics_from_recorded_artifacts(context.view())?;
        let authority = replay_read_authority_for_run_with_retained_source_facts(
            self.store,
            self.registry,
            context.view(),
        )
        .await?;
        let broker = ReplayBroker::from_read_authority(authority)?;
        replay_verifiers::ReplayVerifierRegistry::production().verify(&broker, self.registry)?;
        let lifecycle = context.lifecycle();
        let certified_spec = lifecycle.certified_spec().validated_spec().spec();
        let terminal_policies = store::SideEffectTerminalPolicies::from_spec(certified_spec)?;
        let retained_artifacts = lifecycle
            .retention()
            .map(|retention| retention.reference_count())
            .unwrap_or_default();
        lifecycle
            .with_saga(&certified_spec.saga, &terminal_policies, |saga| {
                ReplayResponse {
                    run_id: run_id.as_str().to_owned(),
                    spec_hash: context.view().spec_hash().as_str().to_owned(),
                    run_mode: run_mode_status(saga.run_mode()),
                    saga: saga_status_with_resources(certified_spec, &lifecycle, None, &saga),
                    attempt_dispositions: attempt_dispositions(&lifecycle),
                    head_seq: context.view().current_run_sequence().unwrap_or_default(),
                    retained_artifacts,
                }
            })
            .map_err(PublicError::from)
    }

    pub(super) async fn public_output(
        &self,
        run_id: &RunId,
        public_schema_id: &SchemaId,
    ) -> Result<PublicOutputResponse, PublicError> {
        let context = self.load_run_context(run_id).await?;
        let authority = public_output_read_authority_for_run(context.view(), public_schema_id)?;
        render_public_output(context.view(), &authority)
    }
}

async fn load_async_verified_run_read_context<S>(
    store: &S,
    registry: &CertificationRegistry,
    run_id: &RunId,
) -> Result<VerifiedRunReadContext, PublicError>
where
    S: store::RunJournalStore + Send + Sync,
{
    let journal = store
        .load_committed_journal(run_id)
        .await
        .map_err(async_app_store_error)?;
    verified_run_read_context_from_committed_journal(registry, journal)
}

async fn load_async_verified_status_read_context<S>(
    store: &S,
    registry: &CertificationRegistry,
    run_id: &RunId,
) -> Result<VerifiedStatusReadContext, PublicError>
where
    S: store::CurrentProjectionStore + Send + Sync,
{
    let journal = store
        .load_committed_journal(run_id)
        .await
        .map_err(async_app_store_error)?;
    let resource_lane_projection = store
        .status_projection_snapshot(run_id)
        .await
        .map_err(async_app_store_error)?;
    let read = verified_run_read_context_from_committed_journal(registry, journal)?;
    Ok(VerifiedStatusReadContext {
        read,
        resource_lane_projection,
    })
}

fn verified_run_read_context_from_committed_journal(
    registry: &CertificationRegistry,
    journal: store::CommittedRunJournal,
) -> Result<VerifiedRunReadContext, PublicError> {
    let certified = certified_spec_from_committed_journal(&journal, registry)?;
    let runtime_spec = CertifiedRuntimeSpec::new(certified)?;
    let run = mfm_runtime::verify_current_run(journal, runtime_spec)?;
    Ok(VerifiedRunReadContext { run })
}
