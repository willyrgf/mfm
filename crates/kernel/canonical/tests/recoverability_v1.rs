use mfm_canonical::{CanonicalValue, RecoverabilityContract, RecoverabilityErrorCode};
use mfm_ids::{DigestAlgorithm, RunId};
use serde_json::Value;

const ANNEX: &[u8] = include_bytes!("../../../../contracts/recoverability/v1/annex.json");
const CORPUS: &[u8] = include_bytes!("../../../../contracts/recoverability/v1/corpus.json");

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex fixture has an even length");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex fixture is ASCII");
            u8::from_str_radix(pair, 16).expect("hex fixture is valid")
        })
        .collect()
}

#[test]
fn embedded_annex_is_the_exact_closed_authority() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex is valid");

    assert_eq!(contract.annex_bytes(), ANNEX);
    RecoverabilityContract::validate_annex_candidate(ANNEX).expect("candidate is valid");

    let mut noncanonical = ANNEX.to_vec();
    noncanonical.push(b'\n');
    let error = RecoverabilityContract::validate_annex_candidate(&noncanonical)
        .expect_err("noncanonical annex bytes are rejected");
    assert_eq!(error.code(), RecoverabilityErrorCode::InvalidAnnex);
}

#[test]
fn corpus_schema_vectors_round_trip_exact_bytes_and_identities() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex is valid");
    let corpus: Value = serde_json::from_slice(CORPUS).expect("corpus is JSON");
    let vectors = corpus["positive_vectors"]
        .as_array()
        .expect("positive vectors are an array");

    for vector in vectors {
        if vector["kind"] != "schema_acceptance" {
            continue;
        }
        let id = vector["id"].as_str().expect("vector id");
        let schema = vector["schema_contract"].as_str().expect("schema contract");
        let input = decode_hex(vector["input_hex"].as_str().expect("input bytes"));
        let expected = decode_hex(vector["canonical_hex"].as_str().expect("canonical bytes"));
        let value = contract
            .strict_decode(schema, &input)
            .unwrap_or_else(|error| panic!("{id}: {error}"));

        assert_eq!(value.as_bytes(), expected, "{id}");
        assert_eq!(
            value.schema_id().as_str(),
            vector["expected_schema_id"].as_str().expect("schema id"),
            "{id}",
        );
        assert_eq!(
            contract
                .content_ref(&value)
                .expect("content ref")
                .content_digest()
                .as_str(),
            vector["expected_content_digest"]
                .as_str()
                .expect("content digest"),
            "{id}",
        );
    }
}

#[test]
fn structured_wire_contract_rejects_old_or_noncanonical_shapes() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex is valid");
    let valid = br#"{"journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"kind":"advanced","reason":null,"run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#;
    contract
        .strict_decode("mfm.drive-response.v1", valid)
        .expect("numeric structured journal head is accepted");

    let old_string_head = br#"{"journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":"1"},"kind":"advanced","reason":null,"run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#;
    assert_eq!(
        contract
            .strict_decode("mfm.drive-response.v1", old_string_head)
            .expect_err("legacy string sequence is rejected")
            .code(),
        RecoverabilityErrorCode::WrongType,
    );

    let graph_field = br#"{"journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"kind":"advanced","node_id":"node:retired","reason":null,"run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#;
    assert_eq!(
        contract
            .strict_decode("mfm.drive-response.v1", graph_field)
            .expect_err("retired graph field is rejected")
            .code(),
        RecoverabilityErrorCode::UnknownField,
    );

    let noncanonical = br#"{"kind":"advanced","journal_head":{"commit_digest":"sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000","run_sequence":1},"reason":null,"run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#;
    assert_eq!(
        contract
            .strict_decode("mfm.drive-response.v1", noncanonical)
            .expect_err("noncanonical key order is rejected")
            .code(),
        RecoverabilityErrorCode::InvalidCanonicalJson,
    );
}

#[test]
fn primitive_canonical_value_rejects_signed_json_numbers() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex is valid");
    for bytes in [b"-1".as_slice(), b"-9223372036854775808"] {
        assert_eq!(
            contract
                .strict_decode("mfm.primitive-canonical_value.v1", bytes)
                .expect_err("signed native number must be rejected")
                .code(),
            RecoverabilityErrorCode::WrongType,
        );
    }
    contract
        .strict_decode("mfm.primitive-canonical_value.v1", b"1")
        .expect("unsigned native number remains accepted");
}

#[test]
fn current_fact_identity_query_and_run_domains_are_derived() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex is valid");
    let corpus: Value = serde_json::from_slice(CORPUS).expect("corpus is JSON");
    let vectors = corpus["positive_vectors"]
        .as_array()
        .expect("positive vectors");
    let vector = vectors
        .iter()
        .find(|vector| vector["id"] == "domain/mfm.fact-query.v1/minimum")
        .expect("fact-query vector");
    let preimage = contract
        .strict_decode(
            vector["preimage_schema"].as_str().expect("preimage schema"),
            &decode_hex(vector["value_hex"].as_str().expect("preimage bytes")),
        )
        .expect("fact-query preimage");
    let digest = contract
        .derive_fact_query_digest(&preimage)
        .expect("fact-query digest");
    assert_eq!(
        digest.as_str(),
        vector["expected"]["value"]
            .as_str()
            .expect("expected digest"),
    );

    let content_vector = vectors
        .iter()
        .find(|vector| vector["id"] == "domain/mfm.fact-content-identity.v1/minimum")
        .expect("fact-content vector");
    let content_preimage = contract
        .strict_decode(
            content_vector["preimage_schema"]
                .as_str()
                .expect("fact-content schema"),
            &decode_hex(
                content_vector["value_hex"]
                    .as_str()
                    .expect("fact-content bytes"),
            ),
        )
        .expect("fact-content preimage");
    assert_eq!(
        contract
            .derive_fact_content_identity(&content_preimage)
            .expect("fact-content identity")
            .as_str(),
        content_vector["expected"]["value"]
            .as_str()
            .expect("expected fact-content identity"),
    );

    let logical_vector = vectors
        .iter()
        .find(|vector| vector["id"] == "domain/mfm.fact-logical-identity.v1/minimum")
        .expect("fact-logical vector");
    let logical_preimage = contract
        .strict_decode(
            logical_vector["preimage_schema"]
                .as_str()
                .expect("fact-logical schema"),
            &decode_hex(
                logical_vector["value_hex"]
                    .as_str()
                    .expect("fact-logical bytes"),
            ),
        )
        .expect("fact-logical preimage");
    assert_eq!(
        contract
            .derive_fact_logical_identity(&logical_preimage)
            .expect("fact-logical identity")
            .as_str(),
        logical_vector["expected"]["value"]
            .as_str()
            .expect("expected fact-logical identity"),
    );

    let run_preimage = CanonicalValue::object([
        (
            "entry_point_operation_id",
            CanonicalValue::String("mfm.portfolio/snapshot".to_owned()),
        ),
        (
            "invocation_identity",
            CanonicalValue::String("00000000-0000-4000-8000-000000000000".to_owned()),
        ),
        (
            "store_scope_id",
            CanonicalValue::String(
                "mfm.store_scope.v1:00000000000000000000000000000000".to_owned(),
            ),
        ),
        (
            "tenant_scope_id",
            CanonicalValue::String(
                "mfm.tenant_scope.v1:11111111111111111111111111111111".to_owned(),
            ),
        ),
    ])
    .expect("run preimage object");
    let run_preimage = contract
        .encode("mfm.run-id-preimage.v1", &run_preimage)
        .expect("run preimage schema");
    let run_id = contract.derive_run_id(&run_preimage).expect("run id");
    assert_eq!(run_id.algorithm(), DigestAlgorithm::Sha256JcsV1);
    assert_eq!(
        RunId::parse(run_id.as_str()).expect("checked run id"),
        run_id
    );

    assert_eq!(
        contract
            .semantic_digest("mfm.retired-domain.v1", &preimage)
            .expect_err("retired domain is absent")
            .code(),
        RecoverabilityErrorCode::UnknownDomain,
    );
}

#[test]
fn negative_corpus_vectors_report_the_frozen_code() {
    let contract = RecoverabilityContract::embedded().expect("embedded annex is valid");
    let corpus: Value = serde_json::from_slice(CORPUS).expect("corpus is JSON");
    for vector in corpus["negative_vectors"]
        .as_array()
        .expect("negative vectors")
    {
        let error = contract
            .strict_decode(
                vector["schema_contract"].as_str().expect("schema contract"),
                &decode_hex(vector["input_hex"].as_str().expect("input bytes")),
            )
            .expect_err("negative vector is rejected");
        assert_eq!(
            error.code().as_str(),
            vector["expected_error"].as_str().expect("error code"),
            "{}",
            vector["id"].as_str().expect("vector id"),
        );
    }
}
