use super::*;

#[test]
fn duplicate_attempt_lifecycle_events_are_rejected() {
    let run_id = run_id(59);
    let artifact_id = artifact_id(60);
    let artifact_digest = content_digest(61);
    let mut store = admitted_store(&run_id, "duplicate-run-start");
    let artifact = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    append_state_attempt_started(&mut store, &run_id, "attempt-start")
        .expect("append attempt start");

    let duplicate_start = append_state_attempt_started(&mut store, &run_id, "attempt-start-2")
        .expect_err("duplicate attempt start rejects");
    assert!(matches!(
        duplicate_start,
        StoreError::ProjectionConflict { .. }
    ));

    append_default_commit(
        &mut store,
        &run_id,
        "attempt-complete",
        terminal_cell_commit_payloads(artifact_id.clone(), artifact_digest.clone()),
        vec![artifact.clone()],
    )
    .expect("append attempt complete");

    let duplicate_terminal = append_default_commit(
        &mut store,
        &run_id,
        "attempt-complete-2",
        terminal_cell_commit_payloads(artifact_id, artifact_digest),
        vec![artifact],
    )
    .expect_err("duplicate attempt terminal rejects");
    assert!(matches!(
        duplicate_terminal,
        StoreError::DuplicateLogicalKey { .. } | StoreError::ProjectionConflict { .. }
    ));
}

#[test]
fn state_attempt_interrupted_projects_retryable_terminal_without_saga_engagement() {
    let run_id = run_id(60);
    let mut store = admitted_store(&run_id, "run-start");
    append_state_attempt_started(&mut store, &run_id, "attempt-start")
        .expect("append attempt start");
    append_state_attempt_interrupted(&mut store, &run_id, "attempt-interrupt")
        .expect("append attempt interruption");

    let projection = store.projection_snapshot();
    let attempt = projection
        .attempts()
        .find_map(|((projected_node_id, projected_attempt_id), projection)| {
            if *projected_node_id == node_id(20) && *projected_attempt_id == attempt_id(23) {
                Some(projection)
            } else {
                None
            }
        })
        .expect("attempt projection");
    assert!(matches!(&attempt.status, AttemptStatus::Interrupted));
    assert!(projection.saga_engagement(&run_id).is_none());
    assert_projection_codecs_round_trip(projection);
}

#[test]
fn state_attempt_interrupted_rejects_duplicate_terminal_event() {
    let run_id = run_id(61);
    let mut store = admitted_store(&run_id, "run-start");
    append_state_attempt_started(&mut store, &run_id, "attempt-start")
        .expect("append attempt start");
    append_state_attempt_interrupted(&mut store, &run_id, "attempt-interrupt")
        .expect("append attempt interruption");

    let duplicate = append_state_attempt_interrupted(&mut store, &run_id, "attempt-interrupt-2")
        .expect_err("duplicate interruption rejects");
    assert!(matches!(duplicate, StoreError::ProjectionConflict { .. }));
}

#[test]
fn state_attempt_interrupted_allows_side_effect_before_invocation_prepared() {
    for (case_label, run_id, artifact_id, artifact_digest, include_claim) in [
        (
            "intent",
            run_id(62),
            artifact_id(81),
            content_digest(82),
            false,
        ),
        (
            "claim",
            run_id(63),
            artifact_id(83),
            content_digest(84),
            true,
        ),
    ] {
        let mut store = admitted_store(&run_id, "run-start");
        append_side_effect_attempt_started(&mut store, &run_id, "sidefx-attempt-start")
            .expect("append sidefx attempt start");
        let mut payloads = vec![side_effect_intent(
            artifact_id.clone(),
            artifact_digest.clone(),
        )];
        if include_claim {
            payloads.push(side_effect_claim());
        }
        append_certified_side_effect_commit(
            &mut store,
            &run_id,
            &format!("sidefx-{case_label}"),
            payloads,
            vec![intent_artifact_ref(artifact_id, artifact_digest)],
        )
        .expect("append sidefx pre-prepare phase");

        append_default_commit(
            &mut store,
            &run_id,
            "sidefx-attempt-interrupt",
            vec![state_attempt_interrupted_for(node_id(70), attempt_id(72))],
            Vec::new(),
        )
        .expect("interruption before sidefx prepare admits");

        let projection = store.projection_snapshot();
        let attempt = projection
            .attempt(&node_id(70), &attempt_id(72))
            .expect("attempt projection");
        assert!(matches!(attempt.status, AttemptStatus::Interrupted));
        let phase = projection
            .side_effect_for_pair(&run_id, &side_effect_pair_id())
            .expect("side-effect projection")
            .ledger_state()
            .expect("ledger state")
            .phase();
        match (include_claim, phase) {
            (false, SideEffectLedgerPhase::IntentPersisted { .. })
            | (true, SideEffectLedgerPhase::Claimed { .. }) => {}
            (_, other) => panic!("{case_label}: unexpected side-effect phase: {other:?}"),
        }
    }
}

#[test]
fn state_attempt_terminals_reject_after_side_effect_authority_without_terminal_evidence() {
    enum Case {
        Interrupted,
        Failed,
    }

    for (case, run_id, commit_key) in [
        (Case::Interrupted, run_id(64), "sidefx-attempt-interrupt"),
        (Case::Failed, run_id(65), "sidefx-attempt-failed-alone"),
    ] {
        let mut store = admitted_store(&run_id, "run-start");
        append_side_effect_prepare(&mut store, &run_id);

        let payload = match case {
            Case::Interrupted => state_attempt_interrupted_for(node_id(70), attempt_id(72)),
            Case::Failed => {
                let mut failure = side_effect_attempt_failed(true);
                set_attempt_failure_node_attempt(
                    &mut failure,
                    submit_node_id(),
                    submit_attempt_id(),
                );
                failure
            }
        };
        let error =
            append_default_commit(&mut store, &run_id, commit_key, vec![payload], Vec::new())
                .expect_err("attempt terminal after side-effect authority rejects");

        match case {
            Case::Interrupted => assert!(matches!(error, StoreError::ProjectionConflict { .. })),
            Case::Failed => assert_projection_conflict_contains(
                error,
                "side-effect attempt failure requires terminal side-effect evidence",
            ),
        }
    }
}

#[test]
fn attempt_terminal_and_cell_terminal_must_commit_together() {
    let run_id = run_id(62);
    let mut store = admitted_store(&run_id, "terminal-pair-run-start");
    append_state_attempt_started(&mut store, &run_id, "attempt-start")
        .expect("append attempt start");

    let completion_only = append_default_commit(
        &mut store,
        &run_id,
        "completion-only",
        vec![state_attempt_completed()],
        Vec::new(),
    )
    .expect_err("completion without terminal cell rejects");
    assert!(matches!(
        completion_only,
        StoreError::ProjectionConflict { .. }
    ));

    let cell_only = append_default_commit(
        &mut store,
        &run_id,
        "cell-only",
        vec![cell_produced(artifact_id(63), content_digest(64))],
        vec![store_artifact_ref(artifact_id(63), content_digest(64))],
    )
    .expect_err("terminal cell without completion rejects");
    assert!(matches!(cell_only, StoreError::ProjectionConflict { .. }));
}

#[test]
fn public_output_must_commit_with_render_receipt_terminal() {
    let run_id = run_id(67);
    let artifact_id = artifact_id(68);
    let artifact_digest = content_digest(69);
    let mut store = admitted_store(&run_id, "public-output-run-start");
    let artifact = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    append_state_attempt_started(&mut store, &run_id, "attempt-start")
        .expect("append attempt start");
    append_default_commit(
        &mut store,
        &run_id,
        "receipt-terminal",
        terminal_cell_commit_payloads(artifact_id.clone(), artifact_digest.clone()),
        vec![artifact],
    )
    .expect("append receipt terminal");

    let split_public_output = append_default_commit(
        &mut store,
        &run_id,
        "split-public-output",
        vec![public_output_produced(
            artifact_id.clone(),
            artifact_digest.clone(),
        )],
        vec![store_artifact_ref(artifact_id, artifact_digest)],
    )
    .expect_err("public output split from terminal commit rejects");
    assert!(matches!(
        split_public_output,
        StoreError::ProjectionConflict { .. }
    ));
}

#[test]
fn side_effect_transition_mismatches_are_rejected() {
    let run_id = run_id(80);
    let artifact_id = artifact_id(81);
    let artifact_digest = content_digest(82);
    let mut store = admitted_store(&run_id, "sidefx-transition-run-start");
    let intent_evidence = intent_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    append_side_effect_attempt_started(&mut store, &run_id, "sidefx-attempt-start")
        .expect("append sidefx attempt start");
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-intent",
        vec![side_effect_intent(artifact_id, artifact_digest)],
        vec![intent_evidence],
    )
    .expect("append intent");
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-claim",
        vec![side_effect_claim()],
        Vec::new(),
    )
    .expect("append claim");

    let wrong_generation = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-prepared-wrong-generation",
        vec![side_effect_prepared(2, "token-1")],
        Vec::new(),
    )
    .expect_err("prepared generation mismatch rejects");
    assert!(matches!(
        wrong_generation,
        StoreError::ProjectionConflict { .. }
    ));

    let wrong_token = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-prepared-wrong-token",
        vec![side_effect_prepared(1, "token-2")],
        Vec::new(),
    )
    .expect_err("prepared token mismatch rejects");
    assert!(matches!(wrong_token, StoreError::ProjectionConflict { .. }));
}

#[test]
fn run_mode_strings_and_terminal_outcome_mapping_are_canonical() {
    let modes = [
        (RunMode::Forward, "forward", None),
        (RunMode::Remediating, "remediating", None),
        (RunMode::ManualBlocked, "manual_blocked", None),
        (RunMode::Completed, "completed", None),
        (
            RunMode::Compensated,
            "compensated",
            Some(events::RunCompletionOutcome::Compensated),
        ),
        (
            RunMode::ManuallyResolved,
            "manually_resolved",
            Some(events::RunCompletionOutcome::ManuallyResolved),
        ),
        (
            RunMode::FailedWithoutAcdcClaim,
            "failed_without_acdc_claim",
            Some(events::RunCompletionOutcome::FailedWithoutAcdcClaim),
        ),
    ];
    for (mode, tag, terminal) in modes {
        assert_eq!(mode.as_str(), tag);
        assert_eq!(mode.saga_terminal_outcome(), terminal);
    }

    let outcomes = [
        (completed_outcome(21), RunMode::Completed, "completed"),
        (
            events::RunCompletionOutcome::Compensated,
            RunMode::Compensated,
            "compensated",
        ),
        (
            events::RunCompletionOutcome::ManuallyResolved,
            RunMode::ManuallyResolved,
            "manually_resolved",
        ),
        (
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            RunMode::FailedWithoutAcdcClaim,
            "failed_without_acdc_claim",
        ),
    ];
    for (outcome, mode, tag) in outcomes {
        assert_eq!(RunMode::from_completion_outcome(&outcome), mode);
        assert_eq!(
            mfm_store::v1::codec::run_completion_outcome_str(&outcome),
            tag
        );
    }

    let owner = events::RunnerInvocationId::new("owner-1").expect("owner");
    let token = side_effect::ClaimFencingToken::new("token-1").expect("token");
    let phases = [
        (
            SideEffectPhase::IntentPersisted {
                invocation_epoch: 1,
            },
            "intent_persisted",
        ),
        (
            SideEffectPhase::Claimed {
                claim_owner: owner.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            },
            "claimed",
        ),
        (
            SideEffectPhase::InvocationPrepared {
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            },
            "invocation_prepared",
        ),
        (
            SideEffectPhase::InvocationStarted {
                claim_owner: owner,
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token,
            },
            "invocation_started",
        ),
        (
            SideEffectPhase::SubmissionObserved {
                invocation_epoch: 1,
            },
            "submission_observed",
        ),
        (
            SideEffectPhase::NotSubmittedProven {
                invocation_epoch: 1,
            },
            "not_submitted_proven",
        ),
        (
            SideEffectPhase::SubmissionUnknown {
                invocation_epoch: 1,
            },
            "submission_unknown",
        ),
        (
            SideEffectPhase::ReceiptObserved {
                invocation_epoch: 1,
            },
            "receipt_observed",
        ),
        (
            SideEffectPhase::ConfirmationObserved {
                invocation_epoch: 1,
            },
            "confirmation_observed",
        ),
        (
            SideEffectPhase::Ambiguous {
                invocation_epoch: 1,
            },
            "ambiguous",
        ),
        (
            SideEffectPhase::Failed {
                invocation_epoch: 1,
                failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
            },
            "failed",
        ),
    ];
    for (phase, tag) in phases {
        assert_eq!(phase.as_str(), tag);
    }
}
