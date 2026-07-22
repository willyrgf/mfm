use super::*;

pub(super) fn fact_key() -> mfm_facts::FactKey {
    fact_subject_evidence().fact_key().clone()
}

pub(super) fn fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.test.fact").expect("fact kind"),
        mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
        schema_id("mfm.test.fact_subject", 37),
        schema_id("mfm.test.fact_response", 36),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("subject.chain").expect("field id"),
                mfm_facts::FactFieldValueType::String,
                mfm_facts::FactFieldExtraction::Subject(
                    mfm_facts::CanonicalValuePath::new("chain").expect("path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("result.height").expect("field id"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Response(
                    mfm_facts::CanonicalValuePath::new("height").expect("path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![
                        mfm_facts::FactQueryOperator::Equal,
                        mfm_facts::FactQueryOperator::GreaterThanOrEqual,
                    ],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .sortable()
                .required(),
            )
            .expect("response field"),
        ],
        vec![mfm_facts::FactOrderingPolicy::new(
            mfm_facts::FactOrderingName::new("result.height.desc").expect("ordering"),
            vec![mfm_facts::FactOrderingTerm::new(
                mfm_facts::FactFieldId::new("result.height").expect("field id"),
                mfm_facts::SortDirection::Descending,
                mfm_facts::NullOrdering::Last,
                false,
            )],
        )
        .expect("height ordering")],
    )
    .expect("fact descriptor")
}

pub(super) fn fact_descriptor_hash() -> ContentDigest {
    fact_descriptor_fixture().descriptor_hash
}

pub(super) fn fact_descriptor_bytes() -> Vec<u8> {
    fact_descriptor_fixture().descriptor_bytes
}

pub(super) fn fact_descriptor_artifact_ref() -> ArtifactEvidenceRef {
    fact_descriptor_fixture().descriptor_evidence
}

pub(super) fn fact_descriptor_fixture() -> FactDescriptorProjectionFixtureForTest {
    fact_descriptor_projection_fixture_for_test(fact_descriptor()).expect("descriptor fixture")
}

pub(super) fn fact_subject_evidence() -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterial::new(
        mfm_canonical::CanonicalValue::object([(
            "chain",
            mfm_canonical::CanonicalValue::String("postgres_test_chain".into()),
        )])
        .expect("subject"),
    )
    .expect("subject material");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(&fact_descriptor()).expect("subject namespace hash");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

pub(super) fn fact_response_bytes() -> Vec<u8> {
    fact_response_bytes_with_height(12_345)
}

pub(super) fn fact_response_bytes_with_height(height: u64) -> Vec<u8> {
    PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"height":{height}}}"#))
        .expect("response bytes")
        .to_vec()
}

pub(super) fn fact_artifact_ref() -> ArtifactEvidenceRef {
    fact_artifact_ref_for_response_bytes(&fact_response_bytes())
}

pub(super) fn fact_artifact_ref_with_height(height: u64) -> ArtifactEvidenceRef {
    fact_artifact_ref_for_response_bytes(&fact_response_bytes_with_height(height))
}

pub(super) fn fact_artifact_ref_for_response_bytes(bytes: &[u8]) -> ArtifactEvidenceRef {
    let digest = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .expect("canonical response bytes")
        .content_digest();
    ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.fact_response", 36)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(30)),
        producer_seed_id: None,
        artifact_role: ArtifactRole::FactResponse,
    }
}

pub(super) fn fact_output_artifact_ref(response: &ArtifactEvidenceRef) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: response.artifact_id.clone(),
        digest: response.digest.clone(),
        byte_len: response.byte_len,
        media_type: response.media_type.clone(),
        schema_id: Some(schema_id("mfm.test.fact_output", 142)),
        semantic_type_id: Some(semantic_id("fact_output", 143)),
        producer_node_id: Some(node_id(30)),
        producer_seed_id: None,
        artifact_role: ArtifactRole::StateOutput,
    }
}

pub(super) fn fact_query_plan() -> mfm_facts::CanonicalFactQueryPlan {
    fact_query_plan_with_limit(Some(1))
}

pub(super) fn fact_query_plan_with_limit(limit: Option<u64>) -> mfm_facts::CanonicalFactQueryPlan {
    let input = mfm_facts::FactQueryInput::new(
        vec![
            mfm_facts::FactQueryPredicate::new(
                mfm_facts::FactFieldId::new("subject.chain").expect("field"),
                mfm_facts::FactQueryOperator::Equal,
                mfm_facts::FactCanonicalScalar::string("postgres_test_chain"),
            ),
            mfm_facts::FactQueryPredicate::new(
                mfm_facts::FactFieldId::new("result.height").expect("field"),
                mfm_facts::FactQueryOperator::GreaterThanOrEqual,
                mfm_facts::FactCanonicalScalar::UnsignedInteger(12_000),
            ),
        ],
        vec![
            mfm_facts::FactFieldId::new("subject.chain").expect("field"),
            mfm_facts::FactFieldId::new("result.height").expect("field"),
        ],
        mfm_facts::FactOrderingName::new("result.height.desc").expect("ordering"),
        limit,
    )
    .expect("fact query input");
    mfm_facts::compile_fact_query_plan(&fact_descriptor(), input).expect("fact query plan")
}

pub(super) fn fact_claim(response: &ArtifactEvidenceRef) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        fact_kind: mfm_facts::FactKind::new("mfm.test.fact").expect("fact kind"),
        fact_descriptor_hash: fact_descriptor_hash(),
        subject: fact_subject_evidence(),
        response: mfm_facts::FactResponseEvidence::new(
            schema_id("mfm.test.fact_response", 36),
            response.digest.clone(),
            response.artifact_id.clone(),
            response.evidence_hash().expect("response evidence hash"),
        ),
    })
    .expect("fact claim")
}

pub(super) fn fact_recorded(response: &ArtifactEvidenceRef) -> KernelEventPayload {
    KernelEventPayload::FactRecorded(events::FactRecorded {
        spec_hash: spec_hash(1),
        node_id: node_id(30),
        attempt_id: attempt_id(31),
        claim: fact_claim(response),
    })
}

pub(super) fn fact_cell_produced(response: &ArtifactEvidenceRef) -> KernelEventPayload {
    let output = fact_output_artifact_ref(response);
    KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(30),
        cell_id: cell_id(145),
        scope_id: scope_id(144),
        attempt_id: attempt_id(31),
        semantic_type_id: semantic_id("fact_output", 143),
        schema_id: schema_id("mfm.test.fact_output", 142),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(181),
        },
        context: spec::CellContextSpec::no_context(),
        artifact_id: output.artifact_id.clone(),
        content_digest: output.digest.clone(),
        evidence_hash: output.evidence_hash().expect("output evidence hash"),
        producer_state_kind: Some(state_kind(30)),
        producer_state_version: Some(
            StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
        ),
    })
}

pub(super) fn fact_attempt_completed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: node_id(30),
        attempt_id: attempt_id(31),
        output_cell_id: cell_id(145),
    })
}

pub(super) fn fact_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(30),
        attempt_id: attempt_id(31),
        attempt_no: 1,
        state_kind: state_kind(30),
        state_version: StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
    })
}
