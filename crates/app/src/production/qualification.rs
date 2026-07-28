use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContractV1};
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
const PRODUCT_COMPONENT_COUNT: usize = 12;
const STATE_COMPONENT_COUNT: usize = 10;
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
const EVM_LIVE_FIXED_SUPPORT_MEMBER_COUNT: usize = 5;

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
const PRODUCT_DEPLOYMENT_SUPPORT_MEMBER_COUNT_WITHOUT_GENERATIONS: usize = 52;
const PRODUCT_ADDITIONAL_SUPPORT_MEMBER_COUNT: usize = 7;

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
];

const EVM_READ_ADAPTER_SUPPORT_PATHS: ComponentSupportPaths = ComponentSupportPaths {
    semantic_path: EVM_READ_ADAPTER_SEMANTIC_PATH,
    semantic_role: EVM_READ_ADAPTER_SEMANTIC_ROLE,
    callback_path: EVM_READ_ADAPTER_CALLBACK_PATH,
    callback_role: EVM_READ_ADAPTER_CALLBACK_ROLE,
    implementation_path: EVM_READ_ADAPTER_PATH,
    implementation_role: EVM_READ_ADAPTER_ROLE,
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
        EVM_READ_ADAPTER_SUPPORT_PATHS,
    ]
}

/// Exact implementation descriptors selected by the sole product package.
pub(super) struct ProductComponentImplementations {
    pub(super) planner: ComponentImplementationDescriptor,
    pub(super) portfolio: mfm_portfolio::PortfolioSnapshotStateImplementations,
    pub(super) evm: mfm_evm::EvmBalanceCollectionStateImplementations,
    pub(super) evm_read_adapter: ComponentImplementationDescriptor,
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
            &self.evm_read_adapter,
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

/// Builds the one shared executable-bound qualification and all twelve implementation
/// descriptors.
///
/// No implementation reference enters the qualification preimage, which keeps the graph acyclic.
pub(super) fn qualify_product_components(
    executable_identity_ref: ContentRef,
    executable_descriptor_bytes: &[u8],
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
            ComponentKind::ReadCapabilityAdapterVerifier,
            &evm_read_capability_ref,
            &evm_read_adapter_callback_ref,
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
    let implementations = ProductComponentImplementations {
        planner,
        portfolio,
        evm,
        evm_read_adapter,
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
        .zip(STATE_SUPPORT_PATHS[3..].iter().copied())
    {
        surface_members.push(state_surface_support_members(
            surface,
            paths,
            object_evidence_contract_ref.clone(),
        )?);
    }
    surface_members.push(evm_read_adapter_surface_support_members(
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
pub(super) fn qualify_evm_live_support(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
    product: &ProductQualification,
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

    support_members.extend([safe_failure_member, safe_classifier_member, binding_member]);
    validate_evm_live_support_closure(
        transport,
        &read_capability_binding,
        &read_capability_binding_ref,
        &admitted_implementation_ref,
        &evidence_contract_ref,
        &support_members,
    )?;

    Ok(EvmLiveQualification {
        read_capability_binding,
        read_capability_binding_ref,
        support_members,
    })
}

/// Assembles the exact `52 + N` product support graph and derives its content-bound scope.
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

    let capability_manifest = product_capability_manifest(&live.read_capability_binding_ref)?;
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
    binding_ref: &ContentRef,
) -> Result<CapabilityBindingManifest, PublicError> {
    let entries = mfm_evm::EVM_READ_OPERATION_IDS
        .into_iter()
        .map(|operation_id| {
            Ok(CapabilityBindingManifestEntry {
                operation_id: stable_id(operation_id)?,
                binding_ref: binding_ref.clone(),
            })
        })
        .collect::<Result<Vec<_>, PublicError>>()?;
    CapabilityBindingManifest::new(entries).map_err(|_| invalid_qualification())
}

fn validate_product_deployment_support_closure(
    support_members: &[QualifiedSupportMember],
    generation_count: usize,
    evidence_contract_ref: &ContentRef,
    state_manifest_ref: &ContentRef,
    capability_manifest_ref: &ContentRef,
    fact_objects: &mfm_evm::EvmBalanceFactSupportObjects,
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

fn validate_evm_live_support_closure(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
    binding: &ReadCapabilityBinding,
    binding_ref: &ContentRef,
    admitted_implementation_ref: &ContentRef,
    evidence_contract_ref: &ContentRef,
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

    for (index, ((component, descriptor), paths)) in components
        .iter()
        .zip(implementations.ordered())
        .zip(component_support_paths())
        .enumerate()
    {
        let expected_kind = if index == 0 {
            ComponentKind::Planner
        } else if index == PRODUCT_COMPONENT_COUNT - 1 {
            ComponentKind::ReadCapabilityAdapterVerifier
        } else {
            ComponentKind::State
        };
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
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn frozen_foundation_objects_keep_exact_bytes_and_identities() {
        let (evidence, evidence_ref) = object_evidence_contract().expect("evidence contract");
        assert_eq!(
            evidence.canonical(),
            &mfm_values::component_object_evidence_contract_canonical()
                .expect("shared evidence canonical")
        );
        assert_eq!(
            evidence_ref,
            mfm_values::component_object_evidence_contract_ref().expect("shared evidence ref")
        );
        assert_eq!(
            evidence.value_contract().semantic_type_id().as_str(),
            "semantic:mfm.product:component-object-evidence-contract:1:sha256-jcs-v1:4b996fbc61b28b6cb505cad6fe32a7f87f39b20ba3275306c31df064d36b57b6"
        );
        assert_eq!(
            evidence.value_contract().evidence_contract_ref(),
            &evidence_ref
        );

        let (profile, profile_ref) =
            qualification_profile(evidence_ref.clone()).expect("qualification profile");
        assert_eq!(
            profile.canonical().as_str(),
            r#"{"version":"mfm.component-qualification-profile.v1"}"#
        );
        assert_eq!(
            profile_ref.schema_id(),
            &annex_schema_id(QUALIFICATION_PROFILE_CONTRACT).expect("profile schema")
        );
        assert_eq!(
            profile_ref.content_digest().as_str(),
            "content:sha256-v1:e384b0ed353d7e9e115d7238feae1b6517bbd5d2cfbbe88ddb655633afa03d30"
        );
        assert_eq!(
            profile.value_contract().semantic_type_id().as_str(),
            "semantic:mfm.product:component-qualification-profile:1:sha256-jcs-v1:c15789545f01147d9c45488411d4a14a5b3cafb5150fe75f82a37e7a1dda1abd"
        );
        assert_eq!(
            profile.value_contract().evidence_contract_ref(),
            &evidence_ref
        );

        let planner_surface =
            mfm_program::CompositePlannerSurface::current().expect("planner surface");
        let (planner_contract, planner_callbacks) =
            planner_surface_support_members(&planner_surface, evidence_ref.clone())
                .expect("planner support members");
        assert_eq!(
            planner_contract.field_path().as_str(),
            PLANNER_SEMANTIC_PATH
        );
        assert_eq!(
            planner_contract.value_contract().role().as_str(),
            PLANNER_SEMANTIC_SUPPORT_ROLE
        );
        assert_eq!(
            planner_contract.canonical(),
            planner_surface.semantic_contract_canonical()
        );
        assert_eq!(
            support_content_ref(&planner_contract).expect("semantic support ref"),
            *planner_surface.semantic_contract_ref()
        );
        assert_eq!(
            planner_contract.value_contract().evidence_contract_ref(),
            &evidence_ref
        );

        assert_eq!(
            planner_callbacks.field_path().as_str(),
            PLANNER_CALLBACK_PATH
        );
        assert_eq!(
            planner_callbacks.value_contract().role().as_str(),
            PLANNER_CALLBACK_SUPPORT_ROLE
        );
        assert_eq!(
            planner_callbacks.canonical(),
            planner_surface.callback_surface_canonical()
        );
        assert_eq!(
            support_content_ref(&planner_callbacks).expect("callback support ref"),
            *planner_surface.callback_surface_ref()
        );
        assert_eq!(
            planner_callbacks.value_contract().evidence_contract_ref(),
            &evidence_ref
        );
    }

    #[test]
    fn product_qualification_is_one_acyclic_twelve_component_bijection() {
        let (executable, executable_bytes) = test_executable();
        let product = qualify_product_components(executable.clone(), executable_bytes.as_bytes())
            .expect("product qualification");

        assert_eq!(
            product.implementations.ordered().len(),
            PRODUCT_COMPONENT_COUNT
        );
        assert!(product
            .implementations
            .ordered()
            .iter()
            .all(|descriptor| descriptor.qualification_ref() == &product.qualification_ref));
        assert_eq!(product.support_members.len(), PRODUCT_SUPPORT_MEMBER_COUNT);

        let qualification_member = product
            .support_members
            .iter()
            .find(|member| member.field_path().as_str() == QUALIFICATION_PATH)
            .expect("qualification member");
        let decoded: Value = serde_json::from_slice(qualification_member.canonical().as_bytes())
            .expect("qualification JSON");
        assert_eq!(decoded["version"], QUALIFICATION_VERSION);
        assert_eq!(
            decoded["executable_identity_ref"],
            serde_json::to_value(executable).expect("executable ref JSON")
        );
        let components = decoded["components"]
            .as_array()
            .expect("component inventory");
        assert_eq!(components.len(), PRODUCT_COMPONENT_COUNT);
        assert_eq!(
            components
                .iter()
                .filter(|component| component["component_kind"] == "planner")
                .count(),
            1
        );
        assert_eq!(
            components
                .iter()
                .filter(|component| component["component_kind"] == "state")
                .count(),
            STATE_COMPONENT_COUNT
        );
        assert_eq!(
            components
                .iter()
                .filter(|component| {
                    component["component_kind"] == "read_capability_adapter_verifier"
                })
                .count(),
            1
        );
        assert!(!qualification_member
            .canonical()
            .as_str()
            .contains("implementation"));
    }

    #[test]
    fn component_support_closure_paths_and_roles_are_frozen() {
        let (executable, executable_bytes) = test_executable();
        let product = qualify_product_components(executable, executable_bytes.as_bytes())
            .expect("product qualification");
        let paths = component_support_paths();
        let expected_component_paths = paths
            .iter()
            .flat_map(|paths| {
                [
                    paths.semantic_path,
                    paths.callback_path,
                    paths.implementation_path,
                ]
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            expected_component_paths.len(),
            COMPONENT_SUPPORT_MEMBER_COUNT
        );
        let actual_component_paths = product
            .support_members
            .iter()
            .map(|member| member.field_path().as_str())
            .filter(|path| expected_component_paths.contains(path))
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_component_paths, expected_component_paths);

        for paths in paths {
            let semantic_member = support_member_at(&product.support_members, paths.semantic_path)
                .expect("semantic support member");
            let callback_member = support_member_at(&product.support_members, paths.callback_path)
                .expect("callback support member");
            let implementation_member =
                support_member_at(&product.support_members, paths.implementation_path)
                    .expect("implementation support member");
            assert_eq!(
                semantic_member.value_contract().role().as_str(),
                paths.semantic_role
            );
            assert_eq!(
                callback_member.value_contract().role().as_str(),
                paths.callback_role
            );
            assert_eq!(
                implementation_member.value_contract().role().as_str(),
                paths.implementation_role
            );
            assert_eq!(
                implementation_member
                    .value_contract()
                    .semantic_type_id()
                    .as_str(),
                "semantic:mfm.product:component-implementation-descriptor:1:sha256-jcs-v1:2fe4bed438b858b7f0b52da9263a0c69ddf580092d5a116155e853b0e24252dc"
            );
        }
    }

    #[test]
    fn evm_live_support_keeps_exact_catalog_order_paths_roles_and_sources() {
        let transport = test_evm_transport();
        let evidence_ref =
            mfm_values::component_object_evidence_contract_ref().expect("evidence ref");
        let (members, catalog_ref, reviewed_source_ref) =
            evm_routing_support_members(&transport, evidence_ref.clone()).expect("routing support");

        assert_eq!(members.len(), 4);
        assert_eq!(
            support_content_ref(
                support_member_at(&members, EVM_ROUTING_CATALOG_PATH).expect("catalog member")
            )
            .expect("catalog ref"),
            catalog_ref
        );
        assert_eq!(
            support_member_at(&members, EVM_ROUTING_CATALOG_PATH)
                .expect("catalog member")
                .value_contract()
                .role()
                .as_str(),
            EVM_ROUTING_CATALOG_ROLE
        );

        for (index, (_, descriptor)) in transport.routing_generation_descriptors().enumerate() {
            let member = support_member_at(
                &members,
                &evm_routing_generation_path(index).expect("generation path"),
            )
            .expect("generation member");
            assert_eq!(
                support_content_ref(member).expect("generation ref"),
                descriptor.content_ref().expect("descriptor ref")
            );
            assert_eq!(
                member.value_contract().role().as_str(),
                EVM_ROUTING_GENERATION_ROLE
            );
            assert_eq!(
                member.value_contract().evidence_contract_ref(),
                &evidence_ref
            );
        }

        let reviewed =
            support_member_at(&members, EVM_REVIEWED_SOURCE_SCOPE_PATH).expect("reviewed scope");
        assert_eq!(
            reviewed.canonical().as_str(),
            r#"{"source_refs":["primary","secondary"],"version":"mfm.evm-live.reviewed-source-scope.v1"}"#
        );
        assert_eq!(
            support_content_ref(reviewed).expect("reviewed source ref"),
            reviewed_source_ref
        );
        assert_eq!(
            reviewed.value_contract().role().as_str(),
            EVM_REVIEWED_SOURCE_SCOPE_ROLE
        );
        assert!(members
            .iter()
            .all(|member| !member.field_path().as_str().contains("operation_catalog")));
        for member in &members {
            for forbidden in [
                "127.0.0.1",
                "sentinel-secret",
                "authorization",
                "endpoint",
                "rpc_url",
            ] {
                assert!(
                    !member.canonical().as_str().contains(forbidden),
                    "support member {} exposed {forbidden}",
                    member.field_path().as_str()
                );
            }
        }
        assert_eq!(
            evm_routing_generation_path(MAX_EVM_ROUTING_GENERATIONS - 1)
                .expect("last generation path"),
            "capability.evm_live.routing_generation.4095"
        );
        assert!(evm_routing_generation_path(MAX_EVM_ROUTING_GENERATIONS).is_err());
    }

    #[test]
    fn evm_live_binding_closes_over_the_one_fixed_adapter_and_dynamic_contracts() {
        let (executable, executable_bytes) = test_executable();
        let product = qualify_product_components(executable, executable_bytes.as_bytes())
            .expect("product qualification");
        let transport = test_evm_transport();
        let live = qualify_evm_live_support(&transport, &product).expect("live EVM qualification");

        assert_eq!(
            live.support_members.len(),
            EVM_LIVE_FIXED_SUPPORT_MEMBER_COUNT + 2
        );
        let fields = live
            .read_capability_binding
            .fields()
            .expect("binding fields");
        assert_eq!(
            fields.capability_contract_ref,
            mfm_evm::evm_read_capability_contract_ref().expect("capability ref")
        );
        assert_eq!(
            fields.admitted_implementation_ref,
            product
                .implementations
                .evm_read_adapter
                .content_ref()
                .expect("adapter implementation ref")
        );
        assert_eq!(
            support_content_ref(
                support_member_at(&live.support_members, EVM_READ_CAPABILITY_BINDING_PATH)
                    .expect("binding member")
            )
            .expect("binding ref"),
            live.read_capability_binding_ref
        );
        assert_eq!(
            support_member_at(&live.support_members, EVM_READ_CAPABILITY_BINDING_PATH)
                .expect("binding member")
                .value_contract()
                .role()
                .as_str(),
            EVM_READ_CAPABILITY_BINDING_ROLE
        );
    }

    #[test]
    fn support_scope_is_order_independent_and_binds_every_member_tuple() {
        let (executable, executable_bytes) = test_executable();
        let product = qualify_product_components(executable, executable_bytes.as_bytes())
            .expect("product qualification");
        let live = qualify_evm_live_support(&test_evm_transport(), &product)
            .expect("live EVM qualification");
        let mut members = product.support_members.clone();
        members.extend(live.support_members);

        let expected = qualified_support_scope_id(&members).expect("support scope");
        members.reverse();
        assert_eq!(
            qualified_support_scope_id(&members).expect("reordered support scope"),
            expected
        );

        let changed_path = members[0].field_path().clone();
        let changed_contract = members[0].value_contract().clone();
        members[0] = QualifiedSupportMember::new(
            changed_path,
            PlainCanonicalJsonBytes::from_json_str("{}").expect("changed canonical"),
            changed_contract,
        );
        assert_ne!(
            qualified_support_scope_id(&members).expect("changed support scope"),
            expected
        );

        members.push(QualifiedSupportMember::new(
            members[0].field_path().clone(),
            members[0].canonical().clone(),
            members[0].value_contract().clone(),
        ));
        assert!(qualified_support_scope_id(&members).is_err());
    }

    #[test]
    fn product_deployment_graph_has_exact_fifty_two_plus_generation_closure() {
        let (executable, executable_bytes) = test_executable();
        let product = qualify_product_components(executable, executable_bytes.as_bytes())
            .expect("product qualification");
        let transport = test_evm_transport();
        let routing_manifest = test_routing_manifest(&transport);
        let live = qualify_evm_live_support(&transport, &product).expect("live qualification");
        let deployment = assemble_qualified_product_deployment(product, live, &routing_manifest)
            .expect("product deployment");

        assert_eq!(
            deployment.support_graph.members().len(),
            PRODUCT_DEPLOYMENT_SUPPORT_MEMBER_COUNT_WITHOUT_GENERATIONS + 2
        );
        assert_eq!(
            deployment.state_manifest.entries().len(),
            STATE_COMPONENT_COUNT
        );
        assert_eq!(
            deployment.capability_manifest.entries().len(),
            mfm_evm::EVM_READ_OPERATION_IDS.len()
        );
        assert!(deployment
            .capability_manifest
            .entries()
            .iter()
            .all(|entry| entry.binding_ref == deployment.read_capability_binding_ref));
        assert_eq!(
            support_content_ref(
                deployment
                    .support_graph
                    .members()
                    .get(
                        &FieldPath::new(STATE_IMPLEMENTATION_MANIFEST_PATH)
                            .expect("state manifest path")
                    )
                    .expect("state manifest member")
            )
            .expect("state manifest ref"),
            deployment.state_manifest_ref
        );
        assert_eq!(
            support_content_ref(
                deployment
                    .support_graph
                    .members()
                    .get(
                        &FieldPath::new(CAPABILITY_BINDING_MANIFEST_PATH)
                            .expect("capability manifest path")
                    )
                    .expect("capability manifest member")
            )
            .expect("capability manifest ref"),
            deployment.capability_manifest_ref
        );
        assert!(deployment
            .support_graph
            .members()
            .values()
            .all(|member| member.value_contract().evidence_contract_ref()
                == &deployment.object_evidence_contract_ref));
    }

    fn test_executable() -> (ContentRef, PlainCanonicalJsonBytes) {
        let canonical = annex_canonical_bytes(
            EXECUTABLE_DESCRIPTOR_CONTRACT,
            br#"{"contract":"mfm.executable-bytes.v1","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
        )
        .expect("test executable descriptor");
        let reference = exact_ref(
            annex_schema_id(EXECUTABLE_DESCRIPTOR_CONTRACT).expect("executable schema"),
            &canonical,
        )
        .expect("test executable ref");
        (reference, canonical)
    }

    fn test_evm_transport() -> mfm_evm_live::transport::EvmJsonRpcTransport {
        use mfm_evm_live::transport::{
            EvmJsonRpcTransport, EvmRoutingCatalogBuilder, EvmRpcAuthorization, EvmRpcEndpoint,
        };
        use zeroize::Zeroizing;

        let mut catalog = EvmRoutingCatalogBuilder::new();
        catalog
            .insert(
                "z-network",
                "secondary",
                2,
                StableId::new("generation-secondary").expect("secondary generation"),
                EvmRpcEndpoint::new("http://127.0.0.1:9").expect("secondary endpoint"),
                None,
            )
            .expect("secondary route");
        catalog
            .insert(
                "a-network",
                "primary",
                1,
                StableId::new("generation-primary").expect("primary generation"),
                EvmRpcEndpoint::new("http://127.0.0.1:9").expect("primary endpoint"),
                Some(
                    EvmRpcAuthorization::new(Zeroizing::new("Bearer sentinel-secret".to_owned()))
                        .expect("primary authorization"),
                ),
            )
            .expect("primary route");
        EvmJsonRpcTransport::new(catalog.build().expect("catalog")).expect("transport")
    }

    fn test_routing_manifest(
        transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
    ) -> mfm_portfolio::PortfolioRoutingManifest {
        let bindings = transport
            .routing_generation_descriptors()
            .map(|(generation_ref, descriptor)| {
                mfm_portfolio::EvmRoutingBinding::new(
                    descriptor.network_id(),
                    generation_ref.clone(),
                )
                .expect("routing binding")
            })
            .collect();
        mfm_portfolio::PortfolioRoutingManifest::new(bindings).expect("routing manifest")
    }
}
