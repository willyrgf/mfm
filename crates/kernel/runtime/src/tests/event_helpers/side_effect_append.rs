use super::*;

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
    let intent_hash = content_digest_json(serde_json::json!({
        "key": key,
        "ledger": ledger.as_str(),
        "run": run_id.as_str(),
    }))
    .expect("intent digest");
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
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::SideEffectIntent,
    };
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, &attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append(
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
                    intent_schema_id: node.config_ref.schema_id.clone(),
                    intent_hash: intent_hash.clone(),
                    intent_artifact_id,
                    intent_artifact_evidence_hash: intent_artifact
                        .evidence_hash()
                        .expect("intent evidence hash"),
                    idempotency_input_schema_id: node.config_ref.schema_id.clone(),
                    idempotency_input_hash: content(0xc3),
                    idempotency_key: events::IdempotencyKeyRef::new(format!("idem-{commit_key}"))
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
                    prepared_artifact_id: None,
                    prepared_hash: None,
                    prepared_artifact_evidence_hash: None,
                },
            ),
        ],
        vec![intent_artifact],
        store::RequiredSideEffectState::Absent,
        true,
    );
    (attempt_id, ledger)
}
