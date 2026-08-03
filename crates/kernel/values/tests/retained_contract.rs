use mfm_canonical::{
    sha256_digest_bytes, CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContract,
};
use mfm_ids::{ContentRef, DigestAlgorithm, SemanticTypeId, StableId};
use mfm_values::{
    component_object_evidence_contract_canonical, component_object_evidence_contract_ref,
    RetainedValueContract,
};

fn contract() -> &'static RecoverabilityContract {
    RecoverabilityContract::embedded().expect("embedded recoverability contract")
}

fn reviewed_ref(label: &str) -> ContentRef {
    let value = contract()
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String(label.to_owned()),
        )
        .expect("reviewed fixture value");
    contract().content_ref(&value).expect("fixture content ref")
}

fn retained_contract(media_type: &str) -> mfm_values::Result<RetainedValueContract> {
    RetainedValueContract::new(
        contract()
            .schema_id("mfm.primitive-canonical_value.v1")
            .expect("fixture schema")
            .clone(),
        SemanticTypeId::new(
            "mfm.test",
            "retained-value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"retained-value"),
        )
        .expect("fixture semantic type"),
        StableId::new("retained-value").expect("fixture role"),
        media_type,
        reviewed_ref("retained-value.evidence"),
    )
}

#[test]
fn component_object_evidence_contract_keeps_exact_bytes_and_identity() {
    let canonical =
        component_object_evidence_contract_canonical().expect("component evidence canonical");
    assert_eq!(
        canonical.as_str(),
        r#"{"version":"mfm.component-object-evidence-contract.v1"}"#
    );

    let content_ref =
        component_object_evidence_contract_ref().expect("component evidence content ref");
    let decoded = contract()
        .strict_decode(
            "mfm.component-object-evidence-contract.v1",
            canonical.as_bytes(),
        )
        .expect("component evidence annex shape");
    assert_eq!(
        content_ref.schema_id().as_str(),
        "schema:mfm.component-object-evidence-contract:1:sha256-jcs-v1:8b6c7e24ab69e0f2b91a273f00c23108d75847d4be9e508ce74c0b9b4aa4245e"
    );
    assert_eq!(decoded.schema_id(), content_ref.schema_id());
    assert_eq!(
        content_ref.content_digest().as_str(),
        "content:sha256-v1:52f0fff94cc51dcdab491860c892253fdc5275a62dee678330689f24ebe4475c"
    );
}

#[test]
fn retained_contract_has_one_exact_annex_backed_round_trip() {
    let original = retained_contract("application/json").expect("retained contract");
    let canonical = original
        .canonical_json()
        .expect("canonical retained contract");
    let decoded =
        RetainedValueContract::strict_decode(canonical.as_bytes()).expect("strict retained decode");
    let serde_decoded: RetainedValueContract =
        serde_json::from_slice(canonical.as_bytes()).expect("strict typed decode");

    assert_eq!(decoded, original);
    assert_eq!(serde_decoded, original);
    assert_eq!(
        decoded
            .validated()
            .expect("annex-validated retained contract")
            .as_bytes(),
        canonical.as_bytes()
    );
}

#[test]
fn retained_contract_rejects_noncanonical_media_types_and_open_fields() {
    for media_type in ["Application/json", "application/json; charset=utf-8"] {
        assert!(
            retained_contract(media_type).is_err(),
            "{media_type} must reject"
        );
    }

    let original = retained_contract("application/json").expect("retained contract");
    let mut value = serde_json::to_value(&original).expect("retained contract fixture serializes");
    value
        .as_object_mut()
        .expect("retained contract object")
        .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
    assert!(
        serde_json::from_value::<RetainedValueContract>(value.clone()).is_err(),
        "typed Deserialize must reject unknown fields"
    );

    let json = serde_json::to_string(&value).expect("mutated fixture JSON");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical mutated JSON");
    assert!(
        RetainedValueContract::strict_decode(canonical.as_bytes()).is_err(),
        "annex strict decode must reject unknown fields"
    );
}

#[test]
fn retained_contract_deserialize_revalidates_invariants() {
    let original = retained_contract("application/json").expect("retained contract");
    let mut value = serde_json::to_value(&original).expect("retained contract fixture serializes");
    value
        .as_object_mut()
        .expect("retained contract object")
        .insert(
            "media_type".to_owned(),
            serde_json::Value::String("Application/json".to_owned()),
        );

    assert!(
        serde_json::from_value::<RetainedValueContract>(value).is_err(),
        "Deserialize must revalidate the annex media-type grammar"
    );
}
