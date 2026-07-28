use super::*;
use crate::errors::runtime_error_with_launch_context;

impl<S> RunServices<S>
where
    S: store::RunJournalStore
        + store::CurrentProjectionStore
        + store::StoreScopeStore
        + store::ExecutionClaimStore
        + Send
        + Sync
        + 'static,
{
    /// Resumes a certified typed run from its store-owned verified journal.
    pub async fn resume_stored_run(&self, run_id: &RunId) -> Result<RunResponse, PublicError> {
        let current = self
            .trusted_run_reader()
            .load_run_context(run_id)
            .await?
            .into_run();
        let outcome = self.drive_until_blocked(current).await?;
        self.run_response_from_verified_current(&outcome.current, outcome.status)
            .await
    }

    /// Records a signed manual resolution and optionally resumes typed scheduler execution.
    pub async fn record_manual_resolution(
        &self,
        req: ManualResolutionRecordRequest,
    ) -> Result<RunResponse, PublicError> {
        let run_id = req.run_id.clone();
        let current = self
            .trusted_run_reader()
            .load_run_context(&run_id)
            .await?
            .into_run();
        let manual_request = manual_resolution_runtime_request(req)?;
        let current = self
            .scheduler
            .record_manual_resolution(self.read.store(), current, manual_request)
            .await?;
        let outcome = self.drive_until_blocked(current).await?;
        self.run_response_from_verified_current(&outcome.current, outcome.status)
            .await
    }

    /// Starts a certified typed run against a durable async typed store.
    pub async fn launch_run(&self, req: RunLaunchRequest) -> Result<RunLaunchOutcome, PublicError> {
        self.trusted_run_reader()
            .validate_identity_material_store_scope(&req.identity_material)
            .await?;
        if req.identity_material.certified_spec_hash != *req.runtime_spec.spec_hash() {
            return Err(run_identity_material_mismatch());
        }
        let run_id = req.identity_material.derive_run_id().map_err(|_| {
            PublicError::backend(
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
        let runtime_spec = req.runtime_spec;
        let launch_evidence = req.evidence;
        loop {
            if let Some(journal) =
                load_optional_committed_journal(self.read.store(), &run_id).await?
            {
                let current = mfm_runtime::verify_current_run(journal, runtime_spec)?;
                return self
                    .attach_to_existing_run(current, &identity_material)
                    .await;
            }

            let launch = self
                .scheduler
                .prepare_run_launch(
                    &runtime_spec,
                    identity_material.clone(),
                    launch_evidence.clone(),
                    store::StreamSeq::FIRST,
                )
                .await
                .map_err(|error| {
                    runtime_error_with_launch_context(error, &launch_evidence.entry_point)
                })?;
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
                    let current = self
                        .scheduler
                        .load_admitted_run(self.read.store(), runtime_spec, &run_id)
                        .await?;
                    let outcome = self
                        .drive_until_blocked_with_existing_claim(
                            current,
                            &execution_scope,
                            &execution_claim_token,
                        )
                        .await?;
                    let run = self
                        .run_response_from_verified_current(&outcome.current, outcome.status)
                        .await?;
                    return Ok(RunLaunchOutcome::Admitted { run });
                }
                Ok(store::CommitOutcome::Idempotent(_)) => {
                    let current = self
                        .scheduler
                        .load_admitted_run(self.read.store(), runtime_spec, &run_id)
                        .await?;
                    return self
                        .attach_to_existing_run(current, &identity_material)
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
                    return Err(PublicError::backend(
                        ErrorClass::Internal,
                        "RunAdmissionBlockedUnexpectedly",
                        "run admission was blocked by a resource lane",
                    ));
                }
                Err(error) => {
                    if let Some(journal) =
                        load_optional_committed_journal(self.read.store(), &run_id).await?
                    {
                        let current = mfm_runtime::verify_current_run(journal, runtime_spec)?;
                        return self
                            .attach_to_existing_run(current, &identity_material)
                            .await;
                    }
                    return Err(error.into());
                }
            }
        }
    }

    /// Starts a prepared run and renders public output if the launch completes.
    pub async fn launch_run_and_render(
        &self,
        request: RunLaunchRequest,
    ) -> Result<RunStartReport, PublicError> {
        let run_id = request.run_id.clone();
        let public_output_schema_id = request
            .runtime_spec
            .spec()
            .public_outputs
            .public_schema_id
            .clone();
        let launch = self.launch_run(request).await?;
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
        current: VerifiedCurrentRun,
        identity_material: &events::RunIdentityMaterialV1,
    ) -> Result<RunLaunchOutcome, PublicError> {
        let admission = store::current_lifecycle::read(current.view()).admission()?;
        if admission.identity_material() != identity_material {
            return Err(run_identity_material_mismatch());
        }
        self.scheduler.validate_admitted_run_binding(&current)?;
        let run = self
            .run_response_from_verified_current(&current, DriveStatus::Observed)
            .await?;
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
        current: VerifiedCurrentRun,
    ) -> Result<VerifiedDriveOutcome, PublicError> {
        let (run_id, execution_scope, completed) = {
            let lifecycle = store::current_lifecycle::read(current.view());
            let identity_material = lifecycle.admission()?.identity_material().clone();
            (
                current.view().run_id().clone(),
                store::ExecutionClaimScope::from_run_identity_material(&identity_material),
                lifecycle.completion().is_some(),
            )
        };
        if completed {
            self.reap_expired_execution_claim_if_present(&execution_scope)
                .await?;
            return Ok(VerifiedDriveOutcome {
                current,
                status: DriveStatus::Observed,
            });
        }

        self.scheduler.validate_admitted_run_binding(&current)?;
        let launch_evidence = stored_launch_evidence_from_verified_run(&current)?;
        self.scheduler
            .validate_admitted_run_ingress_for_pending_nodes(&current, &launch_evidence)
            .await
            .map_err(|error| {
                runtime_error_with_launch_context(error, &launch_evidence.entry_point)
            })?;
        let mut lease = match self
            .acquire_execution_claim_for_drive(&execution_scope, &run_id)
            .await?
        {
            ExecutionClaimAcquire::Acquired(lease) => lease,
            ExecutionClaimAcquire::Busy => {
                return Ok(VerifiedDriveOutcome {
                    current,
                    status: DriveStatus::ExecutionClaimBusy,
                });
            }
        };
        self.drive_until_blocked_with_lease(current, &execution_scope, &mut lease)
            .await
    }

    async fn drive_until_blocked_with_existing_claim(
        &self,
        current: VerifiedCurrentRun,
        execution_scope: &store::ExecutionClaimScope,
        token: &store::AdmissionToken,
    ) -> Result<VerifiedDriveOutcome, PublicError> {
        let run_id = current.view().run_id().clone();
        let mut lease = match self
            .read
            .store()
            .execution_claim_status(execution_scope)
            .await
            .map_err(async_app_store_error)?
        {
            store::ExecutionClaimStatus::Live(lease)
                if lease.holder_run_id == run_id && &lease.token == token =>
            {
                lease
            }
            store::ExecutionClaimStatus::Live(_) => {
                return Ok(VerifiedDriveOutcome {
                    current,
                    status: DriveStatus::ExecutionClaimBusy,
                });
            }
            store::ExecutionClaimStatus::Expired(_) | store::ExecutionClaimStatus::Unclaimed => {
                return Ok(VerifiedDriveOutcome {
                    current,
                    status: DriveStatus::ExecutionClaimLost,
                });
            }
        };
        self.drive_until_blocked_with_lease(current, execution_scope, &mut lease)
            .await
    }

    async fn drive_until_blocked_with_lease(
        &self,
        mut current: VerifiedCurrentRun,
        execution_scope: &store::ExecutionClaimScope,
        lease: &mut store::AdmissionLease,
    ) -> Result<VerifiedDriveOutcome, PublicError> {
        let run_id = current.view().run_id().clone();
        loop {
            if !self
                .renew_execution_claim_or_lost(execution_scope, &run_id, lease)
                .await?
            {
                return Ok(VerifiedDriveOutcome {
                    current,
                    status: DriveStatus::ExecutionClaimLost,
                });
            }
            let step = self
                .drive_once_with_execution_claim(current, &run_id, execution_scope, lease)
                .await?;
            let (next, status, claim_lost) = match step {
                ClaimedDriveStep::Complete { result, claim_lost } => {
                    let (current, status) = result.into_parts();
                    (current, status, claim_lost)
                }
                ClaimedDriveStep::ClaimLost => {
                    let current = self
                        .trusted_run_reader()
                        .load_run_context(&run_id)
                        .await?
                        .into_run();
                    return Ok(VerifiedDriveOutcome {
                        current,
                        status: DriveStatus::ExecutionClaimLost,
                    });
                }
            };
            current = next;
            if claim_lost {
                return Ok(VerifiedDriveOutcome {
                    current,
                    status: DriveStatus::ExecutionClaimLost,
                });
            }

            let completed = store::current_lifecycle::read(current.view())
                .completion()
                .is_some();
            if completed || status == SchedulerStatus::PublicOutputProjected {
                self.release_execution_claim_if_holder(execution_scope, &run_id, lease)
                    .await?;
                return Ok(VerifiedDriveOutcome {
                    current,
                    status: status.into(),
                });
            }
            match status {
                SchedulerStatus::Advanced => {}
                SchedulerStatus::Blocked => {
                    return Ok(VerifiedDriveOutcome {
                        current,
                        status: status.into(),
                    });
                }
                SchedulerStatus::PublicOutputProjected => {
                    return Ok(VerifiedDriveOutcome {
                        current,
                        status: status.into(),
                    });
                }
            }
        }
    }

    async fn acquire_execution_claim_for_drive(
        &self,
        execution_scope: &store::ExecutionClaimScope,
        run_id: &RunId,
    ) -> Result<ExecutionClaimAcquire, PublicError> {
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
    ) -> Result<(), PublicError> {
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
        current: VerifiedCurrentRun,
        run_id: &RunId,
        execution_scope: &store::ExecutionClaimScope,
        lease: &mut store::AdmissionLease,
    ) -> Result<ClaimedDriveStep, PublicError> {
        let scheduler = self.scheduler.clone();
        let step = scheduler.drive_once(
            self.read.store(),
            current,
            execution_scope,
            lease.token.clone(),
        );
        tokio::pin!(step);
        loop {
            tokio::select! {
                result = &mut step => {
                    return match result {
                        Ok(result) => Ok(ClaimedDriveStep::Complete {
                            result,
                            claim_lost: false,
                        }),
                        Err(mfm_runtime::RuntimeError::ExecutionClaim(_)) => {
                            Ok(ClaimedDriveStep::ClaimLost)
                        }
                        Err(error) => Err(error.into()),
                    };
                }
                _ = tokio::time::sleep(self.execution_claim_heartbeat_interval) => {
                    if !self
                        .renew_execution_claim_or_lost(execution_scope, run_id, lease)
                        .await?
                    {
                        return match (&mut step).await {
                            Ok(result) => Ok(ClaimedDriveStep::Complete {
                                result,
                                claim_lost: true,
                            }),
                            Err(mfm_runtime::RuntimeError::ExecutionClaim(_)) => {
                                Ok(ClaimedDriveStep::ClaimLost)
                            }
                            Err(error) => Err(error.into()),
                        };
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
    ) -> Result<bool, PublicError> {
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
    ) -> Result<(), PublicError> {
        self.read
            .store()
            .release_execution_claim(execution_scope, run_id, &lease.token)
            .await
            .map_err(async_app_store_error)?;
        Ok(())
    }
}

async fn load_optional_committed_journal<S>(
    store: &S,
    run_id: &RunId,
) -> Result<Option<store::CommittedRunJournal>, PublicError>
where
    S: store::RunJournalStore + ?Sized,
{
    match store.load_committed_journal(run_id).await {
        Ok(journal) => Ok(Some(journal)),
        Err(error)
            if matches!(
                store::StoreErrorInspection::as_store_error(&error),
                Some(store::StoreError::RunNotFound { .. })
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(async_app_store_error(error)),
    }
}

enum ExecutionClaimAcquire {
    Acquired(store::AdmissionLease),
    Busy,
}

struct VerifiedDriveOutcome {
    current: VerifiedCurrentRun,
    status: DriveStatus,
}

enum ClaimedDriveStep {
    Complete {
        result: mfm_runtime::SchedulerDriveResult,
        claim_lost: bool,
    },
    ClaimLost,
}
