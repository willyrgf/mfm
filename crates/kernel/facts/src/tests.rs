use mfm_canonical::{CanonicalValue, PlainCanonicalJsonBytes};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, FactContentIdentityDigest,
    FactLogicalIdentityDigest, SchemaId, SemanticTypeId, StableId,
};

use crate::*;

const CORPUS: &[u8] = include_bytes!("../../../../contracts/recoverability/v1/corpus.json");

fn digest(seed: u64) -> DigestBytes {
    let mut bytes = [0_u8; 32];
    bytes[24..].copy_from_slice(&seed.to_be_bytes());
    DigestBytes::from_array(bytes)
}

fn schema(seed: u64) -> SchemaId {
    SchemaId::new(
        "mfm.test.fact_value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest(seed),
    )
    .expect("schema")
}

fn content_ref(seed: u64) -> ContentRef {
    ContentRef::new(
        schema(seed),
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, digest(seed)),
    )
    .expect("content ref")
}

fn proposed_value(seed: u64, role: &str, canonical: PlainCanonicalJsonBytes) -> ProposedFactValue {
    ProposedFactValue::new(
        schema(seed),
        SemanticTypeId::new(
            "mfm.test",
            role,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest(seed + 10_000),
        )
        .expect("semantic type"),
        StableId::new(role).expect("role"),
        "application/json",
        content_ref(seed + 20_000),
        canonical,
    )
    .expect("proposed fact value")
}

fn subject(value: &str) -> FactSubject {
    FactSubject::from_canonical_value(
        CanonicalValue::object([("sample", CanonicalValue::String(value.to_owned()))])
            .expect("subject object"),
    )
    .expect("subject")
}

fn predicate(value: &str) -> CanonicalFactPredicate {
    CanonicalFactPredicate::exact_subject(&subject(value)).expect("predicate")
}

fn query(
    descriptor_ref: ContentRef,
    predicate: CanonicalFactPredicate,
    ordering: FactOrdering,
    limit: u32,
    tie_break: FactTieBreak,
) -> FactSelectionQuery {
    FactSelectionQuery::new(
        descriptor_ref,
        predicate,
        None,
        ordering,
        FactSelectionLimit::new(limit).expect("limit"),
        tie_break,
    )
    .expect("query")
}

fn candidate(
    descriptor_ref: ContentRef,
    subject: FactSubject,
    publication_order: u64,
    identity_seed: u64,
) -> FactCandidate<u64> {
    FactCandidate::new(
        descriptor_ref,
        subject,
        FactContentIdentityDigest::from_digest(digest(identity_seed + 10_000)),
        FactLogicalIdentityDigest::from_digest(digest(identity_seed)),
        publication_order,
        identity_seed,
    )
}

fn corpus_vector(id: &str) -> serde_json::Value {
    let corpus: serde_json::Value = serde_json::from_slice(CORPUS).expect("corpus JSON");
    ["positive_vectors", "negative_vectors", "relational_vectors"]
        .into_iter()
        .flat_map(|group| {
            corpus[group]
                .as_array()
                .expect("corpus vector array")
                .iter()
        })
        .find(|vector| vector["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("missing corpus vector {id}"))
        .clone()
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|digits| {
            let text = std::str::from_utf8(digits).expect("hex utf8");
            u8::from_str_radix(text, 16).expect("hex byte")
        })
        .collect()
}

#[test]
fn descriptor_is_an_exact_journal_independent_projection() {
    let kind = FactKind::new("wallet.balance").expect("fact kind");
    let descriptor_ref = content_ref(1);
    let subject_schema = schema(2);
    let response_schema = schema(3);
    let descriptor = FactDescriptor::new(
        kind.clone(),
        descriptor_ref.clone(),
        subject_schema.clone(),
        response_schema.clone(),
    );

    assert_eq!(descriptor.kind(), &kind);
    assert_eq!(descriptor.descriptor_ref(), &descriptor_ref);
    assert_eq!(descriptor.subject_schema_id(), &subject_schema);
    assert_eq!(descriptor.response_schema_id(), &response_schema);
    assert!(FactKind::new("Wallet.balance").is_err());
    assert!(FactKind::new("wallet..balance").is_err());
}

#[test]
fn scalar_subject_and_predicate_are_exact_float_free_annex_values() {
    let scalar = FactScalar::unsigned(42).expect("scalar");
    assert_eq!(scalar.as_bytes(), b"42");
    let scalar_subject = FactSubject::from_scalar(&scalar).expect("subject");
    assert!(CanonicalFactPredicate::exact_subject(&scalar_subject)
        .expect("predicate")
        .matches(&scalar_subject));

    let canonical = CanonicalFactPredicate::from_canonical_value(
        CanonicalValue::object([
            ("z", CanonicalValue::Unsigned(2)),
            ("a", CanonicalValue::Unsigned(1)),
        ])
        .expect("predicate object"),
    )
    .expect("canonical predicate");
    assert_eq!(canonical.canonical_json(), br#"{"a":1,"z":2}"#);
    assert!(CanonicalFactPredicate::from_canonical_json(br#"{"z":2,"a":1}"#).is_err());
    assert!(CanonicalFactPredicate::from_canonical_json(b"1.5").is_err());
    assert!(!predicate("one").matches(&subject("two")));
}

#[test]
fn frozen_fact_query_vector_round_trips_and_derives_its_domain_digest() {
    let domain_vector = corpus_vector("domain/mfm.fact-query.v1/minimum");
    let request_bytes = decode_hex(
        domain_vector["value_hex"]
            .as_str()
            .expect("request vector hex"),
    );
    let decoded =
        FactSelectionRequest::from_canonical_json(&request_bytes).expect("frozen request");
    assert_eq!(decoded.canonical_json(), request_bytes);
    assert_eq!(
        decoded.request_digest().expect("request digest").as_str(),
        domain_vector["expected"]["value"]
            .as_str()
            .expect("expected request digest")
    );

    let schema_vector = corpus_vector("schema/mfm.fact-selection-request.v1/minimum");
    assert_eq!(
        decoded.schema_id().as_str(),
        schema_vector["expected_schema_id"]
            .as_str()
            .expect("expected request schema")
    );
    assert_eq!(
        decoded
            .content_ref()
            .expect("request content ref")
            .content_digest()
            .as_str(),
        schema_vector["expected_content_digest"]
            .as_str()
            .expect("expected request content digest")
    );

    let query_vector = corpus_vector("schema/mfm.fact-selection-query.v1/minimum");
    let query_bytes = decode_hex(
        query_vector["input_hex"]
            .as_str()
            .expect("query vector hex"),
    );
    let decoded_query =
        FactSelectionQuery::from_canonical_json(&query_bytes).expect("frozen query");
    assert_eq!(decoded_query.canonical_json(), query_bytes);
    assert_eq!(
        decoded_query.schema_id().as_str(),
        query_vector["expected_schema_id"]
            .as_str()
            .expect("expected query schema")
    );
    assert_eq!(
        decoded_query
            .content_ref()
            .expect("query content ref")
            .content_digest()
            .as_str(),
        query_vector["expected_content_digest"]
            .as_str()
            .expect("expected query content digest")
    );

    let constructed = FactSelectionRequest::new(vec![query(
        decoded.queries()[0].descriptor_ref().clone(),
        predicate("value"),
        FactOrdering::Ascending,
        1,
        FactTieBreak::FactIdentityAscending,
    )])
    .expect("constructed request");
    assert_eq!(constructed.canonical_json(), request_bytes);
}

#[test]
fn selection_codecs_reject_unknown_fields_floats_and_noncanonical_order() {
    let query = query(
        content_ref(1),
        predicate("value"),
        FactOrdering::Ascending,
        1,
        FactTieBreak::FactIdentityAscending,
    );
    let query_json = std::str::from_utf8(query.canonical_json())
        .expect("query utf8")
        .to_owned();

    let query_with_unknown_field = query_json.replacen(
        ",\"fact_descriptor_ref\"",
        ",\"extra\":false,\"fact_descriptor_ref\"",
        1,
    );
    assert_ne!(query_with_unknown_field, query_json);
    assert!(FactSelectionQuery::from_canonical_json(query_with_unknown_field.as_bytes()).is_err());

    let query_with_float = query_json.replacen(
        "\"canonical_predicate\":{\"sample\":\"value\"}",
        "\"canonical_predicate\":1.5",
        1,
    );
    assert_ne!(query_with_float, query_json);
    assert!(FactSelectionQuery::from_canonical_json(query_with_float.as_bytes()).is_err());

    let request = FactSelectionRequest::new(vec![query]).expect("request");
    let request_json = std::str::from_utf8(request.canonical_json()).expect("request utf8");
    let request_with_unknown_field = request_json.replacen('{', "{\"extra\":false,", 1);
    assert!(
        FactSelectionRequest::from_canonical_json(request_with_unknown_field.as_bytes()).is_err()
    );

    let request_with_noncanonical_order = format!(
        "{{\"queries\":[{query_json}],\"producer_scope\":\"other_runs_in_tenant_scope\",\"version\":\"mfm.fact-selection-request.v1\"}}"
    );
    assert!(
        FactSelectionRequest::from_canonical_json(request_with_noncanonical_order.as_bytes())
            .is_err()
    );
}

#[test]
fn request_and_query_empty_and_maximum_bounds_are_closed() {
    assert!(FactSelectionLimit::new(0).is_err());
    assert!(FactSelectionLimit::new(129).is_err());
    assert_eq!(FactSelectionLimit::new(128).expect("max limit").get(), 128);
    assert!(FactSelectionRequest::new(Vec::new()).is_err());

    let query = query(
        content_ref(1),
        predicate("value"),
        FactOrdering::Ascending,
        128,
        FactTieBreak::FactIdentityAscending,
    );
    assert_eq!(
        FactSelectionQuery::from_canonical_json(query.canonical_json())
            .expect("maximum query")
            .limit()
            .get(),
        128
    );
    let below_minimum = corpus_vector("codec/out-of-bounds/fact-query-limit");
    assert!(FactSelectionQuery::from_canonical_json(&decode_hex(
        below_minimum["input_hex"]
            .as_str()
            .expect("below-minimum query")
    ))
    .is_err());

    let maximum = FactSelectionRequest::new(vec![query.clone(); 128]).expect("maximum request");
    assert_eq!(maximum.queries().len(), 128);
    assert_eq!(
        FactSelectionRequest::from_canonical_json(maximum.canonical_json())
            .expect("maximum request round trip")
            .queries()
            .len(),
        128
    );
    assert!(FactSelectionRequest::new(vec![query; 129]).is_err());

    assert!(FactSelectionRequest::from_canonical_json(
        br#"{"producer_scope":"other_runs_in_tenant_scope","queries":[],"version":"mfm.fact-selection-request.v1"}"#
    )
    .is_err());
}

#[test]
fn content_identity_filter_and_exact_predicate_exclude_nonmatches() {
    let descriptor_ref = content_ref(1);
    let expected_content = FactContentIdentityDigest::from_digest(digest(90));
    let query = FactSelectionQuery::new(
        descriptor_ref.clone(),
        predicate("wanted"),
        Some(expected_content.clone()),
        FactOrdering::Ascending,
        FactSelectionLimit::new(4).expect("limit"),
        FactTieBreak::FactIdentityAscending,
    )
    .expect("query");

    let matching = FactCandidate::new(
        descriptor_ref.clone(),
        subject("wanted"),
        expected_content.clone(),
        FactLogicalIdentityDigest::from_digest(digest(1)),
        1,
        (),
    );
    let wrong_descriptor = FactCandidate::new(
        content_ref(2),
        subject("wanted"),
        expected_content.clone(),
        FactLogicalIdentityDigest::from_digest(digest(2)),
        1,
        (),
    );
    let wrong_subject = FactCandidate::new(
        descriptor_ref.clone(),
        subject("other"),
        expected_content.clone(),
        FactLogicalIdentityDigest::from_digest(digest(3)),
        1,
        (),
    );
    let wrong_content = FactCandidate::new(
        descriptor_ref,
        subject("wanted"),
        FactContentIdentityDigest::from_digest(digest(91)),
        FactLogicalIdentityDigest::from_digest(digest(4)),
        1,
        (),
    );

    assert!(query.matches(&matching));
    assert!(!query.matches(&wrong_descriptor));
    assert!(!query.matches(&wrong_subject));
    assert!(!query.matches(&wrong_content));
    assert_eq!(
        FactSelectionQuery::from_canonical_json(query.canonical_json())
            .expect("round trip")
            .content_identity_filter(),
        Some(&expected_content)
    );
}

#[test]
fn top_k_applies_publication_order_tie_break_and_limit_exactly() {
    let descriptor_ref = content_ref(1);
    let ascending_query = query(
        descriptor_ref.clone(),
        predicate("wanted"),
        FactOrdering::Ascending,
        128,
        FactTieBreak::FactIdentityAscending,
    );
    let mut ascending = FactTopK::new(ascending_query);
    for order in (1..=256).rev() {
        assert!(ascending.consider(candidate(
            descriptor_ref.clone(),
            subject("wanted"),
            order,
            order,
        )));
        assert!(ascending.len() <= 128);
    }
    assert_eq!(
        ascending
            .finish()
            .into_iter()
            .map(|value| value.publication_order())
            .collect::<Vec<_>>(),
        (1..=128).collect::<Vec<_>>()
    );

    let descending_query = query(
        descriptor_ref.clone(),
        predicate("wanted"),
        FactOrdering::Descending,
        128,
        FactTieBreak::FactIdentityDescending,
    );
    let mut descending = FactTopK::new(descending_query);
    for order in 1..=256 {
        descending.consider(candidate(
            descriptor_ref.clone(),
            subject("wanted"),
            order,
            order,
        ));
    }
    assert_eq!(
        descending
            .finish()
            .into_iter()
            .map(|value| value.publication_order())
            .collect::<Vec<_>>(),
        (129..=256).rev().collect::<Vec<_>>()
    );

    let tie_query = query(
        descriptor_ref.clone(),
        predicate("wanted"),
        FactOrdering::Ascending,
        2,
        FactTieBreak::FactIdentityDescending,
    );
    let mut ties = FactTopK::new(tie_query);
    ties.consider(candidate(descriptor_ref.clone(), subject("wanted"), 7, 1));
    ties.consider(candidate(descriptor_ref, subject("wanted"), 7, 2));
    assert_eq!(
        ties.finish()
            .into_iter()
            .map(FactCandidate::into_value)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );

    let ascending_tie_query = query(
        content_ref(1),
        predicate("wanted"),
        FactOrdering::Ascending,
        2,
        FactTieBreak::FactIdentityAscending,
    );
    let mut ascending_ties = FactTopK::new(ascending_tie_query);
    ascending_ties.consider(candidate(content_ref(1), subject("wanted"), 7, 2));
    ascending_ties.consider(candidate(content_ref(1), subject("wanted"), 7, 1));
    assert_eq!(
        ascending_ties
            .finish()
            .into_iter()
            .map(FactCandidate::into_value)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn fact_set_preserves_callback_order_and_rejects_exact_duplicates() {
    let first = FactProposal::new(
        0,
        content_ref(1),
        proposed_value(
            10,
            "fact-subject",
            PlainCanonicalJsonBytes::from_json_str(r#"{"key":"first"}"#).expect("first subject"),
        ),
        proposed_value(
            11,
            "fact-response",
            PlainCanonicalJsonBytes::from_json_str(r#"{"value":"first"}"#).expect("first response"),
        ),
    )
    .expect("first proposal");
    let second = FactProposal::new(
        0,
        content_ref(2),
        proposed_value(
            12,
            "fact-subject",
            PlainCanonicalJsonBytes::from_json_str(r#"{"key":"second"}"#).expect("second subject"),
        ),
        proposed_value(
            13,
            "fact-response",
            PlainCanonicalJsonBytes::from_json_str(r#"{"value":"second"}"#)
                .expect("second response"),
        ),
    )
    .expect("second proposal");

    let set = FactSet::try_from_iter([second.clone(), first.clone()]).expect("ordered set");
    assert_eq!(set.as_slice(), &[second, first.clone()]);
    let cross_slot_duplicate = FactProposal::new(
        1,
        first.descriptor_ref().clone(),
        first.subject().clone(),
        first.response().clone(),
    )
    .expect("cross-slot duplicate");
    assert!(
        FactSet::try_from_iter([first.clone(), cross_slot_duplicate]).is_err(),
        "slot ordinals do not distinguish otherwise exact duplicate proposals"
    );
    assert!(FactSet::try_from_iter([first.clone(), first]).is_err());
    assert!(FactSet::empty().as_slice().is_empty());
}

#[test]
fn fact_set_requires_nondecreasing_fact_slot_ordinals() {
    let proposal = |fact_slot_ordinal, suffix| {
        FactProposal::new(
            fact_slot_ordinal,
            content_ref(1),
            proposed_value(
                10,
                "fact-subject",
                PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"key":"{suffix}"}}"#))
                    .expect("canonical subject"),
            ),
            proposed_value(
                11,
                "fact-response",
                PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":"{suffix}"}}"#))
                    .expect("canonical response"),
            ),
        )
        .expect("fact proposal")
    };
    let first = proposal(0, "first");
    let second = proposal(1, "second");
    let set = FactSet::try_from_iter([first.clone(), second.clone()]).expect("grouped set");
    assert_eq!(set.as_slice(), &[first, second]);
    assert!(FactSet::try_from_iter([proposal(1, "later"), proposal(0, "earlier")]).is_err());
}

#[test]
fn fact_set_enforces_the_transition_emission_bound() {
    let descriptor = content_ref(1);
    let proposals = (0..MAX_FACT_EMISSIONS)
        .map(|ordinal| {
            FactProposal::new(
                0,
                descriptor.clone(),
                proposed_value(
                    10,
                    "fact-subject",
                    PlainCanonicalJsonBytes::from_json_str(&ordinal.to_string())
                        .expect("canonical subject"),
                ),
                proposed_value(
                    11,
                    "fact-response",
                    PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"ordinal":{ordinal}}}"#))
                        .expect("canonical response"),
                ),
            )
            .expect("proposal")
        })
        .collect::<Vec<_>>();
    let maximum = FactSet::try_from_iter(proposals).expect("maximum fact set");
    assert_eq!(maximum.as_slice().len(), MAX_FACT_EMISSIONS);

    let overflow = FactProposal::new(
        0,
        descriptor,
        proposed_value(
            10,
            "fact-subject",
            PlainCanonicalJsonBytes::from_json_str(&MAX_FACT_EMISSIONS.to_string())
                .expect("overflow subject"),
        ),
        proposed_value(
            11,
            "fact-response",
            PlainCanonicalJsonBytes::from_json_str(&format!(
                r#"{{"ordinal":{MAX_FACT_EMISSIONS}}}"#
            ))
            .expect("overflow response"),
        ),
    )
    .expect("overflow proposal");
    assert!(FactSet::try_from_iter(maximum.into_vec().into_iter().chain([overflow])).is_err());
}

#[test]
fn fact_proposal_derives_the_frozen_exact_value_content_identity() {
    let vector = corpus_vector("relation/semantic-vs-content");
    let bytes = decode_hex(vector["value_hex"].as_str().expect("value vector hex"));
    let proposal = FactProposal::new(
        7,
        content_ref(1),
        proposed_value(
            10,
            "fact-subject",
            PlainCanonicalJsonBytes::from_json_str(r#"{"key":"vector"}"#)
                .expect("canonical subject"),
        ),
        proposed_value(
            11,
            "fact-response",
            PlainCanonicalJsonBytes::from_canonical_json_slice(&bytes).expect("canonical response"),
        ),
    )
    .expect("fact proposal");

    assert_eq!(proposal.fact_slot_ordinal(), 7);
    assert_eq!(
        proposal.response().content_ref().content_digest().as_str(),
        vector["content_digest"]
            .as_str()
            .expect("expected content digest")
    );
    assert_eq!(
        proposal.response().evidence_contract_ref(),
        &content_ref(20_011)
    );
}

#[test]
fn request_digest_binds_authored_query_order() {
    let first = query(
        content_ref(1),
        predicate("first"),
        FactOrdering::Ascending,
        1,
        FactTieBreak::FactIdentityAscending,
    );
    let second = query(
        content_ref(2),
        predicate("second"),
        FactOrdering::Descending,
        2,
        FactTieBreak::FactIdentityDescending,
    );
    let left =
        FactSelectionRequest::new(vec![first.clone(), second.clone()]).expect("left request");
    let right = FactSelectionRequest::new(vec![second, first]).expect("right request");

    assert_ne!(left.canonical_json(), right.canonical_json());
    assert_ne!(
        left.request_digest().expect("left digest"),
        right.request_digest().expect("right digest")
    );
    assert_eq!(
        left.producer_scope(),
        FactProducerScope::OtherRunsInTenantScope
    );
}
