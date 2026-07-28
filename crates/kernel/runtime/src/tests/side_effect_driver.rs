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
async fn appended_preparation_settles_side_effect_authority_once() {
    let fixture = fixture_with_first_side_effect_state();
    let settled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        DriverSideEffectRunner::new(&fixture).with_preparation_settlement(Arc::clone(&settled)),
    ));
    let store = RecordingTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start appended side-effect settlement run");

    assert!(
        drive_until_side_effect_preparation(&scheduler, &store, &fixture).await,
        "the side-effect driver must carry settlement through preparation"
    );
    assert_eq!(
        settled.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a durably appended preparation must settle its process-local authority exactly once"
    );
}

#[tokio::test]
async fn uncertain_preparation_append_discards_side_effect_authority() {
    let fixture = fixture_with_first_side_effect_state();
    let settled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        DriverSideEffectRunner::new(&fixture).with_preparation_settlement(Arc::clone(&settled)),
    ));
    let store = StaleOnceTypedRunStore::for_side_effect_preparation();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start side-effect settlement run");

    let preparation_committed =
        drive_until_side_effect_preparation(&scheduler, &store, &fixture).await;

    assert!(
        preparation_committed,
        "the injected append error occurs only after preparation was committed"
    );
    assert_eq!(
        settled.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the side-effect settlement token must be discarded when its append outcome is uncertain"
    );
}

async fn drive_until_side_effect_preparation<S>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    fixture: &Fixture,
) -> bool
where
    S: store::RunJournalStore + store::ExecutionClaimStore + ?Sized,
{
    let mut current = load_fixture_current(scheduler, store, fixture)
        .await
        .expect("load side-effect settlement run");
    for _ in 0..8 {
        let result = drive_current_once_with_claim(scheduler, store, current)
            .await
            .expect("drive through preparation append");
        current = result.into_current_run();
        let mut preparation_committed = false;
        let _ = current.lifecycle().visit_records::<()>(|record| {
            if matches!(
                record.kind(),
                store::current_lifecycle::CurrentRecordKindRef::SideEffectInvocationPrepared(_)
            ) {
                preparation_committed = true;
                return std::ops::ControlFlow::Break(());
            }
            std::ops::ControlFlow::Continue(())
        });
        if preparation_committed {
            return true;
        }
    }
    false
}

#[tokio::test]
async fn side_effect_driver_preserves_concrete_exclusive_resource_key_across_runs() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let mut peer = fixture_with_first_exclusive_side_effect_state();
    assert_eq!(
        peer.runtime_spec.spec_hash(),
        fixture.runtime_spec.spec_hash(),
        "peer run must use the same independently certified spec"
    );
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
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load first current run");
    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "first run claims resource lane"
    );
    let mut claim_seq = None;
    let mut prepared_seq = None;
    let mut resource_key = None;
    let _ = current.lifecycle().visit_records::<()>(|record| {
        match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::ResourceLaneClaimed(payload) => {
                claim_seq = Some(record.sequence());
                resource_key = Some(payload.resource_key.clone());
            }
            store::current_lifecycle::CurrentRecordKindRef::SideEffectInvocationPrepared(_) => {
                prepared_seq = Some(record.sequence());
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    });
    let claim_seq = claim_seq.expect("first run recorded resource lane claim");
    assert!(
        prepared_seq.is_none_or(|prepared_seq| claim_seq < prepared_seq),
        "exclusive invocation prepare must be committed after ResourceLaneClaimed"
    );
    let resource_key = resource_key.expect("first run recorded resource key");
    assert_eq!(resource_key.key.as_str(), "mfm.test.driver.shared-resource");
    let lane_key = store::ResourceLaneKey::from_evidence(&resource_key);
    assert!(current.lifecycle().resource_lane(&lane_key).is_some());

    start_fixture_run(&scheduler, &mut store, &peer, vec![peer.seed_ref.clone()])
        .await
        .expect("start peer run");
    let mut current = load_fixture_current(&scheduler, &store, &peer)
        .await
        .expect("load peer current run");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("peer run starts attempt before observing lane block");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    current = result.into_current_run();
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("peer run blocks on same resource lane");
    assert_eq!(result.status(), SchedulerStatus::Blocked);
    current = result.into_current_run();
    let mut prepared = false;
    let _ = current.lifecycle().visit_records::<()>(|record| {
        if matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::SideEffectInvocationPrepared(_)
        ) {
            prepared = true;
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    });
    assert!(
        !prepared,
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
    let store = started_fixture_store(&scheduler, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "exclusive run terminalizes failed resource-lane claim"
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    let mut claimed = false;
    let mut released = false;
    let mut failed = false;
    let _ = current.lifecycle().visit_records::<()>(|record| {
        match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::ResourceLaneClaimed(_) => {
                claimed = true;
            }
            store::current_lifecycle::CurrentRecordKindRef::ResourceLaneReleased(_) => {
                released = true;
            }
            store::current_lifecycle::CurrentRecordKindRef::SideEffectFailed(_) => {
                failed = true;
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    });
    assert!(
        claimed,
        "failed exclusive attempt records the resource lane claim"
    );
    assert!(
        released,
        "failed exclusive attempt releases the resource lane"
    );
    assert!(
        failed,
        "failed exclusive attempt records terminal side-effect evidence"
    );
    assert_node_failed_with_code(&current, &node.node_id, "runner_output_invalid");
    let mut active_lane = false;
    let _ = current.lifecycle().visit_resource_lanes::<()>(|_| {
        active_lane = true;
        std::ops::ControlFlow::Break(())
    });
    assert!(
        !active_lane,
        "terminal attempt failure releases the exclusive resource lane"
    );
}

#[tokio::test]
async fn runner_block_leaves_started_attempt_open_without_failure() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            BlockingRunner,
        ))
        .expect("blocking runner binding");
    register_fixture_read_runner(&mut registry, &fixture, READ_EXTERNAL_RUNNER);
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    let scheduler = test_scheduler(registry);
    let store = started_fixture_store(&scheduler, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    assert_drive!(
        scheduler,
        store,
        current,
        Blocked,
        "blocked runner leaves attempt open"
    );
    let attempt = current
        .lifecycle()
        .attempt(
            &node.node_id,
            &attempt_id(
                &fixture.run_id,
                fixture.runtime_spec.spec_hash(),
                &node.node_id,
                1,
            )
            .expect("attempt id"),
        )
        .expect("started attempt");
    assert!(matches!(
        attempt.status(),
        store::current_lifecycle::CurrentAttemptStatusRef::Started { .. }
    ));
}

#[tokio::test]
async fn side_effect_verify_block_leaves_framework_attempt_open_without_failure() {
    let fixture = fixture_with_first_side_effect_state();
    let verify_node = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
            )
        })
        .expect("side-effect verify node")
        .clone();
    let scheduler = test_scheduler(registered_first_side_effect_and_verify_runners_with(
        &fixture,
        DriverSideEffectRunner::new(&fixture),
        BlockingRunner,
    ));
    let store = started_fixture_store(&scheduler, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    let mut blocked = false;
    for _ in 0..12 {
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .expect("drive side-effect pair");
        let status = result.status();
        current = result.into_current_run();
        match status {
            SchedulerStatus::Advanced => {}
            SchedulerStatus::Blocked => {
                blocked = true;
                break;
            }
            SchedulerStatus::PublicOutputProjected => {
                panic!("verify runner blocked before public output")
            }
        }
    }
    assert!(blocked, "verify runner must report an operational block");
    let verify_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &verify_node.node_id,
        1,
    )
    .expect("verify attempt id");
    let attempt = current
        .lifecycle()
        .attempt(&verify_node.node_id, &verify_attempt_id)
        .expect("started verify attempt");
    assert!(matches!(
        attempt.status(),
        store::current_lifecycle::CurrentAttemptStatusRef::Started { .. }
    ));
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
    let current = verified_current_for_store(&store, &fixture);

    let output = drive_side_effect_driver_from_current(&current, node, &attempt_id, &callbacks)
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
    let current = verified_current_for_store(&store, &fixture);

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
        let output = drive_side_effect_driver_from_current(&current, node, &attempt_id, &callbacks)
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
    let current = verified_current_for_store(&store, &fixture);
    let held_lane = active_resource_lane_for_pair(&current, &pair_id)
        .expect("exclusive lane must be held before submission recovery");
    let lane_key = held_lane.key().clone();
    let held_run_id = held_lane.holder().run_id().clone();
    let held_pair_id = held_lane.holder().pair_id().clone();
    let held_claim_id = held_lane.claim_id().clone();
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture)
        .with_submission_decision(TestSubmissionDecision::Unknown);

    let output = drive_side_effect_driver_from_current(&current, node, &attempt_id, &callbacks)
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

    let current = verified_current_for_store(&store, &fixture);
    let side_effect = current
        .lifecycle()
        .side_effect(&pair_id)
        .expect("current side effect");
    assert!(matches!(
        side_effect.phase(),
        store::SideEffectPhase::SubmissionUnknown { .. }
    ));
    let lane_after_unknown = current
        .lifecycle()
        .resource_lane(&lane_key)
        .expect("submission-unknown phase must keep the resource lane held");
    assert_eq!(lane_after_unknown.holder().run_id(), &held_run_id);
    assert_eq!(lane_after_unknown.holder().pair_id(), &held_pair_id);
    assert_eq!(lane_after_unknown.claim_id(), &held_claim_id);
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
    let current = verified_current_for_store(&store, &fixture);

    let output = drive_side_effect_verify_driver_from_current(
        &current,
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
    let (scheduler, store) = started_side_effect_fixture_run(&fixture).await;
    let submit_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let verify_node = side_effect_verify_node_for_submit(&fixture, &submit_node).clone();
    let pair_id = fixture_side_effect_pair_id(&fixture, &submit_node);
    let mut saw_lane_claimed = false;
    let mut released_with_receipt_before_output = false;
    let mut terminal_output_after_release = false;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    for _ in 0..10 {
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .expect("advance receipt verification");
        assert_ne!(result.status(), SchedulerStatus::Blocked);
        current = result.into_current_run();
        let active_lane = active_resource_lane_for_pair(&current, &pair_id);
        saw_lane_claimed |= active_lane.is_some();
        let side_effect = current
            .lifecycle()
            .side_effect(&pair_id)
            .expect("current side effect");
        if matches!(
            side_effect.phase(),
            store::SideEffectPhase::ReceiptObserved { .. }
        ) && active_lane.is_none()
            && current.lifecycle().cell(&verify_node.output_cell).is_none()
        {
            released_with_receipt_before_output = true;
        }
        if released_with_receipt_before_output
            && active_lane.is_none()
            && current.lifecycle().cell(&verify_node.output_cell).is_some()
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
    let current = verified_current_for_store(&store, &fixture);

    assert!(matches!(
        drive_side_effect_driver_from_current(&current, node, &attempt_id, &callbacks).await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("unsupported side-effect driver phase: ambiguous")
    ));
}
