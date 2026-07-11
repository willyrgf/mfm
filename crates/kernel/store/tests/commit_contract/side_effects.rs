use super::*;

#[test]
fn side_effect_ledger_purpose_is_required_and_closed() {
    let payload = side_effect_claim();
    let mut missing = payload_json_value(&payload);
    missing
        .as_object_mut()
        .expect("payload object")
        .remove("ledger_purpose");
    assert!(matches!(
        payload_from_json_value(&missing),
        Err(StoreError::Event(message)) if message.contains("ledger_purpose")
    ));

    let mut unknown = payload_json_value(&payload);
    unknown
        .get_mut("ledger_purpose")
        .and_then(serde_json::Value::as_object_mut)
        .expect("ledger purpose object")
        .insert(
            "kind".to_owned(),
            serde_json::Value::String("other".to_owned()),
        );
    assert!(matches!(
        payload_from_json_value(&unknown),
        Err(StoreError::Identity(message))
            if message.contains("unknown side-effect ledger purpose")
    ));
}

#[test]
fn side_effect_ledger_purpose_cannot_change_after_intent() {
    let run_id = run_id(92);
    let artifact_id = artifact_id(93);
    let artifact_digest = content_digest(94);
    let mut store = admitted_store(&run_id, "purpose-run-start");
    append_side_effect_attempt_started(&mut store, &run_id, "purpose-attempt-start")
        .expect("append sidefx attempt start");
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "purpose-intent",
        vec![side_effect_intent(
            artifact_id.clone(),
            artifact_digest.clone(),
        )],
        vec![intent_artifact_ref(artifact_id, artifact_digest)],
    )
    .expect("append intent");

    let mut changed = side_effect_claim();
    let KernelEventPayload::SideEffectClaimed(payload) = &mut changed else {
        unreachable!("helper returns claimed payload");
    };
    payload.ledger_purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: side_effect_pair_id(),
    };
    let error = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "purpose-changed-claim",
        vec![changed],
        Vec::new(),
    )
    .expect_err("ledger purpose change rejects");
    assert!(matches!(
        error,
        StoreError::ProjectionConflict { message, .. }
            if message.contains("ledger purpose changed")
    ));
}

#[test]
fn forward_fence_rejects_boundary_events_after_engagement() {
    fn started_store() -> (RunId, StoreContractRunStore) {
        let run_id = run_id(120);
        let mut store = admitted_store(&run_id, "fence-run-start");
        append_side_effect_attempt_started(&mut store, &run_id, "fence-sidefx-attempt-start")
            .expect("append sidefx attempt start");
        (run_id, store)
    }

    fn intent_with_ref(byte: u8) -> (KernelEventPayload, ArtifactEvidenceRef) {
        let artifact_id = artifact_id(byte);
        let digest = content_digest(byte + 1);
        (
            side_effect_intent(artifact_id.clone(), digest.clone()),
            intent_artifact_ref(artifact_id, digest),
        )
    }

    fn append_before_engagement(
        store: &mut StoreContractRunStore,
        run_id: &RunId,
        commit_key: &str,
        artifact_byte: u8,
        extra_payloads: Vec<KernelEventPayload>,
    ) {
        let (intent, evidence) = intent_with_ref(artifact_byte);
        let mut payloads = vec![intent];
        payloads.extend(extra_payloads);
        append_certified_side_effect_commit(store, run_id, commit_key, payloads, vec![evidence])
            .expect("append before engagement");
    }

    fn reject_after_engagement(
        store: &mut StoreContractRunStore,
        run_id: &RunId,
        engagement_key: &str,
        payloads: Vec<KernelEventPayload>,
    ) {
        append_generic_nonretryable_failure(store, run_id, engagement_key);
        let commit_key = format!("{engagement_key}-reject");
        assert_certified_side_effect_projection_conflict(
            store,
            run_id,
            &commit_key,
            payloads,
            Vec::new(),
            "forward side-effect boundary",
        );
    }

    let (run_id, mut store) = started_store();
    let (intent, intent_ref) = intent_with_ref(130);
    append_generic_nonretryable_failure(&mut store, &run_id, "fence-intent");
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run_id,
        "fence-intent-reject",
        vec![intent],
        vec![intent_ref],
        "forward side-effect boundary",
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(&mut store, &run_id, "fence-claim-intent", 132, Vec::new());
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-claim",
        vec![side_effect_claim()],
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(
        &mut store,
        &run_id,
        "fence-takeover-intent-claim",
        134,
        vec![side_effect_claim()],
    );
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-takeover",
        vec![side_effect_claim_taken_over()],
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(
        &mut store,
        &run_id,
        "fence-prepared-intent-claim",
        136,
        vec![side_effect_claim()],
    );
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-prepared",
        vec![side_effect_prepared(1, "token-1")],
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(
        &mut store,
        &run_id,
        "fence-started-prepare",
        138,
        vec![side_effect_claim(), side_effect_prepared(1, "token-1")],
    );
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-started",
        vec![side_effect_started("owner-1", 1, "token-1")],
    );
}

#[test]
fn remediation_intent_requires_engaged_confirmed_forward_and_unique_link() {
    let policy = compensate_saga_policy();
    let run_id = run_id_with_saga_policy(120, &policy);
    let mut store = admitted_store_with_saga_policy(&run_id, "remediation-run-start", &policy);
    append_forward_confirmation(&mut store, &run_id);
    append_remediation_attempts_started(&mut store, &run_id, "remediation-attempts-started");

    let mut remediation_intent = side_effect_intent(artifact_id(140), content_digest(141));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(1));
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run_id,
        "remediation-before-engagement",
        vec![remediation_intent],
        vec![remediation_intent_artifact_ref(
            artifact_id(140),
            content_digest(141),
        )],
        "prior saga engagement",
    );

    append_generic_nonretryable_failure(&mut store, &run_id, "remediation-engagement");
    let mut wrong_pair = side_effect_intent(artifact_id(148), content_digest(149));
    set_remediation_purpose(&mut wrong_pair, remediation_ledger_key(11));
    let KernelEventPayload::SideEffectIntentPersisted(payload) = &mut wrong_pair else {
        unreachable!("helper returns side-effect intent");
    };
    payload.ledger_purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: remediation_pair_id(),
    };
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run_id,
        "remediation-wrong-pair",
        vec![wrong_pair],
        vec![remediation_intent_artifact_ref(
            artifact_id(148),
            content_digest(149),
        )],
        "forward pair",
    );

    let mut remediation_intent = side_effect_intent(artifact_id(142), content_digest(143));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(1));
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "remediation-admitted",
        vec![remediation_intent],
        vec![remediation_intent_artifact_ref(
            artifact_id(142),
            content_digest(143),
        )],
    )
    .expect("confirmed forward remediation is admitted after engagement");

    let mut duplicate = side_effect_intent(artifact_id(144), content_digest(145));
    set_remediation_purpose(&mut duplicate, remediation_ledger_key(2));
    let error = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "remediation-duplicate",
        vec![duplicate],
        vec![remediation_intent_artifact_ref(
            artifact_id(144),
            content_digest(145),
        )],
    )
    .expect_err("duplicate remediation rejects");
    match error {
        StoreError::LogicalKeyConflict { logical_key }
            if logical_key.as_str().contains("sidefx:remediation") => {}
        StoreError::ProjectionConflict { message, .. }
            if message.contains("already exists for forward pair") => {}
        other => panic!("unexpected duplicate remediation error: {other:?}"),
    }

    let unconfirmed_run_id = run_id_with_saga_policy(121, &policy);
    let mut unconfirmed =
        admitted_store_with_saga_policy(&unconfirmed_run_id, "unconfirmed-run-start", &policy);
    append_side_effect_prepare(&mut unconfirmed, &unconfirmed_run_id);
    append_generic_nonretryable_failure(
        &mut unconfirmed,
        &unconfirmed_run_id,
        "unconfirmed-engagement",
    );
    let mut remediation_intent = side_effect_intent(artifact_id(146), content_digest(147));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(3));
    append_remediation_attempts_started(
        &mut unconfirmed,
        &unconfirmed_run_id,
        "unconfirmed-remediation-attempts-started",
    );
    assert_certified_side_effect_projection_conflict(
        &mut unconfirmed,
        &unconfirmed_run_id,
        "remediation-unconfirmed",
        vec![remediation_intent],
        vec![remediation_intent_artifact_ref(
            artifact_id(146),
            content_digest(147),
        )],
        "requires terminal forward ledger",
    );
}

#[test]
fn remediation_intent_rejects_saga_token_from_same_policy_different_spec_hash() {
    let policy = compensate_saga_policy();
    let run_id = run_id_with_saga_policy(222, &policy);
    let mut store =
        admitted_store_with_saga_policy(&run_id, "remediation-spec-authority-run-start", &policy);
    append_forward_confirmation(&mut store, &run_id);
    append_generic_nonretryable_failure(&mut store, &run_id, "remediation-spec-authority");
    append_remediation_attempts_started(
        &mut store,
        &run_id,
        "remediation-spec-authority-attempts-started",
    );

    let alternate_spec =
        saga_authority_spec_with_authoring_config_hash(policy.clone(), content_digest(188));
    let alternate_spec_hash = alternate_spec
        .spec_hash()
        .expect("alternate saga authority spec hash");
    assert_ne!(
        store
            .projection_snapshot()
            .run_spec_hash(&run_id)
            .expect("run-start spec hash"),
        &alternate_spec_hash
    );

    let mut remediation_intent = side_effect_intent(artifact_id(188), content_digest(189));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(18));
    let KernelEventPayload::SideEffectIntentPersisted(payload) = &mut remediation_intent else {
        unreachable!("helper returns side-effect intent");
    };
    payload.spec_hash = alternate_spec_hash;
    payload.ledger_purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: side_effect_pair_id(),
    };

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("remediation-spec-authority-reject").expect("commit key"),
            payloads: vec![remediation_intent],
            required_artifacts: vec![remediation_intent_artifact_ref(
                artifact_id(188),
                content_digest(189),
            )],
            preconditions: CommitPreconditions {
                certified_run_authority: Some(
                    CertifiedRunStoreAuthority::from_spec(run_id.clone(), &alternate_spec)
                        .expect("alternate certified run authority"),
                ),
                ..CommitPreconditions::default()
            },
        })
        .expect_err("alternate-spec saga token rejects");
    assert_projection_conflict_contains(error, "spec hash does not match run start");
}

#[test]
fn side_effect_ledger_state_exposes_valid_prepared_view() {
    let run_id = run_id(120);
    let mut store = StoreContractRunStore::new();
    append_side_effect_prepare(&mut store, &run_id);

    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    let pair_id = side_effect_pair_id();
    let pair_projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &pair_id)
        .expect("pair projection");
    assert_eq!(pair_projection.ledger_key, projection.ledger_key);
    let state = projection.ledger_state().expect("typed ledger state");
    let pair_state = store
        .projection_snapshot()
        .side_effect_state_for_pair(&run_id, &pair_id)
        .expect("pair state lookup")
        .expect("pair state");
    assert_eq!(pair_state.ledger_purpose(), state.ledger_purpose());
    assert!(matches!(
        pair_state.phase(),
        SideEffectLedgerPhase::Prepared { .. }
    ));
    assert!(state.is_forward_completion_candidate());
    let SideEffectLedgerPhase::Prepared {
        claim,
        prepared_invocation,
        resource_key,
    } = state.phase()
    else {
        panic!("expected prepared state");
    };
    assert_eq!(claim.claim_generation, 1);
    assert!(prepared_invocation.is_none());
    assert!(resource_key.is_none());
}

#[test]
fn side_effect_ledger_state_rejects_malformed_required_phase_evidence() {
    #[derive(Clone, Copy)]
    enum Case {
        MissingActiveClaim,
        MissingConfirmation,
    }

    for (case, expected_error) in [
        (Case::MissingActiveClaim, "phase requires active claim"),
        (
            Case::MissingConfirmation,
            "confirmation evidence is missing",
        ),
    ] {
        let run_id = run_id(120);
        let mut store = StoreContractRunStore::new();
        match case {
            Case::MissingActiveClaim => append_side_effect_prepare(&mut store, &run_id),
            Case::MissingConfirmation => append_forward_confirmation(&mut store, &run_id),
        }
        let mut projection = store
            .projection_snapshot()
            .side_effect_for_pair(&run_id, &side_effect_pair_id())
            .expect("side-effect projection")
            .clone();
        match case {
            Case::MissingActiveClaim => projection.claim = None,
            Case::MissingConfirmation => projection.confirmation = None,
        }

        let error = projection
            .ledger_state()
            .expect_err("malformed side-effect projection rejects");
        assert_projection_conflict_contains(error, expected_error);
    }
}

#[test]
fn side_effect_ledger_state_classifies_ambiguity_as_terminal_not_frontier() {
    let run_id = run_id(120);
    let mut store = StoreContractRunStore::new();
    append_side_effect_observation_setup(&mut store, &run_id);
    let ambiguity_artifact = artifact_id(121);
    let ambiguity_digest = content_digest(122);
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-ambiguity-typestate",
        vec![
            side_effect_ambiguous(ambiguity_artifact.clone(), ambiguity_digest.clone()),
            side_effect_attempt_failed(false),
        ],
        vec![side_effect_evidence(
            ambiguity_artifact,
            ambiguity_digest,
            schema_id("mfm.test.ambiguity", 76),
            ArtifactRole::AmbiguityEvidence,
        )],
    )
    .expect("append ambiguity terminal pair");

    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    let state = projection.ledger_state().expect("typed ledger state");
    assert!(!state.is_forward_completion_candidate());
    assert!(matches!(
        state.phase(),
        SideEffectLedgerPhase::Ambiguous { .. }
    ));
}

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

#[test]
fn side_effect_submission_unknown_recovery_uses_one_submission_result_key() {
    let mut store = StoreContractRunStore::new();
    let run_id = run_id(113);
    append_side_effect_prepare(&mut store, &run_id);
    append_side_effect_started(&mut store, &run_id);

    let unknown_artifact = artifact_id(111);
    let unknown_digest = content_digest(112);
    let unknown_evidence = side_effect_evidence(
        unknown_artifact.clone(),
        unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let unknown_outcome = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-submission-unknown",
        vec![side_effect_submission_unknown(
            unknown_artifact,
            unknown_digest,
        )],
        vec![unknown_evidence],
    )
    .expect("append submission unknown");
    let CommitOutcome::Appended(unknown) = unknown_outcome else {
        panic!("submission unknown should append");
    };
    let submission_result_key = format!(
        "sidefx:forward:{}:invocation:1:submission_result",
        side_effect_pair_id()
    );
    assert_eq!(
        unknown.events()[0].logical_key().as_str(),
        submission_result_key
    );

    let refreshed_unknown_artifact = artifact_id(117);
    let refreshed_unknown_digest = content_digest(118);
    let refreshed_unknown_evidence = side_effect_evidence(
        refreshed_unknown_artifact.clone(),
        refreshed_unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let refreshed_unknown_outcome = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-submission-unknown-refresh",
        vec![side_effect_submission_unknown(
            refreshed_unknown_artifact,
            refreshed_unknown_digest,
        )],
        vec![refreshed_unknown_evidence],
    )
    .expect("refresh submission unknown");
    let CommitOutcome::Appended(refreshed_unknown) = refreshed_unknown_outcome else {
        panic!("refreshed submission unknown should append");
    };
    assert_eq!(
        refreshed_unknown.events()[0].logical_key().as_str(),
        submission_result_key
    );
    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        SideEffectPhase::SubmissionUnknown {
            invocation_epoch: 1
        }
    ));

    let submission_artifact = artifact_id(113);
    let submission_digest = content_digest(114);
    let submission_evidence = side_effect_evidence(
        submission_artifact.clone(),
        submission_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    let observed_outcome = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-submission-observed-after-unknown",
        vec![side_effect_submission_observed(
            submission_artifact,
            submission_digest,
        )],
        vec![submission_evidence],
    )
    .expect("recover observed submission");
    let CommitOutcome::Appended(observed) = observed_outcome else {
        panic!("observed submission should append");
    };
    assert_eq!(
        observed.events()[0].logical_key().as_str(),
        submission_result_key
    );
    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        SideEffectPhase::SubmissionObserved {
            invocation_epoch: 1
        }
    ));

    let duplicate_artifact = artifact_id(115);
    let duplicate_digest = content_digest(116);
    let duplicate_evidence = side_effect_evidence(
        duplicate_artifact.clone(),
        duplicate_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    let duplicate = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-duplicate-observed-after-recovery",
        vec![side_effect_submission_observed(
            duplicate_artifact,
            duplicate_digest,
        )],
        vec![duplicate_evidence],
    )
    .expect_err("duplicate observed submission rejects after recovery");
    assert!(matches!(
        duplicate,
        StoreError::LogicalKeyConflict { .. } | StoreError::ProjectionConflict { .. }
    ));
}
