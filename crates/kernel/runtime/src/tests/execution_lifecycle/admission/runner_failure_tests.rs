use super::*;

#[tokio::test]
async fn runner_invocation_uses_run_admitted_config_evidence_without_reference_event() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let valid_stream = store.load_run_stream(&fixture.run_id);
    assert!(valid_stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
        )
    }));
    drive_ok!(
        scheduler,
        store,
        fixture,
        "drive with RunAdmitted config evidence"
    );
}

#[tokio::test]
async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id)
        )
    });
    {
        let corrupt_store = StaleStreamStore::new(&mut store, corrupt_stream);
        assert!(matches!(
            drive_once_with_claim(&scheduler, &corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InputMaterialization(message))
                if message.contains("is not committed in the run stream")
        ));
    }
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
        1
    );
}

#[tokio::test]
async fn post_start_materialization_failure_terminalizes_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node = node_by_output(&fixture, &fixture.cell_b).clone();
    {
        let corrupt_store = MissingInputArtifactRefStore::new(&mut store, producer_node_id);
        assert_eq!(
            drive_once_with_claim(
                &scheduler,
                &corrupt_store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("terminalize materialization failure"),
            SchedulerStatus::Advanced
        );
    }

    assert_node_failed_with_code_and_retryable(
        &store,
        &consumer_node.node_id,
        "input_materialization_failed",
        true,
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&consumer_node.output_cell)
        .is_none());
}

#[tokio::test]
async fn post_start_runner_errors_follow_terminal_policy_by_error_class() {
    #[derive(Clone, Copy)]
    enum Case {
        RuntimeValidation,
        InvalidRunStream,
    }

    for case in [Case::RuntimeValidation, Case::InvalidRunStream] {
        let fixture = fixture();
        let error = match case {
            Case::RuntimeValidation => RuntimeError::RuntimeValidation(
                "synthetic post-start validation failure".to_owned(),
            ),
            Case::InvalidRunStream => {
                RuntimeError::InvalidRunStream("synthetic corrupt stream authority".to_owned())
            }
        };
        let registry = fixture_registry_with_first_runner(&fixture, "pure", ErrorRunner { error });
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

        match case {
            Case::RuntimeValidation => {
                assert_drive!(
                    scheduler,
                    store,
                    fixture,
                    Advanced,
                    "terminalize runtime validation failure"
                );

                let node = node_by_output(&fixture, &fixture.cell_a);
                assert_node_failed_with_code(&store, &node.node_id, "runtime_validation_failed");
                assert_eq!(
                    runtime_lifecycle_summary(&store, &fixture.run_id),
                    "run=Started attempts[started=0 completed=0 failed=1 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
                );
            }
            Case::InvalidRunStream => {
                assert!(matches!(
                    drive_fixture_once(&scheduler, &mut store, &fixture)
                        .await,
                    Err(RuntimeError::InvalidRunStream(message))
                        if message.contains("synthetic corrupt stream authority")
                ));

                let node = node_by_output(&fixture, &fixture.cell_a);
                let projection_snapshot = store.projection_snapshot();
                let attempts = projection_snapshot
                    .attempts()
                    .filter(|((node_id, _), _)| node_id == &node.node_id)
                    .map(|(_, attempt)| attempt)
                    .collect::<Vec<_>>();
                assert_eq!(attempts.len(), 1);
                assert!(matches!(
                    attempts[0].status,
                    store::AttemptStatus::Started { .. }
                ));
                assert_failure_code_count(&store, "runtime_validation_failed", 0);
                assert_failure_code_count(&store, "runner_output_invalid", 0);
                assert_eq!(
                    runtime_lifecycle_summary(&store, &fixture.run_id),
                    "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
                );
            }
        }
    }
}

#[tokio::test]
async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let forged_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell != fixture.cell_a)
        .expect("second node")
        .clone();
    let certified_cell = fixture
        .runtime_spec
        .cell(&fixture.cell_a)
        .expect("cell a")
        .clone();
    let forged_attempt = AttemptId::from_digest(DigestBytes::from_array([0xfa; 32]));
    let artifact_id = artifact(0xfa);
    let artifact_digest = content(0xfb);
    let forged_artifact = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: artifact_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(certified_cell.schema_id.clone()),
        semantic_type_id: Some(certified_cell.semantic_type_id.clone()),
        producer_node_id: Some(forged_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-attempt-start").expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    attempt_no: 1,
                    state_kind: forged_node.state_kind.clone(),
                    state_version: forged_node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged attempt start");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-terminal").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    cell_id: fixture.cell_a.clone(),
                    scope_id: certified_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: certified_cell.semantic_type_id.clone(),
                    schema_id: certified_cell.schema_id.clone(),
                    value_lineage: certified_cell.value_lineage.clone(),
                    context: certified_cell.context.clone(),
                    artifact_id,
                    content_digest: artifact_digest,
                    evidence_hash: forged_artifact
                        .evidence_hash()
                        .expect("forged terminal evidence hash"),
                    producer_state_kind: Some(forged_node.state_kind.clone()),
                    producer_state_version: Some(forged_node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt,
                    output_cell_id: fixture.cell_a.clone(),
                }),
            ],
            required_artifacts: vec![forged_artifact],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged terminal");

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}
