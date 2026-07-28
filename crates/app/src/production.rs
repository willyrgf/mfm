//! Private production composition for one qualified authoritative application.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use mfm_certify::CompositeCertificationFactory;
use mfm_ids::{AppendRequestId, RunId, StableId, TenantScopeId};
use mfm_program::{QualifiedPlannerRegistration, QualifiedProgramRegistry};
use mfm_runtime::Runtime;
use mfm_spec::{CertifiedJournalProtocolContracts, EntryPointContract};
use mfm_storage_postgres::{open_authoritative, AuthoritativeWriterFence, QualifiedPostgresStore};
use mfm_store::{
    AdmissionMaterial, AdmissionSourceStore, AppendOutcome, AppendRejection, ConfiguredValueStore,
    NewlyAppended, ProposedAdmissionInput, RunAccessAuthority, RunAccessAuthorityIssuer,
    RunJournalStore, SupportStore,
};
use sqlx::postgres::PgPoolOptions;

use self::qualification::{
    assemble_qualified_product_deployment, qualify_evm_live_support, qualify_product_components,
    ProductComponentImplementations, QualifiedProductDeployment,
};
use self::routes::{load_evm_deployment, EvmDeployment};
use crate::application::{ApplicationBackend, AuthorizedRunCall};
use crate::executable_identity::{current_executable_identity, CurrentExecutableIdentity};
use crate::{
    complete_access_audit_page, complete_transition_trace_page, decode_access_audit_page_request,
    decode_transition_trace_page_request, production_database_url, AccessAuditPage,
    AdmissionStatus, AdmitRunRequest, AdmitRunResponse, Application, DriveResponse, ErrorClass,
    ExportRequest, ExportedRunBytes, PageRequest, PublicError, PublicRunView, ReplayRequest,
    ReplayResponse, RunAccessPolicy, TransitionTracePage,
};

mod qualification;
mod routes;

struct ProductionBackend {
    store: QualifiedPostgresStore,
    issuer: RunAccessAuthorityIssuer,
    registry: Arc<QualifiedProgramRegistry>,
    runtime: Runtime<QualifiedPostgresStore>,
}

struct UnavailableReproductionResolver;

impl mfm_replay::v1::ReproductionResolver for UnavailableReproductionResolver {
    fn reproduce_exact<'a>(
        &'a self,
        _canonical_plan: &'a [u8],
    ) -> mfm_replay::v1::ReproductionFuture<'a, mfm_replay::v1::ExactReproduction> {
        Box::pin(async { mfm_replay::v1::ExactReproduction::Unavailable })
    }
}

pub(super) async fn connect<F>(
    database_url: Option<&str>,
    runtime_config_path: Option<&Path>,
    policy: Arc<dyn RunAccessPolicy>,
    deployment_writer_fence: F,
) -> Result<Application, PublicError>
where
    F: AuthoritativeWriterFence + 'static,
{
    let database_url = production_database_url(database_url)?;
    let pool = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .map_err(|_| run_store_unavailable())?;
    let (store, issuer) = open_authoritative(pool, deployment_writer_fence).await?;
    let (executable, evm) = tokio::try_join!(
        current_executable_identity(),
        load_evm_deployment(runtime_config_path)
    )?;
    let (registry, entry_points) =
        assemble_program_registry(&store, &issuer, &executable, evm).await?;
    let store_scope_id = store.store_scope_id().clone();
    let runtime = Runtime::new(store.clone(), Arc::clone(&registry));
    let backend = ProductionBackend {
        store,
        issuer,
        registry,
        runtime,
    };
    Ok(Application::new(
        store_scope_id,
        policy,
        entry_points,
        backend,
    ))
}

async fn assemble_program_registry(
    store: &QualifiedPostgresStore,
    issuer: &RunAccessAuthorityIssuer,
    executable: &CurrentExecutableIdentity,
    evm: EvmDeployment,
) -> Result<(Arc<QualifiedProgramRegistry>, Vec<EntryPointContract>), PublicError> {
    let EvmDeployment {
        transport,
        routing_manifest,
    } = evm;
    let product = qualify_product_components(
        executable.content_ref().clone(),
        executable.descriptor_bytes(),
    )?;
    let live = qualify_evm_live_support(&transport, &product)?;
    let deployment = assemble_qualified_product_deployment(product, live, &routing_manifest)?;
    let QualifiedProductDeployment {
        object_evidence_contract_ref,
        qualification_profile_ref: _qualification_profile_ref,
        qualification_ref,
        implementations,
        read_capability_binding,
        read_capability_binding_ref,
        unit_config_contract,
        state_manifest: _state_manifest,
        state_manifest_ref: _state_manifest_ref,
        capability_manifest: _capability_manifest,
        capability_manifest_ref: _capability_manifest_ref,
        support_graph,
    } = deployment;
    let qualification_scope_id = support_graph.qualification_scope_id().clone();
    let deployment_authority = issuer.authorize_qualified_deployment(qualification_scope_id);
    let admitted_support = store
        .admit_support_graph(&deployment_authority, support_graph)
        .await?;
    let ProductComponentImplementations {
        planner,
        portfolio,
        evm: evm_states,
        evm_read_adapter: _evm_read_adapter,
    } = implementations;
    let planner_contract_ref = planner.semantic_contract_ref().clone();
    let planner_implementation_ref = planner
        .content_ref()
        .map_err(|_| production_registry_invalid())?;
    let planner =
        QualifiedPlannerRegistration::new(planner, Arc::new(CompositeCertificationFactory))
            .map_err(|_| production_registry_invalid())?;

    let entry_point = mfm_portfolio::portfolio_snapshot_entry_point_registration(
        mfm_portfolio::PortfolioSnapshotEntryPointArtifacts::new(
            planner_contract_ref,
            planner_implementation_ref,
            object_evidence_contract_ref.clone(),
            unit_config_contract.clone(),
        )
        .map_err(|_| production_registry_invalid())?,
    )
    .map_err(|_| production_registry_invalid())?;
    let published_entry_point = entry_point.entry_point().clone();
    let portfolio_states = mfm_portfolio::qualify_portfolio_snapshot_states(
        mfm_portfolio::PortfolioSnapshotStateArtifacts::new(
            object_evidence_contract_ref.clone(),
            unit_config_contract.clone(),
            portfolio,
        )
        .map_err(|_| production_registry_invalid())?,
    )
    .map_err(|_| production_registry_invalid())?;
    let evm_states = mfm_evm::qualify_evm_balance_collection_states(
        mfm_evm::EvmBalanceCollectionStateArtifacts::new(
            object_evidence_contract_ref.clone(),
            unit_config_contract,
            read_capability_binding_ref,
            evm_states,
        )
        .map_err(|_| production_registry_invalid())?,
    )
    .map_err(|_| production_registry_invalid())?;
    let read_qualification = mfm_evm_live::EvmReadQualificationArtifacts::new(
        &admitted_support,
        executable.content_ref(),
        &qualification_ref,
        &object_evidence_contract_ref,
    )?;
    let reads = mfm_evm_live::qualify_evm_read_entries(
        read_capability_binding,
        &read_qualification,
        transport,
    )?;
    drop(read_qualification);

    let journal_protocols =
        CertifiedJournalProtocolContracts::current().map_err(|_| production_registry_invalid())?;
    let mut builder = QualifiedProgramRegistry::builder(
        executable.content_ref().clone(),
        planner,
        admitted_support,
        journal_protocols,
    );
    builder
        .register_entry_point(entry_point)
        .map_err(|_| production_registry_invalid())?;
    portfolio_states
        .register_into(&mut builder)
        .map_err(|_| production_registry_invalid())?;
    evm_states
        .register_into(&mut builder)
        .map_err(|_| production_registry_invalid())?;
    reads
        .register_into(&mut builder)
        .map_err(|_| production_registry_invalid())?;
    let registry = Arc::new(builder.build().map_err(|_| production_registry_invalid())?);
    Ok((registry, vec![published_entry_point]))
}

impl ProductionBackend {
    async fn ready(&self) -> Result<(), PublicError> {
        self.store
            .check_ready()
            .await
            .map_err(|_| run_store_unavailable())
    }

    async fn admit(
        &self,
        tenant_scope_id: TenantScopeId,
        entry_point: EntryPointContract,
        request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError> {
        let selector: mfm_portfolio::PortfolioSnapshotSelector =
            serde_json::from_value(request.input().as_json().clone()).map_err(|_| {
                PublicError::bad_request(
                    "AdmissionRequestInvalid",
                    "Admission request does not match the published entry-point input",
                )
            })?;
        let configured_target = StableId::new(selector.target().as_str())
            .map_err(|_| production_admission_invalid())?;
        let authority = self.issuer.authorize_admit(
            tenant_scope_id,
            entry_point.entry_point_id().clone(),
            entry_point.entry_point_operation_id().clone(),
            request.invocation_identity().clone(),
        );
        let registration = self
            .registry
            .entry_point(entry_point.entry_point_id())
            .ok_or_else(production_admission_invalid)?;
        let configured = self
            .store
            .resolve_configured_value(
                &authority,
                entry_point.entry_point_id(),
                &configured_target,
                registration
                    .input_contract()
                    .configured_value_contract()
                    .root_contract(),
            )
            .await?;
        let artifacts = self
            .registry
            .author_and_certify_verified_parts(
                entry_point.entry_point_id(),
                configured.binding(),
                configured.value_ref(),
                configured.bytes(),
            )
            .map_err(|_| production_admission_invalid())?;
        let input = ProposedAdmissionInput::new(
            request
                .input()
                .canonical_json()
                .map_err(|_| production_admission_invalid())?,
            registration
                .input_contract()
                .run_admission_contract()
                .root_contract()
                .clone(),
        );
        let sources = self.store.verify_no_admission_sources(&authority).await?;
        let append_request_id = admission_append_request_id(request.invocation_identity())?;
        let prepared = self.store.prepare_admission(
            &authority,
            append_request_id,
            AdmissionMaterial::new(
                artifacts,
                input,
                &configured,
                self.registry.admitted_support(),
                &sources,
            ),
        )?;
        let run_id = prepared
            .admission()
            .fields()
            .map_err(|_| production_admission_invalid())?
            .run_id;
        let outcome = self
            .store
            .append_admission(&authority, prepared)
            .await
            .map_err(admission_store_error)?;
        let status = admission_status(outcome)?;
        AdmitRunResponse::new(
            &run_id,
            status,
            entry_point.entry_point_id(),
            entry_point.entry_point_operation_id(),
            request.invocation_identity(),
            entry_point.planning_profile_ref(),
        )
    }

    async fn drive(&self, call: &AuthorizedRunCall<'_>) -> Result<DriveResponse, PublicError> {
        let authority = self
            .issuer
            .authorize_drive(call.tenant_scope_id().clone(), call.run_id().clone());
        let outcome = self.runtime.drive_once(authority).await?;
        DriveResponse::from_runtime(call.run_id(), outcome)
    }

    async fn read_public(
        &self,
        call: &AuthorizedRunCall<'_>,
    ) -> Result<PublicRunView, PublicError> {
        let authority = self
            .issuer
            .authorize_read_public(call.tenant_scope_id().clone(), call.run_id().clone());
        let verified = self.store.read_public_run(&authority).await?;
        PublicRunView::from_verified(verified)
    }

    async fn audit(
        &self,
        call: &AuthorizedRunCall<'_>,
        page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError> {
        let position = decode_access_audit_page_request(call.run_id(), &page)?;
        let authority = self
            .issuer
            .authorize_inspect_audit(call.tenant_scope_id().clone(), call.run_id().clone());
        let page = mfm_replay::v1::inspect_access_audit(
            &self.store,
            &authority,
            position.complete_as_of_journal_head.as_ref(),
            position.start,
            position.limit,
        )
        .await?;
        complete_access_audit_page(page)
    }

    async fn trace(
        &self,
        call: &AuthorizedRunCall<'_>,
        page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError> {
        let request = decode_transition_trace_page_request(call.run_id(), &page)?;
        let root_authority = self
            .issuer
            .authorize_inspect_trace(call.tenant_scope_id().clone(), call.run_id().clone());
        let requirements = mfm_replay::v1::discover_transition_trace_sources(
            &self.store,
            &root_authority,
            request,
        )
        .await?;
        let mut source_authorities = Vec::with_capacity(requirements.source_run_ids().len());
        for source_run_id in requirements.source_run_ids() {
            let Some(tenant_scope_id) = call.authorize_trace_source(source_run_id.clone()).await?
            else {
                continue;
            };
            source_authorities.push(
                self.issuer
                    .authorize_inspect_trace(tenant_scope_id, source_run_id.clone()),
            );
        }
        let page = mfm_replay::v1::inspect_transition_trace(
            &self.store,
            &root_authority,
            requirements,
            &source_authorities,
        )
        .await?;
        complete_transition_trace_page(page)
    }

    async fn replay(
        &self,
        call: &AuthorizedRunCall<'_>,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError> {
        let authority = self
            .issuer
            .authorize_replay(call.tenant_scope_id().clone(), call.run_id().clone());
        let verified = mfm_replay::v1::verify_recorded_history(&self.store, &authority).await?;
        match request {
            ReplayRequest::Verify => verified.canonical_result().map_err(Into::into),
            ReplayRequest::Reproduce(input) => {
                let historical = verified
                    .verify_portable_export(input.bytes(), input.content_ref())
                    .map_err(replay_artifact_error)?;
                mfm_replay::v1::reproduce_exact(&historical, &UnavailableReproductionResolver)
                    .await
                    .map_err(Into::into)
            }
            ReplayRequest::CompareCurrent(input) => {
                let historical = verified
                    .verify_portable_export(input.bytes(), input.content_ref())
                    .map_err(replay_artifact_error)?;
                mfm_replay::v1::compare_current(&historical, self.registry.as_ref())
                    .map_err(Into::into)
            }
        }
    }

    async fn export(
        &self,
        call: &AuthorizedRunCall<'_>,
        request: ExportRequest,
    ) -> Result<ExportedRunBytes, PublicError> {
        let root_authority = self
            .issuer
            .authorize_export(call.tenant_scope_id().clone(), call.run_id().clone());
        let mut pending =
            mfm_replay::v1::required_export_source_run_ids(&self.store, &root_authority)
                .await?
                .into_iter()
                .collect::<BTreeSet<_>>();
        let mut dependencies = BTreeMap::<RunId, RunAccessAuthority<mfm_store::Export>>::new();
        while let Some(run_id) = pending.pop_first() {
            if dependencies.contains_key(&run_id) {
                continue;
            }
            let tenant_scope_id = call.authorize_required_dependency(run_id.clone()).await?;
            let authority = self
                .issuer
                .authorize_export(tenant_scope_id, run_id.clone());
            let required = mfm_replay::v1::required_export_source_run_ids(&self.store, &authority)
                .await
                .map_err(export_dependency_discovery_error)?;
            dependencies.insert(run_id, authority);
            pending.extend(
                required
                    .into_iter()
                    .filter(|run_id| !dependencies.contains_key(run_id)),
            );
        }
        let dependencies = dependencies.into_values().collect::<Vec<_>>();
        let export = mfm_replay::trace_export::export_portable_run(
            &self.store,
            &root_authority,
            &dependencies,
            request.kind(),
        )
        .await?;
        Ok(ExportedRunBytes::from_portable(export))
    }
}

#[async_trait::async_trait]
impl ApplicationBackend for ProductionBackend {
    async fn check_ready(&self) -> Result<(), PublicError> {
        self.ready().await
    }

    async fn admit_run(
        &self,
        tenant_scope_id: TenantScopeId,
        entry_point: EntryPointContract,
        request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError> {
        self.admit(tenant_scope_id, entry_point, request).await
    }

    async fn drive_once(&self, call: &AuthorizedRunCall<'_>) -> Result<DriveResponse, PublicError> {
        self.drive(call).await
    }

    async fn read_public_run(
        &self,
        call: &AuthorizedRunCall<'_>,
    ) -> Result<PublicRunView, PublicError> {
        self.read_public(call).await
    }

    async fn replay_run(
        &self,
        call: &AuthorizedRunCall<'_>,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError> {
        self.replay(call, request).await
    }

    async fn read_transition_trace(
        &self,
        call: &AuthorizedRunCall<'_>,
        page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError> {
        self.trace(call, page).await
    }

    async fn read_access_audit(
        &self,
        call: &AuthorizedRunCall<'_>,
        page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError> {
        self.audit(call, page).await
    }

    async fn export_run(
        &self,
        call: &AuthorizedRunCall<'_>,
        request: ExportRequest,
    ) -> Result<ExportedRunBytes, PublicError> {
        self.export(call, request).await
    }
}

fn admission_append_request_id(
    invocation_identity: &mfm_ids::InvocationIdentity,
) -> Result<AppendRequestId, PublicError> {
    AppendRequestId::new(format!("admission/{}", invocation_identity.as_str()))
        .map_err(|_| production_admission_invalid())
}

fn admission_status(outcome: AppendOutcome) -> Result<AdmissionStatus, PublicError> {
    match outcome {
        AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(_)) => {
            Ok(AdmissionStatus::NewlyAdmitted)
        }
        AppendOutcome::AlreadyCommitted(_) => Ok(AdmissionStatus::Attached),
        AppendOutcome::OutcomeUnknown => Ok(AdmissionStatus::OutcomeUnknown),
        AppendOutcome::Rejected(rejection) => Err(match rejection {
            AppendRejection::AdmissionConflict => mfm_store::StoreError::AdmissionConflict.into(),
            AppendRejection::AppendRequestConflict => {
                mfm_store::StoreError::AdmissionConflict.into()
            }
            AppendRejection::StaleHead { expected, actual } => {
                mfm_store::StoreError::HeadMismatch {
                    expected: Box::new(expected),
                    actual,
                }
                .into()
            }
            AppendRejection::RunClosed => mfm_store::StoreError::RunClosed.into(),
        }),
        AppendOutcome::NewlyAppended(
            NewlyAppended::Transition(_)
            | NewlyAppended::Authorization(_)
            | NewlyAppended::Observation(_),
        ) => Err(production_admission_invalid()),
    }
}

fn admission_store_error(error: mfm_storage_postgres::PostgresStoreError) -> PublicError {
    match error {
        mfm_storage_postgres::PostgresStoreError::Store(error)
            if matches!(error.as_ref(), mfm_store::StoreError::AppendRequestConflict) =>
        {
            mfm_store::StoreError::AdmissionConflict.into()
        }
        error => error.into(),
    }
}

fn replay_artifact_error(error: mfm_replay::v1::ReplayError) -> PublicError {
    if error.kind() == mfm_replay::v1::ReplayErrorKind::InvalidExport {
        PublicError::replay_artifact_invalid()
    } else {
        error.into()
    }
}

fn export_dependency_discovery_error(error: mfm_replay::v1::ReplayError) -> PublicError {
    if error.kind() == mfm_replay::v1::ReplayErrorKind::RunNotFound {
        PublicError::replay_verification_failed()
    } else {
        error.into()
    }
}

fn production_admission_invalid() -> PublicError {
    PublicError::backend(
        ErrorClass::Internal,
        "AdmissionPlanningInvalid",
        "The qualified admission could not be prepared",
    )
}

fn production_registry_invalid() -> PublicError {
    PublicError::backend(
        ErrorClass::Internal,
        "ProductionRegistryInvalid",
        "The production program registry is invalid",
    )
}

fn run_store_unavailable() -> PublicError {
    PublicError::backend(
        ErrorClass::ServiceUnavailable,
        "RunStoreUnavailable",
        "The authoritative run store is unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::{export_dependency_discovery_error, replay_artifact_error, ErrorClass};
    use mfm_store::{AppendOutcome, AppendRejection};

    #[test]
    fn admission_append_request_conflict_uses_the_admission_contract() {
        let outcome_error = super::admission_status(AppendOutcome::Rejected(
            AppendRejection::AppendRequestConflict,
        ))
        .expect_err("admission append-request conflict must be redacted");
        let backend_error =
            super::admission_store_error(mfm_storage_postgres::PostgresStoreError::Store(
                Box::new(mfm_store::StoreError::AppendRequestConflict),
            ));
        for error in [outcome_error, backend_error] {
            assert_eq!(error.class, ErrorClass::Conflict);
            assert_eq!(error.code, "AdmissionConflict");
        }
    }

    #[test]
    fn only_caller_export_validation_uses_the_artifact_error_contract() {
        let invalid = replay_artifact_error(mfm_replay::v1::ReplayError::InvalidExport);
        assert_eq!(invalid.class, ErrorClass::BadRequest);
        assert_eq!(invalid.code, "ReplayArtifactInvalid");

        for error in [
            mfm_replay::v1::ReplayError::InvalidRecordedHistory,
            mfm_replay::v1::ReplayError::AuthorityMismatch,
        ] {
            let error = replay_artifact_error(error);
            assert_eq!(error.class, ErrorClass::Internal);
            assert_eq!(error.code, "ReplayVerificationFailed");
        }
    }

    #[test]
    fn authorized_missing_export_dependency_is_an_integrity_failure() {
        let error = export_dependency_discovery_error(mfm_replay::v1::ReplayError::RunNotFound);
        assert_eq!(error.class, ErrorClass::Internal);
        assert_eq!(error.code, "ReplayVerificationFailed");
    }
}
