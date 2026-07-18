use super::*;

#[tokio::test]
async fn side_effect_driver_persists_intent_before_preparation() {
    let fixture = fixture_with_first_side_effect_state();
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output = drive_side_effect_driver_empty(&fixture, &fixture.cell_a, &callbacks)
        .await
        .expect("driver output");

    assert_eq!(output.staged_artifacts().len(), 1);
    assert_eq!(output.payloads().len(), 2);
    assert!(matches!(
        output.payloads()[0],
        RunnerEventPayload::SideEffectIntentPersisted(_)
    ));
    assert!(matches!(
        output.payloads()[1],
        RunnerEventPayload::SideEffectClaimed(_)
    ));
    match &output.payloads()[0] {
        RunnerEventPayload::SideEffectIntentPersisted(payload) => {
            assert_eq!(
                payload.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            );
            assert_eq!(payload.invocation_epoch, 1);
        }
        other => panic!("expected intent payload: {other:?}"),
    }
}

#[tokio::test]
async fn side_effect_driver_preserves_concrete_exclusive_resource_key_across_runs() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let mut peer = fixture.clone();
    refresh_fixture_run_id_with_identity(
        &mut peer,
        alternate_fixture_store_scope_id(),
        content(0xf6),
    );
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();

    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start first run");
    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "first run claims resource lane"
    );
    let first_stream = store.load_run_stream(&fixture.run_id);
    let claim_seq = first_stream
        .iter()
        .find_map(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ResourceLaneClaimed(_)
            )
            .then_some(event.seq())
        })
        .expect("first run recorded resource lane claim");
    let prepared_seq = first_stream.iter().find_map(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectInvocationPrepared(_)
        )
        .then_some(event.seq())
    });
    assert!(
        prepared_seq.is_none_or(|prepared_seq| claim_seq < prepared_seq),
        "exclusive invocation prepare must be committed after ResourceLaneClaimed"
    );
    let resource_key = first_stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ResourceLaneClaimed(payload) => {
                Some(payload.resource_key.clone())
            }
            _ => None,
        })
        .expect("first run recorded resource key");
    assert_eq!(resource_key.key.as_str(), "mfm.test.driver.shared-resource");
    let lane_key = store::ResourceLaneKey::from_evidence(&resource_key);
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());

    start_fixture_run(&scheduler, &mut store, &peer, vec![peer.seed_ref.clone()])
        .await
        .expect("start peer run");
    assert_eq!(
        drive_once(&scheduler, &mut store, &peer.runtime_spec, &peer.run_id)
            .await
            .expect("peer run starts attempt before observing lane block"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        drive_once(&scheduler, &mut store, &peer.runtime_spec, &peer.run_id)
            .await
            .expect("peer run blocks on same resource lane"),
        SchedulerStatus::Blocked
    );
    assert!(
        store
            .load_run_stream(&peer.run_id)
            .iter()
            .all(|event| !matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectInvocationPrepared(_)
            )),
        "peer run must not prepare while the cross-run resource lane is held"
    );
}

#[tokio::test]
async fn exclusive_side_effect_prepare_failure_after_claim_terminalizes_attempt() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        FailingAfterPreclaimRunner::new(&fixture),
    ));
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "exclusive run terminalizes failed resource-lane claim"
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    let stream = store.load_run_stream(&fixture.run_id);
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ResourceLaneClaimed(_)
        )),
        "failed exclusive attempt records the resource lane claim"
    );
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ResourceLaneReleased(_)
        )),
        "failed exclusive attempt releases the resource lane"
    );
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectFailed(_)
        )),
        "failed exclusive attempt records terminal side-effect evidence"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
    assert!(
        store
            .projection_snapshot()
            .resource_lanes()
            .next()
            .is_none(),
        "terminal attempt failure releases the exclusive resource lane"
    );
}

#[tokio::test]
async fn runner_block_leaves_started_attempt_open_without_failure() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let mut registry = fixture_registry_with_first_runner(&fixture, "pure", BlockingRunner);
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    let scheduler = test_scheduler(registry);
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    assert_drive!(
        scheduler,
        store,
        fixture,
        Blocked,
        "blocked runner leaves attempt open"
    );
    let stream = store.load_run_stream(&fixture.run_id);
    assert!(stream.iter().any(|event| matches!(
        event.payload(),
        events::KernelEventPayload::StateAttemptStarted(payload)
            if payload.node_id == node.node_id
    )));
    assert!(stream.iter().all(|event| !matches!(
        event.payload(),
        events::KernelEventPayload::StateAttemptFailed(payload)
            if payload.node_id == node.node_id
    )));
}

#[tokio::test]
async fn side_effect_driver_submits_from_started_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-submit",
        "sidefx-driver-submit",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert_eq!(output.payloads().len(), 1);
    match &output.payloads()[0] {
        RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
            assert_side_effect_binding!(payload, ledger_key, 1);
        }
        other => panic!("expected submission payload: {other:?}"),
    }
}

#[tokio::test]
async fn side_effect_driver_persists_submission_recovery_decisions() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, _ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-recovery",
        "sidefx-driver-recovery",
    );

    for (decision, expected) in [
        (
            TestSubmissionDecision::Unknown,
            "side_effect.submission_unknown",
        ),
        (
            TestSubmissionDecision::NotSubmitted,
            "side_effect.not_submitted_proven",
        ),
        (TestSubmissionDecision::Ambiguous, "side_effect.ambiguous"),
    ] {
        let callbacks =
            TestSideEffectDriverCallbacks::new(&fixture).with_submission_decision(decision);
        let output =
            drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
                .await
                .expect("driver output");
        let actual = output
            .payloads()
            .iter()
            .find_map(|payload| match payload {
                RunnerEventPayload::SideEffectSubmissionUnknown(_) => {
                    Some("side_effect.submission_unknown")
                }
                RunnerEventPayload::SideEffectNotSubmittedProven(_) => {
                    Some("side_effect.not_submitted_proven")
                }
                RunnerEventPayload::SideEffectAmbiguous(_) => Some("side_effect.ambiguous"),
                RunnerEventPayload::ResourceLaneReleaseIntent(_) => None,
                _ => None,
            })
            .unwrap_or_else(|| panic!("unexpected recovery payloads: {:?}", output.payloads()));
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn side_effect_submission_unknown_keeps_exclusive_resource_lane_held() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let pair_id = fixture_side_effect_pair_id(&fixture, node);
    let (attempt_id, _ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-unknown-lane",
        "sidefx-driver-unknown-lane",
    );
    let before_unknown = store.projection_snapshot();
    let (lane_key, held_lane) =
        active_resource_lane_for_pair(&before_unknown, &fixture.run_id, &pair_id)
            .expect("exclusive lane must be held before submission recovery");
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture)
        .with_submission_decision(TestSubmissionDecision::Unknown);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");
    assert!(output
        .payloads()
        .iter()
        .any(|payload| matches!(payload, RunnerEventPayload::SideEffectSubmissionUnknown(_))));
    assert!(output
        .payloads()
        .iter()
        .all(|payload| !matches!(payload, RunnerEventPayload::ResourceLaneReleaseIntent(_))));
    append_erased_runner_output(
        &mut store,
        &fixture.run_id,
        "sidefx-driver-unknown-lane-output",
        output,
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("attempt logical key")],
            required_side_effect_states: vec![store::SideEffectStatePrecondition {
                pair_id: pair_id.clone(),
                required: store::RequiredSideEffectState::InvocationStarted,
            }],
            certified_run_authority: Some(
                store::CertifiedRunStoreAuthority::from_spec(
                    fixture.run_id.clone(),
                    fixture.runtime_spec.spec(),
                )
                .expect("certified run authority"),
            ),
            ..store::CommitPreconditions::default()
        },
    );

    let after_unknown = store.projection_snapshot();
    let projection = after_unknown
        .side_effect_for_pair(&fixture.run_id, &pair_id)
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::SubmissionUnknown { .. }
    ));
    let lane_after_unknown = after_unknown
        .resource_lane(&lane_key)
        .expect("submission-unknown phase must keep the resource lane held");
    assert_eq!(lane_after_unknown.holder, held_lane.holder);
    assert_eq!(lane_after_unknown.claim_id, held_lane.claim_id);
}

#[tokio::test]
async fn side_effect_verify_driver_maps_receipt_to_state_output() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (submit_attempt_id, ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-output",
        "sidefx-driver-output",
    );
    append_synthetic_submission_observed(
        &mut store,
        &fixture,
        node,
        &submit_attempt_id,
        &ledger_key,
        "sidefx-driver-output-submission",
    );
    append_synthetic_receipt_observed(
        &mut store,
        &fixture,
        node,
        &submit_attempt_id,
        &ledger_key,
        "sidefx-driver-output-receipt",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);
    let verify_node = side_effect_verify_node_for_submit(&fixture, node);
    let verify_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &verify_node.node_id,
        1,
    )
    .expect("verify attempt id");

    let output = drive_side_effect_verify_driver_from_store(
        &fixture,
        &store,
        verify_node,
        &verify_attempt_id,
        &callbacks,
    )
    .await
    .expect("driver output");

    assert_eq!(output.staged_artifacts().len(), 1);
    assert_eq!(output.payloads().len(), 1);
    assert!(matches!(
        output.payloads()[0],
        RunnerEventPayload::CellProduced(_)
    ));
}

#[tokio::test]
async fn side_effect_receipt_verification_releases_exclusive_resource_lane() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;
    let submit_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let verify_node = side_effect_verify_node_for_submit(&fixture, &submit_node).clone();
    let pair_id = fixture_side_effect_pair_id(&fixture, &submit_node);
    let mut saw_lane_claimed = false;
    let mut released_with_receipt_before_output = false;
    let mut terminal_output_after_release = false;

    for _ in 0..10 {
        assert_ne!(
            drive_fixture_once(&scheduler, &mut store, &fixture)
                .await
                .expect("advance receipt verification"),
            SchedulerStatus::Blocked
        );
        let snapshot = store.projection_snapshot();
        let active_lane = active_resource_lane_for_pair(&snapshot, &fixture.run_id, &pair_id);
        saw_lane_claimed |= active_lane.is_some();
        let side_effect = snapshot
            .side_effect_for_pair(&fixture.run_id, &pair_id)
            .expect("side-effect projection");
        if matches!(
            side_effect.phase,
            store::SideEffectPhase::ReceiptObserved { .. }
        ) && active_lane.is_none()
            && snapshot.cell_terminal(&verify_node.output_cell).is_none()
        {
            released_with_receipt_before_output = true;
        }
        if released_with_receipt_before_output
            && active_lane.is_none()
            && snapshot.cell_terminal(&verify_node.output_cell).is_some()
        {
            terminal_output_after_release = true;
            break;
        }
    }

    assert!(saw_lane_claimed, "exclusive side effect must claim a lane");
    assert!(
        released_with_receipt_before_output,
        "receipt evidence should release the lane before terminal output"
    );
    assert!(
        terminal_output_after_release,
        "receipt-level terminal output should be produced after lane release"
    );
}

#[tokio::test]
async fn side_effect_driver_rejects_ambiguous_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-ambiguous",
        "sidefx-driver-ambiguous",
    );
    append_synthetic_ambiguous(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-ambiguous-terminal",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    assert!(matches!(
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks).await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("unsupported side-effect driver phase: ambiguous")
    ));
}

#[tokio::test]
async fn side_effect_driver_starts_prepared_projection_before_submission() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-unsupported",
        "sidefx-driver-unsupported",
    );
    let pair_id = fixture_side_effect_pair_id(&fixture, node);
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert_eq!(output.staged_artifacts().len(), 0);
    assert_eq!(output.payloads().len(), 1);
    match &output.payloads()[0] {
        RunnerEventPayload::SideEffectInvocationStarted(payload) => {
            assert_side_effect_binding!(payload, ledger_key, 1);
        }
        other => panic!("expected invocation started payload: {other:?}"),
    }
    let required_artifacts = output
        .staged_artifacts()
        .iter()
        .map(|artifact| artifact.evidence().clone())
        .collect::<Vec<_>>();
    let payloads = output
        .payloads()
        .iter()
        .cloned()
        .map(events::KernelEventPayload::from)
        .collect::<Vec<_>>();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("sidefx-driver-prepared-resume")
                .expect("commit key"),
            payloads: payloads,
            required_artifacts: required_artifacts,
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    pair_id: pair_id.clone(),
                    required: store::RequiredSideEffectState::InvocationPrepared,
                }],
                certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                    fixture.run_id.clone(),
                    fixture.runtime_spec.spec(),
                )
                .expect("certified run authority")),
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append prepared recovery output");

    let projection_snapshot = store.projection_snapshot();
    let projection = projection_snapshot
        .side_effect_state_for_pair(&fixture.run_id, &pair_id)
        .expect("side-effect state")
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase(),
        store::SideEffectLedgerPhase::Started { .. }
    ));
}
