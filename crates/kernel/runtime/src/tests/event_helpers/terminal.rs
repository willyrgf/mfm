use super::*;

pub(in crate::tests::support) fn append_terminal(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    _artifact_id: ArtifactId,
    output_digest: ContentDigest,
) {
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let artifact_id = ArtifactId::from_digest(output_digest.algorithm(), *output_digest.digest());
    let evidence =
        state_output_artifact(node, descriptor, artifact_id.clone(), output_digest.clone());
    let output_bytes = synthetic_artifact_bytes_for_digest(&output_digest);
    let evidence_hash = evidence
        .evidence_hash()
        .expect("manual terminal state output evidence hash");
    store
        .append_prepared_commit_with_artifacts(
            store_typed_commit_request! {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-terminal:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::CellProduced(events::CellProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        cell_id: node.output_cell.clone(),
                        scope_id: node.scope_id.clone(),
                        attempt_id: attempt_id.clone(),
                        semantic_type_id: descriptor.output_semantic_type_id.clone(),
                        schema_id: descriptor.output_schema_id.clone(),
                        value_lineage: output_cell.value_lineage.clone(),
                        context: output_cell.context.clone(),
                        artifact_id,
                        content_digest: output_digest,
                        evidence_hash,
                        producer_state_kind: Some(node.state_kind.clone()),
                        producer_state_version: Some(node.state_version.clone()),
                    }),
                    events::KernelEventPayload::StateAttemptCompleted(
                        events::StateAttemptCompleted {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: node.node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            output_cell_id: node.output_cell.clone(),
                        },
                    ),
                ],
                required_artifacts: vec![evidence.clone()],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    ..store::CommitPreconditions::default()
                },
            },
            vec![(output_bytes, evidence)],
        )
        .expect("append terminal");
}

pub(in crate::tests::support) fn append_public_output_render_failure(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        panic!("expected public-output render node");
    };
    let error = public_output_error();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-public-output-failure:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::PublicOutputRenderFailed(
                    events::PublicOutputRenderFailed {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        public_schema_id: render.public_schema_id.clone(),
                        renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
                        error: error.clone(),
                    },
                ),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable: true,
                    error,
                }),
            ],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append public output failure");
}

pub(in crate::tests::support) fn append_not_submitted_proven(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) {
    let current = verified_current_for_store(store, fixture);
    let side_effect = side_effect_for_attempt(
        &current.runtime_spec(),
        &current.lifecycle(),
        node,
        attempt_id,
    )
    .expect("side-effect lookup")
    .expect("current side effect");
    let ledger_key = side_effect.ledger_key().clone();
    let ledger_purpose = side_effect.ledger_purpose().clone();
    let pair_id = side_effect.pair_id().clone();
    let pair_role = events::SideEffectPairRole::Submit;
    let proof = fixture_side_effect_evidence(57, node.node_id.as_str(), attempt_id.as_str());
    let proof_bytes = canonical_fixture_side_effect_bytes(&proof);
    let proof_hash = digest_for_bytes(&proof_bytes);
    let proof_artifact = ArtifactId::from_digest(proof_hash.algorithm(), *proof_hash.digest());
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: proof_artifact.clone(),
        digest: proof_hash.clone(),
        byte_len: proof_bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::NotSubmittedProof,
    };
    store
        .append_prepared_commit_with_artifacts(
            store_typed_commit_request! {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-not-submitted:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![events::KernelEventPayload::SideEffectNotSubmittedProven(
                    events::side_effect::NotSubmittedProven {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        ledger_key,
                        ledger_purpose,
                        pair_id,
                        pair_role,
                        invocation_epoch,
                        proof_schema_id: node.config_ref.schema_id.clone(),
                        proof_hash,
                        proof_artifact_id: proof_artifact,
                        proof_artifact_evidence_hash: evidence
                            .evidence_hash()
                            .expect("proof evidence hash"),
                    },
                )],
                required_artifacts: vec![evidence.clone()],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                        fixture.run_id.clone(),
                        fixture.runtime_spec.spec(),
                    )
                    .expect("certified run authority")),
                    ..store::CommitPreconditions::default()
                },
            },
            vec![(proof_bytes, evidence)],
        )
        .expect("append not-submitted proof");
}
