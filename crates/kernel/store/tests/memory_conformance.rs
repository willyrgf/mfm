use std::collections::BTreeSet;

#[path = "../../../../tests/support/recoverability_v2.rs"]
mod recoverability_v2_support;

use recoverability_v2_support::{
    assert_lower_layer_owner_vector, for_each_vector, run_consumer, CorpusVector, OwnerVector,
};

const CONSUMER: &str = "mfm-store-memory";
const TOTAL_VECTOR_COUNT: usize = 576;
const OWNER_VECTOR_COUNT: usize = 116;

#[test]
fn memory_store_executes_all_576_frozen_recoverability_vectors() {
    let mut vector_ids = BTreeSet::new();
    for_each_vector(|vector| {
        assert_store_vector_binding(vector);
        assert!(
            vector_ids.insert(vector.id().to_owned()),
            "duplicate frozen vector execution: {}",
            vector.id()
        );
    });
    assert_eq!(vector_ids.len(), TOTAL_VECTOR_COUNT);

    let mut owner_ids = BTreeSet::new();
    run_consumer(CONSUMER, |owner| {
        assert_lower_layer_owner_vector(owner);
        assert_store_owner_vector(owner);
        assert!(
            owner_ids.insert(owner.id().to_owned()),
            "duplicate store owner-vector execution: {}",
            owner.id()
        );
    });
    assert_eq!(owner_ids.len(), OWNER_VECTOR_COUNT);
}

fn assert_store_vector_binding(vector: CorpusVector<'_>) {
    let coverage = vector
        .vector()
        .get("consumer_coverage")
        .and_then(serde_json::Value::as_array)
        .expect("consumer coverage array");
    assert!(
        coverage
            .iter()
            .any(|consumer| consumer.as_str() == Some(CONSUMER)),
        "{} omits the memory-store consumer binding",
        vector.id()
    );
}

fn assert_store_owner_vector(owner: OwnerVector<'_>) {
    let vector = owner.vector();
    let object = vector.as_object().expect("owner vector object");
    assert!(!object.is_empty(), "{}", owner.id());
    match owner {
        OwnerVector::RelationalPositive(_) => assert!(
            matches!(
                owner.kind(),
                "commit_coordinate_separation"
                    | "content_identity_separation"
                    | "export_identity"
                    | "frontier_order"
                    | "identity_separation"
                    | "read_returned_validation_verdict"
                    | "read_safe_failure_verdict"
                    | "relational_acceptance"
                    | "request_identity"
                    | "resource_refold"
                    | "schema_identity"
                    | "type_separation"
            ),
            "{}: unclassified store relational vector {}",
            owner.id(),
            owner.kind()
        ),
        OwnerVector::RelationalRejection(_) => {
            assert_eq!(
                object
                    .get("error_class")
                    .and_then(serde_json::Value::as_str),
                Some("relational"),
                "{}",
                owner.id()
            );
            assert!(
                object
                    .get("expected_error")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|error| !error.is_empty()),
                "{}",
                owner.id()
            );
        }
    }
}
