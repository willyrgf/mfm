use super::*;

#[tokio::test]
async fn runtime_fails_active_forward_attempts_before_saga_terminal() {
    #[derive(Clone, Copy)]
    enum Case {
        PreBoundary,
        NotSubmitted,
    }

    for case in [Case::PreBoundary, Case::NotSubmitted] {
        let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
        let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
        let failing_node = node_by_output(&fixture, &fixture.cell_b).clone();
        let scheduler = match case {
            Case::PreBoundary => test_scheduler(registered_first_side_effect_runners_with(
                &fixture,
                FailActiveSideEffectAfterSagaRunner::before_invocation_started(&fixture),
            )),
            Case::NotSubmitted => test_scheduler(registered_first_side_effect_runners_with(
                &fixture,
                FailActiveSideEffectAfterSagaRunner::new(&fixture)
                    .with_submission_decision(TestSubmissionDecision::NotSubmitted),
            )),
        };
        let mut store = started_fixture_store(&scheduler, &fixture).await;
        let forward_attempt = append_or_get_first_attempt(&mut store, &fixture, &forward_node);

        match case {
            Case::PreBoundary => {
                drive_until_side_effect_attempt_phase(
                    &scheduler,
                    &mut store,
                    &fixture,
                    &forward_node,
                    &forward_attempt,
                    |phase| matches!(phase, store::SideEffectPhase::InvocationPrepared { .. }),
                    "prepare forward side effect",
                )
                .await;
            }
            Case::NotSubmitted => {
                drive_until_side_effect_attempt_phase(
                    &scheduler,
                    &mut store,
                    &fixture,
                    &forward_node,
                    &forward_attempt,
                    |phase| matches!(phase, store::SideEffectPhase::NotSubmittedProven { .. }),
                    "advance forward side effect before not-submitted proof",
                )
                .await;
            }
        }

        let failing_attempt = append_or_get_started_attempt(&mut store, &fixture, &failing_node, 1);
        append_attempt_failure(&mut store, &fixture, &failing_node, &failing_attempt, false);
        let mut current = load_fixture_current(&scheduler, &store, &fixture)
            .await
            .expect("load current run");
        if matches!(case, Case::PreBoundary) {
            assert_eq!(
                with_fixture_saga(&current, &fixture, |saga| saga.run_mode()),
                store::RunMode::FailedWithoutAcdcClaim
            );
        }

        assert_drive!(
            scheduler,
            store,
            current,
            Advanced,
            "fail active forward attempt before saga terminal"
        );
        let side_effect = side_effect_for_attempt(
            &current.runtime_spec(),
            &current.lifecycle(),
            &forward_node,
            &forward_attempt,
        )
        .expect("side-effect lookup")
        .expect("current side effect");
        match (case, side_effect.phase()) {
            (
                Case::PreBoundary,
                store::SideEffectPhase::Failed {
                    failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                    ..
                },
            )
            | (
                Case::NotSubmitted,
                store::SideEffectPhase::Failed {
                    failure_phase: events::side_effect::FailurePhase::AfterNotSubmittedProven,
                    ..
                },
            ) => {}
            (_, phase) => panic!("unexpected failed forward phase: {phase:?}"),
        }
        if matches!(case, Case::PreBoundary) {
            assert!(matches!(
                current
                    .lifecycle()
                    .attempt(&forward_node.node_id, &forward_attempt)
                    .expect("forward attempt")
                    .status(),
                store::current_lifecycle::CurrentAttemptStatusRef::Failed { .. }
            ));
        }
        assert!(current.lifecycle().completion().is_none());

        assert_drive!(
            scheduler,
            store,
            current,
            Advanced,
            "resolve saga terminal after active attempt closed"
        );
        assert!(matches!(
            current
                .lifecycle()
                .completion()
                .expect("run completion")
                .outcome(),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim
        ));
    }
}

#[tokio::test]
async fn runtime_remediates_confirmed_forward_ledgers_in_reverse_confirmation_order() {
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
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding forward b");
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
        &[forward_a_output.clone(), forward_b_output.clone()],
        "drive forward side-effect phase",
    )
    .await;
    let mut current = verified_current_for_store(&store, &fixture);
    assert!(current.lifecycle().cell(&forward_a_output).is_some());
    assert!(current.lifecycle().cell(&forward_b_output).is_some());
    let forward_a_ledger = forward_ledger_for_node(&current, &forward_a.node_id);
    let forward_b_ledger = forward_ledger_for_node(&current, &forward_b.node_id);
    let forward_a_pair = forward_pair_for_ledger(&current, &forward_a_ledger);
    let forward_b_pair = forward_pair_for_ledger(&current, &forward_b_ledger);

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("reload current run after failure");
    with_fixture_saga(&current, &fixture, |saga| {
        assert_eq!(saga.run_mode(), store::RunMode::Remediating);
        let mut obligation_count = 0;
        let _ = saga.visit_obligations::<()>(|_| {
            obligation_count += 1;
            std::ops::ControlFlow::Continue(())
        });
        assert_eq!(obligation_count, 2);
    });

    for _ in 0..20 {
        let status = drive_ok!(scheduler, store, current, "drive remediation phase");
        assert!(
            matches!(
                status,
                SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
            ),
            "unexpected remediation status: {status:?}"
        );
        if with_fixture_saga(&current, &fixture, |saga| saga.run_mode())
            == store::RunMode::Compensated
        {
            break;
        }
    }
    let remediation_order = remediation_intent_forward_links(&store, &fixture);
    assert_eq!(
        remediation_order,
        vec![forward_b_pair.clone(), forward_a_pair.clone()]
    );
    with_fixture_saga(&current, &fixture, |saga| {
        assert_eq!(saga.run_mode(), store::RunMode::Compensated);
        for forward_pair in [&forward_a_pair, &forward_b_pair] {
            let obligation = saga.obligation(forward_pair).expect("forward obligation");
            assert_eq!(
                obligation.classification(),
                store::ForwardLedgerClassification::Owed
            );
            assert!(obligation
                .remediation()
                .expect("remediation ledger")
                .closed());
        }
    });
    for _ in 0..8 {
        if current.lifecycle().run_state() == store::RunState::Completed {
            break;
        }
        let status = drive_ok!(scheduler, store, current, "resolve compensated terminal");
        assert!(matches!(
            status,
            SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
        ));
    }
    assert_eq!(current.lifecycle().run_state(), store::RunState::Completed);
    assert!(matches!(
        current
            .lifecycle()
            .completion()
            .expect("run completion")
            .outcome(),
        events::RunCompletionOutcome::Compensated
    ));
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("completed compensated run");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let _current = result.into_current_run();
}

#[tokio::test]
async fn runtime_compensated_saga_resume_boundaries_do_not_duplicate_mutations() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let remediation_a = fixture
        .runtime_spec
        .spec()
        .remediations
        .get(&forward_a.node_id)
        .expect("remediation a")
        .clone();
    let remediation_b = fixture
        .runtime_spec
        .spec()
        .remediations
        .get(&forward_b.node_id)
        .expect("remediation b")
        .clone();
    let remediation_a_output = effective_output_cell_for_node(&fixture, &remediation_a);
    let remediation_b_output = effective_output_cell_for_node(&fixture, &remediation_b);
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut scheduler = compensated_saga_scheduler(&fixture);
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    drive_until_side_effect_confirmation_without_output(
        &scheduler,
        &mut store,
        &fixture,
        &forward_a,
        &forward_a_output,
        "forward a confirmation before output",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output.clone(), forward_b_output.clone()],
        "forward outputs",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture);
    let mut current = verified_current_for_store(&store, &fixture);
    let forward_a_ledger = forward_ledger_for_node(&current, &forward_a.node_id);
    let forward_b_ledger = forward_ledger_for_node(&current, &forward_b.node_id);
    let forward_a_pair = forward_pair_for_ledger(&current, &forward_a_ledger);
    let forward_b_pair = forward_pair_for_ledger(&current, &forward_b_ledger);

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    current = verified_current_for_store(&store, &fixture);
    with_fixture_saga(&current, &fixture, |saga| {
        assert_eq!(saga.run_mode(), store::RunMode::Remediating);
        let mut obligation_count = 0;
        let _ = saga.visit_obligations::<()>(|_| {
            obligation_count += 1;
            std::ops::ControlFlow::Continue(())
        });
        assert_eq!(obligation_count, 2);
    });

    scheduler = compensated_saga_scheduler(&fixture);
    assert_no_duplicate_side_effect_submissions(&store, &fixture);
    drive_until_remediation_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_b_pair,
        RemediationPhaseCheckpoint::SubmissionObserved,
        "first remedial submission",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_remediation_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_b_pair,
        RemediationPhaseCheckpoint::ConfirmationObserved,
        "first remedial confirmation",
    )
    .await;
    current = verified_current_for_store(&store, &fixture);
    assert!(current.lifecycle().cell(&remediation_b_output).is_none());
    assert_no_duplicate_side_effect_submissions(&store, &fixture);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        std::slice::from_ref(&remediation_b_output),
        "first remedial output",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture);

    drive_until_compensated_before_terminal(&scheduler, &mut store, &fixture).await;
    current = verified_current_for_store(&store, &fixture);
    assert!(matches!(
        remediation_side_effect_for_forward_pair(&current, &forward_a_pair)
            .expect("second remediation side effect")
            .phase(),
        store::SideEffectPhase::ConfirmationObserved { .. }
    ));
    assert!(current.lifecycle().cell(&remediation_a_output).is_none());
    assert_no_duplicate_side_effect_submissions(&store, &fixture);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        std::slice::from_ref(&remediation_a_output),
        "second remedial output",
    )
    .await;
    current = verified_current_for_store(&store, &fixture);
    assert!(current.lifecycle().cell(&remediation_a_output).is_some());
    assert_no_duplicate_side_effect_submissions(&store, &fixture);
    assert_eq!(
        with_fixture_saga(&current, &fixture, |saga| saga.run_mode()),
        store::RunMode::Compensated
    );
    assert_ne!(current.lifecycle().run_state(), store::RunState::Completed);

    scheduler = compensated_saga_scheduler(&fixture);
    current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("reload current run after scheduler restart");
    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "resolve compensated terminal after resume"
    );
    assert_eq!(current.lifecycle().run_state(), store::RunState::Completed);
    assert!(matches!(
        current
            .lifecycle()
            .completion()
            .expect("run completion")
            .outcome(),
        events::RunCompletionOutcome::Compensated
    ));
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("completed compensated run");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let _current = result.into_current_run();

    assert_eq!(
        remediation_intent_forward_links(&store, &fixture),
        vec![forward_b_pair.clone(), forward_a_pair.clone()]
    );
    let current = verified_current_for_store(&store, &fixture);
    for forward_ledger in [&forward_a_ledger, &forward_b_ledger] {
        assert_eq!(
            side_effect_submission_count_for_ledger(&store, &fixture, forward_ledger),
            1
        );
        assert_eq!(
            remediation_submission_count_for_forward_pair(
                &store,
                &fixture,
                &forward_pair_for_ledger(&current, forward_ledger)
            ),
            1
        );
    }
}

#[tokio::test]
async fn runtime_resolves_clean_failure_without_acdc_claim() {
    let fixture = fixture();
    let failure_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_default_fixture_pure_runner(&mut registry, &fixture);
    register_fixture_read_runner(&mut registry, &fixture, READ_EXTERNAL_RUNNER);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");
    with_fixture_saga(&current, &fixture, |saga| {
        assert_eq!(saga.run_mode(), store::RunMode::FailedWithoutAcdcClaim);
        let mut has_obligation = false;
        let _ = saga.visit_obligations::<()>(|_| {
            has_obligation = true;
            std::ops::ControlFlow::Break(())
        });
        assert!(!has_obligation);
    });

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "resolve clean failure terminal"
    );
    assert!(matches!(
        current
            .lifecycle()
            .completion()
            .expect("run completion")
            .outcome(),
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_materializes_confirmed_forward_output_before_failed_without_claim_terminal() {
    let fixture =
        fixture_with_independent_second_node_and_first_exclusive_finalized_side_effect_state();
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_output = effective_output_cell_for_node(&fixture, &forward_node);
    let failure_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding side effect");
    register_fixture_read_runner(&mut registry, &fixture, READ_EXTERNAL_RUNNER);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    let (forward_attempt, ledger) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        &forward_node,
        "wallet-confirmed-forward-output-before-failure",
        "sidefx-confirmed-forward-output-before-failure",
    );
    append_synthetic_submission_observed(
        &mut store,
        &fixture,
        &forward_node,
        &forward_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-submission",
    );
    append_synthetic_submit_boundary_skipped(
        &mut store,
        &fixture,
        &forward_node,
        &forward_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-submit-boundary",
    );
    let verify_node = side_effect_verify_node_for_submit(&fixture, &forward_node).clone();
    let verify_attempt = append_attempt_start(&mut store, &fixture, &verify_node, 1);
    append_synthetic_verify_receipt_observed(
        &mut store,
        &fixture,
        &forward_node,
        &verify_node,
        &verify_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-receipt",
    );
    append_synthetic_verify_confirmation_observed(
        &mut store,
        &fixture,
        &forward_node,
        &verify_node,
        &verify_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-confirmation",
    );
    let mut current = verified_current_for_store(&store, &fixture);
    assert!(current.lifecycle().cell(&forward_output).is_none());
    let side_effect = side_effect_for_attempt(
        &current.runtime_spec(),
        &current.lifecycle(),
        &forward_node,
        &forward_attempt,
    )
    .expect("side-effect lookup")
    .expect("current side effect");
    assert!(matches!(
        side_effect.phase(),
        store::SideEffectPhase::ConfirmationObserved { .. }
    ));

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("reload current run after failure");
    assert_eq!(
        with_fixture_saga(&current, &fixture, |saga| saga.run_mode()),
        store::RunMode::FailedWithoutAcdcClaim
    );

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "materialize confirmed forward output"
    );
    assert!(current.lifecycle().cell(&forward_output).is_some());
    assert!(current.lifecycle().completion().is_none());

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "resolve failed-without-claim terminal"
    );
    assert!(matches!(
        current
            .lifecycle()
            .completion()
            .expect("run completion")
            .outcome(),
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}
