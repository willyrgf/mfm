//! Private production composition for the structured Runtime and RunHistory.

use std::collections::BTreeSet;
use std::sync::Arc;

use alloy_primitives::Address;
use async_trait::async_trait;
use mfm_canonical::RecoverabilityContract;
use mfm_certify::structured::{AdmissionCertificationRegistry, ProgramRegistryBuilder};
use mfm_ids::{
    AppendRequestId, ContentRef, DigestAlgorithm, EntryPointId, InvocationIdentity, RunId,
    SchemaId, StableId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, AccessKind, HistoryObject, ADMISSION_CONFIGURATION_OBJECT_TYPE,
};
use mfm_program::structured::RuntimeResourceAuthority;
use mfm_runtime::history::{HistoryAppendOutcome, StructuredAdmissionCommand};
use mfm_runtime::structured::{
    Runtime, RuntimeError, RuntimeFaultCode, RuntimeFaultPhase, RuntimeFaultSubject,
    RuntimeStoreFaultKind,
};
use mfm_spec::structured::{
    SecretFreeExecutableIdentity, SecretFreeQualificationArtifact, StructuredExpansionProfile,
};
use mfm_spec::{EntryPointContract, PlanningProfile};
use mfm_storage_postgres::{
    open_structured_authoritative_application, ApplicationTargetSessions,
    PostgresConfigurationHistoryBackend, PostgresStructuredHistoryBackend,
};
use mfm_store::structured::{
    AuditRunReader, ConfigurationHistoryReader, ConfigurationStreamKey, ExportRunEvidence,
    ExportRunReader, PhysicalBindingAuthorization, PhysicalBindingSupersession,
    PhysicalBindingVerificationMode, ProposedCanonicalValue, PublicPhysicalBindingVerifier,
    PublicRunEvidence, PublicRunReader, ReplayRunReader, StructuredAdmissionMaterial,
    StructuredStoreError, TraceRunReader, VerifiedConfiguredValue,
};
use mfm_values::{MfmConfig, MfmValue};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::application::{
    run_grant, ApplicationBackend, AuthorizedAdmissionCall, AuthorizedRunCall, EvmWalletDeployment,
    EvmWalletDeploymentParts,
};
use crate::stream_spool::{snapshot_input, WritableSpool};
use crate::{
    complete_access_audit_page, complete_transition_trace_page, decode_access_audit_page_request,
    decode_transition_trace_page_request, AccessAuditPage,
    AdmissionStatus, AdmitRunRequest, AdmitRunResponse, Application, DriveResponse,
    EntryPointContract as PublicEntryPointContract, ErrorClass, ExportRequest, ExportedRun,
    PageRequest, PublicError, PublicRunView, PublicRuntimeFaultAttribution,
    PublicRuntimeFaultPhase, PublicRuntimeFaultSubject, ReplayRequest, ReplayResponse,
    RunAccessPolicy, TransitionTracePage,
};

const EXECUTABLE_ID: &str = "mfm.application/structured-runtime";
const QUALIFICATION_ID: &str = "mfm.application/structured-production-qualification";
const PORTFOLIO_SCOPE_ID: &str = "mfm.portfolio/structured-snapshot-scope";
const BALANCE_SCOPE_ID: &str = "mfm.evm/structured-balance-scope";
const SUBMISSION_SCOPE_ID: &str = "mfm.evm/structured-submission-scope";
const CONFIGURATION_REVISION_SCHEMA: &str = "mfm.structured-configuration-revision";
const STRUCTURED_EXPORT_VERSION: &str = "mfm.structured-portable-run-export.v1";
const MAX_REPLAY_EXPORT_BYTES: u64 = 512 * 1024 * 1024;

type HistoryBackend = PostgresStructuredHistoryBackend;
type ConfigReader = ConfigurationHistoryReader<PostgresConfigurationHistoryBackend>;

struct ProductionBackend {
    public_reader: PublicRunReader<HistoryBackend>,
    trace_reader: TraceRunReader<HistoryBackend>,
    audit_reader: AuditRunReader<HistoryBackend>,
    replay_reader: ReplayRunReader<HistoryBackend>,
    export_reader: ExportRunReader<HistoryBackend>,
    configuration: ConfigReader,
    runtime: Arc<Runtime<mfm_store::structured::StoreHistoryAdapter<HistoryBackend>>>,
    certifier: AdmissionCertificationRegistry,
    routing_manifest: mfm_portfolio::PortfolioRoutingManifest,
    routing_catalog: mfm_evm::EvmRoutingCatalogDescriptor,
    routing_policy: HistoryObject,
    context_manifest: HistoryObject,
    prior_run_source_manifest: HistoryObject,
    broadcast_resource_ref: ContentRef,
    wallet_resource_ref: ContentRef,
    /// Sealed deployment submission semantics derived from the qualified live
    /// signer at assembly time. Every configuration revision is recomputed
    /// against this tuple; untrusted configuration cannot echo or substitute
    /// qualified contract references.
    sealed_submission_semantics: mfm_evm::EvmDeploymentSubmissionSemantics,
    sealed_public_signing_identity: mfm_signing::PublicSigningIdentity,
    submission_route_generation_ref: mfm_evm::EvmWalletReference,
    submission_chain_instance: mfm_evm::EvmChainInstanceBinding,
    wallet_domain_activation_attestation: mfm_evm::WalletNonceDomainActivationAttestation,
}

pub(super) async fn connect(
    sessions: ApplicationTargetSessions,
    policy: Arc<dyn RunAccessPolicy>,
    wallet: EvmWalletDeployment,
) -> Result<Application, PublicError> {
    let EvmWalletDeploymentParts {
        routing_manifest,
        routing_catalog,
        routing_policy,
        context_manifest,
        prior_run_source_manifest,
        portfolio_coverage_inputs,
        sealed_submission_semantics,
        sealed_public_signing_identity,
        submission_bindings,
        balance_bindings,
        wallet_bindings,
    } = wallet.into_parts();

    let assembly = assemble_registry(
        &portfolio_coverage_inputs,
        Arc::clone(&submission_bindings),
        Arc::clone(&balance_bindings),
        Arc::clone(&wallet_bindings),
    )?;
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> =
        Arc::new(ExactPhysicalBindingVerifier::new(
            assembly.physical_binding_purposes.clone(),
            assembly.broadcast_resource_ref.clone(),
            wallet_bindings.resource_contract_ref().clone(),
        )?);
    let (assembled, configuration) = open_structured_authoritative_application(
        sessions,
        assembly.registry,
        physical_verifier,
    )
    .await
    .map_err(|_| run_store_unavailable())?;
    let runtime = Arc::new(assembled.runtime);
    let store_scope_id = assembled
        .public_reader
        .store_identity()
        .store_scope_id
        .clone();
    let backend = ProductionBackend {
        public_reader: assembled.public_reader,
        trace_reader: assembled.trace_reader,
        audit_reader: assembled.audit_reader,
        replay_reader: assembled.replay_reader,
        export_reader: assembled.export_reader,
        configuration,
        runtime,
        certifier: assembly.certifier,
        routing_manifest,
        routing_catalog,
        routing_policy,
        context_manifest,
        prior_run_source_manifest,
        broadcast_resource_ref: assembly.broadcast_resource_ref,
        wallet_resource_ref: wallet_bindings.resource_contract_ref().clone(),
        sealed_submission_semantics,
        sealed_public_signing_identity,
        submission_route_generation_ref: submission_bindings.route_generation_ref().clone(),
        submission_chain_instance: submission_bindings.chain_instance().clone(),
        wallet_domain_activation_attestation: wallet_bindings
            .domain_activation_attestation()
            .clone(),
    };
    Ok(Application::new(
        store_scope_id,
        policy,
        assembly.entry_points,
        backend,
    ))
}

struct RegistryAssembly {
    registry: mfm_certify::structured::QualifiedProgramRegistry,
    certifier: AdmissionCertificationRegistry,
    entry_points: Vec<PublicEntryPointContract>,
    broadcast_resource_ref: ContentRef,
    physical_binding_purposes: Vec<mfm_evm_live::EvmPhysicalBindingPurpose>,
}

fn assemble_registry(
    portfolio_coverage_inputs: &[mfm_evm::EvmBalanceLaneInput],
    submission_bindings: Arc<mfm_evm_live::EvmStructuredLiveBindings>,
    balance_bindings: Arc<mfm_evm_live::EvmStructuredBalanceBindings>,
    wallet_bindings: Arc<mfm_evm_live::EvmStructuredWalletBindings>,
) -> Result<RegistryAssembly, PublicError> {
    let mut builder = ProgramRegistryBuilder::new();
    let executable_ref = builder
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable(EXECUTABLE_ID)?,
        })
        .map_err(|_| registry_invalid())?;
    let qualification_ref = builder
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable(QUALIFICATION_ID)?,
        })
        .map_err(|_| registry_invalid())?;
    let evm_qualification = mfm_evm::EvmSubmissionProcessQualification::new(
        executable_ref.clone(),
        qualification_ref.clone(),
    );
    let portfolio_qualification = mfm_portfolio::PortfolioProcessQualification::new(
        executable_ref.clone(),
        qualification_ref.clone(),
    );

    mfm_evm::register_evm_submission_process(&mut builder, &evm_qualification)
        .map_err(|_| registry_invalid())?;
    mfm_evm::register_evm_balance_process(&mut builder, &evm_qualification)
        .map_err(|_| registry_invalid())?;
    mfm_portfolio::register_portfolio_process(&mut builder, &portfolio_qualification)
        .map_err(|_| registry_invalid())?;
    let mut physical_binding_purposes = mfm_evm_live::register_evm_live_submission_bindings(
        &mut builder,
        &evm_qualification,
        submission_bindings,
    )
    .map_err(|_| registry_invalid())?;
    physical_binding_purposes.extend(
        mfm_evm_live::register_evm_balance_bindings(
            &mut builder,
            &evm_qualification,
            balance_bindings,
        )
        .map_err(|_| registry_invalid())?,
    );
    physical_binding_purposes.extend(
        mfm_evm_live::register_evm_wallet_authority_bindings(
            &mut builder,
            &evm_qualification,
            wallet_bindings,
        )
        .map_err(|_| registry_invalid())?,
    );

    let portfolio_operation = stable(mfm_portfolio::STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID)?;
    let portfolio_program = mfm_portfolio::structured_portfolio_snapshot_program(
        portfolio_operation.clone(),
        stable(PORTFOLIO_SCOPE_ID)?,
        portfolio_coverage_inputs,
    )
    .map_err(|_| registry_invalid())?;
    builder
        .register_entry_point(
            portfolio_operation.clone(),
            portfolio_program,
            expansion_profile(2),
        )
        .map_err(|_| registry_invalid())?;

    let balance_operation = stable(mfm_evm::STRUCTURED_EVM_BALANCE_COLLECTION_OPERATION_ID)?;
    let balance_program = mfm_evm::structured_evm_balance_collection_program(
        balance_operation.clone(),
        stable(BALANCE_SCOPE_ID)?,
        &balance_coverage_inputs(portfolio_coverage_inputs)?,
    )
    .map_err(|_| registry_invalid())?;
    builder
        .register_entry_point(
            balance_operation.clone(),
            balance_program,
            expansion_profile(1),
        )
        .map_err(|_| registry_invalid())?;

    let submission_operation = stable(mfm_evm::EVM_SUBMIT_TRANSACTION_OPERATION_ID)?;
    let submission_program = mfm_evm::structured_evm_submission_entry_program(
        submission_operation.clone(),
        stable(SUBMISSION_SCOPE_ID)?,
    )
    .map_err(|_| registry_invalid())?;
    builder
        .register_entry_point(
            submission_operation.clone(),
            submission_program,
            expansion_profile(1),
        )
        .map_err(|_| registry_invalid())?;

    let expected_entry_points = [
        portfolio_operation.clone(),
        balance_operation,
        submission_operation.clone(),
    ];
    let registry = builder
        .build(&expected_entry_points)
        .map_err(|_| registry_invalid())?;
    let certifier = registry.admission_certification_registry();
    let planning_profile = || {
        PlanningProfile::new(
            executable_ref.clone(),
            qualification_ref.clone(),
            Vec::new(),
            mfm_spec::CanonicalJsonValue::empty_object(),
        )
        .map_err(|_| registry_invalid())
    };
    let entry_points = vec![
        EntryPointContract::new(
            EntryPointId::new(mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID)
                .map_err(|_| registry_invalid())?,
            portfolio_operation,
            planning_profile()?,
            mfm_portfolio::PortfolioSnapshotSelector::schema_id()
                .map_err(|_| registry_invalid())?,
            mfm_portfolio::PortfolioPublicOutputs::schema_id().map_err(|_| registry_invalid())?,
        )
        .map_err(|_| registry_invalid())?,
        EntryPointContract::new(
            EntryPointId::new(mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID)
                .map_err(|_| registry_invalid())?,
            submission_operation,
            planning_profile()?,
            mfm_evm::EvmSubmitTransactionSelector::schema_id().map_err(|_| registry_invalid())?,
            mfm_evm::EvmSubmissionOutput::schema_id().map_err(|_| registry_invalid())?,
        )
        .map_err(|_| registry_invalid())?,
    ];
    let broadcast_resource_ref = mfm_evm::EvmBroadcastResource::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(|_| registry_invalid())?;
    Ok(RegistryAssembly {
        registry,
        certifier,
        entry_points,
        broadcast_resource_ref,
        physical_binding_purposes,
    })
}

fn expansion_profile(max_fan_out_depth: u8) -> StructuredExpansionProfile {
    StructuredExpansionProfile {
        policies: Vec::new(),
        max_occurrences: 4_096,
        max_declarations: 8_192,
        max_lanes: 4_096,
        max_fan_out_depth,
        max_branch_depth: 64,
    }
}

fn balance_coverage_inputs(
    portfolio_inputs: &[mfm_evm::EvmBalanceLaneInput],
) -> Result<Vec<mfm_evm::EvmBalanceLaneInput>, PublicError> {
    let first = portfolio_inputs.first().ok_or_else(registry_invalid)?;
    let account = first
        .source()
        .account_address()
        .map_err(|_| registry_invalid())?;
    let native = mfm_evm::EvmBalanceLaneInput::new(
        0,
        "{}".to_owned(),
        first.binding().clone(),
        first.native_decimals(),
        mfm_evm::EvmBalanceSource::new(account, mfm_evm::EvmBalanceAsset::Native)
            .map_err(|_| registry_invalid())?,
    )
    .map_err(|_| registry_invalid())?;
    let token = mfm_evm::EvmBalanceLaneInput::new(
        0,
        "{}".to_owned(),
        first.binding().clone(),
        first.native_decimals(),
        mfm_evm::EvmBalanceSource::new(
            account,
            mfm_evm::EvmBalanceAsset::erc20(Address::repeat_byte(0x42))
                .map_err(|_| registry_invalid())?,
        )
        .map_err(|_| registry_invalid())?,
    )
    .map_err(|_| registry_invalid())?;
    Ok(vec![native, token])
}

#[async_trait]
impl ApplicationBackend for ProductionBackend {
    async fn check_ready(&self) -> Result<(), PublicError> {
        self.public_reader
            .check_ready()
            .await
            .map_err(classify_store_error)
    }

    async fn admit_run(
        &self,
        call: &AuthorizedAdmissionCall,
        entry_point: PublicEntryPointContract,
        request: AdmitRunRequest,
    ) -> Result<AdmitRunResponse, PublicError> {
        let operation_id = entry_point.entry_point_operation_id().clone();
        if call.entry_point_operation_id() != &operation_id
            || call.invocation_identity() != request.invocation_identity()
        {
            return Err(admission_invalid());
        }
        let prepared = if entry_point.entry_point_id().as_str()
            == mfm_portfolio::PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID
        {
            self.prepare_portfolio_admission(call, &operation_id, &request)
                .await?
        } else if entry_point.entry_point_id().as_str()
            == mfm_evm::EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID
        {
            self.prepare_submission_admission(call, &operation_id, &request)
                .await?
        } else {
            return Err(admission_invalid());
        };
        let (run_id, attempt) = self
            .runtime
            .admit_run(StructuredAdmissionCommand::new(
                call.tenant_scope_id().clone(),
                request.invocation_identity().clone(),
                operation_id.clone(),
                prepared.document,
                prepared.material,
                prepared.initial_values,
                admission_append_request_id(request.invocation_identity())?,
            ))
            .await
            .map_err(classify_runtime_error)?;
        let status = match attempt.outcome() {
            HistoryAppendOutcome::NewlyCommitted(_) => AdmissionStatus::NewlyAdmitted,
            HistoryAppendOutcome::ExistingSame(_) => AdmissionStatus::Attached,
            HistoryAppendOutcome::AcknowledgementUnknown => AdmissionStatus::OutcomeUnknown,
            HistoryAppendOutcome::StaleHead => return Err(admission_conflict()),
        };
        AdmitRunResponse::new(
            &run_id,
            status,
            entry_point.entry_point_id(),
            &operation_id,
            request.invocation_identity(),
            entry_point.planning_profile_ref(),
        )
    }

    async fn drive_once(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::Drive>,
    ) -> Result<DriveResponse, PublicError> {
        debug_assert_eq!(call.grant(), crate::RunAccessGrant::Drive);
        self.load_public_authorized(call.tenant_scope_id(), call.run_id())
            .await?;
        let outcome = self
            .runtime
            .drive_once(call.run_id())
            .await
            .map_err(classify_runtime_error)?;
        let evidence = self
            .load_public_authorized(call.tenant_scope_id(), call.run_id())
            .await?;
        DriveResponse::from_runtime(call.run_id(), outcome, evidence.journal_head())
    }

    async fn read_public_run(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::ReadPublic>,
    ) -> Result<PublicRunView, PublicError> {
        debug_assert_eq!(call.grant(), crate::RunAccessGrant::ReadPublic);
        let evidence = self
            .load_public_authorized(call.tenant_scope_id(), call.run_id())
            .await?;
        PublicRunView::from_structured(&evidence)
    }

    async fn replay_run(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::Replay>,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError> {
        debug_assert_eq!(call.grant(), crate::RunAccessGrant::Replay);
        match request {
            ReplayRequest::Verify => {
                let evidence = self
                    .load_replay_authorized(call.tenant_scope_id(), call.run_id())
                    .await?;
                mfm_replay::structured::project_replay_result(&evidence)
                    .map_err(|_| PublicError::replay_verification_failed())
            }
            ReplayRequest::Reproduce(input) => {
                let evidence = self
                    .load_export_authorized(call.tenant_scope_id(), call.run_id())
                    .await?;
                validate_replay_export(&evidence, input).await?;
                mfm_replay::structured::project_unavailable_reproduction(call.run_id())
                    .map_err(|_| PublicError::replay_verification_failed())
            }
            ReplayRequest::CompareCurrent(input) => {
                let evidence = self
                    .load_export_authorized(call.tenant_scope_id(), call.run_id())
                    .await?;
                validate_replay_export(&evidence, input).await?;
                mfm_replay::structured::project_unavailable_comparison(call.run_id())
                    .map_err(|_| PublicError::replay_verification_failed())
            }
        }
    }

    async fn read_transition_trace(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::InspectTrace>,
        page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError> {
        debug_assert_eq!(call.grant(), crate::RunAccessGrant::InspectTrace);
        let position = decode_transition_trace_page_request(call.run_id(), &page)?;
        let evidence = self
            .load_trace_authorized(call.tenant_scope_id(), call.run_id())
            .await?;
        let page = mfm_replay::structured::project_transition_trace(
            &evidence,
            position.complete_as_of_journal_head.as_ref(),
            position.start,
            position.limit,
        )
        .map_err(|_| page_invalid())?;
        complete_transition_trace_page(page)
    }

    async fn read_access_audit(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::InspectAudit>,
        page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError> {
        debug_assert_eq!(call.grant(), crate::RunAccessGrant::InspectAudit);
        let position = decode_access_audit_page_request(call.run_id(), &page)?;
        let evidence = self
            .load_audit_authorized(call.tenant_scope_id(), call.run_id())
            .await?;
        let page = mfm_replay::structured::project_access_audit(
            &evidence,
            position.complete_as_of_journal_head.as_ref(),
            position.start,
            position.limit,
        )
        .map_err(|_| page_invalid())?;
        complete_access_audit_page(page)
    }

    async fn export_run(
        &self,
        call: &AuthorizedRunCall<'_, run_grant::Export>,
        request: ExportRequest,
    ) -> Result<ExportedRun, PublicError> {
        debug_assert_eq!(call.grant(), crate::RunAccessGrant::Export);
        let evidence = self
            .load_export_authorized(call.tenant_scope_id(), call.run_id())
            .await?;
        write_structured_export(&evidence, request).await
    }
}

struct PreparedAdmission {
    document: mfm_spec::structured::CertifiedProgramDocument,
    material: StructuredAdmissionMaterial,
    initial_values: Vec<ProposedCanonicalValue>,
}

impl ProductionBackend {
    async fn prepare_portfolio_admission(
        &self,
        call: &AuthorizedAdmissionCall,
        operation_id: &StableId,
        request: &AdmitRunRequest,
    ) -> Result<PreparedAdmission, PublicError> {
        let selector: mfm_portfolio::PortfolioSnapshotSelector =
            serde_json::from_value(request.input().as_json().clone())
                .map_err(|_| admission_request_invalid())?;
        if selector.target().as_str() != call.configured_target().as_str() {
            return Err(admission_invalid());
        }
        let configured =
            self
                .resolve_configuration(
                    call.tenant_scope_id(),
                    operation_id,
                    call.configured_target().clone(),
                    mfm_spec::structured::structured_value_contract_ref::<
                        mfm_portfolio::PortfolioConfig,
                    >()
                    .map_err(|_| admission_invalid())?,
                )
                .await?;
        let portfolio: mfm_portfolio::PortfolioConfig =
            serde_json::from_str(configured.canonical_value())
                .map_err(|_| configured_value_invalid())?;
        MfmConfig::validate(&portfolio).map_err(|_| configured_value_invalid())?;
        let portfolio_input = mfm_portfolio::PortfolioSnapshotInput::new(
            selector.clone(),
            portfolio.clone(),
            self.routing_manifest.clone(),
        );
        let lane_inputs = mfm_portfolio::structured_portfolio_lane_inputs(
            selector,
            portfolio,
            self.routing_manifest.clone(),
        )
        .map_err(|_| configured_value_invalid())?;
        let authored = mfm_portfolio::structured_portfolio_snapshot_program(
            operation_id.clone(),
            stable(PORTFOLIO_SCOPE_ID)?,
            &lane_inputs,
        )
        .map_err(|_| configured_value_invalid())?;
        let certified = self
            .certifier
            .certify(operation_id, authored)
            .map_err(|_| configured_value_invalid())?;
        let initial_values =
            vec![ProposedCanonicalValue::from_value(&portfolio_input)
                .map_err(classify_store_error)?];
        Ok(PreparedAdmission {
            document: certified.into_document(),
            material: self.admission_material(&configured, Vec::new())?,
            initial_values,
        })
    }

    async fn prepare_submission_admission(
        &self,
        call: &AuthorizedAdmissionCall,
        operation_id: &StableId,
        request: &AdmitRunRequest,
    ) -> Result<PreparedAdmission, PublicError> {
        let selector: mfm_evm::EvmSubmitTransactionSelector =
            serde_json::from_value(request.input().as_json().clone())
                .map_err(|_| admission_request_invalid())?;
        if selector.target().as_str() != call.configured_target().as_str() {
            return Err(admission_invalid());
        }
        let configured = self
            .resolve_configuration(
                call.tenant_scope_id(),
                operation_id,
                call.configured_target().clone(),
                mfm_spec::structured::structured_value_contract_ref::<
                    mfm_evm::EvmSubmissionConfiguration,
                >()
                .map_err(|_| admission_invalid())?,
            )
            .await?;
        let submission_configuration: mfm_evm::EvmSubmissionConfiguration =
            serde_json::from_str(configured.canonical_value())
                .map_err(|_| configured_value_invalid())?;
        submission_configuration
            .validate()
            .map_err(|_| configured_value_invalid())?;
        // Recompute sealed deployment semantics against the qualified live
        // signer for every configuration revision. Untrusted configuration
        // cannot choose or echo a qualified contract reference.
        let revision_semantics = submission_configuration
            .deployment_semantics(
                self.sealed_submission_semantics.semantic_signer_id(),
                &self.sealed_public_signing_identity,
            )
            .map_err(|_| configured_value_invalid())?;
        if revision_semantics != self.sealed_submission_semantics {
            return Err(configured_value_invalid());
        }
        let route = revision_semantics
            .route_generation_ref()
            .to_content_ref()
            .ok()
            .and_then(|reference| {
                mfm_evm::EvmRoutingGenerationRef::from_content_ref(reference).ok()
            })
            .and_then(|reference| self.routing_catalog.resolve_generation(&reference));
        if submission_configuration
            .transaction_intent()
            .template()
            .target()
            != selector.target()
            || revision_semantics.domain_activation_attestation()
                != &self.wallet_domain_activation_attestation
            || revision_semantics.route_generation_ref() != &self.submission_route_generation_ref
            || revision_semantics.expected_sender()
                != self.sealed_submission_semantics.expected_sender()
            || route.is_none_or(|generation| {
                generation.chain_instance() != &self.submission_chain_instance
            })
        {
            return Err(configured_value_invalid());
        }
        let submission = mfm_evm::EvmSubmissionRequest::from_authorized(
            submission_configuration,
            call.tenant_scope_id().clone(),
            call.authenticated_principal_id().clone(),
            selector.caller_submission_token().clone(),
        )
        .map_err(|_| admission_invalid())?;
        let authored = mfm_evm::structured_evm_submission_entry_program(
            operation_id.clone(),
            stable(SUBMISSION_SCOPE_ID)?,
        )
        .map_err(|_| admission_invalid())?;
        let certified = self
            .certifier
            .certify(operation_id, authored)
            .map_err(|_| admission_invalid())?;
        Ok(PreparedAdmission {
            document: certified.into_document(),
            material: self.admission_material(
                &configured,
                vec![
                    self.broadcast_resource_ref.clone(),
                    self.wallet_resource_ref.clone(),
                ],
            )?,
            initial_values: vec![
                ProposedCanonicalValue::from_value(&submission).map_err(classify_store_error)?
            ],
        })
    }

    async fn resolve_configuration(
        &self,
        tenant: &TenantScopeId,
        operation_id: &StableId,
        target: StableId,
        contract_ref: ContentRef,
    ) -> Result<VerifiedConfiguredValue, PublicError> {
        let key = ConfigurationStreamKey::new(
            self.public_reader.store_identity().store_scope_id.clone(),
            tenant.clone(),
            operation_id.clone(),
            target,
        );
        self.configuration
            .resolve(&key, &contract_ref)
            .await
            .map_err(|error| match error {
                StructuredStoreError::RunNotFound => configured_value_not_found(),
                other => classify_store_error(other),
            })
    }

    fn admission_material(
        &self,
        configured: &VerifiedConfiguredValue,
        stable_resource_refs: Vec<ContentRef>,
    ) -> Result<StructuredAdmissionMaterial, PublicError> {
        let configuration = HistoryObject::new(
            stable(ADMISSION_CONFIGURATION_OBJECT_TYPE)?,
            fixed_schema_id(CONFIGURATION_REVISION_SCHEMA)?,
            canonical_json(configured.revision())
                .map_err(|_| configured_value_invalid())?
                .as_str(),
        )
        .map_err(|_| configured_value_invalid())?;
        StructuredAdmissionMaterial::new(
            configuration,
            self.context_manifest.clone(),
            self.prior_run_source_manifest.clone(),
            self.routing_policy.clone(),
            stable_resource_refs,
        )
        .map_err(classify_store_error)
    }

    async fn load_public_authorized(
        &self,
        tenant_scope_id: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<PublicRunEvidence, PublicError> {
        let evidence = self
            .public_reader
            .load_public(run_id)
            .await
            .map_err(classify_store_error)?;
        if &evidence.admission().tenant_scope_id != tenant_scope_id {
            return Err(PublicError::run_not_found());
        }
        Ok(evidence)
    }

    async fn load_trace_authorized(
        &self,
        tenant_scope_id: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<mfm_store::structured::TraceRunEvidence, PublicError> {
        let evidence = self
            .trace_reader
            .load_transition_trace(run_id)
            .await
            .map_err(classify_store_error)?;
        if &evidence.admission().tenant_scope_id != tenant_scope_id {
            return Err(PublicError::run_not_found());
        }
        Ok(evidence)
    }

    async fn load_audit_authorized(
        &self,
        tenant_scope_id: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<mfm_store::structured::AuditRunEvidence, PublicError> {
        let evidence = self
            .audit_reader
            .load_access_audit(run_id)
            .await
            .map_err(classify_store_error)?;
        if &evidence.admission().tenant_scope_id != tenant_scope_id {
            return Err(PublicError::run_not_found());
        }
        Ok(evidence)
    }

    async fn load_replay_authorized(
        &self,
        tenant_scope_id: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<mfm_store::structured::RecordedRunEvidence, PublicError> {
        let evidence = self
            .replay_reader
            .load_for_recorded_verify(run_id)
            .await
            .map_err(classify_store_error)?;
        if &evidence.admission().tenant_scope_id != tenant_scope_id {
            return Err(PublicError::run_not_found());
        }
        Ok(evidence)
    }

    async fn load_export_authorized(
        &self,
        tenant_scope_id: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<ExportRunEvidence, PublicError> {
        let evidence = self
            .export_reader
            .load_for_export(run_id)
            .await
            .map_err(classify_store_error)?;
        if &evidence.admission().tenant_scope_id != tenant_scope_id {
            return Err(PublicError::run_not_found());
        }
        Ok(evidence)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PhysicalBindingPurposeKey {
    access_kind: AccessKind,
    capability_contract_ref: ContentRef,
    adapter_contract_ref: ContentRef,
    adapter_implementation_ref: ContentRef,
}

struct ExactPhysicalBindingVerifier {
    purposes: Vec<mfm_evm_live::EvmPhysicalBindingPurpose>,
    broadcast_resource_ref: ContentRef,
    wallet_resource_ref: ContentRef,
}

impl ExactPhysicalBindingVerifier {
    fn new(
        purposes: Vec<mfm_evm_live::EvmPhysicalBindingPurpose>,
        broadcast_resource_ref: ContentRef,
        wallet_resource_ref: ContentRef,
    ) -> Result<Self, PublicError> {
        let mut exact_purpose_keys = BTreeSet::new();
        for purpose in &purposes {
            let key = PhysicalBindingPurposeKey {
                access_kind: purpose.access_kind(),
                capability_contract_ref: purpose.capability_contract_ref().clone(),
                adapter_contract_ref: purpose.adapter_contract_ref().clone(),
                adapter_implementation_ref: purpose.adapter_implementation_ref().clone(),
            };
            if !exact_purpose_keys.insert(key) {
                return Err(registry_invalid());
            }
        }
        if purposes.is_empty() {
            return Err(registry_invalid());
        }
        Ok(Self {
            purposes,
            broadcast_resource_ref,
            wallet_resource_ref,
        })
    }

    fn purpose(
        &self,
        access_kind: AccessKind,
        capability_contract_ref: &ContentRef,
        adapter_contract_ref: &ContentRef,
        adapter_implementation_ref: &ContentRef,
    ) -> Result<&mfm_evm_live::EvmPhysicalBindingPurpose, StructuredStoreError> {
        self.purposes
            .iter()
            .find(|purpose| {
                purpose.access_kind() == access_kind
                    && purpose.capability_contract_ref() == capability_contract_ref
                    && purpose.adapter_contract_ref() == adapter_contract_ref
                    && purpose.adapter_implementation_ref() == adapter_implementation_ref
            })
            .ok_or(StructuredStoreError::Certification)
    }
}

impl PublicPhysicalBindingVerifier for ExactPhysicalBindingVerifier {
    fn verify_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        let purpose = self.purpose(
            context.access_kind,
            context.capability_contract_ref,
            context.adapter_contract_ref,
            context.adapter_implementation_ref,
        )?;
        let history = purpose.release_history();
        let release = history
            .release(&certificate.content_ref)
            .ok_or(StructuredStoreError::Certification)?;
        if release.certificate() != certificate
            || certificate.validate().is_err()
            || context.admitted_routing_policy_ref != release.admitted_routing_policy_ref()
            || context.stable_resource_lineage_contract_ref
                != purpose.stable_resource_lineage_contract_ref()
            || matches!(
                context.verification_mode,
                PhysicalBindingVerificationMode::CurrentCandidate
            ) && history.current().certificate() != certificate
        {
            return Err(StructuredStoreError::Certification);
        }
        match (
            context.previous_physical_binding_ref,
            context.minimum_lineage_head_ref,
        ) {
            (None, None) => Ok(()),
            (Some(previous), Some(minimum_head))
                if context.access_kind == AccessKind::Effect
                    && history.is_strict_descendant(&certificate.content_ref, previous)
                    && release.activation_lineage_head_ref() == Some(minimum_head) =>
            {
                Ok(())
            }
            _ => Err(StructuredStoreError::Certification),
        }
    }

    fn verify_supersession(
        &self,
        context: &PhysicalBindingSupersession<'_>,
        public_lineage_head: &HistoryObject,
        evidence: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        let purpose = self.purpose(
            AccessKind::Effect,
            context.capability_contract_ref,
            context.adapter_contract_ref,
            context.adapter_implementation_ref,
        )?;
        let history = purpose.release_history();
        let authorized_release = history
            .release(context.authorized_binding_ref)
            .ok_or(StructuredStoreError::Certification)?;
        let successor = history
            .releases()
            .find(|release| {
                history.is_strict_descendant(
                    &release.certificate().content_ref,
                    &authorized_release.certificate().content_ref,
                ) && release.activation_lineage_head_ref() == Some(context.public_lineage_head_ref)
            })
            .ok_or(StructuredStoreError::Certification)?;
        if purpose.stable_resource_lineage_contract_ref()
            != Some(context.stable_resource_lineage_contract_ref)
            || context.public_lineage_head_ref != &public_lineage_head.content_ref
            || public_lineage_head.validate().is_err()
            || evidence.validate().is_err()
            || matches!(
                context.verification_mode,
                PhysicalBindingVerificationMode::CurrentCandidate
            ) && successor.certificate() != history.current().certificate()
        {
            return Err(StructuredStoreError::Certification);
        }
        if context.stable_resource_lineage_contract_ref == &self.broadcast_resource_ref {
            let claim: mfm_evm::BroadcastLineageHead = evidence
                .decode()
                .map_err(|_| StructuredStoreError::Certification)?;
            if claim.validate().is_ok()
                && claim.public_lineage_head_ref.to_content_ref().ok().as_ref()
                    == Some(&public_lineage_head.content_ref)
            {
                return Ok(());
            }
        } else if context.stable_resource_lineage_contract_ref == &self.wallet_resource_ref {
            let claim: mfm_evm::WalletNonceStoreLineageHead = evidence
                .decode()
                .map_err(|_| StructuredStoreError::Certification)?;
            if claim.validate().is_ok()
                && claim.public_lineage_head_ref.to_content_ref().ok().as_ref()
                    == Some(&public_lineage_head.content_ref)
            {
                return Ok(());
            }
        }
        Err(StructuredStoreError::Certification)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum StructuredExportKind {
    Semantic,
    Audit,
}

impl From<mfm_replay::portable::ExportKind> for StructuredExportKind {
    fn from(value: mfm_replay::portable::ExportKind) -> Self {
        match value {
            mfm_replay::portable::ExportKind::Semantic => Self::Semantic,
            mfm_replay::portable::ExportKind::Audit => Self::Audit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredPortableExport {
    version: String,
    kind: StructuredExportKind,
    store_scope_id: mfm_ids::StoreScopeId,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    journal_head: mfm_journal::structured::JournalHead,
    semantic_head: mfm_journal::structured::SemanticHead,
    records: Vec<mfm_journal::structured::AssignedRecord>,
    objects: Vec<HistoryObject>,
}

async fn write_structured_export(
    evidence: &ExportRunEvidence,
    request: ExportRequest,
) -> Result<ExportedRun, PublicError> {
    let export = structured_export(evidence, request.kind())?;
    let bytes = structured_export_bytes(&export)?;
    let contract = RecoverabilityContract::embedded().map_err(|_| export_stream_io_error())?;
    let content_ref = ContentRef::new(
        contract
            .schema_id("mfm.portable-run-export-stream.v1")
            .map_err(|_| export_stream_io_error())?
            .clone(),
        contract.raw_content_digest(&bytes),
    )
    .map_err(|_| export_stream_io_error())?;
    let mut spool = WritableSpool::create()
        .await
        .map_err(|_| export_stream_io_error())?;
    spool
        .write_all(&bytes)
        .await
        .map_err(|_| export_stream_io_error())?;
    let spool = spool.finish().await.map_err(|_| export_stream_io_error())?;
    Ok(ExportedRun::from_content_ref(content_ref, Box::pin(spool)))
}

fn structured_export(
    evidence: &ExportRunEvidence,
    kind: mfm_replay::portable::ExportKind,
) -> Result<StructuredPortableExport, PublicError> {
    let semantic_cutoff = match evidence.semantic_head() {
        mfm_journal::structured::SemanticHead::Genesis { admission_ref, .. } => admission_ref,
        mfm_journal::structured::SemanticHead::Transition { transition_ref, .. } => transition_ref,
    };
    let run_sequence = match kind {
        mfm_replay::portable::ExportKind::Semantic => semantic_cutoff.run_sequence,
        mfm_replay::portable::ExportKind::Audit => evidence.journal_head().run_sequence,
    };
    let index = usize::try_from(run_sequence)
        .ok()
        .and_then(|sequence| sequence.checked_sub(1))
        .ok_or_else(export_stream_io_error)?;
    let journal_head = evidence
        .journal_heads()
        .get(index)
        .cloned()
        .ok_or_else(export_stream_io_error)?;
    let records = match kind {
        mfm_replay::portable::ExportKind::Semantic => {
            evidence.semantic_records().cloned().collect::<Vec<_>>()
        }
        mfm_replay::portable::ExportKind::Audit => evidence.records().to_vec(),
    };
    if records.is_empty() {
        return Err(export_stream_io_error());
    }
    Ok(StructuredPortableExport {
        version: STRUCTURED_EXPORT_VERSION.to_owned(),
        kind: kind.into(),
        store_scope_id: evidence.admission().store_scope_id.clone(),
        tenant_scope_id: evidence.admission().tenant_scope_id.clone(),
        run_id: evidence.run_id().clone(),
        journal_head,
        semantic_head: evidence.semantic_head().clone(),
        records,
        objects: evidence.objects_through(run_sequence).cloned().collect(),
    })
}

fn structured_export_bytes(export: &StructuredPortableExport) -> Result<Vec<u8>, PublicError> {
    canonical_json(export)
        .map(|canonical| canonical.as_bytes().to_vec())
        .map_err(|_| export_stream_io_error())
}

async fn validate_replay_export(
    evidence: &ExportRunEvidence,
    input: crate::ExportStreamInput,
) -> Result<(), PublicError> {
    let (content_ref, spool) = snapshot_input(input)
        .await
        .map_err(|_| export_stream_io_error())?;
    let mut bytes = Vec::new();
    let mut bounded = spool.take(MAX_REPLAY_EXPORT_BYTES + 1);
    bounded
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| export_stream_io_error())?;
    if bytes.len() as u64 > MAX_REPLAY_EXPORT_BYTES {
        return Err(PublicError::replay_artifact_too_large());
    }
    let contract =
        RecoverabilityContract::embedded().map_err(|_| PublicError::replay_artifact_invalid())?;
    if contract.raw_content_digest(&bytes) != *content_ref.content_digest() {
        return Err(PublicError::replay_artifact_invalid());
    }
    let validated = contract
        .strict_decode("mfm.portable-run-export-stream.v1", &bytes)
        .map_err(|_| PublicError::replay_artifact_invalid())?;
    let supplied: StructuredPortableExport = serde_json::from_slice(validated.as_bytes())
        .map_err(|_| PublicError::replay_artifact_invalid())?;
    let expected = structured_export(evidence, mfm_replay::portable::ExportKind::Semantic)
        .map_err(|_| PublicError::replay_artifact_invalid())?;
    let expected_bytes =
        structured_export_bytes(&expected).map_err(|_| PublicError::replay_artifact_invalid())?;
    let expected_ref = ContentRef::new(
        content_ref.schema_id().clone(),
        contract.raw_content_digest(&expected_bytes),
    )
    .map_err(|_| PublicError::replay_artifact_invalid())?;
    if supplied != expected || bytes != expected_bytes || content_ref != expected_ref {
        return Err(PublicError::replay_artifact_invalid());
    }
    Ok(())
}

fn admission_append_request_id(
    invocation_identity: &InvocationIdentity,
) -> Result<AppendRequestId, PublicError> {
    AppendRequestId::new(format!("admission/{}", invocation_identity.as_str()))
        .map_err(|_| admission_invalid())
}

fn fixed_schema_id(name: &str) -> Result<SchemaId, PublicError> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(format!("mfm.structured-schema.v1:{name}:1").as_bytes()),
    )
    .map_err(|_| registry_invalid())
}

fn stable(value: impl AsRef<str>) -> Result<StableId, PublicError> {
    StableId::new(value).map_err(|_| registry_invalid())
}

fn classify_runtime_error(error: RuntimeError) -> PublicError {
    let public = classify_runtime_fault_code(error.code(), error.store_fault_kind());
    public.with_runtime_fault(public_runtime_fault_attribution(&error))
}

fn classify_runtime_fault_code(
    code: RuntimeFaultCode,
    store_fault_kind: Option<RuntimeStoreFaultKind>,
) -> PublicError {
    match code {
        RuntimeFaultCode::PhysicalBindingUnavailable => PublicError::backend(
            ErrorClass::ServiceUnavailable,
            "PhysicalBindingUnavailable",
            "A qualified physical binding is unavailable",
        ),
        RuntimeFaultCode::CallbackFault => PublicError::internal(
            "StructuredRuntimeCallbackFault",
            "A qualified Runtime callback failed",
        ),
        RuntimeFaultCode::CodecFault => PublicError::internal(
            "StructuredRuntimeCodecFault",
            "A qualified Runtime value failed exact encoding",
        ),
        RuntimeFaultCode::ContractFault => PublicError::internal(
            "StructuredRuntimeContractFault",
            "A qualified Runtime component contract failed verification",
        ),
        RuntimeFaultCode::InvalidEvidence => PublicError::internal(
            "StructuredRuntimeInvalidEvidence",
            "Committed Runtime evidence failed settlement verification",
        ),
        RuntimeFaultCode::CandidateRejected => match store_fault_kind {
            Some(RuntimeStoreFaultKind::StaleHead | RuntimeStoreFaultKind::AppendConflict) => {
                admission_conflict()
            }
            Some(RuntimeStoreFaultKind::BackendUnavailable)
            | Some(RuntimeStoreFaultKind::AcknowledgementUnknown) => run_store_unavailable(),
            _ => PublicError::internal(
                "StructuredRuntimeCandidateRejected",
                "A structured Runtime history candidate was rejected",
            ),
        },
        RuntimeFaultCode::StoreInvalid => match store_fault_kind {
            Some(RuntimeStoreFaultKind::RunNotFound) => PublicError::run_not_found(),
            _ => PublicError::replay_verification_failed(),
        },
        RuntimeFaultCode::StoreUnavailable => run_store_unavailable(),
    }
}

fn public_runtime_fault_attribution(error: &RuntimeError) -> PublicRuntimeFaultAttribution {
    let phase = match error.phase() {
        RuntimeFaultPhase::InvokePure => PublicRuntimeFaultPhase::InvokePure,
        RuntimeFaultPhase::AuthorRequest => PublicRuntimeFaultPhase::AuthorRequest,
        RuntimeFaultPhase::QualifyAccess => PublicRuntimeFaultPhase::QualifyAccess,
        RuntimeFaultPhase::SettleObservation => PublicRuntimeFaultPhase::SettleObservation,
        RuntimeFaultPhase::QualifyCandidate => PublicRuntimeFaultPhase::QualifyCandidate,
        RuntimeFaultPhase::AppendCandidate => PublicRuntimeFaultPhase::AppendCandidate,
        RuntimeFaultPhase::LoadHistory => PublicRuntimeFaultPhase::LoadHistory,
        RuntimeFaultPhase::ResolveAppend => PublicRuntimeFaultPhase::ResolveAppend,
    };
    let subject = match error.subject() {
        RuntimeFaultSubject::Process(component) => PublicRuntimeFaultSubject::Process {
            component_kind: component.component_kind(),
            semantic_contract_ref: component.semantic_contract_ref().clone(),
        },
        RuntimeFaultSubject::Store(store) => PublicRuntimeFaultSubject::Store {
            store_scope_id: store.store_scope_id.clone(),
            store_epoch: store.store_epoch,
        },
    };
    PublicRuntimeFaultAttribution {
        phase,
        run_id: error.run_id().clone(),
        pre_fault_head: error.pre_fault_head().cloned(),
        occurrence_id: error.occurrence_id().cloned(),
        subject,
    }
}

fn classify_store_error(error: StructuredStoreError) -> PublicError {
    match error {
        StructuredStoreError::RunNotFound => PublicError::run_not_found(),
        StructuredStoreError::StaleHead | StructuredStoreError::AppendConflict => {
            admission_conflict()
        }
        StructuredStoreError::BackendUnavailable | StructuredStoreError::AcknowledgementUnknown => {
            run_store_unavailable()
        }
        StructuredStoreError::InvalidHistory
        | StructuredStoreError::CandidateRejected
        | StructuredStoreError::Certification => PublicError::replay_verification_failed(),
    }
}

fn admission_request_invalid() -> PublicError {
    PublicError::bad_request(
        "AdmissionRequestInvalid",
        "Admission request does not match the published entry-point input",
    )
}

fn admission_invalid() -> PublicError {
    PublicError::internal(
        "AdmissionPlanningInvalid",
        "The qualified admission could not be prepared",
    )
}

fn configured_value_not_found() -> PublicError {
    PublicError::not_found(
        "ConfiguredValueNotFound",
        "The selected configured value was not found",
    )
}

fn configured_value_invalid() -> PublicError {
    PublicError::backend(
        ErrorClass::Internal,
        "ConfiguredValueInvalid",
        "The selected configured value is invalid",
    )
}

fn admission_conflict() -> PublicError {
    PublicError::backend(
        ErrorClass::Conflict,
        "AdmissionConflict",
        "The logical run conflicts with committed admission state",
    )
}

fn registry_invalid() -> PublicError {
    PublicError::internal(
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

fn page_invalid() -> PublicError {
    PublicError::bad_request(
        "PageRequestInvalid",
        "The page request or cursor is invalid",
    )
}

fn export_stream_io_error() -> PublicError {
    PublicError::internal(
        "ExportStreamIoFailed",
        "The export stream could not be processed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_and_certificate_refresh_survives_fresh_verifier_construction() {
        let policy_ref = test_object("routing-policy", 1).content_ref;
        let capability_ref = test_object("capability", 2).content_ref;
        let adapter_ref = test_object("adapter", 3).content_ref;
        let implementation_ref = test_object("adapter-implementation", 4).content_ref;
        let old_target_ref = test_object("old-target", 5).content_ref;
        let new_target_ref = test_object("new-target", 6).content_ref;
        let old_certificate = test_object("old-binding", 7);
        let new_certificate = test_object("new-binding", 8);
        let public_head = test_object("broadcast-public-head", 9);
        let broadcast_resource_ref = mfm_evm::EvmBroadcastResource::contract()
            .and_then(|contract| contract.content_ref().map_err(Into::into))
            .expect("broadcast resource reference");
        let wallet_resource_ref = test_object("wallet-resource", 10).content_ref;

        let old_history = mfm_evm_live::EvmPhysicalBindingReleaseHistory::single(
            policy_ref.clone(),
            old_target_ref.clone(),
            old_certificate.clone(),
        )
        .expect("old release history");
        let old_purpose = test_effect_purpose(
            &capability_ref,
            &adapter_ref,
            &implementation_ref,
            &broadcast_resource_ref,
            old_history,
        );
        let old_process = ExactPhysicalBindingVerifier::new(
            vec![old_purpose],
            broadcast_resource_ref.clone(),
            wallet_resource_ref.clone(),
        )
        .expect("old process verifier");
        let old_current = PhysicalBindingAuthorization {
            verification_mode: PhysicalBindingVerificationMode::CurrentCandidate,
            access_kind: AccessKind::Effect,
            capability_contract_ref: &capability_ref,
            capability_implementation_ref: &implementation_ref,
            adapter_contract_ref: &adapter_ref,
            adapter_implementation_ref: &implementation_ref,
            admitted_routing_policy_ref: &policy_ref,
            stable_resource_lineage_contract_ref: Some(&broadcast_resource_ref),
            minimum_lineage_head_ref: None,
            previous_physical_binding_ref: None,
        };
        old_process
            .verify_authorization(&old_current, &old_certificate)
            .expect("old process admits its current root release");

        let refreshed_history = mfm_evm_live::EvmPhysicalBindingReleaseHistory::new(vec![
            mfm_evm_live::EvmPhysicalBindingRelease::new(
                policy_ref.clone(),
                old_target_ref,
                old_certificate.clone(),
                None,
                None,
            )
            .expect("old retained release"),
            mfm_evm_live::EvmPhysicalBindingRelease::new(
                policy_ref.clone(),
                new_target_ref,
                new_certificate.clone(),
                Some(old_certificate.content_ref.clone()),
                Some(public_head.content_ref.clone()),
            )
            .expect("new current release"),
        ])
        .expect("refreshed release history");
        let fresh_process = ExactPhysicalBindingVerifier::new(
            vec![test_effect_purpose(
                &capability_ref,
                &adapter_ref,
                &implementation_ref,
                &broadcast_resource_ref,
                refreshed_history,
            )],
            broadcast_resource_ref.clone(),
            wallet_resource_ref,
        )
        .expect("fresh process verifier");

        let retained_old = PhysicalBindingAuthorization {
            verification_mode: PhysicalBindingVerificationMode::RetainedHistory,
            ..old_current
        };
        fresh_process
            .verify_authorization(&retained_old, &old_certificate)
            .expect("fresh process replays the retained old release");
        assert_eq!(
            fresh_process.verify_authorization(&old_current, &old_certificate),
            Err(StructuredStoreError::Certification)
        );

        let refreshed_current = PhysicalBindingAuthorization {
            verification_mode: PhysicalBindingVerificationMode::CurrentCandidate,
            access_kind: AccessKind::Effect,
            capability_contract_ref: &capability_ref,
            capability_implementation_ref: &implementation_ref,
            adapter_contract_ref: &adapter_ref,
            adapter_implementation_ref: &implementation_ref,
            admitted_routing_policy_ref: &policy_ref,
            stable_resource_lineage_contract_ref: Some(&broadcast_resource_ref),
            minimum_lineage_head_ref: Some(&public_head.content_ref),
            previous_physical_binding_ref: Some(&old_certificate.content_ref),
        };
        fresh_process
            .verify_authorization(&refreshed_current, &new_certificate)
            .expect("fresh process admits the exact current descendant");
        let mut forged_certificate = new_certificate.clone();
        forged_certificate.canonical_json = old_certificate.canonical_json.clone();
        assert_eq!(
            fresh_process.verify_authorization(&refreshed_current, &forged_certificate),
            Err(StructuredStoreError::Certification)
        );
        let non_descendant = PhysicalBindingAuthorization {
            previous_physical_binding_ref: Some(&new_certificate.content_ref),
            ..refreshed_current
        };
        assert_eq!(
            fresh_process.verify_authorization(&non_descendant, &new_certificate),
            Err(StructuredStoreError::Certification)
        );

        let claim = mfm_evm::BroadcastLineageHead {
            lineage_id: "mfm.evm.test/broadcast-lineage".to_owned(),
            generation: 2,
            public_lineage_head_ref: mfm_evm::EvmWalletReference::from_content_ref(
                public_head.content_ref.clone(),
            ),
        };
        let evidence = test_encoded_object("broadcast-evidence", 11, &claim);
        let supersession = PhysicalBindingSupersession {
            verification_mode: PhysicalBindingVerificationMode::CurrentCandidate,
            capability_contract_ref: &capability_ref,
            adapter_contract_ref: &adapter_ref,
            adapter_implementation_ref: &implementation_ref,
            authorized_binding_ref: &old_certificate.content_ref,
            stable_resource_lineage_contract_ref: &broadcast_resource_ref,
            public_lineage_head_ref: &public_head.content_ref,
        };
        fresh_process
            .verify_supersession(&supersession, &public_head, &evidence)
            .expect("supersession proves the retained old-to-current relation");
        let same_release = PhysicalBindingSupersession {
            authorized_binding_ref: &new_certificate.content_ref,
            ..supersession
        };
        assert_eq!(
            fresh_process.verify_supersession(&same_release, &public_head, &evidence),
            Err(StructuredStoreError::Certification)
        );
    }

    fn test_effect_purpose(
        capability_contract_ref: &ContentRef,
        adapter_contract_ref: &ContentRef,
        adapter_implementation_ref: &ContentRef,
        resource_contract_ref: &ContentRef,
        history: mfm_evm_live::EvmPhysicalBindingReleaseHistory,
    ) -> mfm_evm_live::EvmPhysicalBindingPurpose {
        mfm_evm_live::EvmPhysicalBindingPurpose::new(
            AccessKind::Effect,
            capability_contract_ref.clone(),
            adapter_contract_ref.clone(),
            adapter_implementation_ref.clone(),
            Some(resource_contract_ref.clone()),
            history,
        )
        .expect("test physical purpose")
    }

    fn test_object(name: &str, discriminator: u8) -> HistoryObject {
        HistoryObject::new(
            StableId::new(format!("mfm.app.test/{name}")).expect("test object type"),
            SchemaId::new(
                "mfm.app.test-object",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(b"mfm.app.test-object.v1"),
            )
            .expect("test object schema"),
            format!("{{\"discriminator\":{discriminator}}}"),
        )
        .expect("test history object")
    }

    fn test_encoded_object<T: Serialize>(
        name: &str,
        discriminator: u8,
        value: &T,
    ) -> HistoryObject {
        let schema_name = format!("mfm.app.test-encoded-{discriminator}");
        let canonical = canonical_json(value).expect("test canonical evidence");
        HistoryObject::new(
            StableId::new(format!("mfm.app.test/{name}")).expect("test object type"),
            SchemaId::new(
                &schema_name,
                "1",
                DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(
                    format!("mfm.app.test-encoded-{discriminator}.v1").as_bytes(),
                ),
            )
            .expect("test encoded schema"),
            canonical.as_str(),
        )
        .expect("test encoded history object")
    }

    #[test]
    fn every_runtime_fault_code_and_store_disposition_has_a_frozen_public_mapping() {
        for (code, class, public_code, message) in [
            (
                RuntimeFaultCode::PhysicalBindingUnavailable,
                ErrorClass::ServiceUnavailable,
                "PhysicalBindingUnavailable",
                "A qualified physical binding is unavailable",
            ),
            (
                RuntimeFaultCode::CallbackFault,
                ErrorClass::Internal,
                "StructuredRuntimeCallbackFault",
                "A qualified Runtime callback failed",
            ),
            (
                RuntimeFaultCode::CodecFault,
                ErrorClass::Internal,
                "StructuredRuntimeCodecFault",
                "A qualified Runtime value failed exact encoding",
            ),
            (
                RuntimeFaultCode::ContractFault,
                ErrorClass::Internal,
                "StructuredRuntimeContractFault",
                "A qualified Runtime component contract failed verification",
            ),
            (
                RuntimeFaultCode::InvalidEvidence,
                ErrorClass::Internal,
                "StructuredRuntimeInvalidEvidence",
                "Committed Runtime evidence failed settlement verification",
            ),
            (
                RuntimeFaultCode::CandidateRejected,
                ErrorClass::Internal,
                "StructuredRuntimeCandidateRejected",
                "A structured Runtime history candidate was rejected",
            ),
            (
                RuntimeFaultCode::StoreInvalid,
                ErrorClass::Internal,
                "ReplayVerificationFailed",
                "Recorded run evidence failed verification",
            ),
            (
                RuntimeFaultCode::StoreUnavailable,
                ErrorClass::ServiceUnavailable,
                "RunStoreUnavailable",
                "The authoritative run store is unavailable",
            ),
        ] {
            assert_public_mapping(code, None, class, public_code, message);
        }

        for disposition in [
            RuntimeStoreFaultKind::StaleHead,
            RuntimeStoreFaultKind::AppendConflict,
        ] {
            assert_public_mapping(
                RuntimeFaultCode::CandidateRejected,
                Some(disposition),
                ErrorClass::Conflict,
                "AdmissionConflict",
                "The logical run conflicts with committed admission state",
            );
        }
        for disposition in [
            RuntimeStoreFaultKind::BackendUnavailable,
            RuntimeStoreFaultKind::AcknowledgementUnknown,
        ] {
            assert_public_mapping(
                RuntimeFaultCode::CandidateRejected,
                Some(disposition),
                ErrorClass::ServiceUnavailable,
                "RunStoreUnavailable",
                "The authoritative run store is unavailable",
            );
        }
        for disposition in [
            RuntimeStoreFaultKind::RunNotFound,
            RuntimeStoreFaultKind::InvalidHistory,
            RuntimeStoreFaultKind::Certification,
        ] {
            assert_public_mapping(
                RuntimeFaultCode::CandidateRejected,
                Some(disposition),
                ErrorClass::Internal,
                "StructuredRuntimeCandidateRejected",
                "A structured Runtime history candidate was rejected",
            );
        }

        assert_public_mapping(
            RuntimeFaultCode::StoreInvalid,
            Some(RuntimeStoreFaultKind::RunNotFound),
            ErrorClass::NotFound,
            "RunNotFound",
            "The requested run was not found",
        );
        for disposition in [
            RuntimeStoreFaultKind::InvalidHistory,
            RuntimeStoreFaultKind::Certification,
            RuntimeStoreFaultKind::StaleHead,
            RuntimeStoreFaultKind::AppendConflict,
            RuntimeStoreFaultKind::BackendUnavailable,
            RuntimeStoreFaultKind::AcknowledgementUnknown,
        ] {
            assert_public_mapping(
                RuntimeFaultCode::StoreInvalid,
                Some(disposition),
                ErrorClass::Internal,
                "ReplayVerificationFailed",
                "Recorded run evidence failed verification",
            );
        }
    }

    fn assert_public_mapping(
        code: RuntimeFaultCode,
        disposition: Option<RuntimeStoreFaultKind>,
        class: ErrorClass,
        public_code: &str,
        message: &str,
    ) {
        let error = classify_runtime_fault_code(code, disposition);
        assert_eq!(error.class(), class);
        assert_eq!(error.code(), public_code);
        assert_eq!(error.message(), message);
    }
}
