use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, DigestAlgorithm, EntryPointId, SemanticTypeId, StableId};
use mfm_spec::{
    exact_content_ref, schema_id, CanonicalJsonValue, EntryPointContract, PlanningProfile,
    RetainedValueContract,
};

use crate::callbacks::CandidateStateCallbackError;

use super::{
    classify_candidate_plan_compatibility, validate_component_descriptors,
    CandidateCertificationError, CandidateCertificationErrorKind, CandidatePlanCompatibility,
    CompositePlannerSurface, QualifiedCertificationCallback, QualifiedCertificationFactory,
    QualifiedPlannerRegistration, QualifiedProgramDefinition,
};
use crate::ProgramError;

#[test]
fn component_descriptor_validation_does_not_own_production_inventory_cardinality() {
    let descriptor = CompositePlannerSurface::current()
        .expect("planner surface")
        .component_descriptor(fixture_ref("qualification"))
        .expect("planner descriptor");
    let planner = QualifiedPlannerRegistration::new(descriptor, Arc::new(UnusedFactory))
        .expect("planner registration");

    assert!(validate_component_descriptors(
        &planner,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .is_ok());
}

#[test]
fn composite_planner_surface_owns_exact_canonical_contracts_and_refs() {
    let surface = CompositePlannerSurface::current().expect("planner surface");

    assert_eq!(
        surface.semantic_contract_canonical().as_bytes(),
        br#"{"version":"mfm.composite-planner-contract.v1"}"#
    );
    assert_eq!(
        surface.callback_surface_canonical().as_bytes(),
        br#"{"callbacks":["certify"],"version":"mfm.composite-planner-callback-surface.v1"}"#
    );
    assert_eq!(
        surface.semantic_contract_ref(),
        &exact_content_ref(
            schema_id("mfm.composite-planner-contract.v1").expect("semantic schema"),
            surface.semantic_contract_canonical(),
        )
        .expect("semantic ref"),
    );
    assert_eq!(
        surface.callback_surface_ref(),
        &exact_content_ref(
            schema_id("mfm.composite-planner-callback-surface.v1").expect("callback schema"),
            surface.callback_surface_canonical(),
        )
        .expect("callback ref"),
    );
}

#[test]
fn entry_version_and_planner_implementation_do_not_prevent_comparison() {
    let planner_contract = fixture_ref("planner-contract");
    let policies = vec![fixture_ref("policy-a"), fixture_ref("policy-b")];
    let candidate = entry(
        "mfm.test/portfolio@2",
        "mfm.test/portfolio",
        planner_contract.clone(),
        fixture_ref("planner-implementation-current"),
        policies.clone(),
        "same",
    );
    let recorded = entry(
        "mfm.test/portfolio@1",
        "mfm.test/portfolio",
        planner_contract,
        fixture_ref("planner-implementation-recorded"),
        policies,
        "same",
    );
    let configured = retained_contract("configured");

    assert_eq!(
        classify_candidate_plan_compatibility(&candidate, &configured, &recorded, &configured,),
        CandidatePlanCompatibility::Comparable,
    );
}

#[test]
fn semantic_entry_or_configured_contract_difference_is_not_comparable() {
    let planner_contract = fixture_ref("planner-contract");
    let policies = vec![fixture_ref("policy-a"), fixture_ref("policy-b")];
    let candidate = entry(
        "mfm.test/portfolio@2",
        "mfm.test/portfolio",
        planner_contract.clone(),
        fixture_ref("planner-implementation-current"),
        policies.clone(),
        "same",
    );
    let different_operation = entry(
        "mfm.test/portfolio@1",
        "mfm.test/other-operation",
        planner_contract.clone(),
        fixture_ref("planner-implementation-recorded"),
        policies.clone(),
        "same",
    );
    let different_planner_contract = entry(
        "mfm.test/portfolio@1",
        "mfm.test/portfolio",
        fixture_ref("other-planner-contract"),
        fixture_ref("planner-implementation-recorded"),
        policies.clone(),
        "same",
    );
    let different_parameters = entry(
        "mfm.test/portfolio@1",
        "mfm.test/portfolio",
        planner_contract.clone(),
        fixture_ref("planner-implementation-recorded"),
        policies.clone(),
        "different",
    );
    let different_policy_order = entry(
        "mfm.test/portfolio@1",
        "mfm.test/portfolio",
        planner_contract,
        fixture_ref("planner-implementation-recorded"),
        policies.into_iter().rev().collect(),
        "same",
    );
    let different_input_schema = entry_with_schemas(
        "mfm.test/portfolio@1",
        "mfm.test/portfolio",
        fixture_ref("planner-contract"),
        fixture_ref("planner-implementation-recorded"),
        vec![fixture_ref("policy-a"), fixture_ref("policy-b")],
        "same",
        "mfm.input-manifest.v1",
        "mfm.input-manifest.v1",
    );
    let different_output_schema = entry_with_schemas(
        "mfm.test/portfolio@1",
        "mfm.test/portfolio",
        fixture_ref("planner-contract"),
        fixture_ref("planner-implementation-recorded"),
        vec![fixture_ref("policy-a"), fixture_ref("policy-b")],
        "same",
        "mfm.access-audit-entry.v2",
        "mfm.access-audit-entry.v2",
    );
    let configured = retained_contract("configured");

    for recorded in [
        different_operation,
        different_planner_contract,
        different_parameters,
        different_policy_order,
        different_input_schema,
        different_output_schema,
    ] {
        assert_eq!(
            classify_candidate_plan_compatibility(&candidate, &configured, &recorded, &configured,),
            CandidatePlanCompatibility::NotComparable,
        );
    }
    assert_eq!(
        classify_candidate_plan_compatibility(
            &candidate,
            &configured,
            &candidate,
            &retained_contract("different-configured-role"),
        ),
        CandidatePlanCompatibility::NotComparable,
    );
}

#[test]
fn candidate_state_failure_classification_preserves_stage() {
    let integrity =
        CandidateCertificationError::state_callback(CandidateStateCallbackError::Integrity(
            ProgramError::Codec("private test detail".to_owned()),
        ));
    let execution =
        CandidateCertificationError::state_callback(CandidateStateCallbackError::ExecutionPanic);

    assert_eq!(
        integrity.kind(),
        CandidateCertificationErrorKind::IntegrityFailed
    );
    assert_eq!(
        execution.kind(),
        CandidateCertificationErrorKind::ExecutionFailed
    );
    assert!(!integrity.to_string().contains("private test detail"));
}

fn entry(
    entry_point_id: &str,
    operation_id: &str,
    planner_contract: ContentRef,
    planner_implementation: ContentRef,
    policies: Vec<ContentRef>,
    parameters: &str,
) -> EntryPointContract {
    entry_with_schemas(
        entry_point_id,
        operation_id,
        planner_contract,
        planner_implementation,
        policies,
        parameters,
        "mfm.access-audit-entry.v2",
        "mfm.input-manifest.v1",
    )
}

#[allow(clippy::too_many_arguments)]
fn entry_with_schemas(
    entry_point_id: &str,
    operation_id: &str,
    planner_contract: ContentRef,
    planner_implementation: ContentRef,
    policies: Vec<ContentRef>,
    parameters: &str,
    input_schema: &str,
    public_output_schema: &str,
) -> EntryPointContract {
    EntryPointContract::new(
        EntryPointId::new(entry_point_id).expect("entry point"),
        StableId::new(operation_id).expect("operation"),
        PlanningProfile::new(
            planner_contract,
            planner_implementation,
            policies,
            CanonicalJsonValue::new(serde_json::json!({ "parameters": parameters }))
                .expect("profile parameters"),
        )
        .expect("planning profile"),
        schema_id(input_schema).expect("input schema"),
        schema_id(public_output_schema).expect("public-output schema"),
    )
    .expect("entry-point contract")
}

fn retained_contract(role: &str) -> RetainedValueContract {
    RetainedValueContract::new(
        schema_id("mfm.primitive-canonical_value.v1").expect("configured schema"),
        SemanticTypeId::new(
            "mfm.test",
            "configured",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.configured.v1"),
        )
        .expect("configured semantic type"),
        StableId::new(role).expect("configured role"),
        "application/json",
        fixture_ref("configured-evidence"),
    )
    .expect("retained configured contract")
}

fn fixture_ref(label: &str) -> ContentRef {
    let json =
        serde_json::to_string(&serde_json::json!({ "fixture": label })).expect("fixture json");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical fixture");
    exact_content_ref(
        schema_id("mfm.component-implementation-descriptor.v1").expect("fixture schema"),
        &canonical,
    )
    .expect("fixture content reference")
}

struct UnusedFactory;

impl QualifiedCertificationFactory for UnusedFactory {
    fn create(
        &self,
        _definition: Arc<QualifiedProgramDefinition>,
    ) -> crate::Result<Arc<dyn QualifiedCertificationCallback>> {
        Err(ProgramError::Registry(
            "unused test certification factory".to_owned(),
        ))
    }
}
