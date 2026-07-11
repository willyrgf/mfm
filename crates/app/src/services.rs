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
pub(super) struct VerifiedRunReadContext {
    runtime_spec: CertifiedRuntimeSpec,
    view: VerifiedRunHistoryView,
}

impl VerifiedRunReadContext {
    fn runtime_spec(&self) -> &CertifiedRuntimeSpec {
        &self.runtime_spec
    }

    pub(super) fn view(&self) -> &VerifiedRunHistoryView {
        &self.view
    }

    pub(super) fn events(&self) -> &[store::KernelEventEnvelope] {
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

    async fn verify_replay(&self, run_id: &RunId) -> Result<ReplayResponse, AppError> {
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
