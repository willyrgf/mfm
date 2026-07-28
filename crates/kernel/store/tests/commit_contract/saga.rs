use super::*;

#[test]
fn saga_run_completed_requires_terminal_proof() {
    let policy = manual_saga_policy(160);
    let terminal_run = run_id_with_saga_policy(120, &policy);
    let mut store =
        admitted_store_with_saga_policy(&terminal_run, "terminal-quiescence-run-start", &policy);
    append_side_effect_prepare(&mut store, &terminal_run);
    append_side_effect_started(&mut store, &terminal_run);
    append_generic_nonretryable_failure(&mut store, &terminal_run, "terminal-quiescence");
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: terminal_run.clone(),
            expected_next_seq: store.expected_next_seq(&terminal_run),
            commit_key: CommitKey::new("terminal-non-quiescent").expect("commit key"),
            payloads: vec![run_completed_for_run_with_policy(
                terminal_run.clone(),
                events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                policy.clone(),
            )],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(&terminal_run, policy.clone()),
        })
        .expect_err("raw terminal completion rejects without proof");
    assert_invalid_prepared_commit_contains(error, "requires SagaTerminalProof");

    let forged_policy = proof_manual_saga_policy();
    let forged_run = run_id_with_saga_policy(220, &forged_policy);
    let mut forged =
        admitted_store_with_saga_policy(&forged_run, "terminal-forged-run-start", &forged_policy);
    append_manual_blocked_forward_failure(
        &mut forged,
        &forged_run,
        "terminal-forged-sidefx-failures",
    );
    let error = forged
        .append_prepared_commit(typed_commit_request! {
            run_id: forged_run.clone(),
            expected_next_seq: forged.expected_next_seq(&forged_run),
            commit_key: CommitKey::new("terminal-forged-manual").expect("commit key"),
            payloads: vec![run_completed_for_run_with_policy(
                forged_run.clone(),
                events::RunCompletionOutcome::ManuallyResolved,
                forged_policy.clone(),
            )],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(&forged_run, forged_policy.clone()),
        })
        .expect_err("raw forged manual terminal rejects without proof");
    assert_invalid_prepared_commit_contains(error, "requires SagaTerminalProof");
}

#[test]
fn saga_terminal_prepared_commit_accepts_matching_test_token() {
    let policy = SagaPolicySpec::FailWithoutAcdcClaim;
    let run_id = run_id_with_saga_policy(122, &policy);
    let mut store = admitted_store_with_saga_policy(&run_id, "terminal-proof-run-start", &policy);
    append_generic_nonretryable_failure(&mut store, &run_id, "terminal-proof-failure");
    let proof =
        forged_failed_terminal_proof(run_id.clone(), store.expected_next_seq(&run_id), &policy)
            .expect("matching failed-terminal test token");
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("terminal-proof").expect("commit key"),
        payloads: vec![run_completed_for_run_with_policy(
            run_id.clone(),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            policy.clone(),
        )],
        required_artifacts: Vec::new(),
        preconditions: saga_preconditions(&run_id, policy),
    };
    let prepared =
        PreparedCommit::<SagaTerminal>::new(request, CommitArtifactEvidenceSet::empty(), &proof)
            .expect("proof-backed saga terminal commit");
    store
        .append_test_commit_plan(prepared.into())
        .expect("append proof-backed saga terminal");
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[test]
fn saga_terminal_rejects_test_token_from_same_policy_different_spec_hash() {
    let policy = SagaPolicySpec::FailWithoutAcdcClaim;
    let run_id = run_id_with_saga_policy(127, &policy);
    let mut store =
        admitted_store_with_saga_policy(&run_id, "terminal-spec-authority-run-start", &policy);
    append_generic_nonretryable_failure(&mut store, &run_id, "terminal-spec-authority-failure");

    let proof =
        forged_failed_terminal_proof(run_id.clone(), store.expected_next_seq(&run_id), &policy)
            .expect("failed-terminal test token");

    let alternate_spec =
        saga_authority_spec_with_authoring_config_hash(policy.clone(), content_digest(189));
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

    let mut completion = run_completed_for_run_with_policy(
        run_id.clone(),
        events::RunCompletionOutcome::FailedWithoutAcdcClaim,
        policy,
    );
    let KernelEventPayload::RunCompleted(payload) = &mut completion else {
        unreachable!("helper returns run completed payload");
    };
    payload.spec_hash = alternate_spec_hash;

    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("terminal-spec-authority-reject").expect("commit key"),
        payloads: vec![completion],
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions {
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &alternate_spec)
                    .expect("alternate certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    };
    let prepared =
        PreparedCommit::<SagaTerminal>::new(request, CommitArtifactEvidenceSet::empty(), &proof)
            .expect("alternate-spec saga terminal prepared commit");
    let error = store
        .append_test_commit_plan(prepared.into())
        .expect_err("alternate-spec saga token rejects");
    assert_projection_conflict_contains(error, "spec hash does not match run start");
}

#[test]
fn saga_terminal_prepared_commit_rejects_mismatched_test_tokens() {
    #[derive(Clone, Copy)]
    enum Case {
        CrossRun,
        PolicyDigest,
        StalePrefix,
    }

    fn failed_terminal_proof(
        policy: &SagaPolicySpec,
        run_id: &RunId,
        start_key: &str,
        failure_key: &str,
    ) -> (SagaTerminalProof, StreamSeq) {
        let mut store = admitted_store_with_saga_policy(run_id, start_key, policy);
        append_generic_nonretryable_failure(&mut store, run_id, failure_key);
        let expected_next_seq = store.expected_next_seq(run_id);
        let proof = forged_failed_terminal_proof(run_id.clone(), expected_next_seq, policy)
            .expect("failed-terminal test token");
        (proof, expected_next_seq)
    }

    for case in [Case::CrossRun, Case::PolicyDigest, Case::StalePrefix] {
        let policy = SagaPolicySpec::FailWithoutAcdcClaim;
        let (request, proof, expected_error) = match case {
            Case::CrossRun => {
                let proof_run = run_id_with_saga_policy(123, &policy);
                let request_run = run_id_with_saga_policy(124, &policy);
                let (proof, _) = failed_terminal_proof(
                    &policy,
                    &proof_run,
                    "terminal-cross-run-proof-start",
                    "terminal-cross-run-proof-failure",
                );
                (
                    typed_commit_request! {
                        run_id: request_run.clone(),
                        expected_next_seq: StreamSeq::new(1).expect("stream seq"),
                        commit_key: CommitKey::new("terminal-cross-run-request")
                            .expect("commit key"),
                        payloads: vec![run_completed_for_run_with_policy(
                            request_run.clone(),
                            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                            policy.clone(),
                        )],
                        required_artifacts: Vec::new(),
                        preconditions: saga_preconditions(&request_run, policy),
                    },
                    proof,
                    "run id does not match",
                )
            }
            Case::PolicyDigest => {
                let run_id = run_id_with_saga_policy(125, &policy);
                let (proof, expected_next_seq) = failed_terminal_proof(
                    &policy,
                    &run_id,
                    "terminal-policy-proof-start",
                    "terminal-policy-proof-failure",
                );
                let mismatched_policy = compensate_saga_policy();
                (
                    typed_commit_request! {
                        run_id: run_id.clone(),
                        expected_next_seq: expected_next_seq,
                        commit_key: CommitKey::new("terminal-policy-mismatch")
                            .expect("commit key"),
                        payloads: vec![run_completed_for_run_with_policy(
                            run_id.clone(),
                            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                            mismatched_policy.clone(),
                        )],
                        required_artifacts: Vec::new(),
                        preconditions: saga_preconditions(&run_id, mismatched_policy),
                    },
                    proof,
                    "policy digest",
                )
            }
            Case::StalePrefix => {
                let run_id = run_id_with_saga_policy(126, &policy);
                let (proof, _) = failed_terminal_proof(
                    &policy,
                    &run_id,
                    "terminal-current-proof-start",
                    "terminal-current-proof-failure",
                );
                let store =
                    admitted_store_with_saga_policy(&run_id, "terminal-current-run-start", &policy);
                (
                    typed_commit_request! {
                        run_id: run_id.clone(),
                        expected_next_seq: store.expected_next_seq(&run_id),
                        commit_key: CommitKey::new("terminal-current-nonterminal")
                            .expect("commit key"),
                        payloads: vec![run_completed_for_run_with_policy(
                            run_id.clone(),
                            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                            policy.clone(),
                        )],
                        required_artifacts: Vec::new(),
                        preconditions: saga_preconditions(&run_id, policy),
                    },
                    proof,
                    "prefix",
                )
            }
        };
        let error = PreparedCommit::<SagaTerminal>::new(
            request,
            CommitArtifactEvidenceSet::empty(),
            &proof,
        )
        .expect_err("mismatched terminal proof rejects");
        assert_invalid_prepared_commit_contains(error, expected_error);
    }
}

#[test]
fn receipt_observed_forward_ledger_follows_certified_terminal_policy() {
    #[derive(Clone, Copy)]
    enum Case {
        FinalizedPolicy,
        ReceiptPolicy,
    }

    for case in [Case::FinalizedPolicy, Case::ReceiptPolicy] {
        let (
            label,
            run_id,
            start_commit_key,
            failure_commit_key,
            expected_forward_quiescent,
            expected_run_mode,
            expected_classification,
        ) = match case {
            Case::FinalizedPolicy => (
                "finalized policy",
                run_id(127),
                "receipt-quiescence-run-start",
                "receipt-finalized-pending",
                false,
                RunMode::Forward,
                ForwardLedgerClassification::Pending,
            ),
            Case::ReceiptPolicy => (
                "receipt policy",
                run_id(128),
                "receipt-terminal-run-start",
                "receipt-terminal-engagement",
                true,
                RunMode::Remediating,
                ForwardLedgerClassification::Owed,
            ),
        };
        let mut store = admitted_store(&run_id, start_commit_key);
        append_forward_receipt(&mut store, &run_id);
        append_generic_nonretryable_failure(&mut store, &run_id, failure_commit_key);

        let terminal_policies = match case {
            Case::FinalizedPolicy => {
                confirmation_terminal_policies_for_projection(store.projection_snapshot(), &run_id)
            }
            Case::ReceiptPolicy => {
                receipt_terminal_policies_for_projection(store.projection_snapshot(), &run_id)
            }
        };
        let projection = store
            .projection_snapshot()
            .derive_saga_projection(&run_id, &compensate_saga_policy(), &terminal_policies)
            .expect(label);
        assert_eq!(
            projection.forward_quiescent, expected_forward_quiescent,
            "{label}"
        );
        assert_eq!(projection.run_mode, expected_run_mode, "{label}");
        let obligation = projection
            .obligations
            .get(&side_effect_pair_id())
            .expect("receipt-observed forward obligation");
        assert_eq!(
            obligation.classification, expected_classification,
            "{label}"
        );
    }
}

#[test]
fn saga_projection_derives_obligations_and_run_mode_from_policy_and_stream() {
    let policy = SagaPolicySpec::CompensateCompleted {
        on_remediation_unresolved: RemediationUnresolvedSpec::FailWithoutAcdcClaim,
    };
    let run_id = run_id_with_saga_policy(120, &policy);
    let mut store = admitted_store_with_saga_policy(&run_id, "saga-projection-run-start", &policy);
    append_forward_confirmation(&mut store, &run_id);
    append_generic_nonretryable_failure(&mut store, &run_id, "saga-projection-engagement");
    let projection = store
        .projection_snapshot()
        .derive_saga_projection(
            &run_id,
            &policy,
            &confirmation_terminal_policies_for_projection(store.projection_snapshot(), &run_id),
        )
        .expect("saga projection");
    assert_eq!(projection.run_mode, RunMode::Remediating);
    assert!(projection.engagement.is_some());
    assert!(projection.forward_quiescent);
    let obligation = projection
        .obligations
        .get(&side_effect_pair_id())
        .expect("forward obligation");
    assert_eq!(obligation.classification, ForwardLedgerClassification::Owed);
    assert!(obligation.remediation.is_none());

    let manual_policy = SagaPolicySpec::ManualResolution {
        manual: Box::new(ManualResolutionEvidenceSpec {
            evidence_schema: schema_id("mfm.test.manual_evidence", 180),
            authorization: manual_authorization(181),
        }),
    };
    let manual_projection = store
        .projection_snapshot()
        .derive_saga_projection(
            &run_id,
            &manual_policy,
            &confirmation_terminal_policies_for_projection(store.projection_snapshot(), &run_id),
        )
        .expect("saga projection");
    assert_eq!(manual_projection.run_mode, RunMode::ManualBlocked);
    assert_eq!(
        manual_projection.manual_block_reason,
        Some(ManualBlockReason::PolicyManualResolution)
    );

    let remediation_key = remediation_ledger_key(10);
    append_remediation_confirmation(&mut store, &run_id, remediation_key.clone());
    let projection = store
        .projection_snapshot()
        .derive_saga_projection(
            &run_id,
            &policy,
            &confirmation_terminal_policies_for_projection(store.projection_snapshot(), &run_id),
        )
        .expect("saga projection");
    assert_eq!(projection.run_mode, RunMode::Compensated);
    let obligation = projection
        .obligations
        .get(&side_effect_pair_id())
        .expect("forward obligation");
    let remediation = obligation.remediation.as_ref().expect("remediation");
    assert_eq!(remediation.ledger_key, remediation_key);
    assert!(remediation.closed);
    assert_eq!(remediation.unresolved, None);
}
