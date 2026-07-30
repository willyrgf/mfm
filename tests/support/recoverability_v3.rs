use std::collections::BTreeSet;

use mfm_canonical::{
    sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContract, RecoverabilityErrorCode,
};
use serde_json::Value;

const ANNEX_BYTES: &[u8] = include_bytes!("../../contracts/recoverability/v3/annex.json");
const CORPUS_BYTES: &[u8] = include_bytes!("../../contracts/recoverability/v3/corpus.json");
const ANNEX_BYTE_LENGTH: usize = 227_880;
const CORPUS_BYTE_LENGTH: usize = 1_741_323;
const ANNEX_SHA256_HEX: &str = "50cbcaa6185c36ebc652277a178108a8a3785285a5871f5d720e48d2c16dd0d7";
const CORPUS_SHA256_HEX: &str = "ec9220cca6dd0e7da1cfda485006d58353baa3470c3474aba20d782058efc942";
const POSITIVE_VECTOR_COUNT: usize = 441;
const NEGATIVE_VECTOR_COUNT: usize = 62;
const RELATIONAL_VECTOR_COUNT: usize = 84;
const TOTAL_VECTOR_COUNT: usize =
    POSITIVE_VECTOR_COUNT + NEGATIVE_VECTOR_COUNT + RELATIONAL_VECTOR_COUNT;

/// One vector whose owner-specific invariant must be executed by the consumer.
#[derive(Debug, Clone, Copy)]
pub enum OwnerVector<'a> {
    /// A positive relational vector from `relational_vectors`.
    RelationalPositive(&'a Value),
    /// A relational rejection from `negative_vectors`.
    RelationalRejection(&'a Value),
}

impl<'a> OwnerVector<'a> {
    /// Returns the exact vector object from the frozen corpus.
    pub const fn vector(self) -> &'a Value {
        match self {
            Self::RelationalPositive(vector) | Self::RelationalRejection(vector) => vector,
        }
    }

    /// Returns the vector id.
    pub fn id(self) -> &'a str {
        string(self.vector(), "id")
    }

    /// Returns the vector kind.
    pub fn kind(self) -> &'a str {
        string(self.vector(), "kind")
    }
}

/// One exact vector object from a frozen corpus section.
#[derive(Debug, Clone, Copy)]
pub enum CorpusVector<'a> {
    /// A vector from `positive_vectors`.
    Positive(&'a Value),
    /// A vector from `negative_vectors`.
    Negative(&'a Value),
    /// A vector from `relational_vectors`.
    Relational(&'a Value),
}

impl<'a> CorpusVector<'a> {
    /// Returns the exact decoded vector object.
    pub const fn vector(self) -> &'a Value {
        match self {
            Self::Positive(vector) | Self::Negative(vector) | Self::Relational(vector) => vector,
        }
    }

    /// Returns the vector id.
    pub fn id(self) -> &'a str {
        string(self.vector(), "id")
    }

    /// Canonically encodes this exact corpus vector object.
    pub fn canonical_bytes(self) -> Vec<u8> {
        PlainCanonicalJsonBytes::from_json_str(&self.vector().to_string())
            .expect("canonical corpus vector")
            .to_vec()
    }
}

/// Visits all 587 frozen vector objects without filtering or copying corpus authority.
///
/// This optional visitor is used by consumers that must persist every corpus
/// object; the generic executor below does not require a second traversal.
#[allow(dead_code)]
pub fn for_each_vector(mut visit: impl FnMut(CorpusVector<'_>)) {
    let corpus = corpus();
    let positives = array(&corpus, "positive_vectors");
    let negatives = array(&corpus, "negative_vectors");
    let relationals = array(&corpus, "relational_vectors");

    for vector in positives {
        visit(CorpusVector::Positive(vector));
    }
    for vector in negatives {
        visit(CorpusVector::Negative(vector));
    }
    for vector in relationals {
        visit(CorpusVector::Relational(vector));
    }

    assert_eq!(positives.len(), POSITIVE_VECTOR_COUNT);
    assert_eq!(negatives.len(), NEGATIVE_VECTOR_COUNT);
    assert_eq!(relationals.len(), RELATIONAL_VECTOR_COUNT);
    assert_eq!(
        positives.len() + negatives.len() + relationals.len(),
        TOTAL_VECTOR_COUNT
    );
}

/// Executes the complete frozen corpus for one named consumer.
///
/// The callback is mandatory and is invoked exactly once for every positive
/// relational vector and every relational rejection. It must assert the
/// consumer-owned invariant or rejection; returning means that vector passed.
pub fn run_consumer(consumer: &str, mut execute_owner_vector: impl FnMut(OwnerVector<'_>)) {
    let contract = RecoverabilityContract::embedded().expect("embedded recoverability annex");
    assert_artifact_bindings(contract);
    let corpus = corpus();
    assert_eq!(string(&corpus, "contract"), "mfm.recoverability-corpus.v3");
    let coverage = object(&corpus, "coverage");
    let mandatory_consumers: BTreeSet<&str> = array(coverage, "mandatory_consumers")
        .iter()
        .map(|value| value.as_str().expect("mandatory consumer string"))
        .collect();
    assert!(
        mandatory_consumers.contains(consumer),
        "{consumer} is not a mandatory recoverability-v3 consumer"
    );

    let positives = array(&corpus, "positive_vectors");
    let known_domain = positives
        .iter()
        .find(|vector| string(vector, "kind") == "domain_identity")
        .expect("domain vector");
    let known_preimage = contract
        .strict_decode(
            string(known_domain, "preimage_schema"),
            &hex_field(known_domain, "value_hex"),
        )
        .expect("domain preimage");
    let known_content = positives
        .iter()
        .find(|vector| {
            string(vector, "kind") == "schema_acceptance"
                && string(vector, "schema_contract") == "mfm.content-ref.v1"
        })
        .expect("content-ref vector");
    let known_content = contract
        .strict_decode("mfm.content-ref.v1", &hex_field(known_content, "input_hex"))
        .expect("content-ref value");

    let mut positive_count = 0;
    for vector in positives {
        assert_corpus_vector(CorpusVector::Positive(vector));
        assert_consumer_coverage(vector, consumer);
        execute_positive(contract, vector);
        positive_count += 1;
    }

    let mut negative_count = 0;
    let mut owner_rejection_count = 0;
    for vector in array(&corpus, "negative_vectors") {
        assert_corpus_vector(CorpusVector::Negative(vector));
        assert_consumer_coverage(vector, consumer);
        match string(vector, "error_class") {
            "codec" => execute_codec_rejection(
                contract,
                vector,
                known_domain,
                &known_preimage,
                &known_content,
            ),
            "relational" => {
                execute_owner_vector(OwnerVector::RelationalRejection(vector));
                owner_rejection_count += 1;
            }
            other => panic!(
                "{}: unhandled negative error class {other}",
                string(vector, "id")
            ),
        }
        negative_count += 1;
    }

    let mut relational_count = 0;
    for vector in array(&corpus, "relational_vectors") {
        assert_corpus_vector(CorpusVector::Relational(vector));
        assert_consumer_coverage(vector, consumer);
        execute_owner_vector(OwnerVector::RelationalPositive(vector));
        relational_count += 1;
    }

    assert_eq!(positive_count, POSITIVE_VECTOR_COUNT);
    assert_eq!(negative_count, NEGATIVE_VECTOR_COUNT);
    assert_eq!(relational_count, RELATIONAL_VECTOR_COUNT);
    assert_eq!(
        positive_count + negative_count + relational_count,
        TOTAL_VECTOR_COUNT
    );
    assert_eq!(
        positive_count,
        usize_field(coverage, "positive_vector_count")
    );
    assert_eq!(
        negative_count,
        usize_field(coverage, "negative_vector_count")
    );
    assert_eq!(
        relational_count,
        usize_field(coverage, "relational_vector_count")
    );
    assert_eq!(
        owner_rejection_count,
        usize_field(coverage, "relational_error_count")
    );
}

fn assert_corpus_vector(vector: CorpusVector<'_>) {
    assert!(!vector.id().is_empty());
    assert!(!vector.canonical_bytes().is_empty());
}

/// Executes the canonical/identity boundary semantics common to lower-layer consumers.
///
/// Higher-layer owners should call this from their mandatory callback and then
/// assert their own storage, replay, authority, or runtime invariant.
pub fn assert_lower_layer_owner_vector(owner: OwnerVector<'_>) {
    let contract = RecoverabilityContract::embedded().expect("embedded recoverability annex");
    let vector = owner.vector();
    let id = owner.id();
    match owner {
        OwnerVector::RelationalRejection(_) => {
            let input = hex_field(vector, "input_hex");
            let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(&input)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            let value: Value =
                serde_json::from_slice(canonical.as_bytes()).expect("relational rejection JSON");
            let expected_error = string(vector, "expected_error");
            assert_eq!(id.strip_prefix("relational/"), Some(expected_error), "{id}");
            if let Some(case) = value.get("case") {
                assert_eq!(case.as_str(), Some(expected_error), "{id}");
            } else {
                assert!(
                    matches!(
                        expected_error,
                        "canonical_path_invalid" | "reserve_exhausted"
                    ),
                    "{id}: relational rejection omits its case discriminator"
                );
                assert!(
                    value.as_array().is_some_and(|items| !items.is_empty())
                        || value.as_object().is_some_and(|fields| !fields.is_empty()),
                    "{id}: relational rejection input is empty"
                );
            }
            assert!(!string(vector, "rule").is_empty(), "{id}");
            assert!(!string(vector, "target").is_empty(), "{id}");
        }
        OwnerVector::RelationalPositive(_) => match owner.kind() {
            "commit_coordinate_separation" => {
                let candidate = contract
                    .strict_decode(
                        "mfm.commit-candidate-preimage.v2",
                        &hex_field(vector, "candidate_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let committed = contract
                    .strict_decode(
                        "mfm.commit-envelope.v1",
                        &hex_field(vector, "committed_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let candidate: Value =
                    serde_json::from_slice(candidate.as_bytes()).expect("candidate JSON");
                let committed: Value =
                    serde_json::from_slice(committed.as_bytes()).expect("commit JSON");
                let candidate = candidate.as_object().expect("candidate object");
                let committed = committed.as_object().expect("commit object");
                for assigned in [
                    "append_request_id",
                    "committed_at",
                    "run_sequence",
                    "tenant_fact_coordinate",
                ] {
                    assert!(!candidate.contains_key(assigned), "{id}: {assigned}");
                    assert!(committed.contains_key(assigned), "{id}: {assigned}");
                }
            }
            "content_identity_separation" => {
                let base = contract.raw_content_digest(&hex_field(vector, "base_hex"));
                let prefixed = contract.raw_content_digest(&hex_field(vector, "prefixed_hex"));
                let nul = contract.raw_content_digest(&hex_field(vector, "nul_suffixed_hex"));
                assert_eq!(base.as_str(), string(vector, "base_digest"), "{id}");
                assert_eq!(prefixed.as_str(), string(vector, "prefixed_digest"), "{id}");
                assert_eq!(nul.as_str(), string(vector, "nul_suffixed_digest"), "{id}");
                assert_ne!(base, prefixed, "{id}");
                assert_ne!(base, nul, "{id}");
                assert_ne!(prefixed, nul, "{id}");
            }
            "export_identity" => {
                let stream = hex_field(vector, "stream_hex");
                assert_eq!(
                    string(vector, "media_type"),
                    "application/vnd.mfm.run-export-stream.v2+json-seq",
                    "{id}"
                );
                assert_eq!(
                    contract.raw_content_digest(&stream).as_str(),
                    string(vector, "expected_content_digest"),
                    "{id}"
                );
                assert_eq!(
                    contract
                        .schema_id("mfm.portable-run-export-stream.v2")
                        .unwrap_or_else(|error| panic!("{id}: {error}"))
                        .as_str(),
                    string(vector, "expected_schema_id"),
                    "{id}"
                );

                let mut frames = Vec::new();
                for record in stream.split(|byte| *byte == 0x1e).skip(1) {
                    let frame_bytes = record
                        .strip_suffix(b"\n")
                        .unwrap_or_else(|| panic!("{id}: frame lacks LF suffix"));
                    let frame = contract
                        .strict_decode("mfm.portable-run-export-frame.v2", frame_bytes)
                        .unwrap_or_else(|error| panic!("{id}: {error}"));
                    frames.push(
                        serde_json::from_slice::<Value>(frame.as_bytes())
                            .expect("portable frame JSON"),
                    );
                }
                assert_eq!(frames.len(), 2, "{id}");
                assert_eq!(string(&frames[0], "kind"), "header", "{id}");
                assert_eq!(string(&frames[1], "kind"), "end", "{id}");
                assert!(
                    frames.iter().all(|frame| frame
                        .as_object()
                        .expect("frame object")
                        .keys()
                        .all(|key| !key.contains("digest"))),
                    "{id}: stream contains a self digest"
                );
            }
            "frontier_order" => {
                let ancestor = canonical_json_field(vector, "ancestor_hex");
                let descendant = canonical_json_field(vector, "descendant_hex");
                let fork = canonical_json_field(vector, "fork_hex");
                let ancestor = array(&ancestor, "records");
                let descendant = array(&descendant, "records");
                let fork = array(&fork, "records");
                assert!(descendant.starts_with(ancestor), "{id}");
                assert!(!fork.starts_with(descendant), "{id}");
                assert!(!descendant.starts_with(fork), "{id}");
            }
            "identity_separation" => {
                if vector.get("first_envelope_hex").is_some() {
                    let first = hex_field(vector, "first_envelope_hex");
                    let second = hex_field(vector, "second_envelope_hex");
                    PlainCanonicalJsonBytes::from_canonical_json_slice(&first)
                        .expect("first semantic envelope");
                    PlainCanonicalJsonBytes::from_canonical_json_slice(&second)
                        .expect("second semantic envelope");
                    let first_digest = sha256_digest_bytes(&first);
                    let second_digest = sha256_digest_bytes(&second);
                    assert_eq!(
                        first_digest.to_string(),
                        string(vector, "first_digest_hex"),
                        "{id}"
                    );
                    assert_eq!(
                        second_digest.to_string(),
                        string(vector, "second_digest_hex"),
                        "{id}"
                    );
                    assert_ne!(first_digest, second_digest, "{id}");
                    if vector.get("preimage_schema").is_some() {
                        let first_envelope: Value =
                            serde_json::from_slice(&first).expect("first semantic envelope JSON");
                        let second_envelope: Value =
                            serde_json::from_slice(&second).expect("second semantic envelope JSON");
                        assert_eq!(
                            string(&first_envelope, "domain"),
                            string(vector, "domain"),
                            "{id}"
                        );
                        assert_eq!(
                            string(&second_envelope, "domain"),
                            string(vector, "domain"),
                            "{id}"
                        );
                        let base = contract
                            .strict_decode(
                                string(vector, "preimage_schema"),
                                &canonical_json_value(object(&first_envelope, "value")),
                            )
                            .unwrap_or_else(|error| panic!("{id}: {error}"));
                        let changed = contract
                            .strict_decode(
                                string(vector, "preimage_schema"),
                                &canonical_json_value(object(&second_envelope, "value")),
                            )
                            .unwrap_or_else(|error| panic!("{id}: {error}"));
                        let base_digest = contract
                            .derive_cross_run_source_redaction_digest(&base)
                            .unwrap_or_else(|error| panic!("{id}: {error}"));
                        let changed_digest = contract
                            .derive_cross_run_source_redaction_digest(&changed)
                            .unwrap_or_else(|error| panic!("{id}: {error}"));
                        assert_eq!(
                            base_digest.digest().to_string(),
                            first_digest.to_string(),
                            "{id}"
                        );
                        assert_eq!(
                            changed_digest.digest().to_string(),
                            second_digest.to_string(),
                            "{id}"
                        );
                    }
                } else {
                    let value = hex_field(vector, "value_hex");
                    let envelope = hex_field(vector, "semantic_envelope_hex");
                    let content = contract.raw_content_digest(&value);
                    assert_eq!(content.as_str(), string(vector, "content_digest"), "{id}");
                    let semantic = format!("sha256-jcs-v1:{}", sha256_digest_bytes(&envelope));
                    assert_eq!(semantic, string(vector, "semantic_digest"), "{id}");
                    assert_ne!(content.as_str(), semantic, "{id}");
                }
            }
            "read_returned_validation_verdict" => {
                let returned = canonical_json_field(vector, "returned_hex");
                assert_eq!(string(&returned, "outcome"), "returned", "{id}");
                assert_eq!(
                    string(&returned, "derived_terminal_tag"),
                    string(vector, "expected_typed_failure_tag"),
                    "{id}"
                );
                assert_read_verdict_metadata(vector, id);
                assert_typed_failure(contract, vector, id);
            }
            "read_safe_failure_verdict" => {
                let tuple = canonical_json_field(vector, "tuple_hex");
                assert!(
                    matches!(string(&tuple, "outcome"), "did_not_enter" | "indeterminate"),
                    "{id}"
                );
                assert!(!string(&tuple, "stable_code").is_empty(), "{id}");
                assert!(!string(&tuple, "failure_class").is_empty(), "{id}");
                assert!(!string(&tuple, "boundary_stage").is_empty(), "{id}");
                assert_read_verdict_metadata(vector, id);
                if vector.get("expected_typed_failure_hex").is_some() {
                    assert_typed_failure(contract, vector, id);
                }
            }
            "relational_acceptance" => {
                let observation = contract
                    .strict_decode(
                        "mfm.executor-delivery-attempt-observed.v2",
                        &hex_field(vector, "observation_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let proof = contract
                    .strict_decode(
                        "mfm.executor-reference-terminal-proof.v2",
                        &hex_field(vector, "proof_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let tombstone = contract
                    .strict_decode(
                        "mfm.executor-terminal-tombstone.v2",
                        &hex_field(vector, "tombstone_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let observation_json: Value =
                    serde_json::from_slice(observation.as_bytes()).expect("observation JSON");
                let proof_json: Value =
                    serde_json::from_slice(proof.as_bytes()).expect("proof JSON");
                let tombstone_json: Value =
                    serde_json::from_slice(tombstone.as_bytes()).expect("tombstone JSON");
                assert_eq!(
                    string(&observation_json, "attempt_id"),
                    string(&proof_json, "attempt_id"),
                    "{id}"
                );
                assert_eq!(
                    object(object(&observation_json, "outcome"), "returned_outcome"),
                    object(&proof_json, "returned_outcome"),
                    "{id}"
                );
                assert_eq!(
                    string(
                        object(&proof_json, "returned_observation_ref"),
                        "content_digest"
                    ),
                    contract.raw_content_digest(observation.as_bytes()).as_str(),
                    "{id}"
                );
                assert_eq!(
                    string(
                        object(&tombstone_json, "terminal_proof_ref"),
                        "content_digest"
                    ),
                    contract.raw_content_digest(proof.as_bytes()).as_str(),
                    "{id}"
                );
            }
            "request_identity" => {
                let preimage = contract
                    .strict_decode(
                        "mfm.request-digest-preimage.v1",
                        &hex_field(vector, "preimage_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let changed = contract
                    .strict_decode(
                        "mfm.request-digest-preimage.v1",
                        &hex_field(vector, "changed_preimage_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let request = contract
                    .derive_request_digest(&preimage)
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let changed = contract
                    .derive_request_digest(&changed)
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                assert_eq!(request.as_str(), string(vector, "request_digest"), "{id}");
                assert_eq!(
                    changed.as_str(),
                    string(vector, "changed_request_digest"),
                    "{id}"
                );
                assert_ne!(request, changed, "{id}");
            }
            "resource_refold" => {
                let allocation = contract
                    .strict_decode(
                        "mfm.executor-resource-allocated.v1",
                        &hex_field(vector, "allocation_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let restored = contract
                    .strict_decode(
                        "mfm.executor-resource-allocated.v1",
                        &hex_field(vector, "restored_allocation_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                assert_eq!(allocation.as_bytes(), restored.as_bytes(), "{id}");
            }
            "schema_identity" => {
                let descriptor = contract
                    .strict_decode(
                        "mfm.schema-descriptor.v1",
                        &hex_field(vector, "descriptor_preimage_hex"),
                    )
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                let schema_id = contract
                    .derive_schema_id(&descriptor)
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                assert_eq!(schema_id.as_str(), string(vector, "schema_id"), "{id}");
                assert!(
                    !descriptor.as_str().contains(schema_id.as_str()),
                    "{id}: schema id entered its own preimage"
                );
            }
            "type_separation" => {
                let content_ref = hex_field(vector, "content_ref_hex");
                let value_ref = hex_field(vector, "value_ref_hex");
                contract
                    .strict_decode("mfm.content-ref.v1", &content_ref)
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                contract
                    .strict_decode("mfm.value-ref.v1", &value_ref)
                    .unwrap_or_else(|error| panic!("{id}: {error}"));
                assert!(
                    contract
                        .strict_decode("mfm.content-ref.v1", &value_ref)
                        .is_err(),
                    "{id}"
                );
                assert!(
                    contract
                        .strict_decode("mfm.value-ref.v1", &content_ref)
                        .is_err(),
                    "{id}"
                );
            }
            other => panic!("{id}: unhandled relational vector kind {other}"),
        },
    }
}

fn assert_read_verdict_metadata(vector: &Value, id: &str) {
    assert_eq!(
        string(vector, "expected_structural_replay_action"),
        "validate_schema_and_relations_only",
        "{id}"
    );
    assert!(
        matches!(
            string(vector, "expected_reproduction_verdict"),
            "committed_typed_terminal"
                | "insufficient_evidence"
                | "invalid_evidence"
                | "structural_reject_before_callback"
        ),
        "{id}"
    );
}

fn assert_typed_failure(contract: &RecoverabilityContract, vector: &Value, id: &str) {
    let schema_id = string(vector, "expected_typed_failure_schema_id");
    let parts = schema_id.split(':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 5, "typed failure schema id");
    assert_eq!(parts[0], "schema", "typed failure schema id");
    assert_eq!(parts[3], "sha256-jcs-v1", "typed failure schema id");
    let schema_contract = format!("{}.v{}", parts[1], parts[2]);
    let failure = contract
        .strict_decode(
            &schema_contract,
            &hex_field(vector, "expected_typed_failure_hex"),
        )
        .unwrap_or_else(|error| panic!("{id}: {error}"));
    assert_eq!(failure.schema_id().as_str(), schema_id, "{id}");
    let failure: Value = serde_json::from_slice(failure.as_bytes()).expect("typed failure JSON");
    assert_eq!(
        string(&failure, "kind"),
        string(vector, "expected_typed_failure_tag"),
        "{id}"
    );
}

fn canonical_json_field(vector: &Value, key: &str) -> Value {
    let bytes = hex_field(vector, key);
    let canonical =
        PlainCanonicalJsonBytes::from_canonical_json_slice(&bytes).expect("canonical JSON field");
    serde_json::from_slice(canonical.as_bytes()).expect("canonical JSON value")
}

fn canonical_json_value(value: &Value) -> Vec<u8> {
    PlainCanonicalJsonBytes::from_json_str(&value.to_string())
        .expect("canonical JSON value")
        .to_vec()
}

fn assert_artifact_bindings(contract: &RecoverabilityContract) {
    assert_eq!(ANNEX_BYTES.len(), ANNEX_BYTE_LENGTH);
    assert_eq!(CORPUS_BYTES.len(), CORPUS_BYTE_LENGTH);
    assert_eq!(
        sha256_digest_bytes(ANNEX_BYTES).to_string(),
        ANNEX_SHA256_HEX
    );
    assert_eq!(
        sha256_digest_bytes(CORPUS_BYTES).to_string(),
        CORPUS_SHA256_HEX
    );
    assert_eq!(contract.annex_bytes(), ANNEX_BYTES);
    RecoverabilityContract::validate_annex_candidate(ANNEX_BYTES).expect("frozen annex candidate");
    PlainCanonicalJsonBytes::from_canonical_json_slice(ANNEX_BYTES)
        .expect("canonical frozen annex");
    PlainCanonicalJsonBytes::from_canonical_json_slice(CORPUS_BYTES)
        .expect("canonical frozen corpus");

    let corpus = corpus();
    assert_eq!(
        string(&corpus, "annex_content_digest"),
        contract.raw_content_digest(ANNEX_BYTES).as_str()
    );
}

fn execute_positive(contract: &RecoverabilityContract, vector: &Value) {
    let id = string(vector, "id");
    match string(vector, "kind") {
        "schema_acceptance" => {
            let input = hex_field(vector, "input_hex");
            let canonical = hex_field(vector, "canonical_hex");
            let value = contract
                .strict_decode(string(vector, "schema_contract"), &input)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            assert_eq!(value.as_bytes(), canonical, "{id}");
            assert_eq!(
                value.schema_id().as_str(),
                string(vector, "expected_schema_id"),
                "{id}"
            );
            assert_eq!(
                contract
                    .content_ref(&value)
                    .unwrap_or_else(|error| panic!("{id}: {error}"))
                    .content_digest()
                    .as_str(),
                string(vector, "expected_content_digest"),
                "{id}"
            );
        }
        "domain_identity" => {
            let input = hex_field(vector, "value_hex");
            let value = contract
                .strict_decode(string(vector, "preimage_schema"), &input)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            let expected = object(vector, "expected");
            let derived = derive_frozen_domain_identity(contract, string(vector, "domain"), &value);
            assert_eq!(derived, string(expected, "value"), "{id}");
            let digest = contract
                .semantic_digest(string(vector, "domain"), &value)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            assert_eq!(
                digest.digest().to_string(),
                string(expected, "digest_hex"),
                "{id}"
            );
            assert!(derived.ends_with(digest.as_str()), "{id}");
            assert_eq!(
                semantic_envelope(string(vector, "domain"), &input),
                hex_field(vector, "envelope_hex"),
                "{id}"
            );
            let (preimage_schema, result_kind) = contract
                .domain_contract(string(vector, "domain"))
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            assert_eq!(preimage_schema, string(vector, "preimage_schema"), "{id}");
            assert_eq!(result_kind, string(expected, "kind"), "{id}");
        }
        "content_addressing" => {
            assert_eq!(
                contract
                    .raw_content_digest(&hex_field(vector, "input_hex"))
                    .as_str(),
                string(vector, "expected_content_digest"),
                "{id}"
            );
        }
        "legal_batch" => {
            let batch = contract
                .strict_decode("mfm.legal-commit-batch.v2", &hex_field(vector, "batch_hex"))
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            let coordinate = contract
                .strict_decode(
                    "mfm.tenant-fact-coordinate.v1",
                    &hex_field(vector, "coordinate_hex"),
                )
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            assert_eq!(
                batch.schema_id().as_str(),
                string(vector, "batch_schema_id"),
                "{id}"
            );
            assert_eq!(
                coordinate.schema_id().as_str(),
                string(vector, "coordinate_schema_id"),
                "{id}"
            );
        }
        "executor_failure_mapping" => {
            let input = hex_field(vector, "input_hex");
            let value = contract
                .strict_decode(string(vector, "schema_contract"), &input)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            assert_eq!(value.as_bytes(), hex_field(vector, "canonical_hex"), "{id}");
            assert_eq!(
                value.schema_id().as_str(),
                string(vector, "schema_id"),
                "{id}"
            );
            let decoded: Value = serde_json::from_slice(value.as_bytes()).expect("validated JSON");
            let safe_failure = object(&decoded, "safe_failure");
            assert_eq!(
                string(safe_failure, "stable_code"),
                string(vector, "expected_stable_code"),
                "{id}"
            );
            assert_eq!(
                string(safe_failure, "failure_class"),
                string(vector, "expected_failure_class"),
                "{id}"
            );
            assert_eq!(
                string(safe_failure, "boundary_stage"),
                string(vector, "expected_boundary_stage"),
                "{id}"
            );
            assert_eq!(
                string(&decoded, "kind"),
                string(vector, "expected_outcome"),
                "{id}"
            );
        }
        other => panic!("{id}: unhandled positive vector kind {other}"),
    }
}

fn derive_frozen_domain_identity(
    contract: &RecoverabilityContract,
    domain: &str,
    value: &mfm_canonical::ValidatedCanonicalValue,
) -> String {
    macro_rules! derive {
        ($method:ident) => {
            contract
                .$method(value)
                .unwrap_or_else(|error| panic!("{domain}: {error}"))
                .to_string()
        };
    }

    match domain {
        "mfm.admission-logical-key.v1" => derive!(derive_admission_logical_key),
        "mfm.artifact-id.v1" => derive!(derive_artifact_id),
        "mfm.correction-invocation.v1" => {
            derive!(derive_correction_invocation_digest)
        }
        "mfm.effect-key.v1" => derive!(derive_effect_key),
        "mfm.executor-delivery-attempt.v1" => derive!(derive_attempt_id),
        "mfm.executor-frontier.v2" => derive!(derive_executor_frontier_digest),
        "mfm.executor-record.v2" => derive!(derive_executor_record_digest),
        "mfm.fact-content-identity.v1" => {
            derive!(derive_fact_content_identity_digest)
        }
        "mfm.fact-logical-identity.v1" => {
            derive!(derive_fact_logical_identity_digest)
        }
        "mfm.fact-query.v1" => derive!(derive_fact_query_digest),
        "mfm.genesis.v1" => derive!(derive_genesis_digest),
        "mfm.journal-candidate.v2" => derive!(derive_journal_candidate_digest),
        "mfm.journal-commit.v1" => derive!(derive_journal_commit_digest),
        "mfm.journal-record-id.v1" => derive!(derive_record_id),
        "mfm.journal-record.v2" => derive!(derive_journal_record_hash),
        "mfm.node-occurrence.v1" => derive!(derive_node_id),
        "mfm.object-evidence.v1" => derive!(derive_object_evidence_digest),
        "mfm.output-logical-identity.v1" => {
            derive!(derive_output_logical_identity_digest)
        }
        "mfm.request.v1" => derive!(derive_request_digest),
        "mfm.run-id.v1" => derive!(derive_run_id),
        "mfm.run-semantic-state.v1" => {
            derive!(derive_run_semantic_state_digest)
        }
        "mfm.schema.v1" => derive!(derive_schema_id),
        "mfm.cross-run-source-redaction.v1" => {
            derive!(derive_cross_run_source_redaction_digest)
        }
        "mfm.source-closure.v1" => derive!(derive_source_closure_digest),
        "mfm.spec-hash.v1" => derive!(derive_spec_hash),
        "mfm.terminal-effect-evidence.v1" => {
            derive!(derive_terminal_effect_evidence_digest)
        }
        other => panic!("unhandled frozen semantic domain {other}"),
    }
}

fn execute_codec_rejection(
    contract: &RecoverabilityContract,
    vector: &Value,
    known_domain: &Value,
    known_preimage: &mfm_canonical::ValidatedCanonicalValue,
    known_content: &mfm_canonical::ValidatedCanonicalValue,
) {
    let id = string(vector, "id");
    let expected = error_code(string(vector, "expected_error"));
    let input = hex_field(vector, "input_hex");
    let result = match id {
        "codec/invalid-annex" => RecoverabilityContract::validate_annex_candidate(&input),
        "codec/schema-mismatch/cross-run-source-redaction" => contract
            .strict_decode(string(vector, "target"), &input)
            .and_then(|value| {
                contract
                    .derive_cross_run_source_redaction_digest(&value)
                    .map(drop)
            }),
        "codec/unknown-domain" => contract
            .semantic_digest("mfm.unknown.v1", known_preimage)
            .map(drop),
        "codec/schema-mismatch" => contract
            .semantic_digest(string(known_domain, "domain"), known_content)
            .map(drop),
        "codec/unknown-schema" => contract.strict_decode("mfm.unknown.v1", &input).map(drop),
        id if id.starts_with("codec/invalid-canonical-json/") => contract
            .strict_decode("mfm.primitive-canonical_value.v1", &input)
            .map(drop),
        _ => contract
            .strict_decode(string(vector, "target"), &input)
            .map(drop),
    };
    let error = match result {
        Ok(()) => panic!("{id}: rejection was accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code(), expected, "{id}: {error}");
    if let Ok(rejected_text) = std::str::from_utf8(&input) {
        if !rejected_text.is_empty() {
            assert!(
                !error.message().contains(rejected_text),
                "{id}: error echoed rejected input"
            );
        }
    }
}

fn error_code(value: &str) -> RecoverabilityErrorCode {
    match value {
        "duplicate_item" => RecoverabilityErrorCode::DuplicateItem,
        "identity_construction" => RecoverabilityErrorCode::IdentityConstruction,
        "invalid_annex" => RecoverabilityErrorCode::InvalidAnnex,
        "invalid_canonical_json" => RecoverabilityErrorCode::InvalidCanonicalJson,
        "invalid_order" => RecoverabilityErrorCode::InvalidOrder,
        "invalid_value" => RecoverabilityErrorCode::InvalidValue,
        "missing_field" => RecoverabilityErrorCode::MissingField,
        "out_of_bounds" => RecoverabilityErrorCode::OutOfBounds,
        "schema_mismatch" => RecoverabilityErrorCode::SchemaMismatch,
        "unknown_domain" => RecoverabilityErrorCode::UnknownDomain,
        "unknown_field" => RecoverabilityErrorCode::UnknownField,
        "unknown_schema" => RecoverabilityErrorCode::UnknownSchema,
        "wrong_type" => RecoverabilityErrorCode::WrongType,
        other => panic!("unknown codec error code {other}"),
    }
}

fn assert_consumer_coverage(vector: &Value, consumer: &str) {
    assert!(
        array(vector, "consumer_coverage")
            .iter()
            .any(|candidate| candidate.as_str() == Some(consumer)),
        "{} omits {consumer} coverage",
        string(vector, "id")
    );
}

fn semantic_envelope(domain: &str, value_bytes: &[u8]) -> Vec<u8> {
    let value: Value = serde_json::from_slice(value_bytes).expect("domain value");
    PlainCanonicalJsonBytes::from_json_str(
        &serde_json::json!({"domain": domain, "value": value}).to_string(),
    )
    .expect("canonical domain envelope")
    .to_vec()
}

fn corpus() -> Value {
    serde_json::from_slice(CORPUS_BYTES).expect("recoverability corpus")
}

/// Decodes one lowercase hexadecimal corpus field.
pub fn hex_field(value: &Value, key: &str) -> Vec<u8> {
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

/// Returns a required object member from a corpus vector.
pub fn object<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or_else(|| panic!("missing {key}"))
}

/// Returns a required array member from a corpus vector.
pub fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    object(value, key)
        .as_array()
        .unwrap_or_else(|| panic!("{key} is not an array"))
}

/// Returns a required string member from a corpus vector.
pub fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    object(value, key)
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string"))
}

fn usize_field(value: &Value, key: &str) -> usize {
    usize::try_from(
        object(value, key)
            .as_u64()
            .unwrap_or_else(|| panic!("{key} is not an integer")),
    )
    .expect("usize corpus bound")
}
