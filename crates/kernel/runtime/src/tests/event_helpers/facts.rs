use super::*;

pub(in crate::tests::support) fn append_fact_settlement(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    subject_amount: u64,
    response_amount: u64,
) {
    let fact_key = test_fact_key(subject_amount);
    let (evidence, response_bytes) = test_fact_response_artifact(node, response_amount);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let output_bytes = canonical_json(serde_json::json!({ "amount": response_amount }))
        .expect("state output json")
        .to_vec();
    let output_evidence = state_output_artifact_for_bytes(node, descriptor, &output_bytes);
    let output_evidence_hash = output_evidence
        .evidence_hash()
        .expect("state output evidence hash");
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new(format!(
            "manual-fact:{}:{}:{}:{}",
            node.node_id, attempt_id, fact_key, response_amount
        ))
        .expect("commit key"),
        payloads: vec![
            events::KernelEventPayload::FactRecorded(events::FactRecorded {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                claim: test_fact_claim(subject_amount, &evidence),
            }),
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
                artifact_id: output_evidence.artifact_id.clone(),
                content_digest: output_evidence.digest.clone(),
                evidence_hash: output_evidence_hash,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            }),
        ],
        required_artifacts: vec![evidence.clone(), output_evidence.clone()],
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
            certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                fixture.run_id.clone(),
                fixture.runtime_spec.spec(),
            )
            .expect("certified run authority")),
            ..store::CommitPreconditions::default()
        },
    };
    let plan = test_prepared_commit_plan(request, vec![evidence.clone(), output_evidence.clone()])
        .expect("fact settlement commit plan");
    let bundle = store::PreparedCommitBundle::new(
        plan,
        vec![
            store::PreparedArtifactBytes::new(response_bytes, evidence)
                .expect("fact response bytes"),
            store::PreparedArtifactBytes::new(output_bytes, output_evidence)
                .expect("state output bytes"),
        ],
        Vec::new(),
    )
    .expect("fact settlement commit bundle");
    block_on_ready(store.append_prepared_commit_bundle(bundle)).expect("append fact settlement");
}
