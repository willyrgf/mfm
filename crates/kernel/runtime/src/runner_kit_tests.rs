use std::collections::BTreeMap;

use super::*;
use mfm_canonical::CanonicalValue;
use mfm_store::v1::test_support::{
    fact_descriptor_projection_fixture_for_test, fact_projection_fixture_for_test,
    fact_query_receipt_for_test, FactProjectionFixtureInputForTest,
    FactQueryReceiptFixtureInputForTest,
};

fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([byte; 32]),
    )
}

fn artifact_id(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([byte; 32]),
    )
}

fn run_id(byte: u8) -> mfm_ids::RunId {
    mfm_ids::RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([byte; 32]),
    )
}

fn event_id(byte: u8) -> mfm_ids::EventId {
    mfm_ids::EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([byte; 32]),
    )
}

fn schema_id() -> SchemaId {
    SchemaId::new(
        "mfm.test.response",
        "mfm.test.v1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([1; 32]),
    )
    .expect("schema")
}

fn descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("chain.head").expect("kind"),
        schema_id(),
        schema_id(),
        schema_id(),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("subject.height").expect("field"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Subject(
                    mfm_facts::CanonicalValuePath::new("height").expect("extraction"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("result.height").expect("field"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Response(
                    mfm_facts::CanonicalValuePath::new("height").expect("extraction"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .sortable()
                .required(),
            )
            .expect("result field"),
        ],
        vec![mfm_facts::FactOrderingPolicy::new(
            mfm_facts::FactOrderingName::new("result.height.asc").expect("ordering"),
            vec![mfm_facts::FactOrderingTerm::new(
                mfm_facts::FactFieldId::new("result.height").expect("field"),
                mfm_facts::SortDirection::Ascending,
                mfm_facts::NullOrdering::Last,
                true,
            )],
        )
        .expect("ordering")],
    )
    .expect("descriptor")
}

struct FactAuthorityFixture {
    fact_ref: mfm_facts::InternalFactRef,
    descriptor: store::FactDescriptorProjection,
    query: store::FactQueryProjection,
}

fn fact_authority_fixture(subject_height: u64) -> FactAuthorityFixture {
    let descriptor = descriptor();
    let descriptor_fixture = fact_descriptor_projection_fixture_for_test(descriptor.clone())
        .expect("descriptor fixture");
    let fact_fixture = fact_projection_fixture_for_test(
        &descriptor,
        descriptor_fixture.descriptor_hash,
        FactProjectionFixtureInputForTest {
            run_id: run_id(0x10),
            source_seq: 1,
            source_ordinal: 0,
            source_event_id: event_id(0x11),
            node_id: mfm_ids::NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                mfm_ids::DigestBytes::from_array([0x19; 32]),
            ),
            attempt_id: mfm_ids::AttemptId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                mfm_ids::DigestBytes::from_array([0x42; 32]),
            ),
            commit_id: store::CommitKey::new("fact-query-authority").expect("commit key"),
            store_commit_order: 1,
            recorded_at: "2026-07-01T00:00:00Z".to_owned(),
            subject: CanonicalValue::object([("height", CanonicalValue::Unsigned(subject_height))])
                .expect("subject"),
            response: CanonicalValue::object([("height", CanonicalValue::Unsigned(800000))])
                .expect("response"),
            response_schema_id: schema_id(),
            response_artifact_id: None,
        },
    )
    .expect("fact fixture");
    let fact_ref = fact_fixture
        .projection
        .internal_ref()
        .expect("internal fact ref");
    FactAuthorityFixture {
        fact_ref,
        descriptor: descriptor_fixture.projection,
        query: fact_fixture.projection,
    }
}

fn projections(fixture: &FactAuthorityFixture) -> store::ProjectionSnapshot {
    store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
        fact_descriptors: BTreeMap::from([(
            fixture.descriptor.descriptor_hash.clone(),
            fixture.descriptor.clone(),
        )]),
        fact_query_entries: BTreeMap::from([(
            fixture.query.fact_claim_id().clone(),
            fixture.query.clone(),
        )]),
        ..store::ProjectionSnapshotParts::default()
    })
    .expect("projection snapshot")
}

fn query_evidence_artifact() -> RunnerJsonArtifact {
    RunnerJsonArtifact {
        bytes: b"{}".to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: artifact_id(0x30),
            digest: digest(0x31),
            byte_len: 2,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactQueryEvidence,
        },
    }
}

fn query_evidence(fact_ref: mfm_facts::InternalFactRef) -> mfm_facts::FactQueryEvidence {
    let input = mfm_facts::FactQueryInput::new(
        vec![mfm_facts::FactQueryPredicate::new(
            mfm_facts::FactFieldId::new("subject.height").expect("field"),
            mfm_facts::FactQueryOperator::Equal,
            mfm_facts::FactCanonicalScalar::UnsignedInteger(1),
        )],
        vec![mfm_facts::FactFieldId::new("result.height").expect("field")],
        mfm_facts::FactOrderingName::new("result.height.asc").expect("ordering"),
        Some(1),
    )
    .expect("query input");
    let plan = mfm_facts::compile_fact_query_plan(&descriptor(), input).expect("plan");
    let frontier = mfm_facts::StoreReadFrontier::new(
        mfm_ids::StoreScopeId::new("mfm.store_scope.v1:10101010101010101010101010101010")
            .expect("store scope"),
        mfm_facts::StoreCommitOrder::new(1),
    );
    let rows = [mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new())];
    let receipt = fact_query_receipt_for_test(FactQueryReceiptFixtureInputForTest {
        read_frontier: frontier,
        rows: &rows,
        include_returned_field_summaries: false,
        limit: None,
    });
    mfm_facts::FactQueryEvidence::new(
        plan,
        receipt,
        mfm_facts::FactSelectionEvidence::new(digest(0x22), Vec::new(), None).expect("selection"),
    )
}

#[test]
fn fact_query_evidence_retention_refs_include_returned_fact_authority_artifacts() {
    let fixture = fact_authority_fixture(17);
    let projections = projections(&fixture);
    let evidence_artifact = query_evidence_artifact();
    let evidence = query_evidence(fixture.fact_ref.clone());

    let refs = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
        .expect("fact query retention refs");

    assert!(refs.contains(
        &evidence_artifact
            .retention_ref()
            .expect("query evidence ref")
    ));
    assert!(refs.contains(
        &fixture
            .descriptor
            .descriptor_artifact_evidence
            .retention_ref()
            .expect("descriptor ref")
    ));
    assert!(refs.contains(
        &fixture
            .query
            .response_artifact_evidence()
            .expect("response evidence")
            .retention_ref()
            .expect("response ref")
    ));
}

#[test]
fn fact_query_evidence_retention_refs_reject_invalid_returned_fact_authority() {
    for (name, mutation, expected) in [
        ("missing descriptor", 0_u8, "missing descriptor authority"),
        ("missing query", 1, "missing fact query authority"),
        (
            "tampered query",
            2,
            "fact query authority does not match returned ref",
        ),
        (
            "missing response evidence",
            3,
            "missing response artifact authority",
        ),
    ] {
        let fixture = fact_authority_fixture(17);
        let mut parts = store::ProjectionSnapshotParts::from_snapshot(&projections(&fixture));
        match mutation {
            0 => parts.fact_descriptors.clear(),
            1 => parts.fact_query_entries.clear(),
            2 => {
                let tampered = fact_authority_fixture(18).query;
                parts
                    .fact_query_entries
                    .insert(tampered.fact_claim_id().clone(), tampered);
            }
            3 => {
                let query = store::FactQueryProjection::from_internal_ref(
                    &fixture.fact_ref,
                    fixture.query.attempt_id().clone(),
                    fixture.query.commit_id().clone(),
                    fixture.query.store_commit_order(),
                    None,
                    fixture.query.terms().cloned().collect(),
                )
                .expect("query without hydrated response evidence");
                parts
                    .fact_query_entries
                    .insert(query.fact_claim_id().clone(), query);
            }
            _ => unreachable!(),
        }
        let projections = store::ProjectionSnapshot::from_parts(parts).expect("mutated snapshot");
        let evidence = query_evidence(fixture.fact_ref);
        let error =
            fact_query_evidence_retention_refs(&query_evidence_artifact(), &evidence, &projections)
                .expect_err("invalid returned authority rejects");
        assert!(
            matches!(&error, RuntimeError::InvalidRunnerOutput(message) if message.contains(expected)),
            "{name} returned {error:?}"
        );
    }
}
