use super::*;

impl<S, A> RunServices<S, A>
where
    S: store::RunEventStore + store::StoreScopeStore + store::ExecutionClaimStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    /// Resumes a certified typed run from its stored spec artifact.
    pub async fn resume_stored_run(&self, run_id: &RunId) -> Result<RunResponse, AppError> {
        let runtime_spec = self
            .trusted_run_reader()
            .load_run_context(run_id)
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
            .trusted_run_reader()
            .load_run_context(&run_id)
            .await?
            .runtime_spec()
            .clone();
        let manual_request = manual_resolution_runtime_request(req)?;
        self.scheduler
            .record_manual_resolution(self.read.store(), &runtime_spec, &run_id, manual_request)
            .await?;
        let status = self.drive_until_blocked(&runtime_spec, &run_id).await?;
        self.run_response_from_verified_status(&run_id, status)
            .await
    }

    /// Starts a certified typed run against a durable async typed store.
    pub async fn launch_run(&self, req: RunLaunchRequest) -> Result<RunLaunchOutcome, AppError> {
        self.trusted_run_reader()
            .validate_identity_material_store_scope(&req.identity_material)
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
                .read
                .store()
                .load_run_stream(&run_id)
                .await
                .map_err(async_app_store_error)?;
            if !stream.is_empty() {
                return self
                    .attach_to_existing_run(&run_id, &identity_material)
                    .await;
            }
            let expected_next_seq = self
                .read
                .store()
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
                .start_run_with_execution_claim(self.read.store(), launch, execution_claim)
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
                        .read
                        .store()
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
                            self.read
                                .store()
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
                        .read
                        .store()
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
        let context = self
            .trusted_run_reader()
            .load_status_context(run_id)
            .await?;
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
            .read
            .store()
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
        let context = self
            .trusted_run_reader()
            .load_status_context(run_id)
            .await?;
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
            self.read.artifacts(),
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
            .read
            .store()
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
            let context = self
                .trusted_run_reader()
                .load_status_context(run_id)
                .await?;
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
                .read
                .store()
                .execution_claim_status(execution_scope)
                .await
                .map_err(async_app_store_error)?
            {
                store::ExecutionClaimStatus::Live(_) => return Ok(ExecutionClaimAcquire::Busy),
                store::ExecutionClaimStatus::Expired(lease) => {
                    self.read
                        .store()
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
                        .read
                        .store()
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
            .read
            .store()
            .execution_claim_status(execution_scope)
            .await
            .map_err(async_app_store_error)?
        {
            self.read
                .store()
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
            self.read.store(),
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
            .read
            .store()
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
        self.read
            .store()
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
