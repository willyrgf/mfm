use super::*;

#[test]
fn resource_lane_rejects_same_key_for_other_ledgers_same_run_and_cross_run() {
    let mut store = StoreContractRunStore::new();
    let run_a = run_id(201);
    let run_b = run_id(202);
    ensure_test_run_admitted(&mut store, &run_a, "resource-run-a-start");
    ensure_test_run_admitted(&mut store, &run_b, "resource-run-b-start");

    append_side_effect_prepare_for_ledger(
        &mut store,
        &run_a,
        "resource-a-prepare",
        side_effect_ledger_key_with_suffix(1),
        resource_key("wallet-1", 201),
        20,
        true,
    );

    let same_run_outcome = {
        let branch_node = submit_node_id();
        let branch_attempt = submit_attempt_id();
        let mut intent = side_effect_intent(artifact_id(22), content_digest(23));
        let mut claim = side_effect_claim();
        let branch_resource_key = resource_key("wallet-1", 201);
        let mut lane = resource_lane_claim_intent(branch_resource_key.clone());
        let mut prepared =
            side_effect_prepared_with_resource_key(1, "token-1", branch_resource_key);
        for payload in [&mut intent, &mut claim, &mut lane, &mut prepared] {
            set_side_effect_ledger(
                payload,
                side_effect_ledger_key_with_suffix(2),
                events::SideEffectLedgerPurpose::Forward,
            );
            set_side_effect_node_attempt(payload, branch_node.clone(), branch_attempt.clone());
        }
        append_certified_side_effect_commit(
            &mut store,
            &run_a,
            "resource-a-conflict",
            vec![intent, claim, lane, prepared],
            vec![intent_artifact_ref_for_node(
                artifact_id(22),
                content_digest(23),
                branch_node,
            )],
        )
        .expect_err("same-run second forward ledger rejects")
    };
    assert_projection_conflict_contains(
        same_run_outcome,
        "side-effect holder already has active resource lane",
    );

    let cross_run_outcome = {
        let branch_node = submit_node_id();
        let branch_attempt = attempt_id(82);
        let mut intent = side_effect_intent(artifact_id(24), content_digest(25));
        let mut claim = side_effect_claim();
        let branch_resource_key = resource_key("wallet-1", 201);
        let mut lane = resource_lane_claim_intent(branch_resource_key.clone());
        let mut prepared =
            side_effect_prepared_with_resource_key(1, "token-1", branch_resource_key);
        for payload in [&mut intent, &mut claim, &mut lane, &mut prepared] {
            set_side_effect_ledger(
                payload,
                side_effect_ledger_key_with_suffix(3),
                events::SideEffectLedgerPurpose::Forward,
            );
            set_side_effect_node_attempt(payload, branch_node.clone(), branch_attempt.clone());
        }
        append_default_commit(
            &mut store,
            &run_b,
            "resource-b-conflict-attempt-start",
            vec![side_effect_attempt_started_for(
                branch_node.clone(),
                branch_attempt.clone(),
            )],
            Vec::new(),
        )
        .expect("append branch attempt start");
        append_certified_side_effect_commit(
            &mut store,
            &run_b,
            "resource-b-conflict",
            vec![intent, claim, lane, prepared],
            vec![intent_artifact_ref_for_node(
                artifact_id(24),
                content_digest(25),
                branch_node,
            )],
        )
        .expect("cross-run lane conflict blocks")
    };
    assert_resource_lane_blocked(cross_run_outcome, &resource_lane_key("wallet-1"));
}

#[test]
fn resource_lane_holder_identity_includes_run_for_same_ledger_key_across_runs() {
    let mut store = StoreContractRunStore::new();
    let run_a = run_id(213);
    let run_b = run_id(214);
    let shared_ledger = side_effect_ledger_key_with_suffix(11);
    let lane_a = resource_lane_key_with_schema("wallet-6", 213);
    let lane_b = resource_lane_key_with_schema("wallet-7", 214);
    ensure_test_run_admitted(&mut store, &run_a, "resource-same-ledger-a-start");
    ensure_test_run_admitted(&mut store, &run_b, "resource-same-ledger-b-start");

    append_side_effect_prepare_for_ledger(
        &mut store,
        &run_a,
        "resource-same-ledger-a-prepare",
        shared_ledger.clone(),
        resource_key("wallet-6", 213),
        28,
        true,
    );

    let artifact_id = artifact_id(30);
    let artifact_digest = content_digest(31);
    let branch_node = submit_node_id();
    let branch_attempt = attempt_id(82);
    let mut intent = side_effect_intent(artifact_id.clone(), artifact_digest.clone());
    let mut claim = side_effect_claim();
    let branch_resource_key = resource_key("wallet-6", 213);
    let mut lane = resource_lane_claim_intent(branch_resource_key.clone());
    let mut prepared = side_effect_prepared_with_resource_key(1, "token-1", branch_resource_key);
    for payload in [&mut intent, &mut claim, &mut lane, &mut prepared] {
        set_side_effect_ledger(
            payload,
            shared_ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
        set_side_effect_node_attempt(payload, branch_node.clone(), branch_attempt.clone());
    }
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_b.clone(),
            expected_next_seq: store.expected_next_seq(&run_b),
            commit_key: CommitKey::new("resource-same-ledger-b-conflict-attempt-start")
                .expect("commit key"),
            payloads: vec![side_effect_attempt_started_for(
                branch_node.clone(),
                branch_attempt.clone(),
            )],
            required_artifacts: Vec::new(),
            preconditions: store.certified_preconditions(&run_b),
        })
        .expect("append branch attempt start");
    let cross_run_same_resource_outcome = append_certified_side_effect_commit(
        &mut store,
        &run_b,
        "resource-same-ledger-b-conflict",
        vec![intent, claim, lane, prepared],
        vec![intent_artifact_ref_for_node(
            artifact_id,
            artifact_digest,
            branch_node,
        )],
    )
    .expect("same resource remains exclusive across runs");
    assert_resource_lane_blocked(cross_run_same_resource_outcome, &lane_a);

    append_side_effect_prepare_for_ledger_on_attempt(
        &mut store,
        &run_b,
        "resource-same-ledger-b-prepare",
        shared_ledger.clone(),
        resource_key("wallet-7", 214),
        32,
        false,
        submit_node_id(),
        attempt_id(82),
    );

    let projection = store.projection_snapshot();
    let lane_a = projection.resource_lane(&lane_a).expect("run a lane");
    assert_eq!(lane_a.holder.run_id, run_a);
    assert_eq!(lane_a.holder.pair_id, side_effect_pair_id());
    assert_eq!(lane_a.ledger_key, shared_ledger);
    let lane_b = projection.resource_lane(&lane_b).expect("run b lane");
    assert_eq!(lane_b.holder.run_id, run_b);
    assert_eq!(lane_b.holder.pair_id, side_effect_pair_id());
    assert_eq!(lane_b.ledger_key, shared_ledger);
    assert!(projection
        .side_effect_for_pair(&run_a, &side_effect_pair_id())
        .is_some());
    assert!(projection
        .side_effect_for_pair(&run_b, &side_effect_pair_id())
        .is_some());
}

#[test]
fn resource_lane_allows_same_ledger_refresh_with_same_key_only() {
    let run = run_id(203);
    let ledger = side_effect_ledger_key_with_suffix(4);
    let mut store = admitted_store(&run, "resource-refresh-run-start");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run,
        "resource-refresh-prepare",
        ledger.clone(),
        resource_key("wallet-2", 202),
        26,
        true,
    );

    let mut takeover = side_effect_claim_taken_over_with_token("token-2");
    set_side_effect_ledger(
        &mut takeover,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    let mut prepared =
        side_effect_prepared_with_resource_key(2, "token-2", resource_key("wallet-2", 202));
    set_side_effect_ledger(
        &mut prepared,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    append_certified_side_effect_commit(
        &mut store,
        &run,
        "resource-refresh-same-key",
        vec![takeover, prepared],
        Vec::new(),
    )
    .expect("same ledger may refresh the same resource key");

    let mut takeover = side_effect_claim_taken_over_generation(2, 3, "token-3");
    let KernelEventPayload::SideEffectClaimTakenOver(payload) = &mut takeover else {
        unreachable!("helper returns takeover payload");
    };
    payload.previous_claim_owner = events::RunnerInvocationId::new("owner-2").expect("owner");
    payload.new_claim_owner = events::RunnerInvocationId::new("owner-3").expect("owner");
    set_side_effect_ledger(
        &mut takeover,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    let changed_resource_key = resource_key("wallet-3", 203);
    let mut changed_lane = resource_lane_claim_intent(changed_resource_key.clone());
    let mut changed = side_effect_prepared_with_resource_key(3, "token-3", changed_resource_key);
    set_side_effect_ledger(
        &mut changed_lane,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    set_side_effect_ledger(
        &mut changed,
        ledger,
        events::SideEffectLedgerPurpose::Forward,
    );
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run,
        "resource-refresh-changed-key",
        vec![takeover, changed_lane, changed],
        Vec::new(),
        "already has active resource lane",
    );
}

#[test]
fn resource_lane_releases_on_ledger_terminals_and_run_terminal() {
    let run = run_id(204);
    let lane_key = resource_lane_key_with_schema("wallet-4", 204);

    let mut not_submitted_store = admitted_store(&run, "resource-not-submitted-run-start");
    let ledger = side_effect_ledger_key_with_suffix(5);
    append_side_effect_prepare_for_ledger(
        &mut not_submitted_store,
        &run,
        "resource-not-submitted-prepare",
        ledger.clone(),
        resource_key("wallet-4", 204),
        28,
        true,
    );
    let active_lane = not_submitted_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .expect("active lane");
    assert_eq!(active_lane.holder.pair_id, side_effect_pair_id());
    let mut started = side_effect_started("owner-1", 1, "token-1");
    set_side_effect_ledger(
        &mut started,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    let release = resource_lane_release_intent_from_projection(
        &not_submitted_store,
        &run,
        &ledger,
        "side_effect.not_submitted",
    );
    let mut not_submitted = side_effect_not_submitted(artifact_id(30), content_digest(31));
    set_side_effect_ledger(
        &mut not_submitted,
        ledger,
        events::SideEffectLedgerPurpose::Forward,
    );
    append_certified_side_effect_commit(
        &mut not_submitted_store,
        &run,
        "resource-not-submitted-terminal",
        vec![started, release, not_submitted],
        vec![side_effect_evidence(
            artifact_id(30),
            content_digest(31),
            not_submitted_schema(),
            ArtifactRole::NotSubmittedProof,
        )],
    )
    .expect("not-submitted releases lane");
    assert!(not_submitted_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let run = run_id(205);
    let mut confirmation_store = admitted_store(&run, "resource-confirmation-run-start");
    let ledger = side_effect_ledger_key_with_suffix(6);
    append_side_effect_prepare_for_ledger(
        &mut confirmation_store,
        &run,
        "resource-confirmation-prepare",
        ledger.clone(),
        resource_key("wallet-4", 204),
        32,
        true,
    );
    append_side_effect_verify_attempt_started(&mut confirmation_store, &run);
    for (commit_key, mut payload, artifact) in [
        (
            "resource-confirmation-start",
            side_effect_started("owner-1", 1, "token-1"),
            None,
        ),
        (
            "resource-confirmation-submission",
            side_effect_submission_observed(artifact_id(34), content_digest(35)),
            Some((
                artifact_id(34),
                content_digest(35),
                submission_schema(),
                ArtifactRole::Submission,
            )),
        ),
        (
            "resource-confirmation-receipt",
            side_effect_receipt(artifact_id(36), content_digest(37)),
            Some((
                artifact_id(36),
                content_digest(37),
                receipt_schema(),
                ArtifactRole::Receipt,
            )),
        ),
        (
            "resource-confirmation-confirmed",
            side_effect_confirmation(artifact_id(38), content_digest(39)),
            Some((
                artifact_id(38),
                content_digest(39),
                confirmation_schema(),
                ArtifactRole::Confirmation,
            )),
        ),
    ] {
        set_side_effect_ledger(
            &mut payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
        let required_artifacts = artifact
            .map(|(artifact_id, digest, schema_id, role)| {
                vec![side_effect_evidence(artifact_id, digest, schema_id, role)]
            })
            .unwrap_or_default();
        let mut payloads = Vec::new();
        if commit_key == "resource-confirmation-confirmed" {
            payloads.push(resource_lane_release_intent_from_projection(
                &confirmation_store,
                &run,
                &ledger,
                "side_effect.confirmed",
            ));
        }
        payloads.push(payload);
        append_certified_side_effect_commit(
            &mut confirmation_store,
            &run,
            commit_key,
            payloads,
            required_artifacts,
        )
        .expect("append confirmation path event");
    }
    assert!(confirmation_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let run = run_id(206);
    let mut failure_store = admitted_store(&run, "resource-failed-run-start");
    let ledger = side_effect_ledger_key_with_suffix(7);
    append_side_effect_prepare_for_ledger(
        &mut failure_store,
        &run,
        "resource-failed-prepare",
        ledger.clone(),
        resource_key("wallet-4", 204),
        40,
        true,
    );
    append_side_effect_verify_attempt_started(&mut failure_store, &run);
    let release = resource_lane_release_intent_from_projection(
        &failure_store,
        &run,
        &ledger,
        "side_effect.failed",
    );
    let mut failed = side_effect_failed(true);
    set_side_effect_ledger(
        &mut failed,
        ledger,
        events::SideEffectLedgerPurpose::Forward,
    );
    append_certified_side_effect_commit(
        &mut failure_store,
        &run,
        "resource-failed-terminal",
        vec![release, failed, side_effect_attempt_failed(true)],
        Vec::new(),
    )
    .expect("failed releases lane");
    assert!(failure_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let manual_policy = manual_saga_policy(46);
    let run = run_id_with_saga_policy(207, &manual_policy);
    let mut manual_store =
        admitted_store_with_saga_policy(&run, "resource-manual-run-start", &manual_policy);
    let ledger = side_effect_ledger_key_with_suffix(8);
    append_side_effect_prepare_for_ledger(
        &mut manual_store,
        &run,
        "resource-manual-prepare",
        ledger.clone(),
        resource_key("wallet-4", 204),
        42,
        true,
    );
    append_side_effect_verify_attempt_started(&mut manual_store, &run);
    let mut started = side_effect_started("owner-1", 1, "token-1");
    let mut ambiguous = side_effect_ambiguous(artifact_id(44), content_digest(45));
    for payload in [&mut started, &mut ambiguous] {
        set_side_effect_ledger(
            payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    append_certified_side_effect_commit(
        &mut manual_store,
        &run,
        "resource-manual-ambiguous",
        vec![started, ambiguous, side_effect_attempt_failed(false)],
        vec![side_effect_evidence(
            artifact_id(44),
            content_digest(45),
            schema_id("mfm.test.ambiguity", 76),
            ArtifactRole::AmbiguityEvidence,
        )],
    )
    .expect("ambiguous terminal evidence keeps lane");
    assert!(manual_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());
    let records = manual_store.committed_records_for_projection_test(&run);
    let rebuilt = ProjectionSnapshot::rebuild_from_run_stream(&records)
        .expect("rebuild ambiguous terminal records");
    assert!(rebuilt.resource_lane(&lane_key).is_some());

    let run = run_id(208);
    let mut terminal_store = admitted_store(&run, "resource-run-terminal-start");
    let ledger = side_effect_ledger_key_with_suffix(9);
    append_side_effect_prepare_for_ledger(
        &mut terminal_store,
        &run,
        "resource-run-terminal-prepare",
        ledger.clone(),
        resource_key("wallet-4", 204),
        48,
        true,
    );
    assert!(terminal_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());
    let mut release_payloads = vec![
        resource_lane_release_intent_from_projection(
            &terminal_store,
            &run,
            &ledger,
            "run.completed",
        ),
        run_completed_for_run(run.clone(), completed_outcome(209)),
    ];
    terminal_store.certify_payloads_for_run(&run, &mut release_payloads);
    let release_error = terminal_store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: terminal_store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-run-terminal-release").expect("commit key"),
            payloads: release_payloads,
            required_artifacts: Vec::new(),
            preconditions: terminal_store.certified_preconditions(&run),
        })
        .expect_err("run terminal cannot release lane");
    assert_invalid_prepared_commit_contains(
        release_error,
        "resource lane release must precede a matching terminal attempt payload",
    );
    let mut completion_payloads = vec![run_completed_for_run(run.clone(), completed_outcome(209))];
    terminal_store.certify_payloads_for_run(&run, &mut completion_payloads);
    let completion_error = terminal_store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: terminal_store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-run-terminal-active-lane").expect("commit key"),
            payloads: completion_payloads,
            required_artifacts: Vec::new(),
            preconditions: terminal_store.certified_preconditions(&run),
        })
        .expect_err("run terminal rejects active lane");
    assert_projection_conflict_contains(
        completion_error,
        "saga terminal resolution requires terminal saga mode",
    );
    assert!(terminal_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());
}

#[test]
fn resource_lane_reprepare_after_release_requires_stable_resource_key_evidence() {
    let run = run_id(212);
    let ledger = side_effect_ledger_key_with_suffix(12);
    let lane_key = resource_lane_key_with_schema("wallet-stable", 212);
    let mut store = admitted_store(&run, "resource-stability-run-start");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run,
        "resource-stability-prepare",
        ledger.clone(),
        resource_key("wallet-stable", 212),
        52,
        true,
    );
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());

    let mut started = side_effect_started("owner-1", 1, "token-1");
    let mut not_submitted = side_effect_not_submitted(artifact_id(54), content_digest(55));
    for payload in [&mut started, &mut not_submitted] {
        set_side_effect_ledger(
            payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    let release = resource_lane_release_intent_from_projection(
        &store,
        &run,
        &ledger,
        "side_effect.not_submitted",
    );
    append_certified_side_effect_commit(
        &mut store,
        &run,
        "resource-stability-release",
        vec![started, release, not_submitted],
        vec![side_effect_evidence(
            artifact_id(54),
            content_digest(55),
            not_submitted_schema(),
            ArtifactRole::NotSubmittedProof,
        )],
    )
    .expect("not-submitted releases lane");
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let mut retry_claim = side_effect_claim_for_epoch(2, 2, "token-2");
    let mut retry_without_key = side_effect_prepared_for_epoch(2, 2, "token-2");
    for payload in [&mut retry_claim, &mut retry_without_key] {
        set_side_effect_ledger(
            payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run,
        "resource-stability-missing-key",
        vec![retry_claim, retry_without_key],
        Vec::new(),
        "prepared invocation must echo the held resource lane key",
    );

    let mut retry_claim = side_effect_claim_for_epoch(2, 2, "token-2");
    let changed_resource_key = resource_key("wallet-changed", 213);
    let mut retry_changed_lane =
        resource_lane_claim_intent_for_epoch(2, changed_resource_key.clone());
    let mut retry_changed_key = side_effect_prepared_with_resource_key_for_epoch(
        2,
        2,
        "token-2",
        Some(changed_resource_key),
    );
    for payload in [
        &mut retry_claim,
        &mut retry_changed_lane,
        &mut retry_changed_key,
    ] {
        set_side_effect_ledger(
            payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run,
        "resource-stability-changed-key",
        vec![retry_claim, retry_changed_lane, retry_changed_key],
        Vec::new(),
        "resource lane claim changed held resource key",
    );
}

#[test]
fn resource_lane_projection_rebuilds_from_non_terminal_committed_records() {
    let run = run_id(209);
    let lane_key = resource_lane_key_with_schema("wallet-5", 205);
    let mut store = admitted_store(&run, "resource-rebuild-run-start");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run,
        "resource-rebuild-prepare",
        side_effect_ledger_key_with_suffix(10),
        resource_key("wallet-5", 205),
        50,
        true,
    );

    let records = store.committed_records_for_projection_test(&run);
    let rebuilt =
        ProjectionSnapshot::rebuild_from_run_stream(&records).expect("rebuild projection");
    let lane = rebuilt
        .resource_lane(&lane_key)
        .expect("rebuilt non-terminal lane");
    assert_eq!(lane.holder.run_id, run);
    assert_eq!(lane.holder.pair_id, side_effect_pair_id());
    assert_eq!(lane.ledger_key, side_effect_ledger_key_with_suffix(10));
}

#[test]
fn resource_touched_set_evidence_is_schema_checked_at_admission() {
    let run = run_id(210);
    let touched = resource_touched_set(52);
    let mut store = admitted_store(&run, "resource-touched-run-start");
    append_side_effect_observation_setup(&mut store, &run);
    let submission_artifact_id = artifact_id(54);
    let submission_digest = content_digest(55);
    append_certified_side_effect_commit(
        &mut store,
        &run,
        "resource-touched-submission",
        vec![side_effect_submission_observed(
            submission_artifact_id.clone(),
            submission_digest.clone(),
        )],
        vec![side_effect_evidence(
            submission_artifact_id,
            submission_digest,
            submission_schema(),
            ArtifactRole::Submission,
        )],
    )
    .expect("append submission");

    let mut receipt = side_effect_receipt(artifact_id(56), content_digest(57));
    let KernelEventPayload::SideEffectReceiptObserved(payload) = &mut receipt else {
        unreachable!("helper returns receipt");
    };
    payload.resource_touched_set = Some(touched.clone());
    let mut wrong_touched_artifact = resource_touched_set_artifact_ref(&touched);
    wrong_touched_artifact.schema_id = Some(schema_id("mfm.test.wrong_touched_set", 53));
    let wrong_touched_hash = wrong_touched_artifact
        .evidence_hash()
        .expect("wrong touched evidence hash");
    if let Some(touched_set) = payload.resource_touched_set.as_mut() {
        // Pin the exact admitted (wrong) evidence key; schema mismatch is validated separately.
        touched_set.evidence_artifact_evidence_hash = wrong_touched_hash;
        touched_set.evidence_schema_id = schema_id("mfm.test.touched_set", 53);
    }
    let mut wrong_payloads = vec![receipt.clone()];
    store.certify_payloads_for_run(&run, &mut wrong_payloads);
    let error = store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run.clone(),
                expected_next_seq: store.expected_next_seq(&run),
                commit_key: CommitKey::new("resource-touched-wrong-schema").expect("commit key"),
                payloads: wrong_payloads,
                required_artifacts: vec![
                    side_effect_evidence(
                        artifact_id(56),
                        content_digest(57),
                        receipt_schema(),
                        ArtifactRole::Receipt,
                    ),
                    wrong_touched_artifact.clone(),
                ],
                preconditions: store.certified_preconditions(&run),
            },
            vec![
                side_effect_evidence(
                    artifact_id(56),
                    content_digest(57),
                    receipt_schema(),
                    ArtifactRole::Receipt,
                ),
                wrong_touched_artifact,
            ],
        )
        .expect_err("touched-set schema mismatch rejects");
    assert_invalid_prepared_commit_contains(error, "schema_id");

    // Rebuild a clean receipt for the valid admission path (wrong-schema attempt mutated it).
    let mut receipt = side_effect_receipt(artifact_id(56), content_digest(57));
    let KernelEventPayload::SideEffectReceiptObserved(payload) = &mut receipt else {
        unreachable!("helper returns receipt");
    };
    payload.resource_touched_set = Some(touched.clone());
    let mut valid_payloads = vec![receipt];
    store.certify_payloads_for_run(&run, &mut valid_payloads);
    store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run.clone(),
                expected_next_seq: store.expected_next_seq(&run),
                commit_key: CommitKey::new("resource-touched-valid").expect("commit key"),
                payloads: valid_payloads,
                required_artifacts: vec![
                    side_effect_evidence(
                        artifact_id(56),
                        content_digest(57),
                        receipt_schema(),
                        ArtifactRole::Receipt,
                    ),
                    resource_touched_set_artifact_ref(&touched),
                ],
                preconditions: store.certified_preconditions(&run),
            },
            vec![
                side_effect_evidence(
                    artifact_id(56),
                    content_digest(57),
                    receipt_schema(),
                    ArtifactRole::Receipt,
                ),
                resource_touched_set_artifact_ref(&touched),
            ],
        )
        .expect("valid touched-set evidence admits");
    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run, &side_effect_pair_id())
        .expect("side-effect projection");
    assert_eq!(projection.resource_touched_set, Some(touched));
}
