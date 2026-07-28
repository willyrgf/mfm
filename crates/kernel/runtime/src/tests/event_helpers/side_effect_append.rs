use super::*;

impl SyntheticSideEffectAppend<'_> {
    pub(in crate::tests::support) fn append_with_artifact_bytes(
        &self,
        store: &mut TestTypedRunStore,
        commit_key: &str,
        payloads: Vec<events::KernelEventPayload>,
        artifacts: Vec<(Vec<u8>, store::ArtifactEvidenceRef)>,
        required_side_effect_state: store::RequiredSideEffectState,
        require_attempt_started: bool,
    ) {
        let required_artifacts = artifacts
            .iter()
            .map(|(_, evidence)| evidence.clone())
            .collect::<Vec<_>>();
        let required_present_logical_keys = require_attempt_started
            .then(|| {
                store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    self.node.node_id, self.attempt_id
                ))
                .expect("attempt logical key")
            })
            .into_iter()
            .collect::<Vec<_>>();
        store
            .append_prepared_commit_with_artifacts(
                store_typed_commit_request! {
                    run_id: self.run_id.clone(),
                    expected_next_seq: store.expected_next_seq(self.run_id),
                    commit_key: store::CommitKey::new(commit_key).expect("commit key"),
                    payloads: payloads,
                    required_artifacts: required_artifacts,
                    preconditions: store::CommitPreconditions {
                        required_run_state: store::RequiredRunState::NotCompleted,
                        required_present_logical_keys,
                        required_side_effect_states: vec![store::SideEffectStatePrecondition {
                            pair_id: fixture_side_effect_pair_id(self.fixture, self.node),
                            required: required_side_effect_state,
                        }],
                        certified_run_authority: Some(
                            store::CertifiedRunStoreAuthority::from_spec(
                                self.run_id.clone(),
                                self.fixture.runtime_spec.spec(),
                            )
                            .expect("certified run authority"),
                        ),
                        ..store::CommitPreconditions::default()
                    },
                },
                artifacts,
            )
            .expect("append synthetic side-effect commit with artifact bytes");
    }
}

pub(in crate::tests::support) fn canonical_fixture_side_effect_bytes(
    value: &FixtureSideEffectEvidence,
) -> Vec<u8> {
    let json = serde_json::to_string(value).expect("serialize fixture side-effect evidence");
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
        .expect("canonical fixture side-effect evidence")
        .to_vec()
}

pub(in crate::tests::support) fn append_synthetic_exclusive_prepare(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> (AttemptId, events::SideEffectLedgerKey) {
    let attempt_id =
        attempt_id(run_id, fixture.runtime_spec.spec_hash(), &node.node_id, 1).expect("attempt id");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(format!("{commit_key}-attempt-start"))
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no: 1,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic attempt start");

    let ledger =
        events::SideEffectLedgerKey::new(format!("holder-{commit_key}")).expect("holder ledger");
    let intent = fixture_side_effect_evidence(21, node.node_id.as_str(), attempt_id.as_str());
    let idempotency = fixture_side_effect_evidence(34, node.node_id.as_str(), attempt_id.as_str());
    let intent_bytes = canonical_fixture_side_effect_bytes(&intent);
    let intent_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&intent_bytes),
    );
    let idempotency_hash =
        content_digest_json(serde_json::to_value(&idempotency).expect("idempotency value"))
            .expect("idempotency digest");
    let evidence_schema = <FixtureSideEffectEvidence as mfm_values::MfmValue>::schema_id()
        .expect("side-effect evidence schema");
    let resource_key = exclusive_resource_key(fixture, key);
    let resource_lane_requirement_digest = content_digest_json(serde_json::json!({
        "acquisition": "pre_state_invocation",
        "hold": "until_side_effect_terminal",
        "key_schema_id": resource_key.key_schema_id.as_str(),
        "mode": "exclusive",
        "namespace": resource_key.namespace.as_str(),
    }))
    .expect("resource lane requirement digest");
    let intent_artifact_id =
        ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
    let intent_artifact = store::ArtifactEvidenceRef {
        artifact_id: intent_artifact_id.clone(),
        digest: intent_hash.clone(),
        byte_len: intent_bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(evidence_schema.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::SideEffectIntent,
    };
    let prepared = fixture_side_effect_evidence(35, node.node_id.as_str(), attempt_id.as_str());
    let prepared_bytes = canonical_fixture_side_effect_bytes(&prepared);
    let prepared_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&prepared_bytes),
    );
    let prepared_artifact = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(prepared_hash.algorithm(), *prepared_hash.digest()),
        digest: prepared_hash.clone(),
        byte_len: prepared_bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(evidence_schema.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::PreparedInvocation,
    };
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, &attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append_with_artifact_bytes(
        store,
        commit_key,
        vec![
            events::KernelEventPayload::SideEffectIntentPersisted(
                events::side_effect::IntentPersisted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: ledger_purpose.clone(),
                    pair_id: pair_id.clone(),
                    pair_role,
                    invocation_epoch: 1,
                    intent_schema_id: evidence_schema.clone(),
                    intent_hash: intent_hash.clone(),
                    intent_artifact_id,
                    intent_artifact_evidence_hash: intent_artifact
                        .evidence_hash()
                        .expect("intent evidence hash"),
                    idempotency_input_schema_id: evidence_schema.clone(),
                    idempotency_input_hash: idempotency_hash,
                    idempotency_key: crate::side_effect_driver::side_effect_idempotency_key(
                        &idempotency,
                    )
                    .expect("idempotency key"),
                    capability_kind: side_effect_capability_kind(),
                    capability_version: side_effect_capability_version(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                },
            ),
            events::KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role,
                claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                    .expect("fencing token"),
            }),
            events::KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role,
                invocation_epoch: 1,
                resource_key: resource_key.clone(),
                requirement_digest: resource_lane_requirement_digest,
                resolved_by_capability_impl: events::RunnerFactoryId::new(
                    "mfm.test.side_effect_driver",
                )
                .expect("runner factory"),
            }),
            events::KernelEventPayload::SideEffectInvocationPrepared(
                events::side_effect::InvocationPrepared {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose,
                    pair_id: pair_id.clone(),
                    pair_role,
                    invocation_epoch: 1,
                    claim_generation: 1,
                    claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                        .expect("fencing token"),
                    resource_key: Some(resource_key),
                    prepared_schema_id: evidence_schema,
                    prepared_artifact_id: prepared_artifact.artifact_id.clone(),
                    prepared_hash,
                    prepared_artifact_evidence_hash: prepared_artifact
                        .evidence_hash()
                        .expect("prepared evidence hash"),
                },
            ),
        ],
        vec![
            (intent_bytes, intent_artifact),
            (prepared_bytes, prepared_artifact),
        ],
        store::RequiredSideEffectState::Absent,
        true,
    );
    (attempt_id, ledger)
}
