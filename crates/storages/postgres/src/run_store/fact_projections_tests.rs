use super::*;

#[test]
fn physical_fact_projection_validation_rejects_term_descriptor_mismatch() {
    let mut projections = valid_physical_fact_projections();
    let other_descriptor_fixture =
        mfm_store::v1::test_support::fact_descriptor_projection_fixture_for_test(
            fact_descriptor_with_seed(2),
        )
        .expect("other descriptor fixture");
    let mismatched_descriptor_hash = other_descriptor_fixture.descriptor_hash.clone();
    projections.fact_descriptors.insert(
        other_descriptor_fixture.descriptor_hash.clone(),
        other_descriptor_fixture.projection,
    );
    let term = projections
        .fact_term_entries
        .values_mut()
        .next()
        .expect("index term");
    term.fact_descriptor_hash = mismatched_descriptor_hash.clone();

    let error = validate_physical_fact_projections(&projections)
        .expect_err("term descriptor mismatch should reject");
    assert!(
        error
            .to_string()
            .contains(mismatched_descriptor_hash.as_str()),
        "{error}"
    );
}

#[test]
fn physical_fact_projection_validation_rejects_missing_projection_links() {
    #[derive(Clone, Copy)]
    enum MissingLink {
        RecordForIndex,
        DescriptorForAdmission,
        RunAdmissionForRecord,
    }

    for (case, expected) in [
        (MissingLink::RecordForIndex, "has no FactRecorded payload"),
        (
            MissingLink::DescriptorForAdmission,
            "references missing descriptor row",
        ),
        (
            MissingLink::RunAdmissionForRecord,
            "references descriptor not admitted by run",
        ),
    ] {
        let mut projections = valid_physical_fact_projections();
        match case {
            MissingLink::RecordForIndex => {
                let claim_id = projections
                    .fact_index_entries
                    .keys()
                    .next()
                    .expect("index claim id")
                    .clone();
                projections.fact_records.remove(&claim_id);
            }
            MissingLink::DescriptorForAdmission => projections.fact_descriptors.clear(),
            MissingLink::RunAdmissionForRecord => projections.fact_descriptor_admissions.clear(),
        }

        let error =
            validate_physical_fact_projections(&projections).expect_err("missing link rejects");
        assert!(error.to_string().contains(expected), "{error}");
    }
}

fn valid_physical_fact_projections() -> PhysicalFactProjections {
    let descriptor = fact_descriptor();
    let descriptor_fixture =
        mfm_store::v1::test_support::fact_descriptor_projection_fixture_for_test(
            descriptor.clone(),
        )
        .expect("descriptor fixture");
    let fact_fixture = mfm_store::v1::test_support::fact_projection_fixture_for_test(
        &descriptor,
        descriptor_fixture.descriptor_hash.clone(),
        mfm_store::v1::test_support::FactProjectionFixtureInputForTest {
            run_id: run_id(3),
            source_seq: 7,
            source_ordinal: 0,
            source_event_id: event_id(12),
            node_id: node_id(13),
            attempt_id: attempt_id(14),
            commit_id: CommitKey::new("fact-term-descriptor-mismatch").expect("commit key"),
            store_commit_order: 1,
            recorded_at: "2026-01-02T03:04:05Z".to_owned(),
            observed_at: None,
            visibility: mfm_facts::FactVisibility::indexed_default(
                mfm_facts::FactAudience::Platform,
            ),
            subject: mfm_canonical::CanonicalValue::object([(
                "account",
                mfm_canonical::CanonicalValue::String("alice".to_owned()),
            )])
            .expect("subject"),
            response: mfm_canonical::CanonicalValue::object([(
                "ok",
                mfm_canonical::CanonicalValue::Bool(true),
            )])
            .expect("response"),
            request: None,
            response_schema_id: schema_id("response", 5),
            response_artifact_id: None,
            producer: producer(),
        },
    )
    .expect("fact projection fixture");
    let index = fact_fixture.index.expect("indexed fact projection");
    let claim_id = index.fact_claim_id.clone();
    let term = fact_fixture.terms.into_iter().next().expect("index term");
    let descriptor_hash = descriptor_fixture.descriptor_hash.clone();
    let descriptor_admission = FactDescriptorAdmissionProjection {
        run_id: run_id(3),
        descriptor_hash: descriptor_hash.clone(),
        descriptor_artifact_id: descriptor_fixture.descriptor_artifact_id.clone(),
        descriptor_artifact_evidence: descriptor_fixture.descriptor_evidence.clone(),
        source_seq: 1,
        source_ordinal: 0,
        source_event_id: event_id(10),
    };
    PhysicalFactProjections {
        fact_descriptors: BTreeMap::from([(
            descriptor_hash.clone(),
            descriptor_fixture.projection,
        )]),
        fact_descriptor_admissions: BTreeMap::from([(
            (run_id(3), descriptor_hash),
            descriptor_admission,
        )]),
        fact_records: BTreeMap::from([(claim_id.clone(), fact_fixture.record)]),
        fact_index_entries: BTreeMap::from([(claim_id.clone(), index)]),
        fact_term_entries: BTreeMap::from([((claim_id, term.field_id.clone()), term)]),
    }
}

fn fact_descriptor() -> mfm_facts::FactDescriptor {
    fact_descriptor_with_seed(1)
}

fn fact_descriptor_with_seed(seed: u8) -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        fact_kind(),
        schema_id("descriptor", seed),
        schema_id("subject", seed + 1),
        schema_id("response", 5),
        vec![mfm_facts::FactFieldDescriptor::new(
            mfm_facts::FactFieldId::new("subject.account").expect("field id"),
            mfm_facts::FactFieldValueType::String,
            mfm_facts::FactFieldExtraction::Subject(
                mfm_facts::CanonicalValuePath::new("account").expect("extraction"),
            ),
            mfm_facts::FactFieldPolicy::new(
                vec![mfm_facts::FactQueryOperator::Equal],
                mfm_facts::FactFieldExposure::Returnable,
            )
            .required(),
        )
        .expect("field descriptor")],
        Vec::new(),
    )
    .expect("fact descriptor")
}

fn producer() -> mfm_facts::FactProducerProvenance {
    mfm_facts::FactProducerProvenance::new(
        capability_kind(31),
        mfm_ids::CapabilityVersion::new("mfm.pg.test.capability.v1").expect("capability version"),
        adapter_kind(32),
        mfm_ids::AdapterVersion::new("mfm.pg.test.adapter.v1").expect("adapter version"),
    )
}

fn fact_kind() -> mfm_facts::FactKind {
    mfm_facts::FactKind::new("mfm.pg.test.fact").expect("fact kind")
}

fn schema_id(name: &str, seed: u8) -> SchemaId {
    let schema_name = format!("mfm.pg.test.{name}");
    SchemaId::new(
        &schema_name,
        "1",
        mfm_ids::DigestAlgorithm::Sha256JcsV1,
        digest_bytes(seed),
    )
    .expect("schema id")
}

fn capability_kind(seed: u8) -> mfm_ids::CapabilityKind {
    mfm_ids::CapabilityKind::new(
        "mfm.pg.test",
        "capability",
        mfm_ids::DigestAlgorithm::Sha256JcsV1,
        digest_bytes(seed),
    )
    .expect("capability kind")
}

fn adapter_kind(seed: u8) -> mfm_ids::AdapterKind {
    mfm_ids::AdapterKind::new(
        "mfm.pg.test",
        "adapter",
        mfm_ids::DigestAlgorithm::Sha256JcsV1,
        digest_bytes(seed),
    )
    .expect("adapter kind")
}

fn run_id(seed: u8) -> RunId {
    RunId::from_digest(mfm_ids::DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn node_id(seed: u8) -> NodeId {
    NodeId::from_digest(mfm_ids::DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn attempt_id(seed: u8) -> mfm_ids::AttemptId {
    mfm_ids::AttemptId::from_digest(mfm_ids::DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn event_id(seed: u8) -> mfm_ids::EventId {
    mfm_ids::EventId::from_digest(mfm_ids::DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn digest_bytes(seed: u8) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array([seed; 32])
}
