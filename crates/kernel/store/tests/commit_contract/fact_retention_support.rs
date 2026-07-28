use super::*;

pub(super) fn fact_key() -> mfm_facts::FactKey {
    fact_subject_evidence().fact_key().clone()
}

pub(super) fn fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.test.fact").expect("fact kind"),
        mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
        schema_id("mfm.test.fact_subject", 89),
        schema_id("mfm.test.fact_response", 96),
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

pub(super) fn fact_descriptor_fixture(
) -> mfm_store::v1::test_support::FactDescriptorProjectionFixtureForTest {
    fact_descriptor_projection_fixture_for_test(fact_descriptor()).expect("fact descriptor fixture")
}

pub(super) fn fact_query_plan() -> mfm_facts::CanonicalFactQueryPlan {
    let input = mfm_facts::FactQueryInput::new(
        vec![
            mfm_facts::FactQueryPredicate::new(
                mfm_facts::FactFieldId::new("subject.chain").expect("field"),
                mfm_facts::FactQueryOperator::Equal,
                mfm_facts::FactCanonicalScalar::string("store_test_chain"),
            ),
            mfm_facts::FactQueryPredicate::new(
                mfm_facts::FactFieldId::new("result.height").expect("field"),
                mfm_facts::FactQueryOperator::GreaterThanOrEqual,
                mfm_facts::FactCanonicalScalar::UnsignedInteger(800_000),
            ),
        ],
        vec![
            mfm_facts::FactFieldId::new("subject.chain").expect("field"),
            mfm_facts::FactFieldId::new("result.height").expect("field"),
        ],
        mfm_facts::FactOrderingName::new("result.height.desc").expect("ordering"),
        None,
    )
    .expect("fact query input");
    mfm_facts::compile_fact_query_plan(&fact_descriptor(), input).expect("fact query plan")
}

pub(super) fn fact_subject_evidence() -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterial::new(
        mfm_canonical::CanonicalValue::object([(
            "chain",
            mfm_canonical::CanonicalValue::String("store_test_chain".into()),
        )])
        .expect("subject"),
    )
    .expect("subject material");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(&fact_descriptor()).expect("subject namespace hash");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

pub(super) fn fact_response_bytes_for_height(height: u64) -> Vec<u8> {
    PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"height":{height}}}"#))
        .expect("response bytes")
        .to_vec()
}

pub(super) fn fact_response_bytes() -> Vec<u8> {
    fact_response_bytes_for_height(850000)
}

pub(super) fn fact_artifact_ref_for_height(height: u64) -> ArtifactEvidenceRef {
    let bytes = fact_response_bytes_for_height(height);
    let digest = PlainCanonicalJsonBytes::from_canonical_json_slice(&bytes)
        .expect("canonical response bytes")
        .content_digest();
    ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.fact_response", 96)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(90)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::FactResponse,
    }
}

pub(super) fn fact_artifact_ref() -> ArtifactEvidenceRef {
    fact_artifact_ref_for_height(850000)
}

pub(super) fn fact_claim(response: &ArtifactEvidenceRef) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        fact_kind: mfm_facts::FactKind::new("mfm.test.fact").expect("fact kind"),
        fact_descriptor_hash: fact_descriptor_hash(),
        subject: fact_subject_evidence(),
        response: mfm_facts::FactResponseEvidence::new(
            schema_id("mfm.test.fact_response", 96),
            response.digest.clone(),
            response.artifact_id.clone(),
            response.evidence_hash().expect("response evidence hash"),
        ),
    })
    .expect("fact claim")
}

pub(super) fn fact_recorded(response: &ArtifactEvidenceRef) -> KernelEventPayload {
    fact_recorded_for_attempt(response, attempt_id(91))
}

pub(super) fn fact_recorded_for_attempt(
    response: &ArtifactEvidenceRef,
    attempt_id: AttemptId,
) -> KernelEventPayload {
    KernelEventPayload::FactRecorded(events::FactRecorded {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        attempt_id,
        claim: fact_claim(response),
    })
}

pub(super) fn fact_attempt_started() -> KernelEventPayload {
    fact_attempt_started_for(attempt_id(91))
}

pub(super) fn fact_attempt_started_for(attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        attempt_id,
        attempt_no: 1,
        state_kind: state_kind(90),
        state_version: StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
    })
}

pub(super) fn fact_output_artifact_ref_for_height(height: u64) -> ArtifactEvidenceRef {
    let bytes = fact_response_bytes_for_height(height);
    let digest = PlainCanonicalJsonBytes::from_canonical_json_slice(&bytes)
        .expect("canonical output bytes")
        .content_digest();
    ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.side_effect_output", 97)),
        semantic_type_id: Some(semantic_id("side_effect_output", 98)),
        producer_node_id: Some(node_id(90)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::StateOutput,
    }
}

pub(super) fn fact_cell_produced_for_height(
    height: u64,
    attempt_id: AttemptId,
) -> KernelEventPayload {
    let evidence = fact_output_artifact_ref_for_height(height);
    KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        cell_id: cell_id(90),
        scope_id: scope_id(90),
        attempt_id,
        semantic_type_id: semantic_id("side_effect_output", 98),
        schema_id: schema_id("mfm.test.side_effect_output", 97),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(99),
        },
        context: spec::CellContextSpec::no_context(),
        artifact_id: evidence.artifact_id.clone(),
        content_digest: evidence.digest.clone(),
        evidence_hash: evidence.evidence_hash().expect("output evidence hash"),
        producer_state_kind: Some(state_kind(90)),
        producer_state_version: Some(
            StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
        ),
    })
}

pub(super) fn fact_attempt_completed(attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        attempt_id,
        output_cell_id: cell_id(90),
    })
}

pub(super) fn retention_refs_appended(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
) -> KernelEventPayload {
    retention_refs_appended_for_run(&run_id(120), artifact_id, digest, role)
}

pub(super) fn retention_refs_appended_for_run(
    run_id: &RunId,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
) -> KernelEventPayload {
    KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
        run_id: run_id.clone(),
        spec_hash: spec_hash(1),
        refs: vec![events::RetentionRef {
            artifact_id: artifact_id.clone(),
            role,
            evidence_hash: store_artifact_ref(artifact_id, digest.clone())
                .evidence_hash()
                .expect("retention evidence hash"),
            content_digest: digest,
        }],
        reason: events::RetentionReason::RuntimeEvidence,
    })
}

pub(super) fn retention_manifest_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 512,
        media_type: media_type("application/vnd.mfm.retention-manifest+json;version=1"),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::RetentionManifest,
    }
}

pub(super) fn retention_manifest_projected(
    seq: u64,
    digest: ContentDigest,
    previous: Option<ContentDigest>,
    artifact_id: ArtifactId,
) -> KernelEventPayload {
    let evidence = retention_manifest_artifact_ref(artifact_id.clone(), digest.clone());
    KernelEventPayload::RetentionManifestProjected(events::RetentionManifestProjected {
        run_id: run_id(120),
        spec_hash: spec_hash(1),
        manifest_seq: seq,
        manifest_digest: digest,
        previous_manifest_digest: previous,
        manifest_artifact_id: artifact_id,
        manifest_artifact_evidence_hash: evidence.evidence_hash().expect("manifest evidence hash"),
    })
}

pub(super) fn retention_manifest_commit_payloads(
    seq: u64,
    digest: ContentDigest,
    previous: Option<ContentDigest>,
    artifact_id: ArtifactId,
) -> Vec<KernelEventPayload> {
    vec![
        retention_manifest_projected(seq, digest.clone(), previous, artifact_id.clone()),
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: run_id(120),
            spec_hash: spec_hash(1),
            refs: vec![events::RetentionRef {
                artifact_id: artifact_id.clone(),
                role: ArtifactRole::RetentionManifest,
                evidence_hash: retention_manifest_artifact_ref(artifact_id, digest.clone())
                    .evidence_hash()
                    .expect("manifest evidence hash"),
                content_digest: digest,
            }],
            reason: events::RetentionReason::ManifestProjection,
        }),
    ]
}

pub(super) fn projection_differential_summary(
    store: &StoreContractRunStore,
    run_id: &RunId,
    records: &[KernelEventEnvelope],
) -> String {
    let rebuilt =
        ProjectionSnapshot::rebuild_from_run_stream(records).expect("rebuild projections");
    assert_eq!(store.projection_snapshot(), &rebuilt);

    let commit_count = records
        .iter()
        .enumerate()
        .filter(|(index, record)| *index == 0 || records[*index - 1].seq() != record.seq())
        .count();
    let next_seq = records
        .last()
        .map(KernelEventEnvelope::seq)
        .and_then(|sequence| sequence.as_u64().checked_add(1))
        .and_then(|sequence| StreamSeq::new(sequence).ok())
        .expect("projection differential has a bounded committed head");
    projection_snapshot_summary(&rebuilt, run_id, commit_count, records.len(), next_seq)
}

fn projection_snapshot_summary(
    snapshot: &ProjectionSnapshot,
    run_id: &RunId,
    commit_count: usize,
    event_count: usize,
    next_seq: StreamSeq,
) -> String {
    let mut rows = Vec::new();
    rows.push(format!(
        "committed run_state={:?} commits={} events={} next_seq={}",
        snapshot.run_state(run_id),
        commit_count,
        event_count,
        next_seq.as_u64()
    ));

    rows.extend(snapshot.fact_query_entries().map(|(_claim_id, fact)| {
        format!(
            "fact key={} schema={} artifact={}",
            fact.fact_key().as_str(),
            fact.response_schema_id().as_str(),
            fact.artifact_id().as_str()
        )
    }));

    rows.extend(snapshot.side_effects().map(|(ledger_ref, side_effect)| {
        format!(
            "side_effect pair={} phase={} prepared={} resource_key={} touched_set={}",
            ledger_ref.pair_id.as_str(),
            side_effect.phase.as_str(),
            side_effect.prepared_invocation.is_some(),
            side_effect.resource_key.is_some(),
            side_effect.resource_touched_set.is_some()
        )
    }));

    rows.extend(snapshot.resource_lanes().map(|(lane_key, lane)| {
        format!(
            "resource_lane {}:{} holder={} phase_epoch={}",
            lane_key.namespace.as_str(),
            lane_key.key.as_str(),
            lane.holder.pair_id.as_str(),
            lane.invocation_epoch
        )
    }));

    rows.extend(
        snapshot
            .public_outputs()
            .map(|(run_id, schema_id, projection)| {
                let PublicOutputProjection::Produced {
                    rendered_artifact_id,
                    ..
                } = projection
                else {
                    return format!(
                        "public_output run={} schema={} failed",
                        run_id.as_str(),
                        schema_id.as_str()
                    );
                };
                format!(
                    "public_output run={} schema={} rendered_artifact={}",
                    run_id.as_str(),
                    schema_id.as_str(),
                    rendered_artifact_id.is_some()
                )
            }),
    );

    rows.extend(snapshot.retentions().map(|(retention_run_id, retention)| {
        let latest = retention.manifest.as_ref().expect("latest manifest");
        format!(
            "retention run={} refs={} manifests={} latest_seq={}",
            retention_run_id.as_str(),
            retention.refs.len(),
            retention.manifests.len(),
            latest.manifest_seq
        )
    }));

    rows.join("\n")
}

pub(super) fn artifact_role_tag_baselines() -> &'static [(ArtifactRole, &'static str)] {
    &[
        (ArtifactRole::TypedExecutionSpec, "typed_execution_spec"),
        (ArtifactRole::TypedSpecCertificate, "typed_spec_certificate"),
        (ArtifactRole::TypedConfig, "typed_config"),
        (ArtifactRole::SeedInput, "seed_input"),
        (ArtifactRole::StateOutput, "state_output"),
        (ArtifactRole::FactResponse, "fact_response"),
        (ArtifactRole::FactQueryEvidence, "fact_query_evidence"),
        (ArtifactRole::SideEffectIntent, "side_effect_intent"),
        (ArtifactRole::PreparedInvocation, "prepared_invocation"),
        (ArtifactRole::NotSubmittedProof, "not_submitted_proof"),
        (ArtifactRole::Submission, "submission"),
        (
            ArtifactRole::SubmissionUnknownEvidence,
            "submission_unknown_evidence",
        ),
        (ArtifactRole::Receipt, "receipt"),
        (ArtifactRole::Confirmation, "confirmation"),
        (ArtifactRole::AmbiguityEvidence, "ambiguity_evidence"),
        (
            ArtifactRole::ManualResolutionEvidence,
            "manual_resolution_evidence",
        ),
        (
            ArtifactRole::ManualResolutionAuthorization,
            "manual_resolution_authorization",
        ),
        (ArtifactRole::PublicOutput, "public_output"),
        (ArtifactRole::RedactedDiagnostic, "redacted_diagnostic"),
        (ArtifactRole::RetentionManifest, "retention_manifest"),
    ]
}

pub(super) fn spec_artifact_ref() -> ArtifactEvidenceRef {
    let hash = spec_hash(1);
    ArtifactEvidenceRef {
        artifact_id: artifact_id(2),
        digest: ContentDigest::from_digest(hash.algorithm(), *hash.digest()),
        byte_len: 128,
        media_type: media_type(SPEC_MEDIA_TYPE),
        schema_id: Some(spec::typed_execution_spec_schema_id().expect("typed spec schema")),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::TypedExecutionSpec,
    }
}

pub(super) fn certificate_artifact_ref() -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(4),
        digest: content_digest(4),
        byte_len: 64,
        media_type: media_type(mfm_certify::CERTIFICATE_MEDIA_TYPE),
        schema_id: Some(
            mfm_certify::typed_spec_certificate_schema_id().expect("typed certificate schema"),
        ),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::TypedSpecCertificate,
    }
}

pub(super) fn run_artifact_ref(artifact: &ArtifactEvidenceRef) -> events::RunArtifactEvidenceRef {
    events::RunArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        evidence_hash: artifact
            .evidence_hash()
            .expect("run artifact evidence hash"),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    }
}
