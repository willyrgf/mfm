use super::*;
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{ClosedSumPayload, ClosedSumVariant, FailureScope};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "mapping_failure",
    version = "1",
    schema = "mfm.test.mapping_failure"
)]
struct MappingFailure {
    code: u64,
}

struct MappingFixture {
    plan: FailurePlan,
    source_semantic_call_id: SemanticCallId,
    source_slot: LexicalSlot,
    registry: StructuredCertificationRegistry,
}

fn fixture_ref(label: &str) -> ContentRef {
    typed_content_ref("mfm.failure-mapping-test", &label).expect("fixture ref")
}

fn semantic_call(label: &str) -> SemanticCallId {
    SemanticCallPath::new(vec![SemanticPathSegment {
        label: stable_id(label).expect("semantic label"),
        discriminator: None,
    }])
    .expect("semantic path")
    .identity()
    .expect("semantic call id")
}

fn mapping_state(
    registry: &mut StructuredCertificationRegistry,
    plan_path: &StructuralPath,
    ordinal: u32,
    label: &str,
    input_slot: LexicalSlot,
    output_contract_ref: ContentRef,
) -> ExpandedStateBinding {
    let label_id = stable_id(label).expect("mapper label");
    let occurrence_path =
        declaration_path(plan_path, &label_id, ordinal).expect("mapper occurrence path");
    let occurrence_id = occurrence_path
        .occurrence_id()
        .expect("mapper occurrence id");
    let contract = StructuredStateContract::new(
        stable_id(&format!("mfm.test/{label}")).expect("mapper semantic id"),
        StructuredStateExecutionContract::Pure,
        input_slot.contract_ref.clone(),
        output_contract_ref.clone(),
        StructuredFailureContract::Never,
        mfm_spec::structured::StructuredSafeFailureDispositionContract::NotApplicable {},
        None,
    )
    .expect("mapper contract");
    registry.states.insert(
        contract.state_contract_ref.clone(),
        RegisteredState {
            contract: contract.clone(),
        },
    );
    let output_slot = state_output_slot(
        &occurrence_path,
        &occurrence_id,
        &output_contract_ref,
        ResultRole::SuccessOutput,
    );
    ExpandedStateBinding {
        semantic_call_id: semantic_call(label),
        occurrence_id,
        occurrence_path,
        label: label_id,
        contract,
        inputs: vec![input_slot],
        output_slot,
        failure_boundary: CertifiedFailureBoundary::NoFailure(NoFailureBoundary {
            never_contract_ref: never_failure_contract_ref().expect("Never ref"),
        }),
    }
}

fn mapping_fixture(link_count: usize) -> MappingFixture {
    assert!(matches!(link_count, 1 | 2));
    let operation_id = stable_id(&format!("mfm.test/mapping-{link_count}")).expect("operation");
    let root_path =
        StructuralPath::new(vec![StructuralPathSegment::Root { operation_id }]).expect("root path");
    let fragment_path = root_path
        .child(StructuralPathSegment::Fragment {
            label: stable_id("protected").expect("fragment label"),
            expansion_ref: fixture_ref("policy"),
        })
        .expect("fragment path");
    let source_path = declaration_path(
        &fragment_path,
        &stable_id("source").expect("source label"),
        0,
    )
    .expect("source path");
    let source_occurrence_id = source_path.occurrence_id().expect("source occurrence");
    let source_contract_ref = fixture_ref("failure");
    let source_slot = state_output_slot(
        &source_path,
        &source_occurrence_id,
        &source_contract_ref,
        ResultRole::TypedFailure,
    );
    let boundary_id = fragment_path
        .fragment_boundary_id()
        .expect("fragment boundary id");
    let boundary_slot = LexicalSlot {
        lexical_path: fragment_path,
        contract_ref: source_contract_ref.clone(),
        producer: LexicalProducer::FragmentBoundary {
            boundary_id: boundary_id.clone(),
            role: ResultRole::TypedFailure,
            source: Box::new(source_slot.clone()),
        },
    };
    let source_semantic_call_id = semantic_call("source");
    let mut plan = propagation_plan(
        &source_semantic_call_id,
        source_slot.clone(),
        &source_path,
        boundary_id,
        boundary_slot,
    )
    .expect("zero-link plan");
    let FailurePlan::Propagate {
        plan_id,
        plan_path,
        before_boundary,
        mapping_chain,
        boundary_slot,
        ..
    } = &mut plan
    else {
        unreachable!("propagation fixture")
    };
    let mut registry = StructuredCertificationRegistry::default();
    let first_output = if link_count == 1 {
        source_contract_ref.clone()
    } else {
        fixture_ref("normalized-failure")
    };
    let first = mapping_state(
        &mut registry,
        plan_path,
        0,
        "normalize",
        source_slot.clone(),
        first_output,
    );
    let first_output_slot = first.output_slot.clone();
    mapping_chain.push(FailureMappingLink {
        plan_id: plan_id.clone(),
        input_slot: source_slot.clone(),
        output_slot: first_output_slot.clone(),
        mapper: Box::new(first),
    });
    let mapped_target = if link_count == 1 {
        first_output_slot
    } else {
        let second = mapping_state(
            &mut registry,
            plan_path,
            1,
            "rebuild",
            first_output_slot.clone(),
            source_contract_ref,
        );
        let second_output_slot = second.output_slot.clone();
        mapping_chain.push(FailureMappingLink {
            plan_id: plan_id.clone(),
            input_slot: first_output_slot,
            output_slot: second_output_slot.clone(),
            mapper: Box::new(second),
        });
        second_output_slot
    };
    before_boundary.tail = BlockTail::Normal(mapped_target.clone());
    let LexicalProducer::FragmentBoundary { source, .. } = &mut boundary_slot.producer else {
        unreachable!("fixture boundary producer")
    };
    **source = mapped_target;
    MappingFixture {
        plan,
        source_semantic_call_id,
        source_slot,
        registry,
    }
}

fn zero_mapping_fixture() -> MappingFixture {
    let mut fixture = mapping_fixture(1);
    let FailurePlan::Propagate {
        before_boundary,
        mapping_chain,
        boundary_slot,
        ..
    } = &mut fixture.plan
    else {
        unreachable!("propagation fixture")
    };
    mapping_chain.clear();
    before_boundary.tail = BlockTail::Normal(fixture.source_slot.clone());
    let LexicalProducer::FragmentBoundary { source, .. } = &mut boundary_slot.producer else {
        unreachable!("fixture boundary producer")
    };
    **source = fixture.source_slot.clone();
    fixture
}

fn assert_rejected(fixture: &MappingFixture, plan: &FailurePlan) {
    validate_failure_plan(
        plan,
        &fixture.source_semantic_call_id,
        &fixture.source_slot,
        &fixture.registry,
    )
    .expect_err("hostile failure mapping must be rejected");
}

fn eventual_handler_program(
    fixture: &MappingFixture,
    plans: Vec<FailurePlan>,
) -> ExpandedStructuredProgram {
    let FailurePlan::Propagate { mapping_chain, .. } = &fixture.plan else {
        unreachable!("propagation fixture")
    };
    let template = mapping_chain[0].mapper.as_ref();
    let failure_contract = StructuredFailureContract::typed(
        structured_value_contract::<MappingFailure>().expect("mapping failure contract"),
    )
    .expect("typed mapping failure");
    let declarations = plans
        .into_iter()
        .map(|plan| {
            let mut state = template.clone();
            state.failure_boundary = CertifiedFailureBoundary::Typed {
                failure_contract: Box::new(failure_contract.clone()),
                source_slot: fixture.source_slot.clone(),
                plan: Box::new(plan),
            };
            ExpandedDeclaration::State(Box::new(state))
        })
        .collect::<Vec<_>>();
    let operation_id = stable_id("mfm.test/eventual-handler-program").expect("operation id");
    let root_path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: operation_id.clone(),
    }])
    .expect("root path");
    ExpandedStructuredProgram {
        operation_id,
        input_roots: Vec::new(),
        output_contract_ref: template.output_slot.contract_ref.clone(),
        failure_contract: StructuredFailureContract::Never,
        root: ExpandedBlock {
            path: root_path,
            failure_scope: FailureScopeBinding::Inherits {
                scope_id: stable_id("mfm.test/eventual-handler-scope").expect("scope id"),
                failure_contract: StructuredFailureContract::Never,
            },
            declarations,
            failure_exits: Vec::new(),
            tail: BlockTail::Normal(template.output_slot.clone()),
        },
    }
}

#[test]
fn affine_failure_mapping_rejects_each_hostile_substitution() {
    let one = mapping_fixture(1);
    validate_failure_plan(
        &one.plan,
        &one.source_semantic_call_id,
        &one.source_slot,
        &one.registry,
    )
    .expect("one-link mapping");

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain.clear();
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate {
        before_boundary, ..
    } = &mut hostile
    else {
        unreachable!()
    };
    before_boundary.tail = BlockTail::Normal(one.source_slot.clone());
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { boundary_slot, .. } = &mut hostile else {
        unreachable!()
    };
    let LexicalProducer::FragmentBoundary { source, .. } = &mut boundary_slot.producer else {
        unreachable!()
    };
    **source = one.source_slot.clone();
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate {
        plan_id,
        plan_path,
        mapping_chain,
        ..
    } = &mut hostile
    else {
        unreachable!()
    };
    let wrong_id = FailurePlanIdentity {
        source_semantic_call_id: one.source_semantic_call_id.clone(),
        source_slot: one.source_slot.clone(),
        plan_path: plan_path
            .child(StructuralPathSegment::FailurePlan {
                label: stable_id("wrong-plan").expect("wrong plan label"),
            })
            .expect("wrong plan path"),
    }
    .derive()
    .expect("wrong plan id");
    *plan_id = wrong_id.clone();
    mapping_chain[0].plan_id = wrong_id;
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].mapper.inputs.clear();
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].input_slot = mapping_chain[0].output_slot.clone();
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].output_slot = one.source_slot.clone();
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].mapper.occurrence_path = one.source_slot.lexical_path.clone();
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].mapper.contract.execution = StructuredStateExecutionContract::Read {
        capability_contract_ref: fixture_ref("foreign-read"),
    };
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].mapper.contract.failure_contract = StructuredFailureContract::typed(
        structured_value_contract::<MappingFailure>().expect("mapping failure contract"),
    )
    .expect("typed mapper failure");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].mapper.contract.state_contract_ref = fixture_ref("foreign-mapper-state");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].mapper.contract.input_contract_ref =
        fixture_ref("same-type-foreign-input-contract");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain[0].mapper.contract.output_contract_ref =
        fixture_ref("same-type-foreign-output-contract");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate {
        before_boundary,
        mapping_chain,
        ..
    } = &mut hostile
    else {
        unreachable!()
    };
    let route_contract = ClosedSumContract::new(
        mapping_chain[0].mapper.contract.output_contract_ref.clone(),
        vec![ClosedSumVariant {
            canonical_tag: "propagate".to_owned(),
            payloads: vec![ClosedSumPayload {
                payload_path: vec![stable_id("failure").expect("payload path")],
                contract_ref: one.source_slot.contract_ref.clone(),
            }],
        }],
    )
    .expect("default route contract");
    before_boundary.failure_scope = FailureScopeBinding::Owns {
        scope: FailureScope {
            scope_id: stable_id("mfm.test/hostile-default-scope").expect("scope id"),
            failure_contract: StructuredFailureContract::Never,
            default_mappers: vec![FailureMapperRegistration {
                source_failure_contract_ref: one.source_slot.contract_ref.clone(),
                mapper_state_contract_ref: mapping_chain[0]
                    .mapper
                    .contract
                    .state_contract_ref
                    .clone(),
                route_contract,
            }],
        },
    };
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate {
        plan_path,
        mapping_chain,
        ..
    } = &mut hostile
    else {
        unreachable!()
    };
    mapping_chain[0].mapper.occurrence_path = plan_path
        .child(StructuralPathSegment::MatchArm {
            label: stable_id("inactive").expect("arm label"),
            tag: "inactive".to_owned(),
        })
        .and_then(|path| {
            path.child(StructuralPathSegment::Declaration {
                label: mapping_chain[0].mapper.label.clone(),
                ordinal: 0,
            })
        })
        .expect("inactive mapper path");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate {
        plan_path,
        mapping_chain,
        ..
    } = &mut hostile
    else {
        unreachable!()
    };
    mapping_chain[0].mapper.occurrence_path =
        declaration_path(plan_path, &mapping_chain[0].mapper.label, 1)
            .expect("nonterminal mapper path");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { boundary_id, .. } = &mut hostile else {
        unreachable!()
    };
    *boundary_id = one
        .source_slot
        .lexical_path
        .fragment_boundary_id()
        .expect("foreign boundary id");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate {
        boundary_id,
        boundary_slot,
        ..
    } = &mut hostile
    else {
        unreachable!()
    };
    let foreign_path = one
        .source_slot
        .lexical_path
        .child(StructuralPathSegment::Fragment {
            label: stable_id("foreign-protected-boundary").expect("foreign boundary label"),
            expansion_ref: fixture_ref("foreign-boundary-expansion"),
        })
        .expect("foreign boundary path");
    let foreign_boundary_id = foreign_path
        .fragment_boundary_id()
        .expect("foreign boundary id");
    *boundary_id = foreign_boundary_id.clone();
    boundary_slot.lexical_path = foreign_path;
    let LexicalProducer::FragmentBoundary {
        boundary_id: producer_boundary_id,
        ..
    } = &mut boundary_slot.producer
    else {
        unreachable!()
    };
    *producer_boundary_id = foreign_boundary_id;
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { boundary_slot, .. } = &mut hostile else {
        unreachable!()
    };
    let LexicalProducer::FragmentBoundary { role, .. } = &mut boundary_slot.producer else {
        unreachable!()
    };
    *role = ResultRole::SuccessOutput;
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate { boundary_slot, .. } = &mut hostile else {
        unreachable!()
    };
    boundary_slot.contract_ref = fixture_ref("same-type-foreign-boundary-contract");
    assert_rejected(&one, &hostile);

    let mut hostile = one.plan.clone();
    let FailurePlan::Propagate {
        before_boundary,
        mapping_chain,
        ..
    } = &mut hostile
    else {
        unreachable!()
    };
    before_boundary
        .declarations
        .push(ExpandedDeclaration::State(mapping_chain[0].mapper.clone()));
    assert_rejected(&one, &hostile);

    let two = mapping_fixture(2);
    validate_failure_plan(
        &two.plan,
        &two.source_semantic_call_id,
        &two.source_slot,
        &two.registry,
    )
    .expect("two-link mapping");

    let mut hostile = two.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain.swap(0, 1);
    assert_rejected(&two, &hostile);

    let mut hostile = two.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain.remove(0);
    assert_rejected(&two, &hostile);

    let mut hostile = two.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    mapping_chain.push(mapping_chain[0].clone());
    assert_rejected(&two, &hostile);

    let mut hostile = two.plan.clone();
    let FailurePlan::Propagate { mapping_chain, .. } = &mut hostile else {
        unreachable!()
    };
    let same_contract_foreign_producer = LexicalSlot {
        lexical_path: mapping_chain[0].output_slot.lexical_path.clone(),
        contract_ref: mapping_chain[0].output_slot.contract_ref.clone(),
        producer: LexicalProducer::AdmissionRoot {
            root_id: stable_id("foreign-producer").expect("foreign root"),
        },
    };
    mapping_chain[1].input_slot = same_contract_foreign_producer.clone();
    mapping_chain[1].mapper.inputs = vec![same_contract_foreign_producer];
    assert_rejected(&two, &hostile);

    let zero = zero_mapping_fixture();
    validate_failure_plan(
        &zero.plan,
        &zero.source_semantic_call_id,
        &zero.source_slot,
        &zero.registry,
    )
    .expect("zero-link identity mapping");

    let mut hostile = zero.plan.clone();
    let FailurePlan::Propagate {
        plan_id,
        plan_path,
        before_boundary,
        ..
    } = &mut hostile
    else {
        unreachable!()
    };
    let substituted_path = zero
        .source_slot
        .lexical_path
        .child(StructuralPathSegment::FailurePlan {
            label: stable_id("foreign-propagation-plan").expect("foreign plan label"),
        })
        .expect("foreign plan path");
    *plan_path = substituted_path.clone();
    before_boundary.path = substituted_path.clone();
    *plan_id = FailurePlanIdentity {
        source_semantic_call_id: zero.source_semantic_call_id.clone(),
        source_slot: zero.source_slot.clone(),
        plan_path: substituted_path,
    }
    .derive()
    .expect("recomputed hostile plan id");
    assert_rejected(&zero, &hostile);

    let mut hostile = zero.plan.clone();
    let FailurePlan::Propagate { boundary_slot, .. } = &mut hostile else {
        unreachable!()
    };
    boundary_slot.contract_ref = fixture_ref("zero-link-foreign-boundary-contract");
    assert_rejected(&zero, &hostile);
}

#[test]
fn eventual_failure_handler_graph_rejects_missing_duplicate_and_cyclic_plans() {
    let fixture = mapping_fixture(1);

    let missing = eventual_handler_program(&fixture, vec![fixture.plan.clone()]);
    let error = validate_eventual_failure_handlers(&missing)
        .expect_err("propagation without a boundary plan must reject");
    assert!(error
        .to_string()
        .contains("failure propagation has no exact eventual plan"));

    let mut cyclic_plan = fixture.plan.clone();
    let FailurePlan::Propagate { boundary_slot, .. } = &mut cyclic_plan else {
        unreachable!()
    };
    **boundary_slot = fixture.source_slot.clone();
    let cyclic = eventual_handler_program(&fixture, vec![cyclic_plan]);
    let error =
        validate_eventual_failure_handlers(&cyclic).expect_err("self-propagation must reject");
    assert!(error
        .to_string()
        .contains("failure propagation contains a cycle"));

    let duplicate =
        eventual_handler_program(&fixture, vec![fixture.plan.clone(), fixture.plan.clone()]);
    let error = validate_eventual_failure_handlers(&duplicate)
        .expect_err("one source cannot own multiple certified plans");
    assert!(error
        .to_string()
        .contains("typed failure source has duplicate certified plans"));
}
