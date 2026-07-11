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

fn capability_kind() -> CapabilityKind {
    CapabilityKind::new(
        "mfm.test.capability",
        "read",
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([2; 32]),
    )
    .expect("capability")
}

fn adapter_kind() -> AdapterKind {
    AdapterKind::new(
        "mfm.test.adapter",
        "read",
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([3; 32]),
    )
    .expect("adapter")
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

fn fact_producer_node_id() -> mfm_ids::NodeId {
    mfm_ids::NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        mfm_ids::DigestBytes::from_array([0x19; 32]),
    )
}

fn fact_projection_fixture(
    subject_height: u64,
) -> (
    mfm_facts::InternalFactRef,
    store::FactDescriptorProjection,
    store::FactRecordProjection,
    store::FactIndexProjection,
) {
    let descriptor = descriptor();
    let descriptor_fixture = fact_descriptor_projection_fixture_for_test(descriptor.clone())
        .expect("descriptor fixture");
    let fact_fixture = fact_projection_fixture_for_test(
        &descriptor,
        descriptor_fixture.descriptor_hash.clone(),
        FactProjectionFixtureInputForTest {
            run_id: run_id(0x10),
            source_seq: 1,
            source_ordinal: 0,
            source_event_id: event_id(0x11),
            node_id: fact_producer_node_id(),
            attempt_id: mfm_ids::AttemptId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                mfm_ids::DigestBytes::from_array([0x42; 32]),
            ),
            commit_id: store::CommitKey::new("fact-query-authority").expect("commit key"),
            store_commit_order: 1,
            recorded_at: "2026-07-01T00:00:00Z".to_owned(),
            observed_at: None,
            visibility: mfm_facts::FactVisibility::indexed_default(
                mfm_facts::FactAudience::Platform,
            ),
            subject: CanonicalValue::object([("height", CanonicalValue::Unsigned(subject_height))])
                .expect("subject"),
            response: CanonicalValue::object([("height", CanonicalValue::Unsigned(800000))])
                .expect("response"),
            request: None,
            response_schema_id: schema_id(),
            response_artifact_id: None,
            producer: mfm_facts::FactProducerProvenance::new(
                capability_kind(),
                CapabilityVersion::new("mfm.test.capability.v1").expect("capability version"),
                adapter_kind(),
                AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
            ),
        },
    )
    .expect("fact fixture");
    let index = fact_fixture.index.expect("indexed fact fixture");
    let fact_ref = index.internal_ref().expect("internal fact ref");
    (
        fact_ref,
        descriptor_fixture.projection,
        fact_fixture.record,
        index,
    )
}

fn fact_ref() -> mfm_facts::InternalFactRef {
    fact_projection_fixture(17).0
}

fn fact_authority_projections(fact_ref: &mfm_facts::InternalFactRef) -> store::ProjectionSnapshot {
    let (_built_ref, descriptor_projection, record_projection, index_projection) =
        fact_projection_fixture(17);
    assert_eq!(index_projection.fact_claim_id, *fact_ref.fact_claim_id());
    assert_eq!(
        index_projection.source_event_id,
        *fact_ref.source_event_id()
    );
    store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
        fact_descriptors: BTreeMap::from([(
            descriptor_projection.descriptor_hash.clone(),
            descriptor_projection,
        )]),
        fact_records: BTreeMap::from([(
            record_projection.fact_claim_id.clone(),
            record_projection,
        )]),
        fact_index_entries: BTreeMap::from([(
            index_projection.fact_claim_id.clone(),
            index_projection,
        )]),
        ..store::ProjectionSnapshotParts::default()
    })
    .expect("projection snapshot")
}

fn fact_authority_projections_without(
    fact_ref: &mfm_facts::InternalFactRef,
    descriptor: bool,
    record: bool,
    index: bool,
) -> store::ProjectionSnapshot {
    let full = fact_authority_projections(fact_ref);
    let mut parts = store::ProjectionSnapshotParts::from_snapshot(&full);
    if !descriptor {
        parts.fact_descriptors.clear();
    }
    if !record {
        parts.fact_records.clear();
    }
    if !index {
        parts.fact_index_entries.clear();
    }
    store::ProjectionSnapshot::from_parts(parts).expect("filtered fact authority projection")
}

fn fact_authority_projections_with_tampered_subject(
    fact_ref: &mfm_facts::InternalFactRef,
) -> store::ProjectionSnapshot {
    let full = fact_authority_projections(fact_ref);
    let mut parts = store::ProjectionSnapshotParts::from_snapshot(&full);
    parts
        .fact_records
        .get_mut(fact_ref.fact_claim_id())
        .expect("fact record")
        .claim = fact_projection_fixture(18).2.claim;
    store::ProjectionSnapshot::from_parts(parts).expect("tampered fact authority projection")
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
    let query_scope = mfm_facts::FactQueryScope::new(
        mfm_facts::FactAudience::Platform,
        mfm_facts::FactVisibilityScope::Default,
    );
    let store_scope = mfm_facts::StoreScopeRef::new("default").expect("store scope");
    let input = mfm_facts::FactQueryInput::new(
        store_scope.clone(),
        query_scope.clone(),
        mfm_facts::ScopeDecisionEvidence::new(digest(0x19)),
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
        store_scope,
        query_scope,
        mfm_facts::DescriptorCatalogWatermark::new(1),
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
    let fact_ref = fact_ref();
    let evidence_artifact = query_evidence_artifact();
    let projections = fact_authority_projections(&fact_ref);
    let descriptor_projection = projections
        .fact_descriptor(fact_ref.fact_descriptor_hash())
        .expect("descriptor projection");
    let evidence = query_evidence(fact_ref.clone());

    let refs = fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections)
        .expect("fact query retention refs");

    assert!(refs.contains(&events::RetentionRef {
        artifact_id: evidence_artifact.evidence.artifact_id.clone(),
        role: events::ArtifactRole::FactQueryEvidence,
        evidence_hash: evidence_artifact
            .evidence
            .evidence_hash()
            .expect("query evidence hash"),
        content_digest: evidence_artifact.evidence.digest.clone(),
    }));
    assert!(refs.contains(&events::RetentionRef {
        artifact_id: descriptor_projection.descriptor_artifact_id.clone(),
        role: events::ArtifactRole::FactDescriptor,
        evidence_hash: descriptor_projection
            .descriptor_artifact_evidence
            .evidence_hash()
            .expect("descriptor evidence hash"),
        content_digest: fact_ref.fact_descriptor_hash().clone(),
    }));
    assert!(refs.contains(&events::RetentionRef {
        artifact_id: fact_ref.artifact_id().clone(),
        role: events::ArtifactRole::FactResponse,
        evidence_hash: fact_ref.artifact_evidence_hash().clone(),
        content_digest: fact_ref.response_hash().clone(),
    }));
}

#[test]
fn fact_query_evidence_retention_refs_reject_invalid_returned_fact_authority() {
    for (name, descriptor, record, index, tampered, expected) in [
        (
            "missing descriptor",
            false,
            true,
            true,
            false,
            "missing descriptor authority",
        ),
        (
            "missing source fact",
            true,
            false,
            true,
            false,
            "missing source fact authority",
        ),
        (
            "missing indexed fact",
            true,
            true,
            false,
            false,
            "missing indexed fact authority",
        ),
        (
            "tampered subject",
            true,
            true,
            true,
            true,
            "source fact authority does not match",
        ),
    ] {
        let fact_ref = fact_ref();
        let evidence_artifact = query_evidence_artifact();
        let projections = if tampered {
            fact_authority_projections_with_tampered_subject(&fact_ref)
        } else {
            fact_authority_projections_without(&fact_ref, descriptor, record, index)
        };
        let evidence = query_evidence(fact_ref);

        let error =
            match fact_query_evidence_retention_refs(&evidence_artifact, &evidence, &projections) {
                Ok(_) => panic!("{name} authority should reject"),
                Err(error) => error,
            };

        assert!(
            matches!(&error, RuntimeError::InvalidRunnerOutput(message) if message.contains(expected)),
            "{name} returned {error:?}"
        );
    }
}
