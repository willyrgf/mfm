use mfm_canonical::{
    sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue, PlainCanonicalJsonBytes,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_values::{
    CanonicalJsonPersistedSchema, ComponentObjectEvidence, MediaType, RetainedValueContract,
};

/// One test-local schema identity, distinct per name.
fn fixture_schema_id(name: &str) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(name.as_bytes()),
    )
    .expect("fixture schema id")
}

fn reviewed_ref(label: &str) -> ContentRef {
    let value = CanonicalJsonBytes::from_value(&CanonicalValue::String(label.to_owned()));
    ContentRef::new(
        fixture_schema_id("mfm.test.reviewed-evidence"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(value.as_bytes()),
        ),
    )
    .expect("fixture content ref")
}

fn retained_contract(media_type: &str) -> mfm_values::Result<RetainedValueContract> {
    RetainedValueContract::new(
        fixture_schema_id("mfm.test.retained-canonical-value"),
        SemanticTypeId::new(
            "mfm.test",
            "retained-value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"retained-value"),
        )
        .expect("fixture semantic type"),
        StableId::new("retained-value").expect("fixture role"),
        MediaType::new(media_type)?,
        reviewed_ref("retained-value.evidence"),
    )
}

#[test]
fn component_object_evidence_contract_keeps_exact_bytes_and_identity() {
    let evidence = ComponentObjectEvidence::current();
    let canonical = evidence
        .encode_canonical()
        .expect("component evidence canonical");
    assert_eq!(
        canonical.as_str(),
        r#"{"version":"mfm.component-object-evidence-contract.v1"}"#
    );

    let content_ref = evidence
        .content_ref()
        .expect("component evidence content ref");
    assert_eq!(
        content_ref.schema_id().as_str(),
        "schema:mfm.component-object-evidence-contract:1:sha256-jcs-v1:6dc5cc6a8069722963e7e2c6bedbd4e9e182a6b33e6b3b2136cf2441aa7f8b68"
    );
    assert_eq!(
        content_ref.content_digest().as_str(),
        "content:sha256-v1:52f0fff94cc51dcdab491860c892253fdc5275a62dee678330689f24ebe4475c"
    );
}

#[test]
fn retained_contract_has_one_exact_owner_validated_round_trip() {
    let original = retained_contract("application/json").expect("retained contract");
    let canonical = original
        .encode_canonical()
        .expect("canonical retained contract");
    let decoded = RetainedValueContract::decode_canonical(canonical.as_bytes())
        .expect("strict retained decode");
    let serde_decoded: RetainedValueContract =
        serde_json::from_slice(canonical.as_bytes()).expect("strict typed decode");

    assert_eq!(decoded, original);
    assert_eq!(serde_decoded, original);
    assert_eq!(
        decoded
            .encode_canonical()
            .expect("owner-validated retained contract")
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
        RetainedValueContract::decode_canonical(canonical.as_bytes()).is_err(),
        "owner strict decode must reject unknown fields"
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
        "Deserialize must revalidate the owner media-type grammar"
    );
}
