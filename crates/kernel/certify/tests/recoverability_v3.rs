#[path = "../../../../tests/support/recoverability_v3.rs"]
mod recoverability_v3_support;

use mfm_canonical::RecoverabilityContractV3;
use mfm_spec::{CanonicalExpansionPath, CanonicalExpansionStep};
use recoverability_v3_support::{
    assert_lower_layer_owner_vector, for_each_vector, hex_field, run_consumer, string,
    CorpusVector, OwnerVector,
};

#[test]
fn certify_executes_all_587_frozen_recoverability_vectors() {
    let mut visited = 0;
    for_each_vector(|_| visited += 1);
    assert_eq!(visited, 587);

    run_consumer("mfm-certify", |vector| {
        assert_lower_layer_owner_vector(vector);
        assert_certify_owner_vector(vector);
    });
}

fn assert_certify_owner_vector(owner: OwnerVector<'_>) {
    match owner {
        OwnerVector::RelationalRejection(vector)
            if owner.id() == "relational/canonical_path_invalid" =>
        {
            let steps = serde_json::from_slice::<Vec<CanonicalExpansionStep>>(&hex_field(
                vector,
                "input_hex",
            ))
            .expect("canonical path rejection has typed steps");
            assert!(
                CanonicalExpansionPath::new(steps).is_err(),
                "{}",
                owner.id()
            );
        }
        OwnerVector::RelationalRejection(vector) => {
            assert!(!string(vector, "target").is_empty(), "{}", owner.id());
            assert!(!string(vector, "rule").is_empty(), "{}", owner.id());
        }
        OwnerVector::RelationalPositive(vector) => match owner.kind() {
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
            | "type_separation" => {
                assert!(
                    vector.as_object().is_some_and(|fields| !fields.is_empty()),
                    "{}",
                    owner.id()
                );
            }
            other => panic!(
                "{}: unclassified certification owner vector {other}",
                owner.id()
            ),
        },
    }
}

#[test]
fn correction_identity_is_a_private_conformance_fixture_only() {
    let contract = RecoverabilityContractV3::embedded().expect("embedded annex");
    let mut matched = 0;
    for_each_vector(|vector| {
        let CorpusVector::Positive(value) = vector else {
            return;
        };
        if vector.id() != "domain/mfm.correction-invocation.v1/minimum" {
            return;
        }
        let preimage = contract
            .strict_decode(
                string(value, "preimage_schema"),
                &hex_field(value, "value_hex"),
            )
            .expect("private correction preimage");
        let digest = contract
            .semantic_digest(string(value, "domain"), &preimage)
            .expect("private correction digest");
        assert_eq!(digest.as_str(), string(&value["expected"], "value"));
        matched += 1;
    });
    assert_eq!(matched, 1);
}
