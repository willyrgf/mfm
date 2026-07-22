use super::*;

pub(super) fn test_fact_descriptor() -> mfm_facts::FactDescriptor {
    <RuntimeTestFact as mfm_facts::MfmFactType>::descriptor().expect("fact descriptor")
}

pub(super) fn test_fact_key(subject_amount: u64) -> mfm_facts::FactKey {
    test_fact_subject_evidence(subject_amount)
        .fact_key()
        .clone()
}

pub(super) fn test_fact_query_evidence() -> mfm_facts::FactQueryEvidence {
    test_fact_query_evidence_with_returned_refs(Vec::new())
}

pub(super) fn test_fact_query_evidence_with_returned_refs(
    returned_refs: Vec<mfm_facts::InternalFactRef>,
) -> mfm_facts::FactQueryEvidence {
    let input = mfm_facts::FactQueryInput::new(
        vec![mfm_facts::FactQueryPredicate::new(
            mfm_facts::FactFieldId::new("subject.amount").expect("field id"),
            mfm_facts::FactQueryOperator::Equal,
            mfm_facts::FactCanonicalScalar::UnsignedInteger(42),
        )],
        vec![mfm_facts::FactFieldId::new("result.amount").expect("field id")],
        mfm_facts::FactOrderingName::new("result.amount.asc").expect("ordering"),
        Some(10),
    )
    .expect("query input");
    let plan =
        mfm_facts::compile_fact_query_plan(&test_fact_descriptor(), input).expect("query plan");
    let frontier = mfm_facts::StoreReadFrontier::new(
        fixture_store_scope_id(),
        mfm_facts::StoreCommitOrder::new(10),
    );
    let rows = returned_refs
        .into_iter()
        .map(|fact_ref| mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new()))
        .collect::<Vec<_>>();
    let receipt = test_fact_query_receipt(FactQueryReceiptFixtureInputForTest {
        read_frontier: frontier,
        rows: &rows,
        include_returned_field_summaries: false,
        limit: plan.limit(),
    });
    let selection = mfm_facts::FactSelectionEvidence::new(content(0x45), Vec::new(), None)
        .expect("selection evidence");
    mfm_facts::FactQueryEvidence::new(plan, receipt, selection)
}

pub(super) fn test_fact_claim(
    subject_amount: u64,
    response_evidence: &store::ArtifactEvidenceRef,
) -> mfm_facts::FactClaim {
    test_fact_claim_for_descriptor(&test_fact_descriptor(), subject_amount, response_evidence)
}

pub(super) fn test_fact_claim_for_descriptor(
    descriptor: &mfm_facts::FactDescriptor,
    subject_amount: u64,
    response_evidence: &store::ArtifactEvidenceRef,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        fact_kind: descriptor.fact_kind().clone(),
        fact_descriptor_hash: mfm_facts::fact_descriptor_hash(descriptor).expect("descriptor hash"),
        subject: test_fact_subject_evidence_for_descriptor(descriptor, subject_amount),
        response: mfm_facts::FactResponseEvidence::new(
            response_evidence
                .schema_id
                .clone()
                .expect("fact response schema"),
            response_evidence.digest.clone(),
            response_evidence.artifact_id.clone(),
            response_evidence
                .evidence_hash()
                .expect("fact response evidence hash"),
        ),
    })
    .expect("fact claim")
}

pub(super) fn test_fact_subject_evidence(subject_amount: u64) -> mfm_facts::FactSubjectEvidence {
    test_fact_subject_evidence_for_descriptor(&test_fact_descriptor(), subject_amount)
}

pub(super) fn test_fact_subject_evidence_for_descriptor(
    descriptor: &mfm_facts::FactDescriptor,
    subject_amount: u64,
) -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV2::new(
        mfm_canonical::CanonicalValue::object([(
            "amount",
            mfm_canonical::CanonicalValue::Unsigned(subject_amount),
        )])
        .expect("subject"),
    )
    .expect("subject material");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(descriptor).expect("fact subject namespace hash");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

pub(super) fn test_fact_response_artifact(
    node: &spec::NodeSpec,
    amount: u64,
) -> (store::ArtifactEvidenceRef, Vec<u8>) {
    let bytes =
        canonical_json(serde_json::json!({ "amount": amount })).expect("fact response json");
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(
            <CertifierValue as mfm_values::MfmValue>::schema_id().expect("fact response schema"),
        ),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    (evidence, bytes.to_vec())
}

pub(super) fn test_returned_fact_authority(
    fixture: &Fixture,
    node: &spec::NodeSpec,
) -> (
    mfm_facts::InternalFactRef,
    store::FactDescriptorProjection,
    store::FactQueryProjection,
) {
    let descriptor = test_fact_descriptor();
    let source_event_id = EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x77; 32]),
    );
    let descriptor_fixture =
        store::test_support::fact_descriptor_projection_fixture_for_test(descriptor.clone())
            .expect("descriptor projection fixture");
    let fact_fixture = store::test_support::fact_projection_fixture_for_test(
        &descriptor,
        descriptor_fixture.descriptor_hash.clone(),
        store::test_support::FactProjectionFixtureInputForTest {
            run_id: fixture.run_id.clone(),
            source_seq: 2,
            source_ordinal: 0,
            source_event_id,
            node_id: node.node_id.clone(),
            attempt_id: AttemptId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x79; 32]),
            ),
            commit_id: store::CommitKey::new("test-returned-fact").expect("commit key"),
            store_commit_order: 2,
            recorded_at: "2026-07-01T00:00:00Z".to_owned(),
            subject: CanonicalValue::object([("amount", CanonicalValue::Unsigned(17))])
                .expect("fact subject"),
            response: CanonicalValue::object([("amount", CanonicalValue::Unsigned(23))])
                .expect("fact response"),
            response_schema_id: <CertifierValue as mfm_values::MfmValue>::schema_id()
                .expect("fact response schema"),
            response_artifact_id: None,
        },
    )
    .expect("fact projection fixture");
    let fact_ref = fact_fixture
        .projection
        .internal_ref()
        .expect("internal fact ref");
    (
        fact_ref,
        descriptor_fixture.projection,
        fact_fixture.projection,
    )
}

pub(super) fn projection_snapshot_with_returned_fact_authority(
    base: &store::ProjectionSnapshot,
    descriptor: store::FactDescriptorProjection,
    query: store::FactQueryProjection,
) -> store::ProjectionSnapshot {
    let mut parts = store::ProjectionSnapshotParts::from_snapshot(base);
    parts
        .fact_descriptors
        .insert(descriptor.descriptor_hash.clone(), descriptor);
    parts
        .fact_query_entries
        .insert(query.fact_claim_id().clone(), query);
    store::ProjectionSnapshot::from_parts(parts)
        .expect("projection snapshot with returned fact authority")
}
