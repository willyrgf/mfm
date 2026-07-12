use super::*;

#[tokio::test]
async fn recovery_allows_managed_write_artifact_restage_before_terminal_commit() {
    let fixture = fixture_with_first_managed_write_state();
    let output_artifact = artifact(0xa1);
    let output_digest = content(0xa2);
    let node = node_by_output(&fixture, &fixture.cell_a);
    let expected_caps = node
        .capability_bindings
        .capabilities
        .iter()
        .map(|capability| (capability.kind.clone(), capability.version.clone()))
        .collect();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "managed-write",
        RecordingRunner {
            expected_caps,
            output_artifact: output_artifact.clone(),
            output_digest: output_digest.clone(),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());

    drive_ok!(scheduler, store, fixture, "resume managed write");
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}
