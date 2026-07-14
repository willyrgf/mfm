use super::*;

pub(super) fn test_fact_descriptor() -> mfm_facts::FactDescriptor {
    <RuntimeTestFact as mfm_program::MfmFactType>::descriptor().expect("fact descriptor")
}

pub(super) fn test_fact_descriptor_with_kind(kind: &str) -> mfm_facts::FactDescriptor {
    let descriptor = test_fact_descriptor();
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new(kind).expect("fact kind"),
        descriptor.descriptor_schema_id().clone(),
        descriptor.subject_schema_id().clone(),
        descriptor.response_schema_id().clone(),
        descriptor.fields().to_vec(),
        descriptor.orderings().to_vec(),
    )
    .expect("fact descriptor")
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
    let query_scope = mfm_facts::FactQueryScope::new(
        mfm_facts::FactAudience::Platform,
        mfm_facts::FactVisibilityScope::Default,
    );
    let store_scope = mfm_facts::StoreScopeRef::new("default").expect("store scope");
    let input = mfm_facts::FactQueryInput::new(
        store_scope.clone(),
        query_scope.clone(),
        mfm_facts::ScopeDecisionEvidence::new(content(0x42)),
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
        store_scope,
        query_scope,
        mfm_facts::DescriptorCatalogWatermark::new(1),
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

#[allow(clippy::too_many_arguments)]
pub(super) fn test_fact_claim(
    subject_amount: u64,
    request_schema_id: SchemaId,
    request_hash: ContentDigest,
    response_evidence: &store::ArtifactEvidenceRef,
    capability_kind: CapabilityKind,
    capability_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
) -> mfm_facts::FactClaim {
    test_fact_claim_for_descriptor(
        &test_fact_descriptor(),
        subject_amount,
        request_schema_id,
        request_hash,
        response_evidence,
        capability_kind,
        capability_version,
        adapter_kind,
        adapter_version,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn test_fact_claim_for_descriptor(
    descriptor: &mfm_facts::FactDescriptor,
    subject_amount: u64,
    request_schema_id: SchemaId,
    request_hash: ContentDigest,
    response_evidence: &store::ArtifactEvidenceRef,
    capability_kind: CapabilityKind,
    capability_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: descriptor.fact_kind().clone(),
        fact_descriptor_hash: mfm_facts::fact_descriptor_hash(descriptor).expect("descriptor hash"),
        subject: test_fact_subject_evidence_for_descriptor(descriptor, subject_amount),
        observed_at: Some("2026-01-02T03:04:05Z".to_owned()),
        request: Some(mfm_facts::FactRequestEvidence::new(
            request_schema_id,
            request_hash,
        )),
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
        producer: mfm_facts::FactProducerProvenance::new(
            capability_kind,
            capability_version,
            adapter_kind,
            adapter_version,
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
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.amount").expect("field"),
        mfm_facts::FactFieldValueType::UnsignedInteger,
        mfm_facts::FactCanonicalScalar::UnsignedInteger(subject_amount),
    )
    .expect("subject value")])
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
    store::FactRecordProjection,
    store::FactIndexProjection,
    Vec<store::FactIndexTermProjection>,
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
            observed_at: Some("2026-07-01T00:01:00Z".to_owned()),
            visibility: mfm_facts::FactVisibility::indexed_default(
                mfm_facts::FactAudience::Platform,
            ),
            subject: CanonicalValue::object([("amount", CanonicalValue::Unsigned(17))])
                .expect("fact subject"),
            response: CanonicalValue::object([("amount", CanonicalValue::Unsigned(23))])
                .expect("fact response"),
            request: None,
            response_schema_id: <CertifierValue as mfm_values::MfmValue>::schema_id()
                .expect("fact response schema"),
            response_artifact_id: None,
            producer: mfm_facts::FactProducerProvenance::new(
                fixture.cap_kind.clone(),
                fixture.cap_version.clone(),
                fixture.adapter_kind.clone(),
                fixture.adapter_version.clone(),
            ),
        },
    )
    .expect("fact projection fixture");
    let index_projection = fact_fixture.index.expect("indexed fact projection");
    let fact_ref = index_projection.internal_ref().expect("internal fact ref");
    (
        fact_ref,
        descriptor_fixture.projection,
        fact_fixture.record,
        index_projection,
        fact_fixture.terms,
    )
}

pub(super) fn projection_snapshot_with_returned_fact_authority(
    base: &store::ProjectionSnapshot,
    descriptor: store::FactDescriptorProjection,
    record: store::FactRecordProjection,
    index: store::FactIndexProjection,
    terms: Vec<store::FactIndexTermProjection>,
) -> store::ProjectionSnapshot {
    let mut parts = store::ProjectionSnapshotParts::from_snapshot(base);
    parts
        .fact_descriptors
        .insert(descriptor.descriptor_hash.clone(), descriptor);
    parts
        .fact_records
        .insert(record.fact_claim_id.clone(), record);
    parts
        .fact_index_entries
        .insert(index.fact_claim_id.clone(), index);
    parts.fact_term_entries.extend(
        terms
            .into_iter()
            .map(|term| ((term.fact_claim_id.clone(), term.field_id.clone()), term)),
    );
    store::ProjectionSnapshot::from_parts(parts)
        .expect("projection snapshot with returned fact authority")
}
