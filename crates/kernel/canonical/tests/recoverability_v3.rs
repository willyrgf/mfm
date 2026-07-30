use mfm_canonical::{
    CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContract, RecoverabilityErrorCode,
    ReferenceTerminalKind,
};
use mfm_ids::{
    AttemptId, ContentRef, DigestAlgorithm, DigestBytes, EffectKey, SchemaId, SemanticDigest,
    TenantScopeId,
};
use serde_json::Value;

#[path = "../../../../tests/support/recoverability_v3.rs"]
mod recoverability_v3_support;

const CORPUS_BYTES: &[u8] = include_bytes!("../../../../contracts/recoverability/v3/corpus.json");

#[test]
fn canonical_executes_every_frozen_recoverability_vector() {
    recoverability_v3_support::run_consumer("mfm-canonical", |vector| {
        recoverability_v3_support::assert_lower_layer_owner_vector(vector);
    });
}

#[test]
fn typed_encoding_uses_the_same_frozen_schema_boundary() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let value = contract
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String("sample".to_owned()),
        )
        .expect("typed canonical encoding");
    assert_eq!(value.schema_contract(), "mfm.primitive-stable_id.v1");
    assert_eq!(value.as_bytes(), br#""sample""#);
    assert_eq!(
        value.schema_id(),
        contract
            .schema_id("mfm.primitive-stable_id.v1")
            .expect("registered schema")
    );
    assert_eq!(
        contract
            .derive_request_digest(&value)
            .expect_err("fixed request domain rejects another schema")
            .code(),
        RecoverabilityErrorCode::SchemaMismatch
    );
}

#[test]
fn cross_run_source_redaction_derivation_uses_only_the_exact_registered_preimage() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let corpus = corpus();
    let vectors = array(&corpus, "positive_vectors")
        .iter()
        .filter(|vector| {
            vector["kind"].as_str() == Some("domain_identity")
                && vector["domain"].as_str() == Some("mfm.cross-run-source-redaction.v1")
        })
        .collect::<Vec<_>>();
    assert_eq!(vectors.len(), 2, "both closed source-reference variants");

    for vector in vectors {
        let preimage = contract
            .strict_decode(
                "mfm.cross-run-source-ref.v1",
                &hex_field(vector, "value_hex"),
            )
            .expect("cross-run source preimage");
        let digest = contract
            .derive_cross_run_source_redaction_digest(&preimage)
            .expect("redaction digest");
        assert_eq!(
            digest.as_str(),
            string(object(vector, "expected"), "value"),
            "{}",
            string(vector, "id")
        );
    }

    let wrong_preimage = contract
        .strict_decode("mfm.primitive-stable_id.v1", br#""sample""#)
        .expect("other registered preimage");
    assert_eq!(
        contract
            .derive_cross_run_source_redaction_digest(&wrong_preimage)
            .expect_err("wrong preimage schema")
            .code(),
        RecoverabilityErrorCode::SchemaMismatch
    );
}

#[test]
fn schema_identity_lookup_and_decode_require_the_exact_registered_identity() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let schema_id = contract
        .schema_id("mfm.content-ref.v1")
        .expect("content-ref schema")
        .clone();
    let bytes = schema_golden("mfm.content-ref.v1");

    assert_eq!(
        contract
            .schema_contract_for_id(&schema_id)
            .expect("schema contract"),
        "mfm.content-ref.v1"
    );
    let decoded = contract
        .strict_decode_schema_id(&schema_id, &bytes)
        .expect("schema-id decode");
    assert_eq!(decoded.schema_contract(), "mfm.content-ref.v1");
    assert_eq!(decoded.schema_id(), &schema_id);
    assert_eq!(decoded.as_bytes(), bytes);

    let substituted = SchemaId::new(
        "mfm.content-ref",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x7b; 32]),
    )
    .expect("well-formed unregistered schema identity");
    for error in [
        contract
            .schema_contract_for_id(&substituted)
            .expect_err("digest substitution must not resolve"),
        contract
            .strict_decode_schema_id(&substituted, &bytes)
            .expect_err("digest substitution must not decode"),
    ] {
        assert_eq!(error.code(), RecoverabilityErrorCode::UnknownSchema);
    }
}

#[test]
fn reference_projection_resolves_alias_nullable_union_and_terminal_stop() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");

    let alias = contract
        .strict_decode(
            "mfm.capability-binding-ref.v1",
            &schema_golden("mfm.capability-binding-ref.v1"),
        )
        .expect("alias value");
    let alias_edges = contract.reference_edges(&alias).expect("alias edges");
    assert_eq!(alias_edges.len(), 1);
    assert_eq!(alias_edges[0].path().as_str(), "");
    assert_eq!(alias_edges[0].declared_contract(), "mfm.content-ref.v1");
    assert_eq!(
        alias_edges[0].terminal_kind(),
        ReferenceTerminalKind::ContentRef
    );
    assert_eq!(
        alias_edges[0].value().schema_contract(),
        "mfm.content-ref.v1"
    );

    let mut read_authorization: Value =
        serde_json::from_slice(&schema_golden("mfm.external-access-authorized.v1"))
            .expect("authorization golden");
    let value_ref = read_authorization["request_ref"].clone();
    read_authorization["scope"] = serde_json::json!({
        "kind": "read",
        "input_manifest_ref": value_ref.clone(),
    });
    read_authorization["frozen_read_intent_ref"] = value_ref;
    let authorization = contract
        .strict_decode(
            "mfm.external-access-authorized.v1",
            &canonical_value_bytes(&read_authorization),
        )
        .expect("read authorization");
    let edges = contract
        .reference_edges(&authorization)
        .expect("authorization references");
    let projected = edges
        .iter()
        .map(|edge| {
            (
                edge.path().as_str(),
                edge.declared_contract(),
                edge.terminal_kind(),
                edge.value().schema_contract(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        projected,
        vec![
            (
                "/capability_binding_ref",
                "mfm.capability-binding-ref.v1",
                ReferenceTerminalKind::ContentRef,
                "mfm.content-ref.v1",
            ),
            (
                "/frozen_read_intent_ref",
                "mfm.value-ref.v1",
                ReferenceTerminalKind::ValueRef,
                "mfm.value-ref.v1",
            ),
            (
                "/request_ref",
                "mfm.value-ref.v1",
                ReferenceTerminalKind::ValueRef,
                "mfm.value-ref.v1",
            ),
            (
                "/scope/input_manifest_ref",
                "mfm.input-manifest-ref.v1",
                ReferenceTerminalKind::ValueRef,
                "mfm.value-ref.v1",
            ),
        ]
    );
}

#[test]
fn reference_projection_stops_at_a_transport_only_root_value_ref() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let value_ref = contract
        .strict_decode("mfm.value-ref.v1", &schema_golden("mfm.value-ref.v1"))
        .expect("root value ref");

    assert!(
        contract
            .reference_edges(&value_ref)
            .expect("root value-ref projection")
            .is_empty(),
        "an incoming content reference is already the semantic edge"
    );
}

#[test]
fn reference_projection_preserves_canonical_array_paths_and_repeated_values() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let content_ref: Value =
        serde_json::from_slice(&schema_golden("mfm.content-ref.v1")).expect("content ref");
    let value_ref: Value =
        serde_json::from_slice(&schema_golden("mfm.value-ref.v1")).expect("value ref");

    let mut next_content_ref = content_ref.clone();
    next_content_ref["content_digest"] =
        Value::String(format!("content:sha256-v1:{}1", "0".repeat(63)));
    let mut next_value_ref = value_ref.clone();
    next_value_ref["artifact_id"] =
        Value::String(format!("artifact:sha256-jcs-v1:{}1", "0".repeat(63)));
    let closure = serde_json::json!({
        "root_source_manifest_ref": content_ref.clone(),
        "ordered_dependency_refs": [
            content_ref,
            next_content_ref,
        ],
        "ordered_object_refs": [
            value_ref,
            next_value_ref,
        ],
    });
    let closure = contract
        .strict_decode(
            "mfm.source-closure-preimage.v1",
            &canonical_value_bytes(&closure),
        )
        .expect("non-empty closure preimage");
    let edges = contract
        .reference_edges(&closure)
        .expect("closure reference edges");
    assert_eq!(
        edges
            .iter()
            .map(|edge| (edge.path().as_str(), edge.terminal_kind()))
            .collect::<Vec<_>>(),
        vec![
            (
                "/ordered_dependency_refs/0",
                ReferenceTerminalKind::ContentRef,
            ),
            (
                "/ordered_dependency_refs/1",
                ReferenceTerminalKind::ContentRef,
            ),
            ("/ordered_object_refs/0", ReferenceTerminalKind::ValueRef,),
            ("/ordered_object_refs/1", ReferenceTerminalKind::ValueRef,),
            (
                "/root_source_manifest_ref",
                ReferenceTerminalKind::ContentRef,
            ),
        ]
    );

    let claim = contract
        .strict_decode(
            "mfm.fact-claim-envelope.v1",
            &schema_golden("mfm.fact-claim-envelope.v1"),
        )
        .expect("fact claim");
    let claim_edges = contract.reference_edges(&claim).expect("claim edges");
    assert_eq!(
        claim_edges
            .iter()
            .map(|edge| edge.path().as_str())
            .collect::<Vec<_>>(),
        vec!["/fact_descriptor_ref", "/response_ref", "/subject_ref"]
    );
    assert_eq!(
        claim_edges[1].value().as_bytes(),
        claim_edges[2].value().as_bytes(),
        "equal references at distinct canonical paths must not be deduplicated"
    );
}

#[test]
fn reference_projection_never_scans_native_keys_or_text() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let content_ref = contract
        .strict_decode("mfm.content-ref.v1", &schema_golden("mfm.content-ref.v1"))
        .expect("content-ref value")
        .canonical_value()
        .expect("typed content-ref value");
    let native = CanonicalValue::object([
        ("content_ref", content_ref),
        (
            "reference_like_text",
            CanonicalValue::String(
                "content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            ),
        ),
    ])
    .expect("native object");
    let native = contract
        .encode("mfm.primitive-canonical_value.v1", &native)
        .expect("native canonical value");
    assert!(
        contract
            .reference_edges(&native)
            .expect("native projection")
            .is_empty(),
        "untyped keys and text cannot create reference authority"
    );
}

#[test]
fn ids_serde_uses_exact_closed_recoverability_shapes() {
    let tenant = TenantScopeId::new(format!(
        "{}{}",
        TenantScopeId::PREFIX,
        "0123456789abcdef0123456789abcdef"
    ))
    .expect("tenant scope");
    assert_eq!(
        serde_json::from_value::<TenantScopeId>(
            serde_json::to_value(&tenant).expect("serialize tenant")
        )
        .expect("deserialize tenant"),
        tenant
    );

    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let corpus = corpus();
    let vector = array(&corpus, "positive_vectors")
        .iter()
        .find(|vector| {
            string(vector, "kind") == "schema_acceptance"
                && string(vector, "schema_contract") == "mfm.content-ref.v1"
        })
        .expect("content-ref vector");
    let content_ref: ContentRef =
        serde_json::from_slice(&hex_field(vector, "input_hex")).expect("content ref");
    assert_eq!(
        serde_json::to_vec(&content_ref).expect("serialize canonical content ref"),
        hex_field(vector, "input_hex"),
        "ContentRef field order must already be canonical"
    );
    let wire = serde_json::to_value(&content_ref).expect("serialize content ref");
    assert_eq!(
        serde_json::from_value::<ContentRef>(wire.clone()).expect("deserialize content ref"),
        content_ref
    );
    contract
        .strict_decode(
            "mfm.content-ref.v1",
            &PlainCanonicalJsonBytes::from_json_str(&wire.to_string())
                .expect("canonical content ref")
                .to_vec(),
        )
        .expect("annex content-ref schema");

    let mut unknown = wire.clone();
    unknown
        .as_object_mut()
        .expect("content-ref object")
        .insert("unexpected".to_owned(), Value::Bool(true));
    assert!(serde_json::from_value::<ContentRef>(unknown).is_err());

    let mut substituted = wire;
    let digest = substituted["content_digest"]
        .as_str()
        .expect("content digest")
        .replace("sha256-v1", "sha256-jcs-v1");
    substituted["content_digest"] = Value::String(digest);
    assert!(serde_json::from_value::<ContentRef>(substituted).is_err());

    let digest = DigestBytes::from_array([0x5a; 32]);
    for value in [
        serde_json::to_value(SemanticDigest::from_digest(digest)).expect("semantic digest"),
        serde_json::to_value(EffectKey::from_digest(digest)).expect("effect key"),
        serde_json::to_value(AttemptId::from_digest(digest)).expect("attempt id"),
    ] {
        assert!(
            value.is_string(),
            "semantic identities use scalar wire forms"
        );
    }
}

#[test]
fn hostile_candidate_and_value_text_never_enters_public_errors() {
    const CANARY: &str = "fixture_private_schema_field_value";

    let oversized = vec![b' '; 16_777_217];
    let error = RecoverabilityContract::validate_annex_candidate(&oversized)
        .expect_err("oversized annex candidate");
    assert_eq!(error.code(), RecoverabilityErrorCode::InvalidAnnex);

    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    for error in [
        contract
            .strict_decode(CANARY, b"{}")
            .expect_err("unknown schema"),
        contract
            .semantic_digest(
                CANARY,
                &contract
                    .strict_decode("mfm.primitive-stable_id.v1", br#""sample""#)
                    .expect("known value"),
            )
            .expect_err("unknown domain"),
        contract
            .strict_decode("mfm.run-phase.v1", format!("\"{CANARY}\"").as_bytes())
            .expect_err("hostile value"),
    ] {
        assert!(!error.to_string().contains(CANARY));
    }

    let corpus = corpus();
    let content_ref = array(&corpus, "positive_vectors")
        .iter()
        .find(|vector| {
            string(vector, "kind") == "schema_acceptance"
                && string(vector, "schema_contract") == "mfm.content-ref.v1"
        })
        .expect("content-ref vector");
    let mut value: Value =
        serde_json::from_slice(&hex_field(content_ref, "input_hex")).expect("content ref JSON");
    value
        .as_object_mut()
        .expect("content-ref object")
        .insert(CANARY.to_owned(), Value::String(CANARY.to_owned()));
    let error = contract
        .strict_decode("mfm.content-ref.v1", &canonical_value_bytes(&value))
        .expect_err("hostile field");
    assert!(!error.to_string().contains(CANARY));

    let mut candidate: Value =
        serde_json::from_slice(contract.annex_bytes()).expect("annex candidate");
    let descriptor = candidate["schemas"]
        .as_array_mut()
        .expect("schemas")
        .iter_mut()
        .find(|schema| schema["contract"] == "mfm.access-audit-entry.v2")
        .expect("object schema");
    descriptor["shape"]["fields"][0]["name"] = Value::String(CANARY.to_owned());
    let error =
        RecoverabilityContract::validate_annex_candidate(&canonical_value_bytes(&candidate))
            .expect_err("hostile annex field");
    assert!(!error.to_string().contains(CANARY));
}

#[test]
fn annex_rejects_ordering_fields_absent_from_referenced_item_objects() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let mut candidate: Value =
        serde_json::from_slice(contract.annex_bytes()).expect("annex candidate");
    let descriptor = candidate["schemas"]
        .as_array_mut()
        .expect("schemas")
        .iter_mut()
        .find(|schema| schema["contract"] == "mfm.certified-settlement-contract.v1")
        .expect("certified settlement schema");
    let fact_slots = descriptor["shape"]["fields"]
        .as_array_mut()
        .expect("settlement fields")
        .iter_mut()
        .find(|field| field["name"] == "fact_slots")
        .expect("fact slots");
    fact_slots["type"]["ordering"] = Value::String("field:emission_ordinal".to_owned());

    let mut descriptor_preimage = descriptor.clone();
    descriptor_preimage
        .as_object_mut()
        .expect("schema descriptor")
        .remove("schema_id");
    let descriptor_preimage = contract
        .strict_decode(
            "mfm.schema-descriptor.v1",
            &canonical_value_bytes(&descriptor_preimage),
        )
        .expect("mutated schema descriptor preimage");
    let mutated_schema_id = contract
        .derive_schema_id(&descriptor_preimage)
        .expect("mutated schema identity");
    descriptor["schema_id"] = Value::String(mutated_schema_id.to_string());

    let error =
        RecoverabilityContract::validate_annex_candidate(&canonical_value_bytes(&candidate))
            .expect_err("ordering field absent from referenced item object");
    assert_eq!(error.code(), RecoverabilityErrorCode::InvalidAnnex);
}

#[test]
fn candidate_annex_rejects_every_noncanonical_reference_cycle() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex");
    let mut candidate: Value =
        serde_json::from_slice(contract.annex_bytes()).expect("annex candidate");
    let descriptor = candidate["schemas"]
        .as_array_mut()
        .expect("schemas")
        .iter_mut()
        .find(|schema| schema["contract"] == "mfm.run-id.v1")
        .expect("run-id schema");
    descriptor["shape"]["contract"] = Value::String("mfm.run-id.v1".to_owned());
    let error =
        RecoverabilityContract::validate_annex_candidate(&canonical_value_bytes(&candidate))
            .expect_err("unreviewed schema cycle");
    assert_eq!(error.code(), RecoverabilityErrorCode::InvalidAnnex);
}

fn corpus() -> Value {
    serde_json::from_slice(CORPUS_BYTES).expect("recoverability corpus")
}

fn schema_golden(contract: &str) -> Vec<u8> {
    array(&corpus(), "positive_vectors")
        .iter()
        .find(|vector| {
            string(vector, "kind") == "schema_acceptance"
                && string(vector, "schema_contract") == contract
        })
        .map(|vector| hex_field(vector, "canonical_hex"))
        .unwrap_or_else(|| panic!("missing schema golden for {contract}"))
}

fn canonical_value_bytes(value: &Value) -> Vec<u8> {
    PlainCanonicalJsonBytes::from_json_str(&value.to_string())
        .expect("canonical JSON value")
        .to_vec()
}

fn hex_field(value: &Value, key: &str) -> Vec<u8> {
    decode_hex(string(value, key))
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex length");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex UTF-8");
            u8::from_str_radix(text, 16).expect("lowercase hex")
        })
        .collect()
}

fn object<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or_else(|| panic!("missing {key}"))
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    object(value, key)
        .as_array()
        .unwrap_or_else(|| panic!("{key} is not an array"))
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    object(value, key)
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string"))
}
