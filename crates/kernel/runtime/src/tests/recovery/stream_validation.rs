use super::*;

#[tokio::test]
async fn recovery_rejects_split_terminal_cell_and_attempt_completion() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.output_cell_id == fixture.cell_a
        )
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
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
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}
