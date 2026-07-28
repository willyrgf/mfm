use super::*;

#[tokio::test]
async fn store_rejects_fact_without_started_attempt() {
    let fixture = fixture();
    let (_, mut store) = started_fixture_run(&fixture).await;
    let node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_b)
        .expect("read node")
        .clone();
    let (fact_evidence, _fact_bytes) = test_fact_response_artifact(&node, 210);
    assert!(store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-fact").expect("commit key"),
            payloads: vec![events::KernelEventPayload::FactRecorded(
                events::FactRecorded {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: AttemptId::from_digest(DigestBytes::from_array([0xd3; 32])),
                    claim: test_fact_claim(210, &fact_evidence),
                },
            )],
            required_artifacts: vec![fact_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .is_err());
}

#[tokio::test]
async fn replay_rejects_public_output_without_render_attempt() {
    let fixture = fixture();
    let (scheduler, store) = started_fixture_run(&fixture).await;
    let non_render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_a)
        .expect("non-render node")
        .clone();
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load admitted run");
    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("produce public output");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let current = result.into_current_run();
    let mut non_render_attempt_id = None;
    let _ = current.lifecycle().visit_attempts::<()>(|attempt| {
        if attempt.node_id() == &non_render_node.node_id {
            non_render_attempt_id = Some(attempt.attempt_id().clone());
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    });
    let non_render_attempt_id = non_render_attempt_id.expect("non-render attempt");
    let mut records = store.committed_records_for_corruption(&fixture.run_id);
    let public_output_index = records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        })
        .expect("completed run contains public output");
    let mut public_output = match records[public_output_index].payload().clone() {
        events::KernelEventPayload::PublicOutputProduced(payload) => payload,
        _ => unreachable!("located public output record"),
    };
    public_output.node_id = non_render_node.node_id;
    public_output.attempt_id = non_render_attempt_id;
    records[public_output_index] = rewrite_record_payload(
        &records[public_output_index],
        events::KernelEventPayload::PublicOutputProduced(public_output),
    );

    assert!(matches!(
        validate_corrupted_journal_for_tests(
            &store,
            &fixture.runtime_spec,
            &fixture.run_id,
            records,
        ),
        Err(RuntimeError::Store(message))
            if message.contains("public output")
    ));
}

#[tokio::test]
async fn replay_rejects_public_output_with_forged_rendered_digest() {
    let fixture = fixture();
    let (scheduler, store) = started_fixture_run(&fixture).await;
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load admitted run");
    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("produce public output");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);

    let mut records = store.committed_records_for_corruption(&fixture.run_id);
    let public_output_index = records
        .iter()
        .position(|record| {
            matches!(
                record.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        })
        .expect("completed run contains public output");
    let mut public_output = match records[public_output_index].payload().clone() {
        events::KernelEventPayload::PublicOutputProduced(payload) => payload,
        _ => unreachable!("located public output record"),
    };
    public_output.rendered_digest = content(0xf1);
    records[public_output_index] = rewrite_record_payload(
        &records[public_output_index],
        events::KernelEventPayload::PublicOutputProduced(public_output),
    );

    assert!(matches!(
        validate_corrupted_journal_for_tests(
            &store,
            &fixture.runtime_spec,
            &fixture.run_id,
            records,
        ),
        Err(RuntimeError::Store(message))
            if message.contains("incorrect rendered digest")
    ));
}
