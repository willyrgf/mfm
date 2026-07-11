use super::*;

#[test]
fn manual_resolution_requires_manual_blocked_prefix_and_is_unique() {
    let manual_policy = proof_manual_saga_policy();
    let compensate_policy = compensate_saga_policy();
    let non_quiescent_run_id = run_id_with_saga_policy(220, &manual_policy);
    let mut non_quiescent = admitted_store_with_saga_policy(
        &non_quiescent_run_id,
        "manual-quiescence-run-start",
        &manual_policy,
    );
    append_side_effect_prepare(&mut non_quiescent, &non_quiescent_run_id);
    append_side_effect_started(&mut non_quiescent, &non_quiescent_run_id);
    append_generic_nonretryable_failure(
        &mut non_quiescent,
        &non_quiescent_run_id,
        "manual-quiescence",
    );
    let verified = verified_manual_resolution_for_run_seq(
        &non_quiescent_run_id,
        non_quiescent
            .expected_next_seq(&non_quiescent_run_id)
            .as_u64(),
    );
    let prepared = prepared_manual_resolution_commit(
        &verified,
        non_quiescent.expected_next_seq(&non_quiescent_run_id),
        "manual-non-quiescent",
        manual_policy.clone(),
    );
    let error = non_quiescent
        .append_test_commit_plan(prepared.into())
        .expect_err("manual resolution rejects with open non-quiescent attempt");
    assert_projection_conflict_contains(error, "requires no open semantic attempts");

    let remediating_run_id = run_id_with_saga_policy(221, &compensate_policy);
    let mut remediating = admitted_store_with_saga_policy(
        &remediating_run_id,
        "manual-remediating-run-start",
        &compensate_policy,
    );
    append_forward_confirmation(&mut remediating, &remediating_run_id);
    append_generic_nonretryable_failure(
        &mut remediating,
        &remediating_run_id,
        "manual-remediating",
    );
    let error = remediating
        .append_prepared_commit(manual_resolution_request(
            &remediating_run_id,
            remediating.expected_next_seq(&remediating_run_id),
            "manual-remediating-reject",
            151,
            compensate_policy.clone(),
        ))
        .expect_err("manual resolution rejects while remediating");
    assert_invalid_prepared_commit_contains(error, "requires verified manual resolution proof");

    let raw_error = remediating
        .append_prepared_commit(manual_resolution_request(
            &remediating_run_id,
            remediating.expected_next_seq(&remediating_run_id),
            "manual-raw-without-proof",
            151,
            compensate_policy,
        ))
        .expect_err("raw manual resolution rejects without proof");
    assert_invalid_prepared_commit_contains(raw_error, "requires verified manual resolution proof");

    let run_id = run_id_with_saga_policy(222, &manual_policy);
    let mut store = admitted_store_with_saga_policy(&run_id, "manual-run-start", &manual_policy);
    append_manual_blocked_forward_failure(&mut store, &run_id, "manual-clean-sidefx-failures");
    let verified =
        verified_manual_resolution_for_run_seq(&run_id, store.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&run_id),
        "manual-recorded",
        manual_policy.clone(),
    );
    store
        .append_test_commit_plan(prepared.into())
        .expect("manual resolution admitted in manual-blocked mode");
    let verified =
        verified_manual_resolution_for_run_seq(&run_id, store.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&run_id),
        "manual-duplicate",
        manual_policy,
    );
    let error = store
        .append_test_commit_plan(prepared.into())
        .expect_err("duplicate manual resolution rejects");
    match error {
        StoreError::LogicalKeyConflict { .. } | StoreError::DuplicateLogicalKey { .. } => {}
        StoreError::ProjectionConflict { message, .. }
            if message.contains("manual resolution already recorded") => {}
        other => panic!("unexpected duplicate manual resolution error: {other:?}"),
    }
}

#[test]
fn manual_resolution_prepared_commit_requires_matching_verified_proof() {
    let expected_next_seq = StreamSeq::new(7).expect("stream seq");
    let verified = verified_manual_resolution_for_seq(expected_next_seq.as_u64());
    let request = manual_resolution_request_from_verified(
        &verified,
        expected_next_seq,
        "manual-without-proof",
        proof_manual_saga_policy(),
    );
    let artifacts = manual_resolution_artifacts_from_verified(&verified);
    let error = test_prepared_commit_plan(request.clone(), artifacts.clone())
        .expect_err("manual resolution rejects without proof authority");
    assert_invalid_prepared_commit_contains(error, "requires verified manual resolution proof");

    let stale_request = manual_resolution_request_from_verified(
        &verified,
        StreamSeq::new(8).expect("stream seq"),
        "manual-stale-proof",
        proof_manual_saga_policy(),
    );
    let error = PreparedCommit::<ManualResolution>::new(
        stale_request,
        CommitArtifactEvidenceSet::new(artifacts.clone(), artifacts.clone())
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect_err("stale proof rejects");
    assert_invalid_prepared_commit_contains(error, "expected_next_seq");

    let mut mismatched_payload = manual_resolution_payload_from_verified(&verified);
    let KernelEventPayload::ManualResolutionRecorded(payload) = &mut mismatched_payload else {
        unreachable!("helper returns manual resolution payload");
    };
    payload.evidence_hash = content_digest(202);
    let mut mismatched_artifacts = manual_resolution_artifacts_from_verified(&verified);
    mismatched_artifacts[0].digest = content_digest(202);
    payload.evidence_artifact_evidence_hash = mismatched_artifacts[0]
        .evidence_hash()
        .expect("mismatched manual evidence hash");
    let mismatched_request = CommitRequest::from_payloads(
        verified.claim().run_id.clone(),
        expected_next_seq,
        CommitKey::new("manual-mismatched-proof").expect("commit key"),
        vec![mismatched_payload],
        mismatched_artifacts.clone(),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            ..saga_preconditions(&verified.claim().run_id, proof_manual_saga_policy())
        },
    )
    .expect("manual resolution request");
    let error = PreparedCommit::<ManualResolution>::new(
        mismatched_request,
        CommitArtifactEvidenceSet::new(mismatched_artifacts.clone(), mismatched_artifacts)
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect_err("mismatched proof rejects");
    assert_invalid_prepared_commit_contains(error, "artifact refs do not match manual proof");
}

#[test]
fn manual_resolution_open_attempt_scope_is_run_local() {
    enum Case {
        SameRunRejects,
        UnrelatedRunAdmits,
    }

    for case in [Case::SameRunRejects, Case::UnrelatedRunAdmits] {
        let policy = proof_manual_saga_policy();
        match case {
            Case::SameRunRejects => {
                let run_id = run_id_with_saga_policy(220, &policy);
                let mut store = admitted_store_with_saga_policy(
                    &run_id,
                    "manual-open-attempt-run-start",
                    &policy,
                );
                append_forward_confirmation(&mut store, &run_id);
                append_default_commit(
                    &mut store,
                    &run_id,
                    "manual-open-attempt-sidefx-failure",
                    vec![side_effect_attempt_failed(false)],
                    Vec::new(),
                )
                .expect("append confirmed side-effect attempt failure");
                append_default_commit(
                    &mut store,
                    &run_id,
                    "manual-unrelated-open-attempt",
                    vec![fact_attempt_started()],
                    Vec::new(),
                )
                .expect("append unrelated open attempt");

                let verified = verified_manual_resolution_for_run_seq(
                    &run_id,
                    store.expected_next_seq(&run_id).as_u64(),
                );
                let prepared = prepared_manual_resolution_commit(
                    &verified,
                    store.expected_next_seq(&run_id),
                    "manual-open-attempt-reject",
                    policy,
                );
                let error = store
                    .append_test_commit_plan(prepared.into())
                    .expect_err("manual resolution rejects open attempt");
                assert_projection_conflict_contains(error, "requires no open semantic attempts");
            }
            Case::UnrelatedRunAdmits => {
                let manual_run_id = run_id_with_saga_policy(220, &policy);
                let other_run_id = run_id_with_saga_policy(221, &policy);
                let mut store = StoreContractRunStore::new();
                append_run_start_with_saga_policy(
                    &mut store,
                    &manual_run_id,
                    "manual-cross-run-start",
                    &policy,
                );
                append_manual_blocked_forward_failure(
                    &mut store,
                    &manual_run_id,
                    "manual-cross-run-sidefx-failures",
                );

                append_run_start_with_saga_policy(
                    &mut store,
                    &other_run_id,
                    "manual-cross-run-other-start",
                    &policy,
                );
                append_default_commit(
                    &mut store,
                    &other_run_id,
                    "manual-cross-run-other-open-attempt",
                    vec![fact_attempt_started()],
                    Vec::new(),
                )
                .expect("append other run open attempt");
                assert!(store
                    .projection_snapshot()
                    .open_attempt_for_run(&other_run_id)
                    .is_some());
                assert!(store
                    .projection_snapshot()
                    .open_attempt_for_run(&manual_run_id)
                    .is_none());

                let verified = verified_manual_resolution_for_run_seq(
                    &manual_run_id,
                    store.expected_next_seq(&manual_run_id).as_u64(),
                );
                let prepared = prepared_manual_resolution_commit(
                    &verified,
                    store.expected_next_seq(&manual_run_id),
                    "manual-cross-run-admit",
                    policy,
                );
                store
                    .append_test_commit_plan(prepared.into())
                    .expect("manual resolution ignores unrelated run open attempt");
                assert!(store
                    .projection_snapshot()
                    .manual_resolution(&manual_run_id)
                    .is_some());
                assert!(store
                    .projection_snapshot()
                    .open_attempt_for_run(&other_run_id)
                    .is_some());
            }
        }
    }
}

#[test]
fn manual_resolution_rejects_open_side_effect_lane_without_releasing_it() {
    let policy = proof_manual_saga_policy();
    let run_id = run_id_with_saga_policy(220, &policy);
    let lane_key = resource_lane_key_with_schema("wallet-open", 220);
    let open_ledger = side_effect_ledger_key_with_suffix(31);
    let mut store = admitted_store_with_saga_policy(&run_id, "manual-open-lane-run-start", &policy);
    append_side_effect_prepare_for_ledger_on_attempt(
        &mut store,
        &run_id,
        "manual-open-lane-prepare",
        open_ledger.clone(),
        resource_key("wallet-open", 220),
        70,
        true,
        submit_node_id(),
        submit_attempt_id(),
    );
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());

    let verified =
        verified_manual_resolution_for_run_seq(&run_id, store.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&run_id),
        "manual-open-lane-reject",
        policy,
    );
    let error = store
        .append_test_commit_plan(prepared.into())
        .expect_err("manual resolution rejects open side-effect lane attempt");
    assert_projection_conflict_contains(error, "requires no open semantic attempts");
    let lane = store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .expect("open lane remains held after rejected manual resolution");
    assert_eq!(lane.holder.pair_id, side_effect_pair_id());
    assert_eq!(lane.ledger_key, open_ledger);
}

#[test]
fn certified_run_authority_must_match_run_start_policy_digest() {
    let policy = proof_manual_saga_policy();
    let run_id = run_id_with_saga_policy(220, &policy);
    let mut store = admitted_store_with_saga_policy(&run_id, "saga-token-run-start", &policy);
    append_forward_confirmation(&mut store, &run_id);
    append_default_commit(
        &mut store,
        &run_id,
        "saga-token-sidefx-failure",
        vec![side_effect_attempt_failed(false)],
        Vec::new(),
    )
    .expect("append confirmed side-effect attempt failure");

    let verified =
        verified_manual_resolution_for_run_seq(&run_id, store.expected_next_seq(&run_id).as_u64());
    let request = manual_resolution_request_from_verified(
        &verified,
        store.expected_next_seq(&run_id),
        "saga-token-mismatch",
        manual_saga_policy(171),
    );
    let artifacts = manual_resolution_artifacts_from_verified(&verified);
    let error = PreparedCommit::<ManualResolution>::new(
        request,
        CommitArtifactEvidenceSet::new(artifacts.clone(), artifacts)
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect_err("mismatched saga token rejects before admission");
    assert_invalid_prepared_commit_contains(error, "spec hash");
}

#[test]
fn manual_resolution_artifacts_require_dedicated_roles() {
    fn reject_with(
        commit_key: &str,
        mut mutate: impl FnMut(&mut Vec<ArtifactEvidenceRef>),
        expected_field: &'static str,
    ) {
        let policy = proof_manual_saga_policy();
        let run_id = run_id_with_saga_policy(220, &policy);
        let mut store =
            admitted_store_with_saga_policy(&run_id, "manual-artifact-role-run-start", &policy);
        append_forward_confirmation(&mut store, &run_id);
        append_default_commit(
            &mut store,
            &run_id,
            format!("{commit_key}-sidefx-failure"),
            vec![side_effect_attempt_failed(false)],
            Vec::new(),
        )
        .expect("append confirmed side-effect attempt failure");
        let verified = verified_manual_resolution_for_run_seq(
            &run_id,
            store.expected_next_seq(&run_id).as_u64(),
        );
        let request = manual_resolution_request_from_verified(
            &verified,
            store.expected_next_seq(&run_id),
            commit_key,
            policy,
        );
        let mut required_artifacts = manual_resolution_artifacts_from_verified(&verified);
        mutate(&mut required_artifacts);
        let admitted_artifacts = required_artifacts.clone();
        // Pin exact evidence keys to the mutated admitted variants so coverage succeeds and
        // field/role validation is what fails closed. Payload schema/digest remain as built
        // from the verified claim unless the mutation itself changed digest.
        let mut payloads = request.payloads().to_vec();
        if let Some(KernelEventPayload::ManualResolutionRecorded(payload)) = payloads.first_mut() {
            // Only re-pin exact evidence keys. Content digests stay proof-aligned; field/role
            // mismatches are reported against the admitted evidence variant.
            payload.evidence_artifact_evidence_hash = required_artifacts[0]
                .evidence_hash()
                .expect("mutated manual evidence hash");
            payload.authorization_artifact_evidence_hash = required_artifacts[1]
                .evidence_hash()
                .expect("mutated manual authorization hash");
        }
        let request = CommitRequest::from_payloads(
            request.run_id().clone(),
            request.expected_next_seq(),
            request.commit_key().clone(),
            payloads,
            required_artifacts.clone(),
            request.preconditions().clone(),
        )
        .expect("manual request with mutated evidence keys");
        let error = match PreparedCommit::<ManualResolution>::new(
            request,
            CommitArtifactEvidenceSet::new(required_artifacts, admitted_artifacts)
                .expect("manual artifact evidence set"),
            &verified,
        ) {
            Ok(prepared) => store
                .append_test_commit_plan(prepared.into())
                .expect_err("manual artifact mismatch rejects"),
            Err(error) => error,
        };
        assert_invalid_prepared_commit_contains(error, expected_field);
    }

    reject_with(
        "manual-wrong-evidence-role",
        |artifacts| artifacts[0].artifact_role = ArtifactRole::StateOutput,
        "artifact_role",
    );
    reject_with(
        "manual-wrong-authorization-role",
        |artifacts| artifacts[1].artifact_role = ArtifactRole::StateOutput,
        "artifact_role",
    );
    reject_with(
        "manual-wrong-authorization-schema",
        |artifacts| artifacts[1].schema_id = Some(schema_id("mfm.test.wrong_authorization", 200)),
        "schema_id",
    );
    reject_with(
        "manual-wrong-evidence-digest",
        |artifacts| artifacts[0].digest = content_digest(202),
        "digest",
    );
}
