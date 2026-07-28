use super::*;

#[tokio::test]
async fn side_effect_scheduler_commits_durable_ledger_phases_before_output() {
    let fixture = fixture_with_first_side_effect_state();
    let (scheduler, store) = started_side_effect_fixture_run(&fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let verify_node = side_effect_verify_node_for_submit(&fixture, node);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let mut receipt_before_output = false;
    for _ in 0..8 {
        assert_drive!(
            scheduler,
            store,
            current,
            Advanced,
            "drive side effect phase"
        );
        let side_effect = side_effect_for_attempt(
            &current.runtime_spec(),
            &current.lifecycle(),
            node,
            &attempt_id,
        )
        .expect("side-effect lookup")
        .expect("current side effect");
        if matches!(
            side_effect.phase(),
            store::SideEffectPhase::ReceiptObserved { .. }
        ) && current.lifecycle().cell(&verify_node.output_cell).is_none()
        {
            receipt_before_output = true;
            break;
        }
    }
    assert!(receipt_before_output);
    assert!(current
        .lifecycle()
        .cell(&fixture.cell_a)
        .is_some_and(|cell| cell.skipped().is_some()));
    assert!(current.lifecycle().cell(&verify_node.output_cell).is_none());

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "materialize side-effect output"
    );

    assert!(current.lifecycle().cell(&verify_node.output_cell).is_some());
    let side_effect = side_effect_for_attempt(
        &current.runtime_spec(),
        &current.lifecycle(),
        node,
        &attempt_id,
    )
    .expect("side-effect lookup")
    .expect("current side effect");
    assert!(matches!(
        side_effect.phase(),
        store::SideEffectPhase::ReceiptObserved { .. }
    ));
    let mut invocation_started = false;
    let _ = current.lifecycle().visit_records::<()>(|record| {
        if matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::SideEffectInvocationStarted(_)
        ) {
            invocation_started = true;
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    });
    assert!(invocation_started);
    assert_eq!(attempt_started_count(&current, &node.node_id), 1);
    assert_eq!(attempt_started_count(&current, &verify_node.node_id), 1);
}

#[tokio::test]
async fn runtime_rejects_invalid_touched_set_terminal_evidence_cases() {
    enum Case {
        ReceiptWithoutEvidence,
        ConfirmationWithoutEvidence,
        ConfirmationWithoutExactClaim,
    }

    for case in [
        Case::ReceiptWithoutEvidence,
        Case::ConfirmationWithoutEvidence,
        Case::ConfirmationWithoutExactClaim,
    ] {
        match case {
            Case::ReceiptWithoutEvidence => {
                let fixture = fixture_with_first_exact_touched_set_side_effect_state();
                let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;

                assert_invalid_output_after(
                    &scheduler,
                    &mut store,
                    &fixture,
                    4,
                    TOUCHED_SET_EVIDENCE_ERR,
                )
                .await;
            }
            Case::ConfirmationWithoutEvidence => {
                let fixture = fixture_with_first_exact_touched_set_finalized_side_effect_state();
                let scheduler =
                    test_scheduler(registered_first_side_effect_and_verify_runners_with(
                        &fixture,
                        DriverSideEffectRunner::new(&fixture),
                        TouchedSetSideEffectVerifyRunner::with_receipt(&fixture),
                    ));
                let mut store = started_fixture_store(&scheduler, &fixture).await;

                assert_invalid_output_after(
                    &scheduler,
                    &mut store,
                    &fixture,
                    5,
                    TOUCHED_SET_EVIDENCE_ERR,
                )
                .await;
            }
            Case::ConfirmationWithoutExactClaim => {
                let fixture = fixture_with_first_finalized_side_effect_state();
                let scheduler =
                    test_scheduler(registered_first_side_effect_and_verify_runners_with(
                        &fixture,
                        DriverSideEffectRunner::new(&fixture),
                        TouchedSetSideEffectVerifyRunner::with_confirmation(&fixture),
                    ));
                let mut store = started_fixture_store(&scheduler, &fixture).await;

                assert_invalid_output_after(
                    &scheduler,
                    &mut store,
                    &fixture,
                    5,
                    EXACT_TOUCHED_SET_CLAIM_ERR,
                )
                .await;
            }
        }
    }
}

#[tokio::test]
async fn side_effect_not_submitted_resume_completes_submit_boundary() {
    let fixture = fixture_with_first_side_effect_state();
    let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");
    for _ in 0..2 {
        drive_ok!(
            scheduler,
            store,
            current,
            "claim, prepare, and start side effect"
        );
    }

    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    append_not_submitted_proven(&mut store, &fixture, node, &attempt_id, 1);
    current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("reload current run after not-submitted proof");

    drive_ok!(scheduler, store, current, "resume not-submitted");
    let side_effect = side_effect_for_attempt(
        &current.runtime_spec(),
        &current.lifecycle(),
        node,
        &attempt_id,
    )
    .expect("side-effect lookup")
    .expect("current side effect");
    assert!(matches!(
        side_effect.phase(),
        store::SideEffectPhase::NotSubmittedProven { .. }
    ));
    assert!(current
        .lifecycle()
        .cell(&fixture.cell_a)
        .is_some_and(|cell| cell.skipped().is_some()));
    assert_eq!(attempt_started_count(&current, &node.node_id), 1);
}

#[tokio::test]
async fn side_effect_staged_artifact_must_match_payload_ledger_binding() {
    struct WrongLedgerStagedSideEffectRunner;

    impl ErasedNodeRunner for WrongLedgerStagedSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                if side_effect_for_attempt(
                    &ctx.runtime_spec(),
                    ctx.lifecycle(),
                    ctx.node(),
                    ctx.attempt_id(),
                )?
                .is_none()
                {
                    let SideEffectFixtureIntentOutput {
                        ledger,
                        staged_artifact,
                        payload,
                    } = side_effect_fixture_intent_output(
                        &ctx,
                        events::SideEffectLedgerPurpose::Forward,
                        1,
                    )?;
                    let claimed = side_effect_claimed(&ctx, ledger, 1, 1);
                    return Ok(ErasedRunnerOutput::from_parts(
                        vec![staged_artifact],
                        Vec::new(),
                        vec![payload, claimed],
                    ));
                }

                let ledger = side_effect_ledger_key_for_ctx(&ctx);
                let prepared = side_effect_prepared_output(&ctx, ledger, 1, 1)?;
                let prepared_value = fixture_side_effect_evidence_for_ctx(&ctx, 35);
                let artifact_builder = RunnerArtifactBuilder::new(&ctx);
                let artifact = artifact_builder.prepared_invocation(&prepared_value)?;
                let staged_ledger =
                    events::SideEffectLedgerKey::new("wrong-ledger").expect("ledger key");
                let staged_artifact =
                    artifact_builder.staged_side_effect(&artifact, staged_ledger, 1)?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    vec![prepared.payload],
                ))
            })
        }
    }

    let fixture = fixture_with_first_side_effect_state();
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            WrongLedgerStagedSideEffectRunner,
        ))
        .expect("binding a");
    register_fixture_read_runner(&mut registry, &fixture, READ_EXTERNAL_RUNNER);
    let (scheduler, store) = started_fixture_run_with_registry(registry, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "persist staged-artifact side-effect authority"
    );
    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        current,
        fixture,
        "terminalize side-effect artifact binding mismatch"
    );
}

#[tokio::test]
async fn side_effect_ambiguous_phase_blocks_resume() {
    let fixture = fixture_with_first_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, store) = started_fixture_run_with_registry(registry, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    for _ in 0..3 {
        assert_drive!(scheduler, store, current, Advanced, "advance to ambiguity");
    }
    for _ in 0..4 {
        let status = drive_ok!(
            scheduler,
            store,
            current,
            "ambiguous side effect resolves terminal"
        );
        if matches!(status, SchedulerStatus::PublicOutputProjected) {
            break;
        }
    }
    assert!(current.lifecycle().cell(&fixture.cell_a).is_none());
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
async fn side_effect_ambiguity_blocks_independent_ready_nodes() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    append_or_get_first_attempt(&mut store, &fixture, &forward_node);
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    for _ in 0..3 {
        drive_ok!(scheduler, store, current, "advance to ambiguity");
    }
    for _ in 0..8 {
        let status = drive_ok!(scheduler, store, current, "ambiguity resolves terminal");
        assert!(
            matches!(
                status,
                SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
            ),
            "unexpected ambiguity terminal status: {status:?}"
        );
        if current.lifecycle().completion().is_some() {
            break;
        }
    }
    assert!(current.lifecycle().cell(&fixture.cell_b).is_none());
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
async fn side_effect_output_before_terminal_evidence_is_rejected() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            PrematureSideEffectOutputRunner {
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    register_fixture_read_runner(&mut registry, &fixture, READ_EXTERNAL_RUNNER);
    let (scheduler, store) = started_fixture_run_with_registry(registry, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "persist premature-output side-effect authority"
    );
    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        current,
        fixture,
        "terminalize premature side-effect output"
    );
}

#[tokio::test]
async fn verify_output_after_receipt_follows_terminal_policy() {
    #[derive(Clone, Copy)]
    enum Case {
        Receipt,
        Finalized,
    }

    for case in [Case::Receipt, Case::Finalized] {
        let fixture = match case {
            Case::Receipt => fixture_with_first_exclusive_side_effect_state(),
            Case::Finalized => runtime_side_effect_fixture(
                RuntimeSideEffectFixtureShape::Chained,
                RuntimeSideEffectClaim::Exclusive,
                spec::SideEffectVerificationSpec::Finalized { depth: 1 },
            ),
        };
        let (wallet_label, sidefx_label, terminal_artifact, terminal_digest) = match case {
            Case::Receipt => (
                "wallet-submit-output-receipt",
                "sidefx-submit-output-receipt",
                artifact(0xe1),
                content(0xe2),
            ),
            Case::Finalized => (
                "wallet-submit-output-finalized",
                "sidefx-submit-output-finalized",
                artifact(0xe3),
                content(0xe4),
            ),
        };
        let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
        let submit_node = node_by_output(&fixture, &fixture.cell_a);
        let submit_attempt_id = append_synthetic_exclusive_receipt_phase(
            &mut store,
            &fixture,
            submit_node,
            wallet_label,
            sidefx_label,
        );
        let verify_node = side_effect_verify_node_for_submit(&fixture, submit_node);
        let verify_attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &verify_node.node_id,
            1,
        )
        .expect("verify attempt id");

        let current = verified_current_for_store(&store, &fixture);
        let validation = side_effect_lifecycle::validate_terminal_batch_evidence(
            &current.runtime_spec(),
            &current.lifecycle(),
            verify_node,
            &verify_attempt_id,
            false,
        );
        match case {
            Case::Receipt => validation.expect("receipt policy permits output after receipt"),
            Case::Finalized => {
                let error = validation.expect_err("finalized policy requires confirmation");
                assert!(error
                    .to_string()
                    .contains("produced output before certified terminal evidence"));
            }
        }

        append_terminal(
            &mut store,
            &fixture,
            verify_node,
            &verify_attempt_id,
            terminal_artifact,
            terminal_digest,
        );
        let journal = block_on_ready(store.load_committed_journal(&fixture.run_id))
            .expect("load committed terminal-evidence journal");
        let historical =
            verify_current_run(journal, recertified_runtime_spec(&fixture.runtime_spec))
                .map(|_| ());
        match case {
            Case::Receipt => {
                historical.expect("historical validation permits receipt-terminal verify output");
                let current = verified_current_for_store(&store, &fixture);
                assert!(current
                    .lifecycle()
                    .attempt(&submit_node.node_id, &submit_attempt_id)
                    .is_some());
            }
            Case::Finalized => {
                let error = historical.expect_err("historical validation requires confirmation");
                assert!(
                    error
                        .to_string()
                        .contains("produced before terminal evidence"),
                    "unexpected historical validation error: {error}"
                );
            }
        }
    }
}

#[test]
fn side_effect_terminal_payloads_derive_attempt_failure_payload() {
    enum Case {
        Failure,
        Ambiguous,
    }

    for (case, expected_error_code) in [
        (Case::Failure, "sidefx_failed"),
        (Case::Ambiguous, "side_effect_ambiguous"),
    ] {
        let fixture = fixture_with_first_side_effect_state();
        let node = node_by_output(&fixture, &fixture.cell_a);
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            1,
        )
        .expect("attempt id");
        let ledger_purpose = side_effect_ledger_purpose();
        let (pair_id, pair_role) = side_effect_pair_fields_for_purpose(
            &fixture.runtime_spec,
            &node.node_id,
            &ledger_purpose,
            events::SideEffectPairRole::Verify,
        );
        let runner_payload = match case {
            Case::Failure => RunnerEventPayload::SideEffectFailed(events::side_effect::Failed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: side_effect_ledger_key(1),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                retryable: false,
                error: side_effect_error(false),
            }),
            Case::Ambiguous => {
                RunnerEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: side_effect_ledger_key(1),
                    ledger_purpose,
                    pair_id,
                    pair_role,
                    invocation_epoch: 1,
                    ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                        .expect("ambiguity code"),
                    evidence_schema_id: node.config_ref.schema_id.clone(),
                    evidence_hash: content(0xca),
                    evidence_artifact_id: artifact(0xcb),
                    evidence_artifact_evidence_hash: content(0xca),
                })
            }
        };
        let payloads = runner_payloads_with_derived_lifecycle(
            &fixture.runtime_spec,
            node,
            &attempt_id,
            Vec::new(),
            vec![runner_payload],
        )
        .expect("derive lifecycle");
        let failure = payloads
            .iter()
            .find_map(|payload| match payload {
                events::KernelEventPayload::StateAttemptFailed(payload) => Some(payload),
                _ => None,
            })
            .expect("derived attempt failure");
        assert!(!failure.retryable);
        assert_eq!(failure.error.code.as_str(), expected_error_code);
    }
}
