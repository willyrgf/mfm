use super::*;

#[test]
fn side_effect_phase_order_and_fencing_are_enforced() {
    let mut stale_owner_store = StoreContractRunStore::new();
    let stale_owner_run = run_id(100);
    append_side_effect_prepare(&mut stale_owner_store, &stale_owner_run);
    let stale_owner = append_certified_side_effect_commit(
        &mut stale_owner_store,
        &stale_owner_run,
        "sidefx-started-stale-owner",
        vec![side_effect_started("owner-2", 1, "token-1")],
        Vec::new(),
    )
    .expect_err("stale owner rejects");
    assert!(matches!(stale_owner, StoreError::ProjectionConflict { .. }));

    let mut stale_token_store = StoreContractRunStore::new();
    let stale_token_run = run_id(101);
    append_side_effect_prepare(&mut stale_token_store, &stale_token_run);
    let stale_token = append_certified_side_effect_commit(
        &mut stale_token_store,
        &stale_token_run,
        "sidefx-started-stale-token",
        vec![side_effect_started("owner-1", 1, "token-2")],
        Vec::new(),
    )
    .expect_err("stale token rejects");
    assert!(matches!(stale_token, StoreError::ProjectionConflict { .. }));

    let mut takeover_store = StoreContractRunStore::new();
    let takeover_run = run_id(102);
    append_side_effect_prepare(&mut takeover_store, &takeover_run);
    let reused_token_takeover = append_certified_side_effect_commit(
        &mut takeover_store,
        &takeover_run,
        "sidefx-takeover-reused-token",
        vec![side_effect_claim_taken_over_with_token("token-1")],
        Vec::new(),
    )
    .expect_err("takeover reusing fencing token rejects");
    assert!(matches!(
        reused_token_takeover,
        StoreError::ProjectionConflict { .. }
    ));
    append_certified_side_effect_commit(
        &mut takeover_store,
        &takeover_run,
        "sidefx-takeover",
        vec![side_effect_claim_taken_over()],
        Vec::new(),
    )
    .expect("takeover before invocation started succeeds");
    append_certified_side_effect_commit(
        &mut takeover_store,
        &takeover_run,
        "sidefx-prepared-after-takeover",
        vec![side_effect_prepared(2, "token-2")],
        Vec::new(),
    )
    .expect("prepare after takeover succeeds");
    append_certified_side_effect_commit(
        &mut takeover_store,
        &takeover_run,
        "sidefx-started-after-takeover",
        vec![side_effect_started("owner-2", 2, "token-2")],
        Vec::new(),
    )
    .expect("start after takeover succeeds");
    let late_takeover = append_certified_side_effect_commit(
        &mut takeover_store,
        &takeover_run,
        "sidefx-late-takeover",
        vec![KernelEventPayload::SideEffectClaimTakenOver(
            side_effect::ClaimTakenOver {
                spec_hash: spec_hash(1),
                node_id: submit_node_id(),
                attempt_id: submit_attempt_id(),
                ledger_key: side_effect_ledger_key(),
                ledger_purpose: side_effect_ledger_purpose(),
                pair_id: side_effect_pair_id(),
                pair_role: events::SideEffectPairRole::Submit,
                previous_claim_owner: events::RunnerInvocationId::new("owner-2")
                    .expect("previous owner"),
                new_claim_owner: events::RunnerInvocationId::new("owner-3").expect("new owner"),
                invocation_epoch: 1,
                previous_claim_generation: 2,
                claim_generation: 3,
                claim_fencing_token: side_effect::ClaimFencingToken::new("token-3").expect("token"),
            },
        )],
        Vec::new(),
    )
    .expect_err("takeover after invocation started rejects");
    assert!(matches!(
        late_takeover,
        StoreError::ProjectionConflict { .. }
    ));

    let mut receipt_store = StoreContractRunStore::new();
    let receipt_run = run_id(103);
    append_side_effect_observation_setup(&mut receipt_store, &receipt_run);
    let receipt_artifact = artifact_id(103);
    let receipt_digest = content_digest(104);
    let receipt_evidence = side_effect_evidence(
        receipt_artifact.clone(),
        receipt_digest.clone(),
        receipt_schema(),
        ArtifactRole::Receipt,
    );
    let receipt_without_submission = append_certified_side_effect_commit(
        &mut receipt_store,
        &receipt_run,
        "sidefx-receipt-without-submission",
        vec![side_effect_receipt(receipt_artifact, receipt_digest)],
        vec![receipt_evidence],
    )
    .expect_err("receipt without submission rejects");
    assert!(matches!(
        receipt_without_submission,
        StoreError::ProjectionConflict { .. }
    ));

    let mut confirmation_store = StoreContractRunStore::new();
    let confirmation_run = run_id(104);
    append_side_effect_observation_setup(&mut confirmation_store, &confirmation_run);
    let submission_artifact = artifact_id(105);
    let submission_digest = content_digest(106);
    let submission_evidence = side_effect_evidence(
        submission_artifact.clone(),
        submission_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    append_certified_side_effect_commit(
        &mut confirmation_store,
        &confirmation_run,
        "sidefx-submission",
        vec![side_effect_submission_observed(
            submission_artifact,
            submission_digest,
        )],
        vec![submission_evidence],
    )
    .expect("append submission");
    let confirmation_artifact = artifact_id(107);
    let confirmation_digest = content_digest(108);
    let confirmation_evidence = side_effect_evidence(
        confirmation_artifact.clone(),
        confirmation_digest.clone(),
        confirmation_schema(),
        ArtifactRole::Confirmation,
    );
    let confirmation_without_receipt = append_certified_side_effect_commit(
        &mut confirmation_store,
        &confirmation_run,
        "sidefx-confirmation-without-receipt",
        vec![side_effect_confirmation(
            confirmation_artifact,
            confirmation_digest,
        )],
        vec![confirmation_evidence],
    )
    .expect_err("confirmation without receipt rejects");
    assert!(matches!(
        confirmation_without_receipt,
        StoreError::ProjectionConflict { .. }
    ));

    let unknown_artifact = artifact_id(109);
    let unknown_digest = content_digest(110);
    let unknown_evidence = side_effect_evidence(
        unknown_artifact.clone(),
        unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let duplicate_submit = append_certified_side_effect_commit(
        &mut confirmation_store,
        &confirmation_run,
        "sidefx-duplicate-submit",
        vec![side_effect_submission_unknown(
            unknown_artifact,
            unknown_digest,
        )],
        vec![unknown_evidence],
    )
    .expect_err("duplicate submit rejects");
    assert!(matches!(
        duplicate_submit,
        StoreError::DuplicateLogicalKey { .. }
            | StoreError::LogicalKeyConflict { .. }
            | StoreError::ProjectionConflict { .. }
    ));

    let mut failure_store = StoreContractRunStore::new();
    let failure_run = run_id(105);
    append_side_effect_failure_setup(&mut failure_store, &failure_run);
    let mismatched_retryability = append_certified_side_effect_commit(
        &mut failure_store,
        &failure_run,
        "sidefx-failure-mismatched-retryability",
        vec![side_effect_failed(false), side_effect_attempt_failed(true)],
        Vec::new(),
    )
    .expect_err("mismatched retryability rejects");
    assert!(matches!(
        mismatched_retryability,
        StoreError::ProjectionConflict { .. }
    ));

    let mut failed_without_attempt_store = StoreContractRunStore::new();
    let failed_without_attempt_run = run_id(106);
    append_side_effect_failure_setup(
        &mut failed_without_attempt_store,
        &failed_without_attempt_run,
    );
    assert_certified_side_effect_projection_conflict(
        &mut failed_without_attempt_store,
        &failed_without_attempt_run,
        "sidefx-failed-without-attempt",
        vec![side_effect_failed(false)],
        Vec::new(),
        "side-effect failure requires matching StateAttemptFailed",
    );

    let mut failed_with_attempt_store = StoreContractRunStore::new();
    let failed_with_attempt_run = run_id(107);
    append_side_effect_failure_setup(&mut failed_with_attempt_store, &failed_with_attempt_run);
    append_certified_side_effect_commit(
        &mut failed_with_attempt_store,
        &failed_with_attempt_run,
        "sidefx-failed-with-attempt",
        vec![side_effect_failed(false), side_effect_attempt_failed(false)],
        Vec::new(),
    )
    .expect("side-effect failure with matching attempt failure accepts");

    let mut failed_mismatch_store = StoreContractRunStore::new();
    let failed_mismatch_run = run_id(108);
    append_side_effect_failure_setup(&mut failed_mismatch_store, &failed_mismatch_run);
    let mut mismatched_attempt = side_effect_attempt_failed(false);
    set_attempt_failure_node_attempt(&mut mismatched_attempt, node_id(230), attempt_id(232));
    assert_certified_side_effect_projection_conflict(
        &mut failed_mismatch_store,
        &failed_mismatch_run,
        "sidefx-failed-mismatched-attempt",
        vec![side_effect_failed(false), mismatched_attempt],
        Vec::new(),
        "side-effect failure requires matching StateAttemptFailed",
    );

    let mut ambiguity_without_attempt_store = StoreContractRunStore::new();
    let ambiguity_without_attempt_run = run_id(109);
    append_side_effect_prepare(
        &mut ambiguity_without_attempt_store,
        &ambiguity_without_attempt_run,
    );
    append_side_effect_started(
        &mut ambiguity_without_attempt_store,
        &ambiguity_without_attempt_run,
    );
    append_side_effect_verify_attempt_started(
        &mut ambiguity_without_attempt_store,
        &ambiguity_without_attempt_run,
    );
    let ambiguity_artifact = artifact_id(113);
    let ambiguity_digest = content_digest(114);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    assert_certified_side_effect_projection_conflict(
        &mut ambiguity_without_attempt_store,
        &ambiguity_without_attempt_run,
        "sidefx-ambiguity-without-attempt",
        vec![side_effect_ambiguous(ambiguity_artifact, ambiguity_digest)],
        vec![ambiguity_evidence],
        "side-effect ambiguity requires matching StateAttemptFailed",
    );

    let mut ambiguity_with_attempt_store = StoreContractRunStore::new();
    let ambiguity_with_attempt_run = run_id(110);
    append_side_effect_prepare(
        &mut ambiguity_with_attempt_store,
        &ambiguity_with_attempt_run,
    );
    append_side_effect_started(
        &mut ambiguity_with_attempt_store,
        &ambiguity_with_attempt_run,
    );
    append_side_effect_verify_attempt_started(
        &mut ambiguity_with_attempt_store,
        &ambiguity_with_attempt_run,
    );
    let ambiguity_artifact = artifact_id(115);
    let ambiguity_digest = content_digest(116);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    append_certified_side_effect_commit(
        &mut ambiguity_with_attempt_store,
        &ambiguity_with_attempt_run,
        "sidefx-ambiguity-with-attempt",
        vec![
            side_effect_ambiguous(ambiguity_artifact, ambiguity_digest),
            side_effect_attempt_failed(false),
        ],
        vec![ambiguity_evidence],
    )
    .expect("side-effect ambiguity with matching non-retryable attempt failure accepts");

    let mut ambiguity_retryable_attempt_store = StoreContractRunStore::new();
    let ambiguity_retryable_attempt_run = run_id(111);
    append_side_effect_prepare(
        &mut ambiguity_retryable_attempt_store,
        &ambiguity_retryable_attempt_run,
    );
    append_side_effect_started(
        &mut ambiguity_retryable_attempt_store,
        &ambiguity_retryable_attempt_run,
    );
    append_side_effect_verify_attempt_started(
        &mut ambiguity_retryable_attempt_store,
        &ambiguity_retryable_attempt_run,
    );
    let ambiguity_artifact = artifact_id(117);
    let ambiguity_digest = content_digest(118);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    assert_certified_side_effect_projection_conflict(
        &mut ambiguity_retryable_attempt_store,
        &ambiguity_retryable_attempt_run,
        "sidefx-ambiguity-retryable-attempt",
        vec![
            side_effect_ambiguous(ambiguity_artifact, ambiguity_digest),
            side_effect_attempt_failed(true),
        ],
        vec![ambiguity_evidence],
        "side-effect ambiguity retryability must match attempt failure",
    );

    let mut ambiguity_mismatch_store = StoreContractRunStore::new();
    let ambiguity_mismatch_run = run_id(112);
    append_side_effect_observation_setup(&mut ambiguity_mismatch_store, &ambiguity_mismatch_run);
    let ambiguity_artifact = artifact_id(119);
    let ambiguity_digest = content_digest(120);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    let mut mismatched_attempt = side_effect_attempt_failed(false);
    set_attempt_failure_node_attempt(&mut mismatched_attempt, node_id(230), attempt_id(232));
    assert_certified_side_effect_projection_conflict(
        &mut ambiguity_mismatch_store,
        &ambiguity_mismatch_run,
        "sidefx-ambiguity-mismatched-attempt",
        vec![
            side_effect_ambiguous(ambiguity_artifact, ambiguity_digest),
            mismatched_attempt,
        ],
        vec![ambiguity_evidence],
        "side-effect ambiguity requires matching StateAttemptFailed",
    );
}
