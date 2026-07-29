use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_canonical::{
    sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContractV1,
    ValidatedCanonicalValueV1,
};
use mfm_executor::{
    ExecutorBinding, ExecutorContractDescriptor, ExecutorDeployment, ResourceOwnership,
    VerifiedExecutorBinding,
};
use mfm_ids::{ContentRef, DigestAlgorithm, FieldPath, SchemaId, SemanticTypeId, StableId};
use mfm_journal::v1::ReadCapabilityBinding;
use mfm_spec::{
    exact_content_ref, CapabilityBindingManifest, CapabilityBindingManifestEntry,
    ComponentImplementationDescriptor, ComponentKind, RetainedValueContract,
    StateImplementationManifest, StateImplementationManifestEntry,
};
use mfm_store::{QualifiedSupportGraph, QualifiedSupportMember};
use serde::Serialize;

use crate::PublicError;

const OBJECT_EVIDENCE_SEMANTIC_NAME: &str = "component-object-evidence-contract";
const OBJECT_EVIDENCE_ROLE: &str = "mfm.product.qualification.object-evidence-contract";
const OBJECT_EVIDENCE_PATH: &str = "qualification.object_evidence_contract";

const QUALIFICATION_PROFILE_VERSION: &str = "mfm.component-qualification-profile.v1";
const QUALIFICATION_PROFILE_CONTRACT: &str = "mfm.component-qualification-profile.v1";
const QUALIFICATION_PROFILE_SEMANTIC_NAME: &str = "component-qualification-profile";
const QUALIFICATION_PROFILE_ROLE: &str = "mfm.product.qualification.profile";
const QUALIFICATION_PROFILE_PATH: &str = "qualification.profile";

const QUALIFICATION_VERSION: &str = "mfm.component-qualification.v1";
const QUALIFICATION_CONTRACT: &str = "mfm.component-qualification.v1";
const QUALIFICATION_SEMANTIC_NAME: &str = "component-qualification";
const QUALIFICATION_ROLE: &str = "mfm.product.qualification.descriptor";
const QUALIFICATION_PATH: &str = "qualification.descriptor";

const COMPONENT_IMPLEMENTATION_SEMANTIC_NAME: &str = "component-implementation-descriptor";
const PLANNER_SEMANTIC_NAME: &str = "composite-planner-contract";
const PLANNER_SEMANTIC_SUPPORT_ROLE: &str = "mfm.qualification.planner.semantic-contract";
const PLANNER_SEMANTIC_PATH: &str = "planner.semantic_contract";
const PLANNER_CALLBACK_SEMANTIC_NAME: &str = "composite-planner-callback-surface";
const PLANNER_CALLBACK_SUPPORT_ROLE: &str = "mfm.qualification.planner.callback-surface";
const PLANNER_CALLBACK_PATH: &str = "planner.callback_surface";
const EXECUTABLE_DESCRIPTOR_CONTRACT: &str = "mfm.executable-bytes-descriptor.v1";
const EXECUTABLE_DESCRIPTOR_SEMANTIC_NAME: &str = "executable-bytes-descriptor";
const EXECUTABLE_DESCRIPTOR_ROLE: &str = "mfm.qualification.executable-identity";
const EXECUTABLE_DESCRIPTOR_PATH: &str = "executable.identity";
const MEDIA_TYPE_JSON: &str = "application/json";
const PRODUCT_COMPONENT_COUNT: usize = 14;
const STATE_COMPONENT_COUNT: usize = 11;
const COMPONENT_SUPPORT_MEMBER_COUNT: usize = PRODUCT_COMPONENT_COUNT * 3;
const FOUNDATION_SUPPORT_MEMBER_COUNT: usize = 4;
const PRODUCT_SUPPORT_MEMBER_COUNT: usize =
    COMPONENT_SUPPORT_MEMBER_COUNT + FOUNDATION_SUPPORT_MEMBER_COUNT;

const PLANNER_ROLE: &str = "mfm.product.component.planner";
const PLANNER_PATH: &str = "components.planner";
const STATE_SEMANTIC_ROLE: &str = "mfm.qualification.state.semantic-contract";
const STATE_CALLBACK_ROLE: &str = "mfm.qualification.state.callback-surface";
const STATE_IMPLEMENTATION_ROLE: &str = "mfm.qualification.state.implementation";
const EVM_READ_ADAPTER_SEMANTIC_ROLE: &str =
    "mfm.qualification.read-capability-adapter-verifier.semantic-contract";
const EVM_READ_ADAPTER_SEMANTIC_PATH: &str = "capability.evm_live.semantic_contract";
const EVM_READ_ADAPTER_CALLBACK_ROLE: &str =
    "mfm.qualification.read-capability-adapter-verifier.callback-surface";
const EVM_READ_ADAPTER_CALLBACK_PATH: &str = "capability.evm_live.callback_surface";
const EVM_READ_ADAPTER_ROLE: &str =
    "mfm.qualification.read-capability-adapter-verifier.implementation";
const EVM_READ_ADAPTER_PATH: &str = "capability.evm_live.adapter_implementation";
const EVM_WALLET_EXECUTOR_SEMANTIC_ROLE: &str =
    "mfm.qualification.executor-client-verifier.semantic-contract";
const EVM_WALLET_EXECUTOR_SEMANTIC_PATH: &str = "capability.evm_wallet.semantic_contract";
const EVM_WALLET_EXECUTOR_CALLBACK_ROLE: &str =
    "mfm.qualification.executor-client-verifier.callback-surface";
const EVM_WALLET_EXECUTOR_CALLBACK_PATH: &str = "capability.evm_wallet.callback_surface";
const EVM_WALLET_EXECUTOR_ROLE: &str = "mfm.qualification.executor-client-verifier.implementation";
const EVM_WALLET_EXECUTOR_PATH: &str = "capability.evm_wallet.executor_implementation";
const EVM_EXECUTOR_DEPLOYMENT_PATH: &str = "capability.evm_wallet.executor_deployment";
const EVM_EXECUTOR_DEPLOYMENT_ROLE: &str = "mfm.qualification.evm-wallet.executor-deployment";
const EVM_RESOURCE_OWNERSHIP_PATH: &str = "capability.evm_wallet.resource_ownership";
const EVM_RESOURCE_OWNERSHIP_ROLE: &str = "mfm.qualification.evm-wallet.resource-ownership";
const EVM_EXECUTOR_BINDING_PATH: &str = "capability.evm_wallet.executor_binding";
const EVM_EXECUTOR_BINDING_ROLE: &str = "mfm.qualification.evm-wallet.executor-binding";
const EVM_WALLET_SIGNER_BINDING_PATH: &str = "capability.evm_wallet.signer_binding";
const EVM_WALLET_SIGNER_BINDING_ROLE: &str = "mfm.qualification.evm-wallet.signer-binding";
const EVM_WALLET_NONCE_POLICY_PATH: &str = "capability.evm_wallet.nonce_policy";
const EVM_WALLET_NONCE_POLICY_ROLE: &str = "mfm.qualification.evm-wallet.nonce-policy";
const EVM_WALLET_INITIAL_NONCE_PATH: &str = "capability.evm_wallet.initial_nonce";
const EVM_WALLET_INITIAL_NONCE_ROLE: &str = "mfm.qualification.evm-wallet.initial-nonce";
const EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_PATH: &str =
    "capability.evm_wallet.already_known_classifier";
const EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_ROLE: &str =
    "mfm.qualification.evm-wallet.already-known-classifier";
const EVM_WALLET_FINALITY_POLICY_PATH: &str = "capability.evm_wallet.finality_policy";
const EVM_WALLET_FINALITY_POLICY_ROLE: &str = "mfm.qualification.evm-wallet.finality-policy";
const EVM_WALLET_ASSURANCE_POLICY_PATH: &str = "capability.evm_wallet.assurance_policy";
const EVM_WALLET_ASSURANCE_POLICY_ROLE: &str = "mfm.qualification.evm-wallet.assurance-policy";
const EVM_WALLET_REQUEST_QUALIFICATION_PATH: &str = "capability.evm_wallet.request_qualification";
const EVM_WALLET_REQUEST_QUALIFICATION_ROLE: &str =
    "mfm.qualification.evm-wallet.request-qualification";

const EVM_ROUTING_CATALOG_PATH: &str = "capability.evm_live.routing_catalog";
const EVM_ROUTING_CATALOG_ROLE: &str = "mfm.qualification.evm-live.routing-catalog";
const EVM_ROUTING_GENERATION_PATH_PREFIX: &str = "capability.evm_live.routing_generation";
const EVM_ROUTING_GENERATION_ROLE: &str = "mfm.qualification.evm-live.routing-generation";
const EVM_REVIEWED_SOURCE_SCOPE_PATH: &str = "capability.evm_live.reviewed_source_scope";
const EVM_REVIEWED_SOURCE_SCOPE_ROLE: &str = "mfm.qualification.evm-live.reviewed-source-scope";
const EVM_SAFE_FAILURE_CONTRACT_PATH: &str = "capability.evm_live.safe_failure_contract";
const EVM_SAFE_FAILURE_CONTRACT_ROLE: &str = "mfm.qualification.evm-live.safe-failure-contract";
const EVM_SAFE_FAILURE_CLASSIFIER_PATH: &str = "capability.evm_live.safe_failure_classifier";
const EVM_SAFE_FAILURE_CLASSIFIER_ROLE: &str = "mfm.qualification.evm-live.safe-failure-classifier";
const EVM_READ_CAPABILITY_BINDING_PATH: &str = "capability.evm_live.read_capability_binding";
const EVM_READ_CAPABILITY_BINDING_ROLE: &str = "mfm.qualification.evm-live.read-capability-binding";
const EVM_REVIEWED_SOURCE_SCOPE_SCHEMA_NAME: &str = "mfm.evm-live.reviewed-source-scope";
const EVM_REVIEWED_SOURCE_SCOPE_SEMANTIC_NAME: &str = "reviewed-source-scope";
const EVM_REVIEWED_SOURCE_SCOPE_VERSION: &str = "mfm.evm-live.reviewed-source-scope.v1";
const EVM_READ_CAPABILITY_BINDING_SEMANTIC_NAME: &str = "read-capability-binding";
const MAX_EVM_ROUTING_GENERATIONS: usize = 4_096;
const EVM_LIVE_FIXED_SUPPORT_MEMBER_COUNT: usize = 15;

const UNIT_CONFIG_ROLE: &str = "mfm.product.framework.unit-config";
const STATE_IMPLEMENTATION_MANIFEST_PATH: &str = "manifests.state_implementation";
const CAPABILITY_BINDING_MANIFEST_PATH: &str = "manifests.capability_binding";
const EVM_BALANCE_FACT_DESCRIPTOR_PATH: &str = "facts.evm_balance.descriptor";
const EVM_BALANCE_FACT_DESCRIPTOR_ROLE: &str = "mfm.product.fact.evm-balance.descriptor";
const EVM_BALANCE_FACT_SUBJECT_EVIDENCE_PATH: &str = "facts.evm_balance.subject_evidence";
const EVM_BALANCE_FACT_SUBJECT_EVIDENCE_ROLE: &str =
    "mfm.product.fact.evm-balance.subject-evidence-contract";
const EVM_BALANCE_FACT_RESPONSE_EVIDENCE_PATH: &str = "facts.evm_balance.response_evidence";
const EVM_BALANCE_FACT_RESPONSE_EVIDENCE_ROLE: &str =
    "mfm.product.fact.evm-balance.response-evidence-contract";
const PRODUCT_DEPLOYMENT_SUPPORT_MEMBER_COUNT_WITHOUT_GENERATIONS: usize = 68;
const PRODUCT_ADDITIONAL_SUPPORT_MEMBER_COUNT: usize = 17;

const QUALIFIED_SUPPORT_SCOPE_DOMAIN: &str = "mfm.product.qualified-support-scope.v1";
const QUALIFIED_SUPPORT_SCOPE_VERSION: &str =
    "mfm.product.portfolio-snapshot-evm-qualified-support-scope.v1";
const QUALIFIED_SUPPORT_SCOPE_SEMANTIC_NAME: &str =
    "portfolio-snapshot-evm-qualified-support-scope";

#[derive(Clone, Copy)]
struct ComponentSupportPaths {
    semantic_path: &'static str,
    semantic_role: &'static str,
    callback_path: &'static str,
    callback_role: &'static str,
    implementation_path: &'static str,
    implementation_role: &'static str,
}

const PLANNER_SUPPORT_PATHS: ComponentSupportPaths = ComponentSupportPaths {
    semantic_path: PLANNER_SEMANTIC_PATH,
    semantic_role: PLANNER_SEMANTIC_SUPPORT_ROLE,
    callback_path: PLANNER_CALLBACK_PATH,
    callback_role: PLANNER_CALLBACK_SUPPORT_ROLE,
    implementation_path: PLANNER_PATH,
    implementation_role: PLANNER_ROLE,
};

const STATE_SUPPORT_PATHS: [ComponentSupportPaths; STATE_COMPONENT_COUNT] = [
    state_support_paths(
        "state.portfolio_validate_selection.semantic_contract",
        "state.portfolio_validate_selection.callback_surface",
        "state.portfolio_validate_selection.implementation",
    ),
    state_support_paths(
        "state.portfolio_assemble_snapshot.semantic_contract",
        "state.portfolio_assemble_snapshot.callback_surface",
        "state.portfolio_assemble_snapshot.implementation",
    ),
    state_support_paths(
        "state.portfolio_project_report.semantic_contract",
        "state.portfolio_project_report.callback_surface",
        "state.portfolio_project_report.implementation",
    ),
    state_support_paths(
        "state.evm_bootstrap_source.semantic_contract",
        "state.evm_bootstrap_source.callback_surface",
        "state.evm_bootstrap_source.implementation",
    ),
    state_support_paths(
        "state.evm_read_initial_anchor.semantic_contract",
        "state.evm_read_initial_anchor.callback_surface",
        "state.evm_read_initial_anchor.implementation",
    ),
    state_support_paths(
        "state.evm_read_token_decimals.semantic_contract",
        "state.evm_read_token_decimals.callback_surface",
        "state.evm_read_token_decimals.implementation",
    ),
    state_support_paths(
        "state.evm_read_native_balance.semantic_contract",
        "state.evm_read_native_balance.callback_surface",
        "state.evm_read_native_balance.implementation",
    ),
    state_support_paths(
        "state.evm_read_token_balance.semantic_contract",
        "state.evm_read_token_balance.callback_surface",
        "state.evm_read_token_balance.implementation",
    ),
    state_support_paths(
        "state.evm_confirm_anchor.semantic_contract",
        "state.evm_confirm_anchor.callback_surface",
        "state.evm_confirm_anchor.implementation",
    ),
    state_support_paths(
        "state.evm_aggregate_balances.semantic_contract",
        "state.evm_aggregate_balances.callback_surface",
        "state.evm_aggregate_balances.implementation",
    ),
    state_support_paths(
        "state.evm_submit_transaction.semantic_contract",
        "state.evm_submit_transaction.callback_surface",
        "state.evm_submit_transaction.implementation",
    ),
];

const EVM_READ_ADAPTER_SUPPORT_PATHS: ComponentSupportPaths = ComponentSupportPaths {
    semantic_path: EVM_READ_ADAPTER_SEMANTIC_PATH,
    semantic_role: EVM_READ_ADAPTER_SEMANTIC_ROLE,
    callback_path: EVM_READ_ADAPTER_CALLBACK_PATH,
    callback_role: EVM_READ_ADAPTER_CALLBACK_ROLE,
    implementation_path: EVM_READ_ADAPTER_PATH,
    implementation_role: EVM_READ_ADAPTER_ROLE,
};

const EVM_WALLET_EXECUTOR_SUPPORT_PATHS: ComponentSupportPaths = ComponentSupportPaths {
    semantic_path: EVM_WALLET_EXECUTOR_SEMANTIC_PATH,
    semantic_role: EVM_WALLET_EXECUTOR_SEMANTIC_ROLE,
    callback_path: EVM_WALLET_EXECUTOR_CALLBACK_PATH,
    callback_role: EVM_WALLET_EXECUTOR_CALLBACK_ROLE,
    implementation_path: EVM_WALLET_EXECUTOR_PATH,
    implementation_role: EVM_WALLET_EXECUTOR_ROLE,
};

const fn state_support_paths(
    semantic_path: &'static str,
    callback_path: &'static str,
    implementation_path: &'static str,
) -> ComponentSupportPaths {
    ComponentSupportPaths {
        semantic_path,
        semantic_role: STATE_SEMANTIC_ROLE,
        callback_path,
        callback_role: STATE_CALLBACK_ROLE,
        implementation_path,
        implementation_role: STATE_IMPLEMENTATION_ROLE,
    }
}

fn component_support_paths() -> [ComponentSupportPaths; PRODUCT_COMPONENT_COUNT] {
    [
        PLANNER_SUPPORT_PATHS,
        STATE_SUPPORT_PATHS[0],
        STATE_SUPPORT_PATHS[1],
        STATE_SUPPORT_PATHS[2],
        STATE_SUPPORT_PATHS[3],
        STATE_SUPPORT_PATHS[4],
        STATE_SUPPORT_PATHS[5],
        STATE_SUPPORT_PATHS[6],
        STATE_SUPPORT_PATHS[7],
        STATE_SUPPORT_PATHS[8],
        STATE_SUPPORT_PATHS[9],
        STATE_SUPPORT_PATHS[10],
        EVM_READ_ADAPTER_SUPPORT_PATHS,
        EVM_WALLET_EXECUTOR_SUPPORT_PATHS,
    ]
}

/// Exact implementation descriptors selected by the sole product package.
pub(super) struct ProductComponentImplementations {
    pub(super) planner: ComponentImplementationDescriptor,
    pub(super) portfolio: mfm_portfolio::PortfolioSnapshotStateImplementations,
    pub(super) evm: mfm_evm::EvmBalanceCollectionStateImplementations,
    pub(super) evm_submit_transaction: ComponentImplementationDescriptor,
    pub(super) evm_read_adapter: ComponentImplementationDescriptor,
    pub(super) evm_wallet_executor: ComponentImplementationDescriptor,
}

impl ProductComponentImplementations {
    fn ordered(&self) -> [&ComponentImplementationDescriptor; PRODUCT_COMPONENT_COUNT] {
        [
            &self.planner,
            &self.portfolio.validate_selection,
            &self.portfolio.assemble_snapshot,
            &self.portfolio.project_report,
            &self.evm.bootstrap_source,
            &self.evm.read_initial_anchor,
            &self.evm.read_token_decimals,
            &self.evm.read_native_balance,
            &self.evm.read_token_balance,
            &self.evm.confirm_anchor,
            &self.evm.aggregate_balances,
            &self.evm_submit_transaction,
            &self.evm_read_adapter,
            &self.evm_wallet_executor,
        ]
    }
}

/// Complete fixed qualification output awaiting the remaining deployment support objects.
pub(super) struct ProductQualification {
    pub(super) object_evidence_contract_ref: ContentRef,
    pub(super) qualification_profile_ref: ContentRef,
    pub(super) qualification_ref: ContentRef,
    pub(super) implementations: ProductComponentImplementations,
    pub(super) support_members: Vec<QualifiedSupportMember>,
}

/// Exact live EVM support objects and the binding they certify.
pub(super) struct EvmLiveQualification {
    pub(super) read_capability_binding: ReadCapabilityBinding,
    pub(super) read_capability_binding_ref: ContentRef,
    pub(super) executor_binding: VerifiedExecutorBinding,
    pub(super) wallet_request_qualification: Arc<mfm_evm_live::EvmWalletRequestQualification>,
    pub(super) support_members: Vec<QualifiedSupportMember>,
}

/// Complete transient qualified product deployment before its one store admission.
pub(super) struct QualifiedProductDeployment {
    pub(super) object_evidence_contract_ref: ContentRef,
    pub(super) qualification_profile_ref: ContentRef,
    pub(super) qualification_ref: ContentRef,
    pub(super) implementations: ProductComponentImplementations,
    pub(super) read_capability_binding: ReadCapabilityBinding,
    pub(super) read_capability_binding_ref: ContentRef,
    pub(super) executor_binding: VerifiedExecutorBinding,
    pub(super) wallet_request_qualification: Arc<mfm_evm_live::EvmWalletRequestQualification>,
    pub(super) unit_config_contract: RetainedValueContract,
    pub(super) state_manifest: StateImplementationManifest,
    pub(super) state_manifest_ref: ContentRef,
    pub(super) capability_manifest: CapabilityBindingManifest,
    pub(super) capability_manifest_ref: ContentRef,
    pub(super) support_graph: QualifiedSupportGraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct QualifiedComponent {
    component_kind: ComponentKind,
    semantic_contract_ref: ContentRef,
    callback_surface_ref: ContentRef,
}

#[derive(Serialize)]
struct QualificationDescriptor<'a> {
    version: &'static str,
    executable_identity_ref: &'a ContentRef,
    qualification_profile_ref: &'a ContentRef,
    components: &'a [QualifiedComponent],
}

#[derive(Serialize)]
struct VersionDescriptor {
    version: &'static str,
}

#[derive(Serialize)]
struct ReviewedSourceScope {
    source_refs: Vec<String>,
    version: &'static str,
}

#[derive(Serialize)]
struct QualifiedSupportScopePreimage<'a> {
    version: &'static str,
    members: Vec<QualifiedSupportScopeMember<'a>>,
}

#[derive(Serialize)]
struct QualifiedSupportScopeMember<'a> {
    field_path: &'a FieldPath,
    content_ref: ContentRef,
    semantic_type_id: &'a SemanticTypeId,
    role: &'a StableId,
    media_type: &'a str,
    evidence_contract_ref: &'a ContentRef,
}

#[derive(Serialize)]
struct QualifiedSupportScopeEnvelope<'a> {
    domain: &'static str,
    value: QualifiedSupportScopePreimage<'a>,
}

/// Builds the one shared executable-bound qualification and all implementation
/// descriptors.
///
/// No implementation reference enters the qualification preimage, which keeps the graph acyclic.
pub(super) fn qualify_product_components(
    executable_identity_ref: ContentRef,
    executable_descriptor_bytes: &[u8],
    wallet_executor_contract: &ExecutorContractDescriptor,
) -> Result<ProductQualification, PublicError> {
    let (object_evidence_member, object_evidence_contract_ref) = object_evidence_contract()?;
    let (profile_member, qualification_profile_ref) =
        qualification_profile(object_evidence_contract_ref.clone())?;
    let executable_member = executable_identity_member(
        &executable_identity_ref,
        executable_descriptor_bytes,
        object_evidence_contract_ref.clone(),
    )?;
    let planner_surface =
        mfm_program::CompositePlannerSurface::current().map_err(|_| invalid_qualification())?;
    let (planner_semantic_member, planner_callback_member) =
        planner_surface_support_members(&planner_surface, object_evidence_contract_ref.clone())?;
    let planner_semantic_contract_ref = planner_surface.semantic_contract_ref().clone();
    let planner_callback_surface_ref = planner_surface.callback_surface_ref().clone();

    let portfolio_surfaces = mfm_portfolio::portfolio_snapshot_callback_surfaces()
        .map_err(|_| invalid_qualification())?;
    let evm_surfaces =
        mfm_evm::evm_balance_collection_callback_surfaces().map_err(|_| invalid_qualification())?;
    let evm_read_capability_ref =
        mfm_evm::evm_read_capability_contract_ref().map_err(|_| invalid_qualification())?;
    let evm_read_adapter_callback_ref =
        mfm_evm_live::evm_adapter_callback_surface_ref().map_err(|_| invalid_qualification())?;
    let wallet_surface =
        mfm_evm::evm_submit_transaction_callback_surface().map_err(|_| invalid_qualification())?;
    let wallet_executor_contract_ref = wallet_executor_contract
        .reference()
        .map_err(|_| invalid_qualification())?;
    let wallet_executor_callback_ref = mfm_evm_live::evm_wallet_target_callback_surface_ref()
        .map_err(|_| invalid_qualification())?;

    let components = vec![
        qualified_component(
            ComponentKind::Planner,
            &planner_semantic_contract_ref,
            &planner_callback_surface_ref,
        ),
        qualified_component(
            ComponentKind::State,
            portfolio_surfaces.validate_selection().state_contract_ref(),
            portfolio_surfaces.validate_selection().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            portfolio_surfaces.assemble_snapshot().state_contract_ref(),
            portfolio_surfaces.assemble_snapshot().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            portfolio_surfaces.project_report().state_contract_ref(),
            portfolio_surfaces.project_report().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            evm_surfaces.bootstrap_source().state_contract_ref(),
            evm_surfaces.bootstrap_source().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            evm_surfaces.read_initial_anchor().state_contract_ref(),
            evm_surfaces.read_initial_anchor().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            evm_surfaces.read_token_decimals().state_contract_ref(),
            evm_surfaces.read_token_decimals().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            evm_surfaces.read_native_balance().state_contract_ref(),
            evm_surfaces.read_native_balance().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            evm_surfaces.read_token_balance().state_contract_ref(),
            evm_surfaces.read_token_balance().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            evm_surfaces.confirm_anchor().state_contract_ref(),
            evm_surfaces.confirm_anchor().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            evm_surfaces.aggregate_balances().state_contract_ref(),
            evm_surfaces.aggregate_balances().content_ref(),
        ),
        qualified_component(
            ComponentKind::State,
            wallet_surface.state_contract_ref(),
            wallet_surface.content_ref(),
        ),
        qualified_component(
            ComponentKind::ReadCapabilityAdapterVerifier,
            &evm_read_capability_ref,
            &evm_read_adapter_callback_ref,
        ),
        qualified_component(
            ComponentKind::ExecutorClientVerifier,
            &wallet_executor_contract_ref,
            &wallet_executor_callback_ref,
        ),
    ];
    let (qualification_member, qualification_ref) = qualification_descriptor(
        &executable_identity_ref,
        &qualification_profile_ref,
        components.clone(),
        object_evidence_contract_ref.clone(),
    )?;

    let planner = planner_surface
        .component_descriptor(qualification_ref.clone())
        .map_err(|_| invalid_qualification())?;
    let portfolio = mfm_portfolio::PortfolioSnapshotStateImplementations {
        validate_selection: state_implementation(
            portfolio_surfaces.validate_selection(),
            &qualification_ref,
        )?,
        assemble_snapshot: state_implementation(
            portfolio_surfaces.assemble_snapshot(),
            &qualification_ref,
        )?,
        project_report: state_implementation(
            portfolio_surfaces.project_report(),
            &qualification_ref,
        )?,
    };
    let evm = mfm_evm::EvmBalanceCollectionStateImplementations {
        bootstrap_source: state_implementation(
            evm_surfaces.bootstrap_source(),
            &qualification_ref,
        )?,
        read_initial_anchor: state_implementation(
            evm_surfaces.read_initial_anchor(),
            &qualification_ref,
        )?,
        read_token_decimals: state_implementation(
            evm_surfaces.read_token_decimals(),
            &qualification_ref,
        )?,
        read_native_balance: state_implementation(
            evm_surfaces.read_native_balance(),
            &qualification_ref,
        )?,
        read_token_balance: state_implementation(
            evm_surfaces.read_token_balance(),
            &qualification_ref,
        )?,
        confirm_anchor: state_implementation(evm_surfaces.confirm_anchor(), &qualification_ref)?,
        aggregate_balances: state_implementation(
            evm_surfaces.aggregate_balances(),
            &qualification_ref,
        )?,
    };
    let evm_read_adapter = component_implementation(
        ComponentKind::ReadCapabilityAdapterVerifier,
        evm_read_capability_ref,
        evm_read_adapter_callback_ref,
        &qualification_ref,
    )?;
    let evm_submit_transaction = state_implementation(&wallet_surface, &qualification_ref)?;
    let evm_wallet_executor = component_implementation(
        ComponentKind::ExecutorClientVerifier,
        wallet_executor_contract_ref,
        wallet_executor_callback_ref,
        &qualification_ref,
    )?;
    let implementations = ProductComponentImplementations {
        planner,
        portfolio,
        evm,
        evm_submit_transaction,
        evm_read_adapter,
        evm_wallet_executor,
    };

    let mut surface_members = Vec::with_capacity(PRODUCT_COMPONENT_COUNT);
    surface_members.push((planner_semantic_member, planner_callback_member));
    for (surface, paths) in portfolio_surfaces
        .ordered()
        .into_iter()
        .zip(STATE_SUPPORT_PATHS[..3].iter().copied())
    {
        surface_members.push(state_surface_support_members(
            surface,
            paths,
            object_evidence_contract_ref.clone(),
        )?);
    }
    for (surface, paths) in evm_surfaces
        .ordered()
        .into_iter()
        .zip(STATE_SUPPORT_PATHS[3..10].iter().copied())
    {
        surface_members.push(state_surface_support_members(
            surface,
            paths,
            object_evidence_contract_ref.clone(),
        )?);
    }
    surface_members.push(state_surface_support_members(
        &wallet_surface,
        STATE_SUPPORT_PATHS[10],
        object_evidence_contract_ref.clone(),
    )?);
    surface_members.push(evm_read_adapter_surface_support_members(
        object_evidence_contract_ref.clone(),
    )?);
    surface_members.push(evm_wallet_executor_surface_support_members(
        wallet_executor_contract,
        object_evidence_contract_ref.clone(),
    )?);

    let paths = component_support_paths();
    let descriptors = implementations.ordered();
    if surface_members.len() != PRODUCT_COMPONENT_COUNT {
        return Err(invalid_qualification());
    }
    let mut component_members = Vec::with_capacity(COMPONENT_SUPPORT_MEMBER_COUNT);
    for (((semantic_member, callback_member), descriptor), component_paths) in
        surface_members.into_iter().zip(descriptors).zip(paths)
    {
        component_members.extend([
            semantic_member,
            callback_member,
            component_implementation_member(
                descriptor,
                component_paths.implementation_role,
                component_paths.implementation_path,
                object_evidence_contract_ref.clone(),
            )?,
        ]);
    }

    let mut support_members = Vec::with_capacity(PRODUCT_SUPPORT_MEMBER_COUNT);
    support_members.extend([
        object_evidence_member,
        profile_member,
        executable_member,
        qualification_member,
    ]);
    support_members.extend(component_members);
    validate_component_support_closure(
        &components,
        &implementations,
        &qualification_ref,
        &support_members,
    )?;

    Ok(ProductQualification {
        object_evidence_contract_ref,
        qualification_profile_ref,
        qualification_ref,
        implementations,
        support_members,
    })
}

/// Builds the exact deployment-dependent EVM support closure.
#[allow(clippy::too_many_arguments)]
pub(super) fn qualify_evm_live_support(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
    product: &ProductQualification,
    executor_contract: ExecutorContractDescriptor,
    executor_deployment: ExecutorDeployment,
    resource_ownership: ResourceOwnership,
    route_generation_ref: mfm_evm::EvmRoutingGenerationRef,
    initial_nonce_descriptor: mfm_evm::EvmWalletInitialNonceDescriptor,
    signer_binding: &mfm_signing::VerifiedGenerationGuardedSignerBinding,
) -> Result<EvmLiveQualification, PublicError> {
    let evidence_contract_ref = product.object_evidence_contract_ref.clone();
    let (mut support_members, routing_catalog_ref, reviewed_source_scope_ref) =
        evm_routing_support_members(transport, evidence_contract_ref.clone())?;

    let safe_failure_ref =
        mfm_evm::evm_safe_failure_contract_ref().map_err(|_| invalid_qualification())?;
    let safe_failure_member = support_member(
        EVM_SAFE_FAILURE_CONTRACT_PATH,
        mfm_evm::evm_safe_failure_contract_canonical().map_err(|_| invalid_qualification())?,
        mfm_evm::evm_safe_failure_support_contract(
            stable_id(EVM_SAFE_FAILURE_CONTRACT_ROLE)?,
            evidence_contract_ref.clone(),
        )
        .map_err(|_| invalid_qualification())?,
    )?;

    let safe_classifier_ref =
        mfm_evm_live::evm_safe_classifier_contract_ref().map_err(|_| invalid_qualification())?;
    let safe_classifier_member = support_member(
        EVM_SAFE_FAILURE_CLASSIFIER_PATH,
        mfm_evm_live::evm_safe_classifier_canonical().map_err(|_| invalid_qualification())?,
        mfm_evm_live::evm_safe_classifier_support_contract(
            stable_id(EVM_SAFE_FAILURE_CLASSIFIER_ROLE)?,
            evidence_contract_ref.clone(),
        )
        .map_err(|_| invalid_qualification())?,
    )?;

    let capability_contract_ref =
        mfm_evm::evm_read_capability_contract_ref().map_err(|_| invalid_qualification())?;
    let admitted_implementation_ref = product
        .implementations
        .evm_read_adapter
        .content_ref()
        .map_err(|_| invalid_qualification())?;
    let read_capability_binding = ReadCapabilityBinding::new(
        &capability_contract_ref,
        &admitted_implementation_ref,
        &safe_classifier_ref,
        &safe_failure_ref,
        &reviewed_source_scope_ref,
        &routing_catalog_ref,
    )
    .map_err(|_| invalid_qualification())?;
    let binding_canonical =
        PlainCanonicalJsonBytes::from_canonical_json_slice(read_capability_binding.as_bytes())
            .map_err(|_| invalid_qualification())?;
    let read_capability_binding_ref = read_capability_binding
        .content_ref()
        .map_err(|_| invalid_qualification())?;
    let binding_member = support_member(
        EVM_READ_CAPABILITY_BINDING_PATH,
        binding_canonical,
        retained_contract_in(
            read_capability_binding.schema_id().clone(),
            "mfm.evm-live",
            EVM_READ_CAPABILITY_BINDING_SEMANTIC_NAME,
            EVM_READ_CAPABILITY_BINDING_ROLE,
            evidence_contract_ref.clone(),
        )?,
    )?;

    let executor_contract_ref = executor_contract
        .reference()
        .map_err(|_| invalid_qualification())?;
    let executor_implementation_ref = product
        .implementations
        .evm_wallet_executor
        .content_ref()
        .map_err(|_| invalid_qualification())?;
    let executor_deployment_ref = executor_deployment
        .reference()
        .map_err(|_| invalid_qualification())?;
    let executor_binding = ExecutorBinding::new(
        executor_contract_ref,
        executor_implementation_ref,
        executor_deployment_ref,
    )
    .map_err(|_| invalid_qualification())?;
    let admitted_tenant_scope_id = executor_deployment.tenant_scope_id().clone();
    let executor_binding = VerifiedExecutorBinding::verify(
        executor_binding,
        executor_contract,
        executor_deployment,
        Some(resource_ownership),
        &admitted_tenant_scope_id,
    )
    .map_err(|_| invalid_qualification())?;
    let wallet_request_qualification = Arc::new(
        mfm_evm_live::EvmWalletRequestQualification::qualify(
            transport,
            route_generation_ref,
            executor_binding.clone(),
            signer_binding,
            initial_nonce_descriptor,
            evidence_contract_ref.clone(),
        )
        .map_err(|_| invalid_qualification())?,
    );
    let executor_deployment_member = executor_support_member(
        EVM_EXECUTOR_DEPLOYMENT_PATH,
        EVM_EXECUTOR_DEPLOYMENT_ROLE,
        "evm-wallet-executor-deployment",
        executor_binding
            .deployment()
            .validated()
            .map_err(|_| invalid_qualification())?,
        evidence_contract_ref.clone(),
    )?;
    let resource_ownership_member = executor_support_member(
        EVM_RESOURCE_OWNERSHIP_PATH,
        EVM_RESOURCE_OWNERSHIP_ROLE,
        "evm-wallet-resource-ownership",
        executor_binding
            .resource_ownership()
            .ok_or_else(invalid_qualification)?
            .validated()
            .map_err(|_| invalid_qualification())?,
        evidence_contract_ref.clone(),
    )?;
    let executor_binding_member = executor_support_member(
        EVM_EXECUTOR_BINDING_PATH,
        EVM_EXECUTOR_BINDING_ROLE,
        "evm-wallet-executor-binding",
        executor_binding
            .binding()
            .validated()
            .map_err(|_| invalid_qualification())?,
        evidence_contract_ref.clone(),
    )?;
    let signer_descriptor_member = wallet_support_member(
        EVM_WALLET_SIGNER_BINDING_PATH,
        EVM_WALLET_SIGNER_BINDING_ROLE,
        "mfm.signing",
        "generation-guarded-signer-descriptor",
        PlainCanonicalJsonBytes::from_canonical_json_slice(
            wallet_request_qualification
                .signer_descriptor()
                .canonical()
                .as_bytes(),
        )
        .map_err(|_| invalid_qualification())?,
        wallet_request_qualification
            .signer_descriptor()
            .reference()
            .clone(),
        evidence_contract_ref.clone(),
    )?;
    let nonce_policy_member = wallet_support_member(
        EVM_WALLET_NONCE_POLICY_PATH,
        EVM_WALLET_NONCE_POLICY_ROLE,
        "mfm.evm",
        "nonce-policy",
        mfm_evm::evm_wallet_nonce_policy_canonical().map_err(|_| invalid_qualification())?,
        wallet_request_qualification
            .resource_policy_binding()
            .policy_ref()
            .clone(),
        evidence_contract_ref.clone(),
    )?;
    let initial_nonce_member = wallet_support_member(
        EVM_WALLET_INITIAL_NONCE_PATH,
        EVM_WALLET_INITIAL_NONCE_ROLE,
        "mfm.evm",
        "initial-nonce",
        wallet_request_qualification
            .initial_nonce_descriptor()
            .canonical()
            .map_err(|_| invalid_qualification())?,
        wallet_request_qualification
            .resource_policy_binding()
            .policy_configuration_ref()
            .clone(),
        evidence_contract_ref.clone(),
    )?;
    let already_known_classifier_member = wallet_support_member(
        EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_PATH,
        EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_ROLE,
        "mfm.evm-live",
        "already-known-classifier",
        mfm_evm_live::evm_already_known_classifier_canonical()
            .map_err(|_| invalid_qualification())?,
        wallet_request_qualification
            .already_known_classifier_ref()
            .to_content_ref()
            .map_err(|_| invalid_qualification())?,
        evidence_contract_ref.clone(),
    )?;
    let finality_policy_member = wallet_support_member(
        EVM_WALLET_FINALITY_POLICY_PATH,
        EVM_WALLET_FINALITY_POLICY_ROLE,
        "mfm.evm",
        "finality-policy",
        mfm_evm::evm_wallet_finality_policy_canonical().map_err(|_| invalid_qualification())?,
        wallet_request_qualification
            .finality_policy_ref()
            .to_content_ref()
            .map_err(|_| invalid_qualification())?,
        evidence_contract_ref.clone(),
    )?;
    let assurance_policy_member = wallet_support_member(
        EVM_WALLET_ASSURANCE_POLICY_PATH,
        EVM_WALLET_ASSURANCE_POLICY_ROLE,
        "mfm.evm",
        "assurance-policy",
        mfm_evm::evm_wallet_assurance_policy_canonical().map_err(|_| invalid_qualification())?,
        wallet_request_qualification
            .assurance_policy_ref()
            .to_content_ref()
            .map_err(|_| invalid_qualification())?,
        evidence_contract_ref.clone(),
    )?;
    let request_qualification_member = wallet_support_member(
        EVM_WALLET_REQUEST_QUALIFICATION_PATH,
        EVM_WALLET_REQUEST_QUALIFICATION_ROLE,
        "mfm.evm-live",
        "request-qualification",
        wallet_request_qualification.canonical().clone(),
        wallet_request_qualification.reference().clone(),
        evidence_contract_ref.clone(),
    )?;

    support_members.extend([
        safe_failure_member,
        safe_classifier_member,
        binding_member,
        executor_deployment_member,
        resource_ownership_member,
        executor_binding_member,
        signer_descriptor_member,
        nonce_policy_member,
        initial_nonce_member,
        already_known_classifier_member,
        finality_policy_member,
        assurance_policy_member,
        request_qualification_member,
    ]);
    validate_evm_live_support_closure(
        transport,
        &read_capability_binding,
        &read_capability_binding_ref,
        &admitted_implementation_ref,
        &evidence_contract_ref,
        &executor_binding,
        wallet_request_qualification.as_ref(),
        &support_members,
    )?;

    Ok(EvmLiveQualification {
        read_capability_binding,
        read_capability_binding_ref,
        executor_binding,
        wallet_request_qualification,
        support_members,
    })
}

/// Assembles the exact `68 + N` product support graph and derives its content-bound scope.
pub(super) fn assemble_qualified_product_deployment(
    product: ProductQualification,
    live: EvmLiveQualification,
    routing_manifest: &mfm_portfolio::PortfolioRoutingManifest,
) -> Result<QualifiedProductDeployment, PublicError> {
    let evidence_contract_ref = product.object_evidence_contract_ref.clone();
    let unit_config_contract = mfm_program::unit_config_value_contract(
        stable_id(UNIT_CONFIG_ROLE)?,
        evidence_contract_ref.clone(),
    )
    .map_err(|_| invalid_qualification())?;
    let (unit_config_canonical, _) =
        mfm_program::encode_config(&mfm_program::UnitConfig::default())
            .map_err(|_| invalid_qualification())?;
    let unit_config_member = support_member(
        mfm_portfolio::PORTFOLIO_SNAPSHOT_UNIT_CONFIG_MEMBER_PATH,
        unit_config_canonical,
        unit_config_contract.clone(),
    )?;

    let (routing_canonical, routing_ref) =
        mfm_program::encode_mfm_value(routing_manifest).map_err(|_| invalid_qualification())?;
    let routing_contract =
        mfm_portfolio::portfolio_snapshot_value_contracts(evidence_contract_ref.clone())
            .map_err(|_| invalid_qualification())?
            .routing_manifest()
            .clone();
    let routing_member = support_member(
        mfm_portfolio::PORTFOLIO_SNAPSHOT_ROUTING_MANIFEST_MEMBER_PATH,
        routing_canonical,
        routing_contract,
    )?;
    if support_content_ref(&routing_member)? != routing_ref {
        return Err(invalid_qualification());
    }

    let state_manifest = product_state_manifest(&product.implementations)?;
    let state_manifest_ref = state_manifest
        .content_ref()
        .map_err(|_| invalid_qualification())?;
    let state_manifest_member = support_member(
        STATE_IMPLEMENTATION_MANIFEST_PATH,
        state_manifest
            .canonical_json()
            .map_err(|_| invalid_qualification())?,
        StateImplementationManifest::retained_contract().map_err(|_| invalid_qualification())?,
    )?;

    let capability_manifest = product_capability_manifest(
        &live.read_capability_binding_ref,
        live.executor_binding.binding_ref().as_content_ref(),
    )?;
    let capability_manifest_ref = capability_manifest
        .content_ref()
        .map_err(|_| invalid_qualification())?;
    let capability_manifest_member = support_member(
        CAPABILITY_BINDING_MANIFEST_PATH,
        capability_manifest
            .canonical_json()
            .map_err(|_| invalid_qualification())?,
        CapabilityBindingManifest::retained_contract().map_err(|_| invalid_qualification())?,
    )?;

    let fact_objects =
        mfm_evm::evm_balance_fact_support_objects().map_err(|_| invalid_qualification())?;
    let fact_descriptor_member = support_member(
        EVM_BALANCE_FACT_DESCRIPTOR_PATH,
        fact_objects.descriptor_canonical().clone(),
        fact_objects
            .descriptor_support_contract(
                stable_id(EVM_BALANCE_FACT_DESCRIPTOR_ROLE)?,
                evidence_contract_ref.clone(),
            )
            .map_err(|_| invalid_qualification())?,
    )?;
    let fact_subject_evidence_member = support_member(
        EVM_BALANCE_FACT_SUBJECT_EVIDENCE_PATH,
        fact_objects.subject_evidence_canonical().clone(),
        fact_objects
            .evidence_support_contract(
                stable_id(EVM_BALANCE_FACT_SUBJECT_EVIDENCE_ROLE)?,
                evidence_contract_ref.clone(),
            )
            .map_err(|_| invalid_qualification())?,
    )?;
    let fact_response_evidence_member = support_member(
        EVM_BALANCE_FACT_RESPONSE_EVIDENCE_PATH,
        fact_objects.response_evidence_canonical().clone(),
        fact_objects
            .evidence_support_contract(
                stable_id(EVM_BALANCE_FACT_RESPONSE_EVIDENCE_ROLE)?,
                evidence_contract_ref.clone(),
            )
            .map_err(|_| invalid_qualification())?,
    )?;

    let generation_count = live
        .support_members
        .len()
        .checked_sub(EVM_LIVE_FIXED_SUPPORT_MEMBER_COUNT)
        .ok_or_else(invalid_qualification)?;
    let ProductQualification {
        object_evidence_contract_ref,
        qualification_profile_ref,
        qualification_ref,
        implementations,
        support_members: mut all_support_members,
    } = product;
    let EvmLiveQualification {
        read_capability_binding,
        read_capability_binding_ref,
        executor_binding,
        wallet_request_qualification,
        support_members: live_support_members,
    } = live;
    all_support_members.extend(live_support_members);
    all_support_members.extend([
        unit_config_member,
        routing_member,
        state_manifest_member,
        capability_manifest_member,
        fact_descriptor_member,
        fact_subject_evidence_member,
        fact_response_evidence_member,
    ]);
    validate_product_deployment_support_closure(
        &all_support_members,
        generation_count,
        &object_evidence_contract_ref,
        &state_manifest_ref,
        &capability_manifest_ref,
        &fact_objects,
        &executor_binding,
        wallet_request_qualification.as_ref(),
    )?;
    let qualification_scope_id = qualified_support_scope_id(&all_support_members)?;
    let support_graph = QualifiedSupportGraph::new(qualification_scope_id, all_support_members)
        .map_err(|_| invalid_qualification())?;

    Ok(QualifiedProductDeployment {
        object_evidence_contract_ref,
        qualification_profile_ref,
        qualification_ref,
        implementations,
        read_capability_binding,
        read_capability_binding_ref,
        executor_binding,
        wallet_request_qualification,
        unit_config_contract,
        state_manifest,
        state_manifest_ref,
        capability_manifest,
        capability_manifest_ref,
        support_graph,
    })
}

fn product_state_manifest(
    implementations: &ProductComponentImplementations,
) -> Result<StateImplementationManifest, PublicError> {
    let mut entries = Vec::with_capacity(STATE_COMPONENT_COUNT);
    for descriptor in implementations.ordered() {
        if descriptor.component_kind() != ComponentKind::State {
            continue;
        }
        entries.push(StateImplementationManifestEntry {
            state_contract_ref: descriptor.semantic_contract_ref().clone(),
            component_implementation_ref: descriptor
                .content_ref()
                .map_err(|_| invalid_qualification())?,
        });
    }
    if entries.len() != STATE_COMPONENT_COUNT {
        return Err(invalid_qualification());
    }
    StateImplementationManifest::new(entries).map_err(|_| invalid_qualification())
}

fn product_capability_manifest(
    read_binding_ref: &ContentRef,
    executor_binding_ref: &ContentRef,
) -> Result<CapabilityBindingManifest, PublicError> {
    let mut entries = mfm_evm::EVM_READ_OPERATION_IDS
        .into_iter()
        .map(|operation_id| {
            Ok(CapabilityBindingManifestEntry {
                operation_id: stable_id(operation_id)?,
                binding_ref: read_binding_ref.clone(),
            })
        })
        .collect::<Result<Vec<_>, PublicError>>()?;
    entries.push(CapabilityBindingManifestEntry {
        operation_id: stable_id(mfm_evm::EVM_SUBMIT_TRANSACTION_OPERATION_ID)?,
        binding_ref: executor_binding_ref.clone(),
    });
    CapabilityBindingManifest::new(entries).map_err(|_| invalid_qualification())
}

#[allow(clippy::too_many_arguments)]
fn validate_product_deployment_support_closure(
    support_members: &[QualifiedSupportMember],
    generation_count: usize,
    evidence_contract_ref: &ContentRef,
    state_manifest_ref: &ContentRef,
    capability_manifest_ref: &ContentRef,
    fact_objects: &mfm_evm::EvmBalanceFactSupportObjects,
    executor_binding: &VerifiedExecutorBinding,
    wallet_request_qualification: &mfm_evm_live::EvmWalletRequestQualification,
) -> Result<(), PublicError> {
    if !(1..=MAX_EVM_ROUTING_GENERATIONS).contains(&generation_count)
        || support_members.len()
            != PRODUCT_DEPLOYMENT_SUPPORT_MEMBER_COUNT_WITHOUT_GENERATIONS + generation_count
        || support_members
            .iter()
            .map(QualifiedSupportMember::field_path)
            .collect::<BTreeSet<_>>()
            .len()
            != support_members.len()
        || support_members
            .iter()
            .any(|member| member.value_contract().evidence_contract_ref() != evidence_contract_ref)
    {
        return Err(invalid_qualification());
    }

    let expected_paths_and_roles = [
        (
            mfm_portfolio::PORTFOLIO_SNAPSHOT_UNIT_CONFIG_MEMBER_PATH,
            UNIT_CONFIG_ROLE,
        ),
        (
            mfm_portfolio::PORTFOLIO_SNAPSHOT_ROUTING_MANIFEST_MEMBER_PATH,
            "mfm.portfolio.value.routing-manifest",
        ),
        (
            STATE_IMPLEMENTATION_MANIFEST_PATH,
            "mfm.admission.state-implementation-manifest",
        ),
        (
            CAPABILITY_BINDING_MANIFEST_PATH,
            "mfm.admission.capability-binding-manifest",
        ),
        (
            EVM_BALANCE_FACT_DESCRIPTOR_PATH,
            EVM_BALANCE_FACT_DESCRIPTOR_ROLE,
        ),
        (
            EVM_BALANCE_FACT_SUBJECT_EVIDENCE_PATH,
            EVM_BALANCE_FACT_SUBJECT_EVIDENCE_ROLE,
        ),
        (
            EVM_BALANCE_FACT_RESPONSE_EVIDENCE_PATH,
            EVM_BALANCE_FACT_RESPONSE_EVIDENCE_ROLE,
        ),
        (EVM_EXECUTOR_DEPLOYMENT_PATH, EVM_EXECUTOR_DEPLOYMENT_ROLE),
        (EVM_RESOURCE_OWNERSHIP_PATH, EVM_RESOURCE_OWNERSHIP_ROLE),
        (EVM_EXECUTOR_BINDING_PATH, EVM_EXECUTOR_BINDING_ROLE),
        (
            EVM_WALLET_SIGNER_BINDING_PATH,
            EVM_WALLET_SIGNER_BINDING_ROLE,
        ),
        (EVM_WALLET_NONCE_POLICY_PATH, EVM_WALLET_NONCE_POLICY_ROLE),
        (EVM_WALLET_INITIAL_NONCE_PATH, EVM_WALLET_INITIAL_NONCE_ROLE),
        (
            EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_PATH,
            EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_ROLE,
        ),
        (
            EVM_WALLET_FINALITY_POLICY_PATH,
            EVM_WALLET_FINALITY_POLICY_ROLE,
        ),
        (
            EVM_WALLET_ASSURANCE_POLICY_PATH,
            EVM_WALLET_ASSURANCE_POLICY_ROLE,
        ),
        (
            EVM_WALLET_REQUEST_QUALIFICATION_PATH,
            EVM_WALLET_REQUEST_QUALIFICATION_ROLE,
        ),
    ];
    if expected_paths_and_roles.len() != PRODUCT_ADDITIONAL_SUPPORT_MEMBER_COUNT {
        return Err(invalid_qualification());
    }
    for (path, role) in expected_paths_and_roles {
        if support_member_at(support_members, path)?
            .value_contract()
            .role()
            .as_str()
            != role
        {
            return Err(invalid_qualification());
        }
    }

    if support_content_ref(support_member_at(
        support_members,
        STATE_IMPLEMENTATION_MANIFEST_PATH,
    )?)? != *state_manifest_ref
        || support_content_ref(support_member_at(
            support_members,
            CAPABILITY_BINDING_MANIFEST_PATH,
        )?)? != *capability_manifest_ref
        || support_content_ref(support_member_at(
            support_members,
            EVM_BALANCE_FACT_DESCRIPTOR_PATH,
        )?)? != *fact_objects.descriptor_ref()
        || support_content_ref(support_member_at(
            support_members,
            EVM_BALANCE_FACT_SUBJECT_EVIDENCE_PATH,
        )?)? != *fact_objects.subject_evidence_ref()
        || support_content_ref(support_member_at(
            support_members,
            EVM_BALANCE_FACT_RESPONSE_EVIDENCE_PATH,
        )?)? != *fact_objects.response_evidence_ref()
        || support_content_ref(support_member_at(
            support_members,
            EVM_EXECUTOR_DEPLOYMENT_PATH,
        )?)? != executor_binding
            .deployment()
            .reference()
            .map_err(|_| invalid_qualification())?
            .as_content_ref()
            .clone()
        || support_content_ref(support_member_at(
            support_members,
            EVM_RESOURCE_OWNERSHIP_PATH,
        )?)? != executor_binding
            .resource_ownership()
            .ok_or_else(invalid_qualification)?
            .reference()
            .map_err(|_| invalid_qualification())?
            .as_content_ref()
            .clone()
        || support_content_ref(support_member_at(
            support_members,
            EVM_EXECUTOR_BINDING_PATH,
        )?)? != executor_binding.binding_ref().as_content_ref().clone()
        || wallet_request_qualification.executor_binding() != executor_binding
        || wallet_request_qualification.object_evidence_contract_ref() != evidence_contract_ref
        || !wallet_support_members_match(support_members, wallet_request_qualification)?
        || support_members
            .iter()
            .filter(|member| {
                member
                    .field_path()
                    .as_str()
                    .starts_with(EVM_ROUTING_GENERATION_PATH_PREFIX)
            })
            .count()
            != generation_count
        || support_members
            .iter()
            .any(|member| member.field_path().as_str().contains("operation_catalog"))
    {
        return Err(invalid_qualification());
    }
    Ok(())
}

fn evm_routing_support_members(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
    evidence_contract_ref: ContentRef,
) -> Result<(Vec<QualifiedSupportMember>, ContentRef, ContentRef), PublicError> {
    let catalog = transport.routing_catalog_descriptor();
    let ordered_generation_refs = catalog.ordered_generation_refs();
    if ordered_generation_refs.is_empty()
        || ordered_generation_refs.len() > MAX_EVM_ROUTING_GENERATIONS
        || ordered_generation_refs
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid_qualification());
    }

    let catalog_canonical = catalog.canonical().map_err(|_| invalid_qualification())?;
    let catalog_ref = catalog.content_ref().map_err(|_| invalid_qualification())?;
    let catalog_member = support_member(
        EVM_ROUTING_CATALOG_PATH,
        catalog_canonical,
        mfm_evm_live::transport::routing_catalog_descriptor_support_contract(
            stable_id(EVM_ROUTING_CATALOG_ROLE)?,
            evidence_contract_ref.clone(),
        )
        .map_err(|_| invalid_qualification())?,
    )?;

    let mut members =
        Vec::with_capacity(EVM_LIVE_FIXED_SUPPORT_MEMBER_COUNT + ordered_generation_refs.len());
    members.push(catalog_member);
    let mut source_refs = BTreeSet::new();
    let mut descriptor_count = 0_usize;
    for (index, (generation_ref, descriptor)) in
        transport.routing_generation_descriptors().enumerate()
    {
        let Some(catalog_generation_ref) = ordered_generation_refs.get(index) else {
            return Err(invalid_qualification());
        };
        if generation_ref != catalog_generation_ref
            || descriptor
                .content_ref()
                .map_err(|_| invalid_qualification())?
                != generation_ref
                    .to_content_ref()
                    .map_err(|_| invalid_qualification())?
        {
            return Err(invalid_qualification());
        }
        source_refs.insert(descriptor.source_ref().to_owned());
        members.push(support_member(
            &evm_routing_generation_path(index)?,
            descriptor
                .canonical()
                .map_err(|_| invalid_qualification())?,
            mfm_evm_live::transport::routing_generation_descriptor_support_contract(
                stable_id(EVM_ROUTING_GENERATION_ROLE)?,
                evidence_contract_ref.clone(),
            )
            .map_err(|_| invalid_qualification())?,
        )?);
        descriptor_count += 1;
    }
    if descriptor_count != ordered_generation_refs.len() {
        return Err(invalid_qualification());
    }

    let reviewed_source_canonical = canonical_json(&ReviewedSourceScope {
        source_refs: source_refs.into_iter().collect(),
        version: EVM_REVIEWED_SOURCE_SCOPE_VERSION,
    })?;
    let reviewed_source_schema = descriptor_schema_id(EVM_REVIEWED_SOURCE_SCOPE_SCHEMA_NAME)?;
    let reviewed_source_scope_ref =
        exact_ref(reviewed_source_schema.clone(), &reviewed_source_canonical)?;
    members.push(support_member(
        EVM_REVIEWED_SOURCE_SCOPE_PATH,
        reviewed_source_canonical,
        retained_contract_in(
            reviewed_source_schema,
            "mfm.evm-live",
            EVM_REVIEWED_SOURCE_SCOPE_SEMANTIC_NAME,
            EVM_REVIEWED_SOURCE_SCOPE_ROLE,
            evidence_contract_ref,
        )?,
    )?);
    Ok((members, catalog_ref, reviewed_source_scope_ref))
}

#[allow(clippy::too_many_arguments)]
fn validate_evm_live_support_closure(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
    binding: &ReadCapabilityBinding,
    binding_ref: &ContentRef,
    admitted_implementation_ref: &ContentRef,
    evidence_contract_ref: &ContentRef,
    executor_binding: &VerifiedExecutorBinding,
    wallet_request_qualification: &mfm_evm_live::EvmWalletRequestQualification,
    support_members: &[QualifiedSupportMember],
) -> Result<(), PublicError> {
    let generation_count = transport
        .routing_catalog_descriptor()
        .ordered_generation_refs()
        .len();
    if support_members.len() != EVM_LIVE_FIXED_SUPPORT_MEMBER_COUNT + generation_count
        || support_members
            .iter()
            .map(QualifiedSupportMember::field_path)
            .collect::<BTreeSet<_>>()
            .len()
            != support_members.len()
        || support_members
            .iter()
            .any(|member| member.value_contract().evidence_contract_ref() != evidence_contract_ref)
    {
        return Err(invalid_qualification());
    }

    let expected_paths_and_roles = [
        (EVM_ROUTING_CATALOG_PATH, EVM_ROUTING_CATALOG_ROLE),
        (
            EVM_REVIEWED_SOURCE_SCOPE_PATH,
            EVM_REVIEWED_SOURCE_SCOPE_ROLE,
        ),
        (
            EVM_SAFE_FAILURE_CONTRACT_PATH,
            EVM_SAFE_FAILURE_CONTRACT_ROLE,
        ),
        (
            EVM_SAFE_FAILURE_CLASSIFIER_PATH,
            EVM_SAFE_FAILURE_CLASSIFIER_ROLE,
        ),
        (
            EVM_READ_CAPABILITY_BINDING_PATH,
            EVM_READ_CAPABILITY_BINDING_ROLE,
        ),
        (EVM_EXECUTOR_DEPLOYMENT_PATH, EVM_EXECUTOR_DEPLOYMENT_ROLE),
        (EVM_RESOURCE_OWNERSHIP_PATH, EVM_RESOURCE_OWNERSHIP_ROLE),
        (EVM_EXECUTOR_BINDING_PATH, EVM_EXECUTOR_BINDING_ROLE),
        (
            EVM_WALLET_SIGNER_BINDING_PATH,
            EVM_WALLET_SIGNER_BINDING_ROLE,
        ),
        (EVM_WALLET_NONCE_POLICY_PATH, EVM_WALLET_NONCE_POLICY_ROLE),
        (EVM_WALLET_INITIAL_NONCE_PATH, EVM_WALLET_INITIAL_NONCE_ROLE),
        (
            EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_PATH,
            EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_ROLE,
        ),
        (
            EVM_WALLET_FINALITY_POLICY_PATH,
            EVM_WALLET_FINALITY_POLICY_ROLE,
        ),
        (
            EVM_WALLET_ASSURANCE_POLICY_PATH,
            EVM_WALLET_ASSURANCE_POLICY_ROLE,
        ),
        (
            EVM_WALLET_REQUEST_QUALIFICATION_PATH,
            EVM_WALLET_REQUEST_QUALIFICATION_ROLE,
        ),
    ];
    for (path, role) in expected_paths_and_roles {
        if support_member_at(support_members, path)?
            .value_contract()
            .role()
            .as_str()
            != role
        {
            return Err(invalid_qualification());
        }
    }
    for index in 0..generation_count {
        let member = support_member_at(support_members, &evm_routing_generation_path(index)?)?;
        if member.value_contract().role().as_str() != EVM_ROUTING_GENERATION_ROLE {
            return Err(invalid_qualification());
        }
    }

    let fields = binding.fields().map_err(|_| invalid_qualification())?;
    if fields.capability_contract_ref
        != mfm_evm::evm_read_capability_contract_ref().map_err(|_| invalid_qualification())?
        || &fields.admitted_implementation_ref != admitted_implementation_ref
        || fields.safe_classifier_contract_ref
            != support_content_ref(support_member_at(
                support_members,
                EVM_SAFE_FAILURE_CLASSIFIER_PATH,
            )?)?
        || fields.safe_failure_contract_ref
            != support_content_ref(support_member_at(
                support_members,
                EVM_SAFE_FAILURE_CONTRACT_PATH,
            )?)?
        || fields.reviewed_source_scope_ref
            != support_content_ref(support_member_at(
                support_members,
                EVM_REVIEWED_SOURCE_SCOPE_PATH,
            )?)?
        || fields.routing_catalog_ref
            != support_content_ref(support_member_at(
                support_members,
                EVM_ROUTING_CATALOG_PATH,
            )?)?
        || binding_ref
            != &support_content_ref(support_member_at(
                support_members,
                EVM_READ_CAPABILITY_BINDING_PATH,
            )?)?
        || support_content_ref(support_member_at(
            support_members,
            EVM_EXECUTOR_DEPLOYMENT_PATH,
        )?)? != executor_binding
            .deployment()
            .reference()
            .map_err(|_| invalid_qualification())?
            .as_content_ref()
            .clone()
        || support_content_ref(support_member_at(
            support_members,
            EVM_RESOURCE_OWNERSHIP_PATH,
        )?)? != executor_binding
            .resource_ownership()
            .ok_or_else(invalid_qualification)?
            .reference()
            .map_err(|_| invalid_qualification())?
            .as_content_ref()
            .clone()
        || support_content_ref(support_member_at(
            support_members,
            EVM_EXECUTOR_BINDING_PATH,
        )?)? != executor_binding.binding_ref().as_content_ref().clone()
        || wallet_request_qualification.executor_binding() != executor_binding
        || wallet_request_qualification.object_evidence_contract_ref() != evidence_contract_ref
        || wallet_request_qualification.routing_catalog_ref()
            != &support_content_ref(support_member_at(
                support_members,
                EVM_ROUTING_CATALOG_PATH,
            )?)?
        || !wallet_support_members_match(support_members, wallet_request_qualification)?
    {
        return Err(invalid_qualification());
    }
    Ok(())
}

fn evm_routing_generation_path(index: usize) -> Result<String, PublicError> {
    if index >= MAX_EVM_ROUTING_GENERATIONS {
        return Err(invalid_qualification());
    }
    Ok(format!("{EVM_ROUTING_GENERATION_PATH_PREFIX}.{index:04}"))
}

/// Derives the content-dependent qualification scope for one complete support graph.
pub(super) fn qualified_support_scope_id(
    support_members: &[QualifiedSupportMember],
) -> Result<SemanticTypeId, PublicError> {
    if support_members.is_empty() {
        return Err(invalid_qualification());
    }
    let mut ordered = BTreeMap::new();
    for member in support_members {
        if ordered.insert(member.field_path(), member).is_some() {
            return Err(invalid_qualification());
        }
    }
    let members = ordered
        .into_values()
        .map(|member| {
            let contract = member.value_contract();
            Ok(QualifiedSupportScopeMember {
                field_path: member.field_path(),
                content_ref: support_content_ref(member)?,
                semantic_type_id: contract.semantic_type_id(),
                role: contract.role(),
                media_type: contract.media_type(),
                evidence_contract_ref: contract.evidence_contract_ref(),
            })
        })
        .collect::<Result<Vec<_>, PublicError>>()?;
    let canonical = canonical_json(&QualifiedSupportScopeEnvelope {
        domain: QUALIFIED_SUPPORT_SCOPE_DOMAIN,
        value: QualifiedSupportScopePreimage {
            version: QUALIFIED_SUPPORT_SCOPE_VERSION,
            members,
        },
    })?;
    SemanticTypeId::new(
        "mfm.product",
        QUALIFIED_SUPPORT_SCOPE_SEMANTIC_NAME,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(canonical.as_bytes()),
    )
    .map_err(|_| invalid_qualification())
}

fn qualified_component(
    component_kind: ComponentKind,
    semantic_contract_ref: &ContentRef,
    callback_surface_ref: &ContentRef,
) -> QualifiedComponent {
    QualifiedComponent {
        component_kind,
        semantic_contract_ref: semantic_contract_ref.clone(),
        callback_surface_ref: callback_surface_ref.clone(),
    }
}

fn qualification_descriptor(
    executable_identity_ref: &ContentRef,
    qualification_profile_ref: &ContentRef,
    components: Vec<QualifiedComponent>,
    object_evidence_contract_ref: ContentRef,
) -> Result<(QualifiedSupportMember, ContentRef), PublicError> {
    let mut canonical_components = components
        .into_iter()
        .map(|component| Ok((canonical_json(&component)?, component)))
        .collect::<Result<Vec<_>, PublicError>>()?;
    canonical_components.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
    if canonical_components
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0)
    {
        return Err(invalid_qualification());
    }
    let components = canonical_components
        .into_iter()
        .map(|(_, component)| component)
        .collect::<Vec<_>>();
    if components.len() != PRODUCT_COMPONENT_COUNT
        || components
            .iter()
            .filter(|component| component.component_kind == ComponentKind::Planner)
            .count()
            != 1
        || components
            .iter()
            .filter(|component| component.component_kind == ComponentKind::State)
            .count()
            != STATE_COMPONENT_COUNT
        || components
            .iter()
            .filter(|component| {
                component.component_kind == ComponentKind::ReadCapabilityAdapterVerifier
            })
            .count()
            != 1
    {
        return Err(invalid_qualification());
    }
    let canonical = annex_canonical(
        QUALIFICATION_CONTRACT,
        &QualificationDescriptor {
            version: QUALIFICATION_VERSION,
            executable_identity_ref,
            qualification_profile_ref,
            components: &components,
        },
    )?;
    let schema_id = annex_schema_id(QUALIFICATION_CONTRACT)?;
    let content_ref = exact_ref(schema_id.clone(), &canonical)?;
    let contract = retained_contract(
        schema_id,
        QUALIFICATION_SEMANTIC_NAME,
        QUALIFICATION_ROLE,
        object_evidence_contract_ref,
    )?;
    let member = support_member(QUALIFICATION_PATH, canonical, contract)?;
    Ok((member, content_ref))
}

fn object_evidence_contract() -> Result<(QualifiedSupportMember, ContentRef), PublicError> {
    let canonical = mfm_values::component_object_evidence_contract_canonical()
        .map_err(|_| invalid_qualification())?;
    let content_ref = mfm_values::component_object_evidence_contract_ref()
        .map_err(|_| invalid_qualification())?;
    if content_ref != exact_ref(content_ref.schema_id().clone(), &canonical)? {
        return Err(invalid_qualification());
    }
    let contract = retained_contract(
        content_ref.schema_id().clone(),
        OBJECT_EVIDENCE_SEMANTIC_NAME,
        OBJECT_EVIDENCE_ROLE,
        content_ref.clone(),
    )?;
    let member = support_member(OBJECT_EVIDENCE_PATH, canonical, contract)?;
    Ok((member, content_ref))
}

fn qualification_profile(
    object_evidence_contract_ref: ContentRef,
) -> Result<(QualifiedSupportMember, ContentRef), PublicError> {
    let canonical = annex_canonical(
        QUALIFICATION_PROFILE_CONTRACT,
        &VersionDescriptor {
            version: QUALIFICATION_PROFILE_VERSION,
        },
    )?;
    let schema_id = annex_schema_id(QUALIFICATION_PROFILE_CONTRACT)?;
    let content_ref = exact_ref(schema_id.clone(), &canonical)?;
    let contract = retained_contract(
        schema_id,
        QUALIFICATION_PROFILE_SEMANTIC_NAME,
        QUALIFICATION_PROFILE_ROLE,
        object_evidence_contract_ref,
    )?;
    let member = support_member(QUALIFICATION_PROFILE_PATH, canonical, contract)?;
    Ok((member, content_ref))
}

fn planner_surface_support_members(
    surface: &mfm_program::CompositePlannerSurface,
    object_evidence_contract_ref: ContentRef,
) -> Result<(QualifiedSupportMember, QualifiedSupportMember), PublicError> {
    let semantic_contract = retained_contract_in(
        surface.semantic_contract_ref().schema_id().clone(),
        "mfm.program",
        PLANNER_SEMANTIC_NAME,
        PLANNER_SEMANTIC_SUPPORT_ROLE,
        object_evidence_contract_ref.clone(),
    )?;
    let callback_contract = retained_contract_in(
        surface.callback_surface_ref().schema_id().clone(),
        "mfm.program",
        PLANNER_CALLBACK_SEMANTIC_NAME,
        PLANNER_CALLBACK_SUPPORT_ROLE,
        object_evidence_contract_ref,
    )?;
    Ok((
        support_member(
            PLANNER_SEMANTIC_PATH,
            surface.semantic_contract_canonical().clone(),
            semantic_contract,
        )?,
        support_member(
            PLANNER_CALLBACK_PATH,
            surface.callback_surface_canonical().clone(),
            callback_contract,
        )?,
    ))
}

fn executable_identity_member(
    expected_ref: &ContentRef,
    descriptor_bytes: &[u8],
    object_evidence_contract_ref: ContentRef,
) -> Result<QualifiedSupportMember, PublicError> {
    let canonical = annex_canonical_bytes(EXECUTABLE_DESCRIPTOR_CONTRACT, descriptor_bytes)?;
    let schema_id = annex_schema_id(EXECUTABLE_DESCRIPTOR_CONTRACT)?;
    if expected_ref != &exact_ref(schema_id.clone(), &canonical)? {
        return Err(invalid_qualification());
    }
    let contract = retained_contract_in(
        schema_id,
        "mfm.recoverability",
        EXECUTABLE_DESCRIPTOR_SEMANTIC_NAME,
        EXECUTABLE_DESCRIPTOR_ROLE,
        object_evidence_contract_ref,
    )?;
    support_member(EXECUTABLE_DESCRIPTOR_PATH, canonical, contract)
}

fn state_implementation<S>(
    surface: &S,
    qualification_ref: &ContentRef,
) -> Result<ComponentImplementationDescriptor, PublicError>
where
    S: StateCallbackSurface,
{
    component_implementation(
        ComponentKind::State,
        surface.state_contract_ref().clone(),
        surface.callback_surface_ref().clone(),
        qualification_ref,
    )
}

trait StateCallbackSurface {
    fn state_contract_ref(&self) -> &ContentRef;
    fn state_contract_canonical(&self) -> &PlainCanonicalJsonBytes;
    fn callback_surface_ref(&self) -> &ContentRef;
    fn callback_surface_canonical(&self) -> &PlainCanonicalJsonBytes;
    fn state_contract_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract, PublicError>;
    fn callback_surface_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract, PublicError>;
}

impl StateCallbackSurface for mfm_portfolio::PortfolioStateCallbackSurface {
    fn state_contract_ref(&self) -> &ContentRef {
        self.state_contract_ref()
    }

    fn state_contract_canonical(&self) -> &PlainCanonicalJsonBytes {
        self.state_contract_canonical()
    }

    fn callback_surface_ref(&self) -> &ContentRef {
        self.content_ref()
    }

    fn callback_surface_canonical(&self) -> &PlainCanonicalJsonBytes {
        self.canonical()
    }

    fn state_contract_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract, PublicError> {
        self.state_contract_support_contract(role, evidence_contract_ref)
            .map_err(|_| invalid_qualification())
    }

    fn callback_surface_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract, PublicError> {
        self.support_contract(role, evidence_contract_ref)
            .map_err(|_| invalid_qualification())
    }
}

impl StateCallbackSurface for mfm_evm::EvmStateCallbackSurface {
    fn state_contract_ref(&self) -> &ContentRef {
        self.state_contract_ref()
    }

    fn state_contract_canonical(&self) -> &PlainCanonicalJsonBytes {
        self.state_contract_canonical()
    }

    fn callback_surface_ref(&self) -> &ContentRef {
        self.content_ref()
    }

    fn callback_surface_canonical(&self) -> &PlainCanonicalJsonBytes {
        self.canonical()
    }

    fn state_contract_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract, PublicError> {
        self.state_contract_support_contract(role, evidence_contract_ref)
            .map_err(|_| invalid_qualification())
    }

    fn callback_surface_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract, PublicError> {
        self.support_contract(role, evidence_contract_ref)
            .map_err(|_| invalid_qualification())
    }
}

fn state_surface_support_members<S>(
    surface: &S,
    paths: ComponentSupportPaths,
    object_evidence_contract_ref: ContentRef,
) -> Result<(QualifiedSupportMember, QualifiedSupportMember), PublicError>
where
    S: StateCallbackSurface,
{
    let semantic_member = support_member(
        paths.semantic_path,
        surface.state_contract_canonical().clone(),
        surface.state_contract_support_contract(
            stable_id(paths.semantic_role)?,
            object_evidence_contract_ref.clone(),
        )?,
    )?;
    let callback_member = support_member(
        paths.callback_path,
        surface.callback_surface_canonical().clone(),
        surface.callback_surface_support_contract(
            stable_id(paths.callback_role)?,
            object_evidence_contract_ref,
        )?,
    )?;
    Ok((semantic_member, callback_member))
}

fn evm_read_adapter_surface_support_members(
    object_evidence_contract_ref: ContentRef,
) -> Result<(QualifiedSupportMember, QualifiedSupportMember), PublicError> {
    let semantic_member = support_member(
        EVM_READ_ADAPTER_SUPPORT_PATHS.semantic_path,
        mfm_evm::evm_read_capability_contract_canonical().map_err(|_| invalid_qualification())?,
        mfm_evm::evm_read_capability_support_contract(
            stable_id(EVM_READ_ADAPTER_SUPPORT_PATHS.semantic_role)?,
            object_evidence_contract_ref.clone(),
        )
        .map_err(|_| invalid_qualification())?,
    )?;
    let callback_member = support_member(
        EVM_READ_ADAPTER_SUPPORT_PATHS.callback_path,
        mfm_evm_live::evm_adapter_callback_surface_canonical()
            .map_err(|_| invalid_qualification())?,
        mfm_evm_live::evm_adapter_callback_surface_support_contract(
            stable_id(EVM_READ_ADAPTER_SUPPORT_PATHS.callback_role)?,
            object_evidence_contract_ref,
        )
        .map_err(|_| invalid_qualification())?,
    )?;
    Ok((semantic_member, callback_member))
}

fn evm_wallet_executor_surface_support_members(
    executor_contract: &ExecutorContractDescriptor,
    object_evidence_contract_ref: ContentRef,
) -> Result<(QualifiedSupportMember, QualifiedSupportMember), PublicError> {
    let validated = executor_contract
        .validated()
        .map_err(|_| invalid_qualification())?;
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(validated.as_bytes())
        .map_err(|_| invalid_qualification())?;
    let semantic_member = support_member(
        EVM_WALLET_EXECUTOR_SUPPORT_PATHS.semantic_path,
        canonical,
        retained_contract(
            validated.schema_id().clone(),
            "evm-wallet-executor-contract",
            EVM_WALLET_EXECUTOR_SUPPORT_PATHS.semantic_role,
            object_evidence_contract_ref.clone(),
        )?,
    )?;
    let callback_member = support_member(
        EVM_WALLET_EXECUTOR_SUPPORT_PATHS.callback_path,
        mfm_evm_live::evm_wallet_target_callback_surface_canonical()
            .map_err(|_| invalid_qualification())?,
        mfm_evm_live::evm_wallet_target_callback_surface_support_contract(
            stable_id(EVM_WALLET_EXECUTOR_SUPPORT_PATHS.callback_role)?,
            object_evidence_contract_ref,
        )
        .map_err(|_| invalid_qualification())?,
    )?;
    Ok((semantic_member, callback_member))
}

fn wallet_support_member(
    path: &str,
    role: &str,
    semantic_namespace: &str,
    semantic_name: &str,
    canonical: PlainCanonicalJsonBytes,
    expected_ref: ContentRef,
    object_evidence_contract_ref: ContentRef,
) -> Result<QualifiedSupportMember, PublicError> {
    let member = support_member(
        path,
        canonical,
        retained_contract_in(
            expected_ref.schema_id().clone(),
            semantic_namespace,
            semantic_name,
            role,
            object_evidence_contract_ref,
        )?,
    )?;
    if support_content_ref(&member)? != expected_ref {
        return Err(invalid_qualification());
    }
    Ok(member)
}

fn wallet_support_members_match(
    support_members: &[QualifiedSupportMember],
    qualification: &mfm_evm_live::EvmWalletRequestQualification,
) -> Result<bool, PublicError> {
    Ok(support_content_ref(support_member_at(
        support_members,
        EVM_WALLET_SIGNER_BINDING_PATH,
    )?)? == *qualification.signer_descriptor().reference()
        && support_content_ref(support_member_at(
            support_members,
            EVM_WALLET_NONCE_POLICY_PATH,
        )?)? == *qualification.resource_policy_binding().policy_ref()
        && support_content_ref(support_member_at(
            support_members,
            EVM_WALLET_INITIAL_NONCE_PATH,
        )?)? == *qualification
            .resource_policy_binding()
            .policy_configuration_ref()
        && support_content_ref(support_member_at(
            support_members,
            EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_PATH,
        )?)? == qualification
            .already_known_classifier_ref()
            .to_content_ref()
            .map_err(|_| invalid_qualification())?
        && support_content_ref(support_member_at(
            support_members,
            EVM_WALLET_FINALITY_POLICY_PATH,
        )?)? == qualification
            .finality_policy_ref()
            .to_content_ref()
            .map_err(|_| invalid_qualification())?
        && support_content_ref(support_member_at(
            support_members,
            EVM_WALLET_ASSURANCE_POLICY_PATH,
        )?)? == qualification
            .assurance_policy_ref()
            .to_content_ref()
            .map_err(|_| invalid_qualification())?
        && support_content_ref(support_member_at(
            support_members,
            EVM_WALLET_REQUEST_QUALIFICATION_PATH,
        )?)? == *qualification.reference())
}

fn executor_support_member(
    path: &str,
    role: &str,
    semantic_name: &str,
    validated: ValidatedCanonicalValueV1,
    object_evidence_contract_ref: ContentRef,
) -> Result<QualifiedSupportMember, PublicError> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(validated.as_bytes())
        .map_err(|_| invalid_qualification())?;
    support_member(
        path,
        canonical,
        retained_contract(
            validated.schema_id().clone(),
            semantic_name,
            role,
            object_evidence_contract_ref,
        )?,
    )
}

fn component_implementation(
    component_kind: ComponentKind,
    semantic_contract_ref: ContentRef,
    callback_surface_ref: ContentRef,
    qualification_ref: &ContentRef,
) -> Result<ComponentImplementationDescriptor, PublicError> {
    ComponentImplementationDescriptor::new(
        component_kind,
        semantic_contract_ref,
        callback_surface_ref,
        qualification_ref.clone(),
    )
    .map_err(|_| invalid_qualification())
}

fn component_implementation_member(
    descriptor: &ComponentImplementationDescriptor,
    role: &str,
    path: &str,
    object_evidence_contract_ref: ContentRef,
) -> Result<QualifiedSupportMember, PublicError> {
    let canonical = descriptor
        .canonical_json()
        .map_err(|_| invalid_qualification())?;
    let schema_id = descriptor
        .content_ref()
        .map_err(|_| invalid_qualification())?
        .schema_id()
        .clone();
    let contract = retained_contract(
        schema_id,
        COMPONENT_IMPLEMENTATION_SEMANTIC_NAME,
        role,
        object_evidence_contract_ref,
    )?;
    support_member(path, canonical, contract)
}

fn validate_component_support_closure(
    components: &[QualifiedComponent],
    implementations: &ProductComponentImplementations,
    qualification_ref: &ContentRef,
    support_members: &[QualifiedSupportMember],
) -> Result<(), PublicError> {
    if components.len() != PRODUCT_COMPONENT_COUNT
        || support_members.len() != PRODUCT_SUPPORT_MEMBER_COUNT
        || support_members
            .iter()
            .map(|member| member.field_path().as_str())
            .collect::<BTreeSet<_>>()
            .len()
            != support_members.len()
    {
        return Err(invalid_qualification());
    }
    for path in [
        OBJECT_EVIDENCE_PATH,
        QUALIFICATION_PROFILE_PATH,
        QUALIFICATION_PATH,
        EXECUTABLE_DESCRIPTOR_PATH,
    ] {
        support_member_at(support_members, path)?;
    }

    for ((component, descriptor), paths) in components
        .iter()
        .zip(implementations.ordered())
        .zip(component_support_paths())
    {
        let expected_kind = component.component_kind;
        let semantic_member = support_member_at(support_members, paths.semantic_path)?;
        let callback_member = support_member_at(support_members, paths.callback_path)?;
        let implementation_member = support_member_at(support_members, paths.implementation_path)?;
        let semantic_ref = support_content_ref(semantic_member)?;
        let callback_ref = support_content_ref(callback_member)?;
        let implementation_ref = support_content_ref(implementation_member)?;
        let descriptor_ref = descriptor
            .content_ref()
            .map_err(|_| invalid_qualification())?;
        if component.component_kind != expected_kind
            || descriptor.component_kind() != expected_kind
            || descriptor.semantic_contract_ref() != &component.semantic_contract_ref
            || descriptor.callback_surface_ref() != &component.callback_surface_ref
            || descriptor.qualification_ref() != qualification_ref
            || semantic_ref != component.semantic_contract_ref
            || callback_ref != component.callback_surface_ref
            || implementation_ref != descriptor_ref
            || semantic_member.value_contract().role().as_str() != paths.semantic_role
            || callback_member.value_contract().role().as_str() != paths.callback_role
            || implementation_member.value_contract().role().as_str() != paths.implementation_role
        {
            return Err(invalid_qualification());
        }
    }
    Ok(())
}

fn support_member_at<'a>(
    support_members: &'a [QualifiedSupportMember],
    path: &str,
) -> Result<&'a QualifiedSupportMember, PublicError> {
    support_members
        .iter()
        .find(|member| member.field_path().as_str() == path)
        .ok_or_else(invalid_qualification)
}

fn support_content_ref(member: &QualifiedSupportMember) -> Result<ContentRef, PublicError> {
    exact_ref(
        member.value_contract().schema_id().clone(),
        member.canonical(),
    )
}

fn support_member(
    path: &str,
    canonical: PlainCanonicalJsonBytes,
    value_contract: RetainedValueContract,
) -> Result<QualifiedSupportMember, PublicError> {
    let field_path = FieldPath::new(path).map_err(|_| invalid_qualification())?;
    Ok(QualifiedSupportMember::new(
        field_path,
        canonical,
        value_contract,
    ))
}

fn retained_contract(
    schema_id: SchemaId,
    semantic_name: &str,
    role: &str,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract, PublicError> {
    retained_contract_in(
        schema_id,
        "mfm.product",
        semantic_name,
        role,
        evidence_contract_ref,
    )
}

fn retained_contract_in(
    schema_id: SchemaId,
    semantic_namespace: &str,
    semantic_name: &str,
    role: &str,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract, PublicError> {
    RetainedValueContract::new(
        schema_id,
        semantic_type_id(semantic_namespace, semantic_name)?,
        StableId::new(role).map_err(|_| invalid_qualification())?,
        MEDIA_TYPE_JSON,
        evidence_contract_ref,
    )
    .map_err(|_| invalid_qualification())
}

fn stable_id(raw: &str) -> Result<StableId, PublicError> {
    StableId::new(raw).map_err(|_| invalid_qualification())
}

fn semantic_type_id(namespace: &str, name: &str) -> Result<SemanticTypeId, PublicError> {
    SemanticTypeId::new(
        namespace,
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:{namespace}:{name}:1").as_bytes()),
    )
    .map_err(|_| invalid_qualification())
}

fn annex_schema_id(contract: &str) -> Result<SchemaId, PublicError> {
    mfm_spec::schema_id(contract).map_err(|_| invalid_qualification())
}

fn descriptor_schema_id(name: &str) -> Result<SchemaId, PublicError> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:1").as_bytes()),
    )
    .map_err(|_| invalid_qualification())
}

fn exact_ref(
    schema_id: SchemaId,
    canonical: &PlainCanonicalJsonBytes,
) -> Result<ContentRef, PublicError> {
    exact_content_ref(schema_id, canonical).map_err(|_| invalid_qualification())
}

fn canonical_json(value: &impl Serialize) -> Result<PlainCanonicalJsonBytes, PublicError> {
    let json = serde_json::to_string(value).map_err(|_| invalid_qualification())?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| invalid_qualification())
}

fn annex_canonical(
    contract: &str,
    value: &impl Serialize,
) -> Result<PlainCanonicalJsonBytes, PublicError> {
    let canonical = canonical_json(value)?;
    annex_canonical_bytes(contract, canonical.as_bytes())
}

fn annex_canonical_bytes(
    contract: &str,
    bytes: &[u8],
) -> Result<PlainCanonicalJsonBytes, PublicError> {
    let annex = RecoverabilityContractV1::embedded().map_err(|_| invalid_qualification())?;
    let validated = annex
        .strict_decode(contract, bytes)
        .map_err(|_| invalid_qualification())?;
    PlainCanonicalJsonBytes::from_canonical_json_slice(validated.as_bytes())
        .map_err(|_| invalid_qualification())
}

fn invalid_qualification() -> PublicError {
    PublicError::internal(
        "ProductionQualificationInvalid",
        "The production component qualification is invalid",
    )
}

#[cfg(test)]
#[path = "qualification_tests.rs"]
mod tests;
