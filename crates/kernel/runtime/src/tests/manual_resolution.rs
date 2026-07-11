use super::*;

#[tokio::test]
async fn runtime_resolves_manual_resolution_terminal() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let mut store = TestTypedRunStore::new();
    let artifact_store = Arc::new(store.clone());
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry.clone(), &fixture),
        artifact_store.clone(),
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_drive!(scheduler, store, fixture, Advanced, "advance to ambiguity");
    }
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::ManualBlocked);
    assert_eq!(
        saga.manual_block_reason,
        Some(store::ManualBlockReason::PolicyManualResolution)
    );

    append_manual_resolution(
        &scheduler,
        &mut store,
        &fixture,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::ManuallyResolved);

    let fresh_scheduler = SerialTypedScheduler::new(
        register_fixture_capabilities(registry, &fixture),
        artifact_store,
    );
    assert_drive!(
        fresh_scheduler,
        store,
        fixture,
        Advanced,
        "resolve manual terminal"
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::ManuallyResolved
    ));
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_prefix_with_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    for _ in 0..2 {
        assert_drive!(
            scheduler,
            store,
            fixture,
            Advanced,
            "advance to manual block"
        );
    }
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::ManualBlocked);

    let node_b = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("node b")
        .clone();
    append_attempt_start(&mut store, &fixture, &node_b, 2);

    let spec::SagaPolicySpec::ManualResolution { manual } = &fixture.runtime_spec.spec().saga
    else {
        panic!("manual resolution fixture policy");
    };
    let error = build_manual_resolution_prefix_authority_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store,
        manual.clone(),
    )
    .expect_err("manual prefix rejects open attempt");
    assert!(
        matches!(&error, RuntimeError::InvalidRunStream(message) if message.contains("requires no open semantic attempts")),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_missing_manual_terminal_authorization_artifact_leaves_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let mut store = TestTypedRunStore::new();
    let artifact_store = Arc::new(FilteringRuntimeArtifactStore::new(store.clone()));
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry, &fixture),
        artifact_store.clone(),
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_drive!(scheduler, store, fixture, Advanced, "advance to ambiguity");
    }
    append_manual_resolution(
        &scheduler,
        &mut store,
        &fixture,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let manual = store
        .load_run_stream(&fixture.run_id)
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("manual resolution recorded");
    artifact_store.hide_artifact(manual.authorization_artifact_id.clone());
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let resolve_node = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .expect("resolve saga terminal node");
    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture)
            .await,
        Err(RuntimeError::Store(message)) if message.contains("missing artifact")
    ));
    store.assert_run_stream_len(&fixture.run_id, stream_len_before + 1);
    let resolve_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &resolve_node.node_id,
        1,
    )
    .expect("resolve attempt id");
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&resolve_node.node_id, &resolve_attempt)
            .expect("open resolve attempt")
            .status,
        store::AttemptStatus::Started { .. }
    ));
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_before_manual_blocked() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let evidence_bytes = br#"{"operator_note":"too_early"}"#.to_vec();

    let error = record_manual_resolution(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
        ManualResolutionRequest {
            outcome: events::ManualResolutionOutcome::ConfirmRemediated,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes: br#"{}"#.to_vec(),
            note: None,
        },
    )
    .await
    .expect_err("manual resolution before block rejects");

    assert!(
        matches!(error, RuntimeError::InvalidRunStream(_)),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_rejects_forward_node_emitting_remediation_ledger_purpose() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            ForwardEmitsRemediationPurposeRunner,
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, &fixture);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize wrong forward ledger purpose"
    );
}

#[tokio::test]
async fn runtime_rejects_remediation_node_emitting_forward_ledger_purpose() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "pure",
            BlockingRunner,
        ))
        .expect("binding failure node");
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output, forward_b_output],
        "drive forward side-effect phase",
    )
    .await;
    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize wrong remediation ledger purpose"
    );
    assert_failure_code_count(&store, "runner_output_invalid", 1);
}
