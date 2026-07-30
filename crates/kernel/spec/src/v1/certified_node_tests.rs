use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, DigestAlgorithm, FieldPath, SemanticTypeId, StableId};
use mfm_values::{component_object_evidence_contract_ref, RetainedValueContract};

use super::{
    select_unique_invariant_fact_slot, validate_certified_graph, CertifiedFactSlot,
    CertifiedFrameBinding, CertifiedInputBinding, CertifiedInputDestination, CertifiedNodeContract,
    CertifiedSettlementContract, CertifiedSourceSelector, CertifiedStateExecution,
    FactInvariantCore,
};
use crate::{
    exact_content_ref, schema_id, CanonicalExpansionPath, CanonicalExpansionStep, SpecError,
};

fn retained(role: &str) -> RetainedValueContract {
    RetainedValueContract::new(
        schema_id("mfm.access-audit-entry.v2").expect("schema"),
        SemanticTypeId::new(
            "mfm.test",
            role,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(format!("semantic:mfm.test:{role}:1").as_bytes()),
        )
        .expect("semantic type"),
        StableId::new(role).expect("role"),
        "application/json",
        component_object_evidence_contract_ref().expect("object evidence contract"),
    )
    .expect("retained contract")
}

fn content_ref(label: &str) -> ContentRef {
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&serde_json::json!({"fixture": label}).to_string())
            .expect("canonical fixture");
    exact_content_ref(
        schema_id("mfm.access-audit-entry.v2").expect("schema"),
        &canonical,
    )
    .expect("content ref")
}

fn fact_slot(
    ordinal: u32,
    minimum: u32,
    maximum: u32,
    response_contract: RetainedValueContract,
) -> CertifiedFactSlot {
    CertifiedFactSlot::new(
        ordinal,
        minimum,
        maximum,
        content_ref(&format!("descriptor-{ordinal}")),
        retained(&format!("subject-{ordinal}")),
        response_contract,
    )
    .expect("fact slot")
}

fn settlement(fact_slots: Vec<CertifiedFactSlot>) -> CertifiedSettlementContract {
    CertifiedSettlementContract::new(None, Vec::new(), fact_slots).expect("settlement")
}

fn config_binding(label: &str) -> CertifiedFrameBinding {
    CertifiedFrameBinding::new(
        retained(&format!("{label}-config")),
        CertifiedSourceSelector::Config {
            source_field_path: None,
        },
    )
}

fn node(
    label: &str,
    config_binding: CertifiedFrameBinding,
    context_binding: Option<CertifiedFrameBinding>,
    input_bindings: Vec<CertifiedInputBinding>,
    settlement_contract: CertifiedSettlementContract,
) -> CertifiedNodeContract {
    CertifiedNodeContract::new(
        CanonicalExpansionPath::new(vec![
            CanonicalExpansionStep::EntryPoint,
            CanonicalExpansionStep::Authored {
                stable_key: StableId::new(label).expect("stable key"),
                ordinal: 0,
            },
        ])
        .expect("expansion path"),
        content_ref(&format!("{label}-state")),
        config_binding,
        context_binding,
        retained(&format!("{label}-input")),
        input_bindings,
        CertifiedStateExecution::Pure,
        settlement_contract,
    )
    .expect("certified node")
}

fn producer(fact_slots: Vec<CertifiedFactSlot>) -> CertifiedNodeContract {
    node(
        "producer",
        config_binding("producer"),
        None,
        Vec::new(),
        settlement(fact_slots),
    )
}

fn node_fact(producer: &CertifiedNodeContract, emission_ordinal: u32) -> CertifiedSourceSelector {
    CertifiedSourceSelector::NodeFact {
        producer_node_id: producer.node_id().clone(),
        emission_ordinal,
    }
}

#[test]
fn graph_accepts_node_fact_with_one_invariant_slot_and_matching_contract() {
    let response_contract = retained("matching-response");
    let producer = producer(vec![fact_slot(0, 1, 1, response_contract.clone())]);
    let consumer = node(
        "consumer",
        CertifiedFrameBinding::new(response_contract, node_fact(&producer, 0)),
        None,
        Vec::new(),
        settlement(Vec::new()),
    );

    validate_certified_graph(&[producer, consumer]).expect("valid node-fact graph");
}

#[test]
fn graph_rejects_node_fact_in_variable_cardinality_gap() {
    let producer = producer(vec![
        fact_slot(0, 1, 2, retained("variable-response")),
        fact_slot(1, 1, 1, retained("after-variable-response")),
    ]);
    let consumer = node(
        "consumer",
        CertifiedFrameBinding::new(retained("gap-destination"), node_fact(&producer, 1)),
        None,
        Vec::new(),
        settlement(Vec::new()),
    );

    let error = validate_certified_graph(&[producer, consumer]).expect_err("gap must fail");
    assert!(matches!(
        error,
        SpecError::Invariant(message)
            if message.contains("does not fall in a certified fact-slot invariant core")
    ));
}

#[test]
fn graph_rejects_node_fact_beyond_all_invariant_slots() {
    let response_contract = retained("bounded-response");
    let producer = producer(vec![fact_slot(0, 1, 1, response_contract.clone())]);
    let consumer = node(
        "consumer",
        config_binding("consumer"),
        Some(CertifiedFrameBinding::new(
            response_contract,
            node_fact(&producer, 1),
        )),
        Vec::new(),
        settlement(Vec::new()),
    );

    let error =
        validate_certified_graph(&[producer, consumer]).expect_err("out of range must fail");
    assert!(matches!(
        error,
        SpecError::Invariant(message)
            if message.contains("does not fall in a certified fact-slot invariant core")
    ));
}

#[test]
fn graph_rejects_node_fact_destination_contract_mismatch() {
    let producer = producer(vec![fact_slot(0, 1, 1, retained("producer-response"))]);
    let binding = CertifiedInputBinding::new(
        FieldPath::new("value").expect("destination path"),
        CertifiedInputDestination::OrdinaryValue,
        retained("consumer-destination"),
        vec![node_fact(&producer, 0)],
    )
    .expect("input binding");
    let consumer = node(
        "consumer",
        config_binding("consumer"),
        None,
        vec![binding],
        settlement(Vec::new()),
    );

    let error = validate_certified_graph(&[producer, consumer]).expect_err("mismatch must fail");
    assert!(matches!(
        error,
        SpecError::Invariant(message)
            if message.contains("differs from its destination retained-value contract")
    ));
}

#[test]
fn invariant_core_resolver_defensively_rejects_multiple_matches() {
    let first = fact_slot(0, 1, 1, retained("first-response"));
    let second = fact_slot(1, 1, 1, retained("second-response"));
    let cores = vec![
        FactInvariantCore {
            slot: &first,
            invariant_start: 0,
            invariant_end: 2,
        },
        FactInvariantCore {
            slot: &second,
            invariant_start: 1,
            invariant_end: 3,
        },
    ];

    let error = select_unique_invariant_fact_slot(cores, 1).expect_err("overlap must be rejected");
    assert!(matches!(
        error,
        SpecError::Invariant(message)
            if message.contains("multiple certified fact-slot invariant cores")
    ));
}
