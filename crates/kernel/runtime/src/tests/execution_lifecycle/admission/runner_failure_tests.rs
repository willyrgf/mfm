use super::*;

#[tokio::test]
async fn runner_invocation_uses_run_admitted_config_evidence_without_reference_event() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    let mut referenced_config = false;
    let _ = current.lifecycle().visit_records(|record| {
        referenced_config |= matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
        );
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert!(!referenced_config);
    drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("drive with RunAdmitted config evidence");
}

#[tokio::test]
async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("produce first cell");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    drop(result);

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let valid_records = store.committed_records_for_corruption(&fixture.run_id);
    let corrupt_records = rewrite_records_without_payloads(&valid_records, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id)
        )
    });
    let backend = StaticRunJournalBackendForTest::new(
        fixture.run_id.clone(),
        corrupt_records,
        store.committed_artifact_authority_for_corruption(&fixture.run_id),
    );
    let journal = poll_ready_store_future_for_test(backend.load_committed_journal(&fixture.run_id))
        .expect("structurally valid journal without the producer artifact reference");
    let corrupt_current =
        verify_current_run(journal, recertified_runtime_spec(&fixture.runtime_spec))
            .expect("journal verification permits the missing explicit reference");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &consumer_node_id,
        1,
    )
    .expect("consumer attempt id");
    let error = {
        let runtime_spec = corrupt_current.runtime_spec();
        let node = runtime_spec
            .node(&consumer_node_id)
            .expect("certified consumer node");
        let descriptor = runtime_spec
            .state_descriptor_for_node(node)
            .expect("consumer descriptor");
        let output_cell = runtime_spec
            .cell(&node.output_cell)
            .expect("consumer output cell");
        crate::invocation::InvocationBuilder::new(crate::invocation::InvocationBuilderInput {
            runtime_spec,
            run_id: corrupt_current.view().run_id(),
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no: 1,
            lifecycle: corrupt_current.lifecycle(),
        })
        .build()
        .err()
        .expect("input materialization must require the explicit artifact reference")
    };
    assert!(matches!(
        error,
        RuntimeError::InputMaterialization(message)
            if message.contains("is not committed in the run journal")
    ));
    assert_eq!(
        attempt_started_count(&corrupt_current, &consumer_node_id),
        0,
        "corrupt persisted history must reject before consumer invocation"
    );
}

#[tokio::test]
async fn missing_committed_input_object_rejects_before_attempt_start() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("produce first cell");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    let current = result.into_current_run();
    let consumer_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let produced = current
        .lifecycle()
        .cell(&fixture.cell_a)
        .expect("producer terminal")
        .produced()
        .expect("produced input");
    store
        .remove_committed_artifact_for_corruption(produced.artifact_id(), produced.evidence_hash());
    load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect_err("missing committed input bytes must fail closed");
    assert_eq!(attempt_started_count(&current, &consumer_node.node_id), 0);
    assert!(current
        .lifecycle()
        .cell(&consumer_node.output_cell)
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
        let scheduler = fixture_scheduler(registry, &fixture);
        let mut store = TestTypedRunStore::new();
        let current = started_fixture_current(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start runner-error fixture");

        match case {
            Case::RuntimeValidation => {
                let result = drive_current_once_with_claim(&scheduler, &store, current)
                    .await
                    .expect("terminalize runtime validation failure");
                assert_eq!(result.status(), SchedulerStatus::Advanced);
                let current = result.into_current_run();
                let node = node_by_output(&fixture, &fixture.cell_a);
                assert_node_failed_with_code(&current, &node.node_id, "runtime_validation_failed");
                assert_eq!(
                    runtime_lifecycle_summary(&current),
                    "run=Started attempts[started=0 completed=0 failed=1 interrupted=0 total=1] cells=0 side_effects=0 lanes=0 public_outputs=0 retentions=1"
                );
            }
            Case::InvalidRunStream => {
                assert!(matches!(
                    drive_current_once_with_claim(&scheduler, &store, current).await,
                    Err(RuntimeError::InvalidRunStream(message))
                        if message.contains("synthetic corrupt stream authority")
                ));

                let current = load_fixture_current(&scheduler, &store, &fixture)
                    .await
                    .expect("reload started attempt");
                let node = node_by_output(&fixture, &fixture.cell_a);
                let mut started_attempts = 0;
                let _ = current.lifecycle().visit_attempts(|attempt| {
                    if attempt.node_id() == &node.node_id
                        && matches!(
                            attempt.status(),
                            store::current_lifecycle::CurrentAttemptStatusRef::Started { .. }
                        )
                    {
                        started_attempts += 1;
                    }
                    std::ops::ControlFlow::<()>::Continue(())
                });
                assert_eq!(started_attempts, 1);
                assert_failure_code_count(&current, "runtime_validation_failed", 0);
                assert_failure_code_count(&current, "runner_output_invalid", 0);
                assert_eq!(
                    runtime_lifecycle_summary(&current),
                    "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=0 lanes=0 public_outputs=0 retentions=1"
                );
            }
        }
    }
}

#[tokio::test]
async fn journal_load_rejects_terminal_cell_producer_outside_certified_spec() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");

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
    let artifact_bytes = br#"{"forged":true}"#.to_vec();
    let artifact_digest = digest_for_bytes(&artifact_bytes);
    let artifact_id =
        ArtifactId::from_digest(artifact_digest.algorithm(), *artifact_digest.digest());
    let forged_artifact = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: artifact_digest.clone(),
        byte_len: artifact_bytes.len() as u64,
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
    let forged_terminal = store_typed_commit_request! {
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
        required_artifacts: vec![forged_artifact.clone()],
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            ..store::CommitPreconditions::default()
        },
    };
    store
        .append_prepared_commit_with_artifacts(
            forged_terminal,
            vec![(artifact_bytes, forged_artifact)],
        )
        .expect("append forged terminal");

    drop(current);
    let error = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect_err("terminal cell producer outside certified spec must fail closed");
    assert!(matches!(
        error,
        RuntimeError::Store(message)
            if message.contains("produced cell")
                && message.contains("does not match certified metadata")
    ));
}
