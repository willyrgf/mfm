use mfm_canonical::{CanonicalValue, PlainCanonicalJsonBytes};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, FactContentIdentityDigest,
    FactLogicalIdentityDigest, SchemaId, SemanticTypeId, StableId,
};
use mfm_values::{CanonicalJsonPersistedSchema, MediaType, MfmValue};
use serde::Serialize;

use crate::*;

const REQUEST_DIGEST_GOLDEN: &str =
    "sha256-jcs-v1:f9235915865b53d69812d8f74c904ac44926b1dc9a72fe78a162a0da55c78fa0";
const REQUEST_SCHEMA_GOLDEN: &str = "schema:mfm.fact.selection_request:1:sha256-jcs-v1:723d756c46826ca54470e118d01525926777e2e4b85d803817791105b0bf130e";

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

fn scan_bounds() -> FactSelectionScanBounds {
    FactSelectionScanBounds::new(
        MAX_FACT_SCAN_PUBLICATIONS,
        MAX_FACT_SCAN_FACTS,
        MAX_FACT_SCAN_RETAINED_SOURCE_BYTES,
        MAX_FACT_SCAN_SELECTED_RESULTS,
        MAX_FACT_SCAN_RESPONSE_BYTES,
    )
    .expect("scan bounds")
}

fn request(
    source_manifest_ref: ContentRef,
    queries: Vec<FactSelectionQuery>,
) -> FactSelectionRequest {
    FactSelectionRequest::new(source_manifest_ref, scan_bounds(), queries).expect("request")
}

fn canonical_json<T: Serialize>(value: &T) -> PlainCanonicalJsonBytes {
    PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(value).expect("owner serialization"),
    )
    .expect("owner canonical JSON")
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
        MediaType::new("application/json").expect("media type"),
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
    CanonicalFactPredicate::exact_subject(&subject(value))
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
fn scalar_subject_and_predicate_are_exact_float_free_canonical_values() {
    let scalar = FactScalar::unsigned(42).expect("scalar");
    assert_eq!(
        scalar.canonical_json().expect("scalar bytes").as_bytes(),
        b"42"
    );
    assert!(FactScalar::from_canonical_value(CanonicalValue::Signed(42)).is_err());
    assert!(FactScalar::from_canonical_value(CanonicalValue::Signed(-1)).is_err());
    let scalar_subject = FactSubject::from_scalar(&scalar);
    assert!(CanonicalFactPredicate::exact_subject(&scalar_subject).matches(&scalar_subject));

    let canonical = CanonicalFactPredicate::from_canonical_value(
        CanonicalValue::object([
            ("z", CanonicalValue::Unsigned(2)),
            ("a", CanonicalValue::Unsigned(1)),
        ])
        .expect("predicate object"),
    )
    .expect("canonical predicate");
    assert_eq!(
        canonical
            .canonical_json()
            .expect("predicate bytes")
            .as_bytes(),
        br#"{"a":1,"z":2}"#
    );
    assert!(CanonicalFactPredicate::from_canonical_json(br#"{"z":2,"a":1}"#).is_err());
    assert!(CanonicalFactPredicate::from_canonical_json(b"1.5").is_err());
    assert!(!predicate("one").matches(&subject("two")));
}

/// Production-valid goldens for the current fact request/query owners.
///
/// These are constructed from current typed owners, not decoded from frozen
/// bytes: the schema reset changes the request's nested selector reference, so
/// only the owner-built values are meaningful acceptance evidence.
#[test]
fn current_fact_request_and_query_owners_have_production_valid_goldens() {
    let descriptor = content_ref(1);
    let constructed = request(
        content_ref(2),
        vec![query(
            descriptor.clone(),
            predicate("value"),
            FactOrdering::Ascending,
            1,
            FactTieBreak::FactIdentityAscending,
        )],
    );

    let request_json = canonical_json(&constructed);
    FactSelectionRequest::schema_descriptor()
        .expect("request descriptor")
        .identity
        .validate_canonical_value(request_json.as_bytes())
        .expect("request matches its complete direct schema");
    let round_tripped: FactSelectionRequest =
        serde_json::from_slice(request_json.as_bytes()).expect("current request round trip");
    assert_eq!(canonical_json(&round_tripped), request_json);
    assert_eq!(
        round_tripped.request_digest().expect("request digest"),
        constructed.request_digest().expect("request digest"),
    );
    assert_eq!(
        round_tripped.selector_contract_ref(),
        &prior_run_fact_selector_contract_ref().expect("selector contract"),
    );

    let queries = constructed.queries();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].descriptor_ref(), &descriptor);
    let query_json = queries[0].encode_canonical().expect("query bytes");
    let query_round_trip =
        FactSelectionQuery::decode_canonical(query_json.as_bytes()).expect("query round trip");
    assert_eq!(query_round_trip, queries[0]);

    assert_eq!(
        constructed
            .request_digest()
            .expect("request digest")
            .as_str(),
        REQUEST_DIGEST_GOLDEN,
    );
    assert_eq!(
        FactSelectionRequest::schema_id()
            .expect("request schema")
            .as_str(),
        REQUEST_SCHEMA_GOLDEN,
    );
}

#[test]
fn direct_selection_owners_reject_unknown_fields_floats_and_noncanonical_order() {
    let query = query(
        content_ref(1),
        predicate("value"),
        FactOrdering::Ascending,
        1,
        FactTieBreak::FactIdentityAscending,
    );
    let query_bytes = query.encode_canonical().expect("query bytes");
    let query_json = query_bytes.as_str().to_owned();

    let query_with_unknown_field = query_json.replacen(
        ",\"fact_descriptor_ref\"",
        ",\"extra\":false,\"fact_descriptor_ref\"",
        1,
    );
    assert_ne!(query_with_unknown_field, query_json);
    assert!(FactSelectionQuery::decode_canonical(query_with_unknown_field.as_bytes()).is_err());

    let query_with_float = query_json.replacen(
        "\"canonical_predicate\":{\"sample\":\"value\"}",
        "\"canonical_predicate\":1.5",
        1,
    );
    assert_ne!(query_with_float, query_json);
    assert!(FactSelectionQuery::decode_canonical(query_with_float.as_bytes()).is_err());

    let request = request(content_ref(2), vec![query]);
    let request_bytes = canonical_json(&request);
    let request_json = request_bytes.as_str();
    let request_with_unknown_field = request_json.replacen('{', "{\"extra\":false,", 1);
    assert!(serde_json::from_str::<FactSelectionRequest>(&request_with_unknown_field).is_err());

    let suffix = ",\"version\":\"mfm.fact-selection-request.v1\"}";
    let body = request_json
        .strip_suffix(suffix)
        .expect("canonical request suffix");
    let request_with_noncanonical_order = format!(
        "{{\"version\":\"mfm.fact-selection-request.v1\",{}{}",
        &body[1..],
        "}"
    );
    assert!(PlainCanonicalJsonBytes::from_canonical_json_slice(
        request_with_noncanonical_order.as_bytes()
    )
    .is_err());
    assert!(
        serde_json::from_str::<FactSelectionRequest>(r#"{"superseded_wrapper":"e30"}"#).is_err()
    );
}

#[test]
fn request_and_query_empty_and_maximum_bounds_are_closed() {
    assert!(FactSelectionLimit::new(0).is_err());
    assert!(FactSelectionLimit::new(129).is_err());
    assert_eq!(FactSelectionLimit::new(128).expect("max limit").get(), 128);
    assert!(FactSelectionRequest::new(content_ref(2), scan_bounds(), Vec::new()).is_err());

    let query = query(
        content_ref(1),
        predicate("value"),
        FactOrdering::Ascending,
        128,
        FactTieBreak::FactIdentityAscending,
    );
    assert_eq!(
        FactSelectionQuery::decode_canonical(
            query.encode_canonical().expect("query bytes").as_bytes()
        )
        .expect("maximum query")
        .limit()
        .get(),
        128
    );
    let below_minimum = query
        .encode_canonical()
        .expect("query bytes")
        .as_str()
        .to_owned()
        .replace("\"limit\":128", "\"limit\":0");
    assert!(FactSelectionQuery::decode_canonical(below_minimum.as_bytes()).is_err());

    let maximum = request(content_ref(2), vec![query.clone(); 128]);
    assert_eq!(maximum.queries().len(), 128);
    let maximum_json = canonical_json(&maximum);
    assert_eq!(
        serde_json::from_slice::<FactSelectionRequest>(maximum_json.as_bytes())
            .expect("maximum request round trip")
            .queries()
            .len(),
        128,
    );
    assert!(FactSelectionRequest::new(content_ref(2), scan_bounds(), vec![query; 129]).is_err());

    assert!(serde_json::from_slice::<FactSelectionRequest>(
        br#"{"producer_scope":"other_runs_in_tenant_scope","queries":[],"version":"mfm.fact-selection-request.v1"}"#
    )
    .is_err());
}

#[test]
fn scan_bounds_and_read_values_reject_every_out_of_contract_shape() {
    let maximum = scan_bounds();
    assert_eq!(
        serde_json::from_value::<FactSelectionScanBounds>(
            serde_json::to_value(&maximum).expect("scan bounds JSON")
        )
        .expect("scan bounds round trip"),
        maximum
    );
    for values in [
        (0, 1, 1, 1, 1),
        (1, 0, 1, 1, 1),
        (1, 1, 0, 1, 1),
        (1, 1, 1, 0, 1),
        (1, 1, 1, 1, 0),
        (MAX_FACT_SCAN_PUBLICATIONS + 1, 1, 1, 1, 1),
        (1, MAX_FACT_SCAN_FACTS + 1, 1, 1, 1),
        (1, 1, MAX_FACT_SCAN_RETAINED_SOURCE_BYTES + 1, 1, 1),
        (1, 1, 1, MAX_FACT_SCAN_SELECTED_RESULTS + 1, 1),
        (1, 1, 1, 1, MAX_FACT_SCAN_RESPONSE_BYTES + 1),
    ] {
        assert!(
            FactSelectionScanBounds::new(values.0, values.1, values.2, values.3, values.4).is_err()
        );
    }
    let exact_work = FactSelectionScanBounds::new_with_work_bounds(
        MAX_FACT_SCAN_PUBLICATIONS,
        MAX_FACT_SCAN_FACTS,
        MAX_FACT_SCAN_RETAINED_SOURCE_BYTES,
        MAX_FACT_SCAN_SELECTED_RESULTS,
        MAX_FACT_SCAN_RESPONSE_BYTES,
        MAX_FACT_SCAN_DISTINCT_PRODUCERS,
        MAX_FACT_SCAN_PRODUCER_FOLD_BATCHES,
        MAX_FACT_SCAN_PAGES,
    )
    .expect("exact producer/page budgets");
    assert_eq!(
        exact_work.maximum_distinct_producers(),
        MAX_FACT_SCAN_DISTINCT_PRODUCERS
    );
    for (producers, folds, pages) in [
        (
            MAX_FACT_SCAN_DISTINCT_PRODUCERS + 1,
            MAX_FACT_SCAN_PRODUCER_FOLD_BATCHES,
            MAX_FACT_SCAN_PAGES,
        ),
        (
            MAX_FACT_SCAN_DISTINCT_PRODUCERS,
            MAX_FACT_SCAN_PRODUCER_FOLD_BATCHES + 1,
            MAX_FACT_SCAN_PAGES,
        ),
        (
            MAX_FACT_SCAN_DISTINCT_PRODUCERS,
            MAX_FACT_SCAN_PRODUCER_FOLD_BATCHES,
            MAX_FACT_SCAN_PAGES + 1,
        ),
    ] {
        assert!(FactSelectionScanBounds::new_with_work_bounds(
            MAX_FACT_SCAN_PUBLICATIONS,
            MAX_FACT_SCAN_FACTS,
            MAX_FACT_SCAN_RETAINED_SOURCE_BYTES,
            MAX_FACT_SCAN_SELECTED_RESULTS,
            MAX_FACT_SCAN_RESPONSE_BYTES,
            producers,
            folds,
            pages,
        )
        .is_err());
    }
    assert!(serde_json::from_str::<FactSelectionScanBounds>(
        r#"{"maximum_publications":1,"maximum_facts":1,"maximum_retained_source_bytes":1,"maximum_selected_results":1,"maximum_response_bytes":1,"extra":false}"#
    )
    .is_err());
    assert!(FactSelectionReadResponse::from_canonical_json("").is_err());
    assert!(FactSelectionReadResponse::from_canonical_json(r#"{"value":1.5}"#).is_err());
    assert!(FactSelectionReadResponse::from_canonical_json(r#"{"z":0,"a":1}"#).is_err());
    assert!(serde_json::from_str::<FactSelectionReadFailure>(r#"{"code":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<FactSelectionReadFailure>(
        r#"{"code":"store_unavailable","detail":"secret"}"#
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
    let query_bytes = query.encode_canonical().expect("query bytes");
    assert_eq!(
        FactSelectionQuery::decode_canonical(query_bytes.as_bytes())
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
fn fact_proposal_derives_the_exact_raw_byte_value_content_identity() {
    // The response identity is the raw-byte digest of the exact canonical
    // value, independent of any schema identity carried beside it.
    let bytes = br#"{"sample":"value"}"#;
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
            PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).expect("canonical response"),
        ),
    )
    .expect("fact proposal");

    assert_eq!(proposal.fact_slot_ordinal(), 7);
    assert_eq!(
        proposal.response().content_ref().content_digest().as_str(),
        "content:sha256-v1:c60c60f74cab47015f6a1502867fc80e6147638ed2f7cf7de95ca9d0efcb46f7"
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
    let left = request(content_ref(3), vec![first.clone(), second.clone()]);
    let right = request(content_ref(3), vec![second, first]);

    assert_ne!(canonical_json(&left), canonical_json(&right));
    assert_ne!(
        left.request_digest().expect("left digest"),
        right.request_digest().expect("right digest")
    );
    assert_eq!(
        left.producer_scope(),
        FactProducerScope::OtherRunsInTenantScope
    );
}

#[test]
fn fact_read_response_retains_its_opaque_canonical_boundary() {
    let response = FactSelectionReadResponse::from_canonical_json(
        r#"{"completeness_mode":"complete_through_authorization_frontier","query_results":[]}"#,
    )
    .expect("response");
    let response_wire = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&response).expect("response wire JSON"),
    )
    .expect("canonical response wire");
    assert!(response_wire
        .as_str()
        .starts_with(r#"{"canonical_response_base64url":"#));
    FactSelectionReadResponse::schema_descriptor()
        .expect("response descriptor")
        .identity
        .validate_canonical_value(response_wire.as_bytes())
        .expect("response wire matches its complete schema");
    assert_eq!(
        serde_json::from_slice::<FactSelectionReadResponse>(response_wire.as_bytes())
            .expect("response wire decode"),
        response
    );
    assert!(serde_json::from_str::<FactSelectionReadResponse>(
        r#"{"canonical_response_json":"{}"}"#
    )
    .is_err());
}
