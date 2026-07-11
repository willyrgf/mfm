use super::*;

#[tokio::test]
async fn side_effect_scheduler_commits_durable_ledger_phases_before_output() {
    let fixture = fixture_with_first_side_effect_state();
    let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;

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
            fixture,
            Advanced,
            "drive side effect phase"
        );
        let projection_snapshot = store.projection_snapshot();
        let projection = side_effect_projection_for_attempt(
            &fixture.runtime_spec,
            &fixture.run_id,
            &projection_snapshot,
            node,
            &attempt_id,
        )
        .expect("projection lookup")
        .expect("side-effect projection");
        if matches!(
            projection.phase,
            store::SideEffectPhase::ReceiptObserved { .. }
        ) && projection_snapshot
            .cell_terminal(&verify_node.output_cell)
            .is_none()
        {
            receipt_before_output = true;
            break;
        }
    }
    assert!(receipt_before_output);
    assert!(matches!(
        store.projection_snapshot().cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Skipped { .. })
    ));
    assert!(store
        .projection_snapshot()
        .cell_terminal(&verify_node.output_cell)
        .is_none());

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "materialize side-effect output"
    );

    assert!(store
        .projection_snapshot()
        .cell_terminal(&verify_node.output_cell)
        .is_some());
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        &fixture.run_id,
        &projection_snapshot,
        node,
        &attempt_id,
    )
    .expect("projection lookup")
    .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::ReceiptObserved { .. }
    ));
    assert!(store
        .load_run_stream(&fixture.run_id)
        .iter()
        .any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )));
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &verify_node.node_id),
        1
    );
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
                    3,
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
                    4,
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
                    4,
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
    drive_ok!(scheduler, store, fixture, "prepare and start side effect");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    append_not_submitted_proven(&mut store, &fixture, node, &attempt_id, 1);

    drive_ok!(scheduler, store, fixture, "resume not-submitted");
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        &fixture.run_id,
        &projection_snapshot,
        node,
        &attempt_id,
    )
    .expect("projection lookup")
    .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::NotSubmittedProven { .. }
    ));
    assert!(matches!(
        projection_snapshot.cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Skipped { .. })
    ));
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
}

#[tokio::test]
async fn side_effect_staged_artifact_must_match_payload_ledger_binding() {
    struct WrongLedgerStagedSideEffectRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    impl ErasedNodeRunner for WrongLedgerStagedSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let ledger = side_effect_ledger_key_for_ctx(&ctx);
                let ledger_purpose = side_effect_ledger_purpose_for_ctx(&ctx);
                let (pair_id, pair_role) = side_effect_pair_fields_for_ctx(
                    &ctx,
                    &ledger_purpose,
                    events::SideEffectPairRole::Submit,
                );
                let staged_ledger =
                    events::SideEffectLedgerKey::new("wrong-ledger").expect("ledger key");
                let intent_bytes = br#"{"intent":"wrong-ledger"}"#.to_vec();
                let intent_hash = digest_for_bytes(&intent_bytes);
                let intent_artifact_id =
                    ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
                let evidence = store::ArtifactEvidenceRef {
                    artifact_id: intent_artifact_id.clone(),
                    digest: intent_hash.clone(),
                    byte_len: intent_bytes.len() as u64,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.node().config_ref.schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::SideEffectIntent,
                };
                let intent_artifact_evidence_hash = evidence.evidence_hash().map_err(|error| {
                    RuntimeError::InvalidRunnerOutput(format!("intent evidence hash: {error}"))
                })?;
                let staged_artifact = StagedArtifact::inline_side_effect_artifact(
                    &ctx,
                    intent_bytes,
                    evidence,
                    staged_ledger,
                    1,
                )?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    vec![
                        RunnerEventPayload::SideEffectIntentPersisted(
                            events::side_effect::IntentPersisted {
                                spec_hash: ctx.spec_hash().clone(),
                                node_id: ctx.node().node_id.clone(),
                                scope_id: ctx.node().scope_id.clone(),
                                attempt_id: ctx.attempt_id().clone(),
                                ledger_key: ledger.clone(),
                                ledger_purpose,
                                pair_id,
                                pair_role,
                                invocation_epoch: 1,
                                intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                                intent_hash,
                                intent_artifact_id,
                                intent_artifact_evidence_hash,
                                idempotency_input_schema_id: ctx
                                    .node()
                                    .config_ref
                                    .schema_id
                                    .clone(),
                                idempotency_input_hash: content(0xc3),
                                idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                                    .expect("idempotency key"),
                                capability_kind: self.cap_kind.clone(),
                                capability_version: self.cap_version.clone(),
                                adapter_kind: self.adapter_kind.clone(),
                                adapter_version: self.adapter_version.clone(),
                            },
                        ),
                        side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                        side_effect_prepared(&ctx, ledger, 1, 1),
                    ],
                ))
            })
        }
    }

    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            WrongLedgerStagedSideEffectRunner {
                cap_kind: side_effect_capability_kind(),
                cap_version: side_effect_capability_version(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
            },
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, &fixture);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
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
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    for _ in 0..3 {
        assert_drive!(scheduler, store, fixture, Advanced, "advance to ambiguity");
    }
    assert_drive!(
        scheduler,
        store,
        fixture,
        PublicOutputProjected,
        "ambiguous side effect resolves terminal"
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
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

    for _ in 0..3 {
        drive_ok!(scheduler, store, fixture, "advance to ambiguity");
    }
    for _ in 0..8 {
        let status = drive_ok!(scheduler, store, fixture, "ambiguity resolves terminal");
        assert!(
            matches!(
                status,
                SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
            ),
            "unexpected ambiguity terminal status: {status:?}"
        );
        if store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .is_some()
        {
            break;
        }
    }
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_none());
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn side_effect_output_before_terminal_evidence_is_rejected() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
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
    register_read_external_fixture_runner(&mut registry, &fixture);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
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

        let snapshot = store.projection_snapshot();
        let validation = side_effect_lifecycle::validate_terminal_batch_evidence(
            &fixture.runtime_spec,
            &fixture.run_id,
            &snapshot,
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
        let historical = validate_runtime_stream_for_tests(
            &fixture.runtime_spec,
            &fixture.run_id,
            &store.load_run_stream(&fixture.run_id),
        );
        match case {
            Case::Receipt => {
                historical.expect("historical validation permits receipt-terminal verify output");
                assert!(store
                    .projection_snapshot()
                    .attempt(&submit_node.node_id, &submit_attempt_id)
                    .is_some());
            }
            Case::Finalized => {
                let error = historical.expect_err("historical validation requires confirmation");
                assert!(error
                    .to_string()
                    .contains("produced output before certified terminal evidence"));
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
