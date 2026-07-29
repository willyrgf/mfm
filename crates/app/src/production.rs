//! Private production composition for one qualified authoritative application.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_certify::CompositeCertificationFactory;
use mfm_executor::KeyedExecutorLedger;
use mfm_ids::{AppendRequestId, RunId, StableId, TenantScopeId};
use mfm_keystore::KeystoreSignerProvider;
use mfm_program::{
    QualifiedExecutorExpansion, QualifiedPlannerRegistration, QualifiedProgramRegistry,
};
use mfm_runtime::Runtime;
use mfm_signing::{
    GenerationGuardedDeterministicSigningProvider,
    GenerationGuardedDeterministicSigningProviderBinder,
};
use mfm_spec::{CertifiedJournalProtocolContracts, EntryPointContract};
use mfm_storage_executor_postgres::{
    open_executor_store, ExecutorWriterGenerationFence, QualifiedPostgresExecutorStore,
};
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
use crate::application::{
    ApplicationBackend, AuthorizedRunCall, EvmWalletDeployment, EvmWalletDeploymentParts,
};
use crate::executable_identity::{current_executable_identity, CurrentExecutableIdentity};
use crate::stream_spool::{snapshot_input, WritableSpool};
use crate::{
    complete_access_audit_page, complete_transition_trace_page, decode_access_audit_page_request,
    decode_transition_trace_page_request, production_database_url, AccessAuditPage,
    AdmissionStatus, AdmitRunRequest, AdmitRunResponse, Application, DriveResponse, ErrorClass,
    ExportRequest, ExportedRun, PageRequest, PublicError, PublicRunView, ReplayRequest,
    ReplayResponse, RunAccessPolicy, TransitionTracePage,
};

mod qualification;
mod routes;

struct ProductionBackend {
    store: QualifiedPostgresStore,
    executor_store: QualifiedPostgresExecutorStore,
    issuer: RunAccessAuthorityIssuer,
    registry: Arc<QualifiedProgramRegistry>,
    runtime: Runtime<QualifiedPostgresStore>,
    wallet_request_qualification: Arc<mfm_evm_live::EvmWalletRequestQualification>,
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

pub(super) async fn connect<RunFence, ExecutorFence>(
    database_url: Option<&str>,
    runtime_config_path: Option<&Path>,
    policy: Arc<dyn RunAccessPolicy>,
    deployment_writer_fence: RunFence,
    wallet: EvmWalletDeployment,
    executor_writer_fence: ExecutorFence,
) -> Result<Application, PublicError>
where
    RunFence: AuthoritativeWriterFence + 'static,
    ExecutorFence: ExecutorWriterGenerationFence + 'static,
{
    let database_url = production_database_url(database_url)?;
    let pool = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .map_err(|_| run_store_unavailable())?;
    let (store, issuer) = open_authoritative(pool, deployment_writer_fence).await?;
    let wallet = wallet.into_parts();
    let signer_ref = wallet.signer_binding.signer_ref().clone();
    let (executable, evm) = tokio::try_join!(
        current_executable_identity(),
        load_evm_deployment(runtime_config_path, signer_ref)
    )?;
    let (registry, entry_points, executor_store, wallet_request_qualification) =
        assemble_program_registry(
            &store,
            &issuer,
            &executable,
            evm,
            wallet,
            executor_writer_fence,
        )
        .await?;
    let store_scope_id = store.store_scope_id().clone();
    let runtime = Runtime::new(store.clone(), Arc::clone(&registry));
    let backend = ProductionBackend {
        store,
        executor_store,
        issuer,
        registry,
        runtime,
        wallet_request_qualification,
    };
    Ok(Application::new(
        store_scope_id,
        policy,
        entry_points,
        backend,
    ))
}

async fn assemble_program_registry<ExecutorFence>(
    store: &QualifiedPostgresStore,
    issuer: &RunAccessAuthorityIssuer,
    executable: &CurrentExecutableIdentity,
    evm: EvmDeployment,
    wallet: EvmWalletDeploymentParts,
    executor_writer_fence: ExecutorFence,
) -> Result<
    (
        Arc<QualifiedProgramRegistry>,
        Vec<EntryPointContract>,
        QualifiedPostgresExecutorStore,
        Arc<mfm_evm_live::EvmWalletRequestQualification>,
    ),
    PublicError,
>
where
    ExecutorFence: ExecutorWriterGenerationFence,
{
    let EvmDeployment {
        transport,
        routing_manifest,
        signer: resolved_signer,
    } = evm;
    let EvmWalletDeploymentParts {
        executor_pool,
        executor_contract,
        executor_deployment,
        resource_ownership,
        route_generation_ref,
        initial_nonce_descriptor,
        signer_binding,
        signer_generation_guard,
    } = wallet;
    let product = qualify_product_components(
        executable.content_ref().clone(),
        executable.descriptor_bytes(),
        &executor_contract,
    )?;
    let live = qualify_evm_live_support(
        &transport,
        &product,
        executor_contract,
        executor_deployment,
        resource_ownership,
        route_generation_ref,
        initial_nonce_descriptor,
        &signer_binding,
    )?;
    let deployment = assemble_qualified_product_deployment(product, live, &routing_manifest)?;
    let QualifiedProductDeployment {
        object_evidence_contract_ref,
        qualification_profile_ref: _qualification_profile_ref,
        qualification_ref,
        implementations,
        read_capability_binding,
        read_capability_binding_ref,
        executor_binding,
        wallet_request_qualification,
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
    let executor_store = open_executor_store(
        executor_pool,
        executor_binding.clone(),
        executor_writer_fence,
    )
    .await
    .map_err(|_| wallet_executor_unavailable())?;
    let ledger = KeyedExecutorLedger::new(executor_store.clone(), executor_binding.clone())
        .map_err(|_| production_registry_invalid())?;
    let (entry_id, keystore_path, unlock_file_path) = resolved_signer.into_parts();
    let provider_binding = signer_binding.clone();
    let provider_generation_guard = Arc::clone(&signer_generation_guard);
    let signer_binder =
        GenerationGuardedDeterministicSigningProviderBinder::new(signer_binding, move || {
            let provider = KeystoreSignerProvider::new(
                provider_binding.clone(),
                entry_id,
                keystore_path.clone(),
                unlock_file_path.clone(),
                Arc::clone(&provider_generation_guard),
            );
            Box::pin(async move {
                provider.map(|provider| {
                    Arc::new(provider) as Arc<dyn GenerationGuardedDeterministicSigningProvider>
                })
            })
        });
    let wallet_executor = mfm_evm_live::EvmWalletExecutor::new(
        ledger,
        signer_binder,
        Arc::clone(&wallet_request_qualification),
    )
    .map_err(|_| production_registry_invalid())?;
    let ProductComponentImplementations {
        planner,
        portfolio,
        evm: evm_states,
        evm_submit_transaction,
        evm_read_adapter: _evm_read_adapter,
        evm_wallet_executor,
    } = implementations;
    let planner_contract_ref = planner.semantic_contract_ref().clone();
    let planner_implementation_ref = planner
        .content_ref()
        .map_err(|_| production_registry_invalid())?;
    let planner =
        QualifiedPlannerRegistration::new(planner, Arc::new(CompositeCertificationFactory))
            .map_err(|_| production_registry_invalid())?;

    let portfolio_entry_point = mfm_portfolio::portfolio_snapshot_entry_point_registration(
        mfm_portfolio::PortfolioSnapshotEntryPointArtifacts::new(
            planner_contract_ref.clone(),
            planner_implementation_ref.clone(),
            object_evidence_contract_ref.clone(),
            unit_config_contract.clone(),
        )
        .map_err(|_| production_registry_invalid())?,
    )
    .map_err(|_| production_registry_invalid())?;
    let published_portfolio_entry_point = portfolio_entry_point.entry_point().clone();
    let wallet_entry_point = mfm_evm::evm_submit_transaction_entry_point_registration(
        mfm_evm::EvmSubmitTransactionEntryPointArtifacts::new(
            planner_contract_ref,
            planner_implementation_ref,
            object_evidence_contract_ref.clone(),
            unit_config_contract.clone(),
        )
        .map_err(|_| production_registry_invalid())?,
    )
    .map_err(|_| production_registry_invalid())?;
    let published_wallet_entry_point = wallet_entry_point.entry_point().clone();
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
            unit_config_contract.clone(),
            read_capability_binding_ref,
            evm_states,
        )
        .map_err(|_| production_registry_invalid())?,
    )
    .map_err(|_| production_registry_invalid())?;
    let executor_contract_ref = executor_binding
        .contract()
        .reference()
        .map_err(|_| production_registry_invalid())?;
    let executor_binding_ref = executor_binding.binding_ref().as_content_ref().clone();
    let retained = executor_binding.contract().retained_closure_contract();
    let wallet_state = mfm_evm::qualify_evm_submit_transaction_state(
        mfm_evm::EvmSubmitTransactionStateArtifacts::new(
            object_evidence_contract_ref.clone(),
            unit_config_contract.clone(),
            executor_contract_ref.clone(),
            executor_binding_ref,
            retained.ensure_result_contract().clone(),
            retained.terminal_evidence_contract().clone(),
            evm_submit_transaction,
        )
        .map_err(|_| production_registry_invalid())?,
    )
    .map_err(|_| production_registry_invalid())?;
    let operation_contract = executor_binding
        .contract()
        .required_plan_expansions()
        .first()
        .cloned()
        .ok_or_else(production_registry_invalid)?;
    let effect = mfm_runtime::qualify_effect_executor(
        StableId::new(mfm_evm::EVM_SUBMIT_TRANSACTION_OPERATION_ID)
            .map_err(|_| production_registry_invalid())?,
        executor_binding,
        operation_contract,
        evm_wallet_executor,
        wallet_executor,
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
        .register_entry_point(portfolio_entry_point)
        .map_err(|_| production_registry_invalid())?
        .register_entry_point(wallet_entry_point)
        .map_err(|_| production_registry_invalid())?;
    portfolio_states
        .register_into(&mut builder)
        .map_err(|_| production_registry_invalid())?;
    evm_states
        .register_into(&mut builder)
        .map_err(|_| production_registry_invalid())?;
    wallet_state
        .register_into(&mut builder)
        .map_err(|_| production_registry_invalid())?;
    reads
        .register_into(&mut builder)
        .map_err(|_| production_registry_invalid())?;
    builder
        .register_executor(QualifiedExecutorExpansion::Leaf {
            executor_contract_ref,
        })
        .map_err(|_| production_registry_invalid())?
        .register_effect(effect)
        .map_err(|_| production_registry_invalid())?;
    let registry = Arc::new(builder.build().map_err(|_| production_registry_invalid())?);
    Ok((
        registry,
        vec![
            published_portfolio_entry_point,
            published_wallet_entry_point,
        ],
        executor_store,
        wallet_request_qualification,
    ))
}

impl ProductionBackend {
    async fn ready(&self) -> Result<(), PublicError> {
        self.store
            .check_ready()
            .await
            .map_err(|_| run_store_unavailable())?;
        let readiness = self
            .executor_store
            .readiness()
            .await
            .map_err(|_| wallet_executor_unavailable())?;
        if !readiness.ledger_ready() || !readiness.fence_ready() {
            return Err(wallet_executor_unavailable());
        }
        Ok(())
    }

    async fn admit(
        &self,
        tenant_scope_id: TenantScopeId,
        entry_point: EntryPointContract,
        request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError> {
        let (configured_target, wallet_selector) = if entry_point.entry_point_id().as_str()
            == mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID
        {
            let selector: mfm_portfolio::PortfolioSnapshotSelector =
                serde_json::from_value(request.input().as_json().clone()).map_err(|_| {
                    PublicError::bad_request(
                        "AdmissionRequestInvalid",
                        "Admission request does not match the published entry-point input",
                    )
                })?;
            (
                StableId::new(selector.target().as_str())
                    .map_err(|_| production_admission_invalid())?,
                None,
            )
        } else if entry_point.entry_point_id().as_str()
            == mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID
        {
            let selector: mfm_evm::EvmSubmitTransactionSelector =
                serde_json::from_value(request.input().as_json().clone()).map_err(|_| {
                    PublicError::bad_request(
                        "AdmissionRequestInvalid",
                        "Admission request does not match the published entry-point input",
                    )
                })?;
            (
                StableId::new(selector.target().as_str())
                    .map_err(|_| production_admission_invalid())?,
                Some(selector),
            )
        } else {
            return Err(PublicError::bad_request(
                "AdmissionRequestInvalid",
                "Admission request does not match the published entry-point input",
            ));
        };
        let authority = self.issuer.authorize_admit(
            tenant_scope_id.clone(),
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
        if let Some(selector) = wallet_selector {
            qualify_wallet_admission_request(
                self.wallet_request_qualification.as_ref(),
                &tenant_scope_id,
                &selector,
                configured.bytes(),
            )?;
        }
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
                let (content_ref, reader) = snapshot_input(input)
                    .await
                    .map_err(|_| export_stream_io_error())?;
                let historical = verified
                    .verify_export_stream(reader, &content_ref)
                    .await
                    .map_err(replay_artifact_error)?;
                mfm_replay::v1::reproduce_exact(&historical, &UnavailableReproductionResolver)
                    .await
                    .map_err(Into::into)
            }
            ReplayRequest::CompareCurrent(input) => {
                let (content_ref, reader) = snapshot_input(input)
                    .await
                    .map_err(|_| export_stream_io_error())?;
                let historical = verified
                    .verify_export_stream(reader, &content_ref)
                    .await
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
    ) -> Result<ExportedRun, PublicError> {
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
        let mut spool = WritableSpool::create()
            .await
            .map_err(|_| export_stream_io_error())?;
        let metadata = mfm_replay::trace_export::write_portable_run_export_stream(
            &self.store,
            &root_authority,
            &dependencies,
            request.kind(),
            &mut spool,
        )
        .await?;
        let spool = spool.finish().await.map_err(|_| export_stream_io_error())?;
        Ok(ExportedRun::from_parts(metadata, Box::pin(spool)))
    }
}

fn qualify_wallet_admission_request(
    qualification: &mfm_evm_live::EvmWalletRequestQualification,
    tenant_scope_id: &TenantScopeId,
    selector: &mfm_evm::EvmSubmitTransactionSelector,
    configured_bytes: &[u8],
) -> Result<(), PublicError> {
    let canonical =
        PlainCanonicalJsonBytes::from_canonical_json_slice(configured_bytes).map_err(|_| {
            PublicError::backend(
                ErrorClass::Internal,
                "ConfiguredTransactionInvalid",
                "The configured transaction does not match its qualified contract",
            )
        })?;
    let request = mfm_program::decode_boundary::<mfm_evm::EvmSubmitTransactionRequest>(&canonical)
        .map_err(|_| production_admission_invalid())?;
    qualification
        .verify_request(&request)
        .map_err(|_| production_admission_invalid())?;
    if request
        .policy()
        .tenant_scope_id()
        .map_err(|_| production_admission_invalid())?
        != *tenant_scope_id
        || request.template().target() != selector.target()
    {
        return Err(production_admission_invalid());
    }
    Ok(())
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
    ) -> Result<ExportedRun, PublicError> {
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
    match error.kind() {
        mfm_replay::v1::ReplayErrorKind::InvalidExport => PublicError::replay_artifact_invalid(),
        mfm_replay::v1::ReplayErrorKind::ExportStreamIo => export_stream_io_error(),
        _ => error.into(),
    }
}

fn export_stream_io_error() -> PublicError {
    PublicError::internal(
        "ExportStreamIoFailed",
        "The export stream could not be processed",
    )
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

fn wallet_executor_unavailable() -> PublicError {
    PublicError::backend(
        ErrorClass::ServiceUnavailable,
        "EvmWalletExecutorUnavailable",
        "The qualified EVM wallet executor is unavailable",
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

        let error = replay_artifact_error(mfm_replay::v1::ReplayError::ExportStreamIo {
            source: std::io::Error::other("private sentinel"),
        });
        assert_eq!(error.class, ErrorClass::Internal);
        assert_eq!(error.code, "ExportStreamIoFailed");
        assert_eq!(error.message, "The export stream could not be processed");
    }

    #[test]
    fn authorized_missing_export_dependency_is_an_integrity_failure() {
        let error = export_dependency_discovery_error(mfm_replay::v1::ReplayError::RunNotFound);
        assert_eq!(error.class, ErrorClass::Internal);
        assert_eq!(error.code, "ReplayVerificationFailed");
    }
}
