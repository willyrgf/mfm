use super::*;

#[tokio::test]
async fn recovery_rejects_split_terminal_cell_and_attempt_completion() {
    let fixture = fixture();
    let (scheduler, store) = started_fixture_run(&fixture).await;
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("produce first cell");
    assert_eq!(result.status(), SchedulerStatus::Advanced);

    let valid_records = store.committed_records_for_corruption(&fixture.run_id);
    let corrupt_records = rewrite_records_without_payloads(&valid_records, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.output_cell_id == fixture.cell_a
        )
    });
    assert!(matches!(
        validate_corrupted_journal_for_tests(
            &store,
            &fixture.runtime_spec,
            &fixture.run_id,
            corrupt_records,
        ),
        Err(RuntimeError::Store(message))
            if message.contains("terminal cell requires matching attempt completion")
    ));
}

#[tokio::test]
async fn recovery_rejects_attempt_started_before_inputs_were_terminal() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let node_a = node_by_output(&fixture, &fixture.cell_a);
    let node_b = node_by_output(&fixture, &fixture.cell_b);
    append_attempt_start(&mut store, &fixture, node_b, 1);
    let attempt_a = append_attempt_start(&mut store, &fixture, node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        node_a,
        &attempt_a,
        artifact(0xa1),
        content(0xa2),
    );

    assert!(matches!(
        load_fixture_current(&scheduler, &store, &fixture).await,
        Err(RuntimeError::Store(message))
            if message.contains("started before input cell")
                && message.contains("was terminal")
    ));
}
