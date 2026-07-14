use super::*;

pub(in crate::tests::support) fn append_synthetic_invocation_started(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append(
        store,
        commit_key,
        vec![events::KernelEventPayload::SideEffectInvocationStarted(
            events::side_effect::InvocationStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
                claim_generation: 1,
                claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                    .expect("fencing token"),
            },
        )],
        Vec::new(),
        store::RequiredSideEffectState::InvocationPrepared,
        false,
    );
}

pub(in crate::tests::support) fn append_synthetic_exclusive_started(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> (AttemptId, events::SideEffectLedgerKey) {
    let (attempt_id, ledger_key) =
        append_synthetic_exclusive_prepare(store, fixture, run_id, node, key, commit_key);
    append_synthetic_invocation_started(
        store,
        fixture,
        run_id,
        node,
        &attempt_id,
        &ledger_key,
        &format!("{commit_key}-started"),
    );
    (attempt_id, ledger_key)
}

pub(in crate::tests::support) fn append_synthetic_submission_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xd7);
    let digest = content(0xd8);
    let evidence = side_effect_evidence(
        node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Submission,
    );
    let side_effect = SyntheticSideEffectAppend::new(fixture, &fixture.run_id, node, attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append(
        store,
        commit_key,
        vec![events::KernelEventPayload::SideEffectSubmissionObserved(
            events::side_effect::SubmissionObserved {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                submission_schema_id: node.config_ref.schema_id.clone(),
                submission_hash: digest.clone(),
                submission_artifact_id: artifact_id,
                submission_artifact_evidence_hash: evidence
                    .evidence_hash()
                    .expect("submission evidence hash"),
            },
        )],
        vec![evidence],
        store::RequiredSideEffectState::InvocationStarted,
        false,
    );
}

pub(in crate::tests::support) fn append_synthetic_receipt_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    if store
        .projection_snapshot()
        .cell_terminal(&node.output_cell)
        .is_none()
    {
        append_synthetic_submit_boundary_skipped(
            store,
            fixture,
            node,
            attempt_id,
            ledger,
            &format!("{commit_key}-submit-boundary"),
        );
    }
    let verify_node = side_effect_verify_node_for_submit(fixture, node).clone();
    let verify_attempt_id = append_or_get_started_attempt(store, fixture, &verify_node, 1);
    append_synthetic_verify_receipt_observed(
        store,
        fixture,
        node,
        &verify_node,
        &verify_attempt_id,
        ledger,
        commit_key,
    );
}

pub(in crate::tests::support) fn append_synthetic_exclusive_receipt_phase(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> AttemptId {
    let (attempt_id, ledger_key) =
        append_synthetic_exclusive_started(store, fixture, &fixture.run_id, node, key, commit_key);
    append_synthetic_submission_observed(
        store,
        fixture,
        node,
        &attempt_id,
        &ledger_key,
        &format!("{commit_key}-submission"),
    );
    append_synthetic_receipt_observed(
        store,
        fixture,
        node,
        &attempt_id,
        &ledger_key,
        &format!("{commit_key}-receipt"),
    );
    attempt_id
}

pub(in crate::tests::support) fn append_synthetic_submit_boundary_skipped(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    _ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("submit output cell");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellSkipped(events::CellSkipped {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    cell_id: node.output_cell.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    context: output_cell.context.clone(),
                    skip_reason: events::SkipReason {
                        code: events::ErrorCode::new("side_effect_submission_boundary")
                            .expect("skip code"),
                        safe_message: "side-effect submit boundary recorded; verification is delegated to the paired verify node".to_owned(),
                    },
                }),
                events::KernelEventPayload::StateAttemptCompleted(
                    events::StateAttemptCompleted {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        output_cell_id: node.output_cell.clone(),
                    },
                ),
            ],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    pair_id: fixture_side_effect_pair_id(fixture, node),
                    required: store::RequiredSideEffectState::SubmissionResult,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic submit boundary skipped");
}

pub(in crate::tests::support) fn append_synthetic_verify_receipt_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    submit_node: &spec::NodeSpec,
    verify_node: &spec::NodeSpec,
    verify_attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xd9);
    let digest = content(0xda);
    let evidence = side_effect_evidence(
        verify_node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Receipt,
    );
    let side_effect =
        SyntheticSideEffectAppend::new(fixture, &fixture.run_id, verify_node, verify_attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) =
        side_effect.pair_fields(submit_node, events::SideEffectPairRole::Verify);
    side_effect.append(
        store,
        commit_key,
        vec![events::KernelEventPayload::SideEffectReceiptObserved(
            events::side_effect::ReceiptObserved {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: verify_node.node_id.clone(),
                attempt_id: verify_attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                receipt_schema_id: verify_node.config_ref.schema_id.clone(),
                receipt_hash: digest.clone(),
                receipt_artifact_id: artifact_id,
                receipt_artifact_evidence_hash: evidence
                    .evidence_hash()
                    .expect("receipt evidence hash"),
                replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.replay")
                    .expect("replay verifier"),
                resource_touched_set: None,
            },
        )],
        vec![evidence],
        store::RequiredSideEffectState::SubmissionResult,
        true,
    );
}

pub(in crate::tests::support) fn append_synthetic_verify_confirmation_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    submit_node: &spec::NodeSpec,
    verify_node: &spec::NodeSpec,
    verify_attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xdb);
    let digest = content(0xdc);
    let evidence = side_effect_evidence(
        verify_node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Confirmation,
    );
    let side_effect =
        SyntheticSideEffectAppend::new(fixture, &fixture.run_id, verify_node, verify_attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) =
        side_effect.pair_fields(submit_node, events::SideEffectPairRole::Verify);
    let release = synthetic_resource_lane_release(
        store,
        fixture,
        &fixture.run_id,
        verify_node,
        verify_attempt_id,
        ledger,
        "side_effect.confirmed",
    );
    let mut payloads = Vec::new();
    if let Some(release) = release {
        payloads.push(release);
    }
    payloads.push(events::KernelEventPayload::SideEffectConfirmationObserved(
        events::side_effect::ConfirmationObserved {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: verify_node.node_id.clone(),
            attempt_id: verify_attempt_id.clone(),
            ledger_key: ledger.clone(),
            ledger_purpose,
            pair_id,
            pair_role,
            invocation_epoch: 1,
            confirmation_schema_id: verify_node.config_ref.schema_id.clone(),
            confirmation_hash: digest.clone(),
            confirmation_artifact_id: artifact_id,
            confirmation_artifact_evidence_hash: evidence
                .evidence_hash()
                .expect("confirmation evidence hash"),
            replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.replay")
                .expect("replay verifier"),
            resource_touched_set: None,
        },
    ));
    side_effect.append(
        store,
        commit_key,
        payloads,
        vec![evidence],
        store::RequiredSideEffectState::ReceiptObserved,
        true,
    );
}

pub(in crate::tests::support) fn synthetic_resource_lane_release(
    store: &TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    _attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    reason: &str,
) -> Option<events::KernelEventPayload> {
    let pair_id = match &node.framework {
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => verify.pair_id.clone(),
        _ => fixture
            .runtime_spec
            .side_effect_pair_for_submit_node(&node.node_id)
            .cloned()
            .expect("side-effect pair"),
    };
    let holder = store::SideEffectPairLedgerRef::new(run_id.clone(), pair_id);
    let snapshot = store.projection_snapshot();
    let (_, lane) = snapshot
        .resource_lanes()
        .find(|(_, projection)| projection.holder == holder)?;
    Some(events::KernelEventPayload::ResourceLaneReleaseIntent(
        events::ResourceLaneReleaseIntent {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            ledger_key: ledger.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: lane.holder.pair_id.clone(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch: lane.invocation_epoch,
            claim_id: lane.claim_id.clone(),
            release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
            release_reason: events::ResourceLaneReleaseReason::new(reason).expect("release reason"),
        },
    ))
}

pub(in crate::tests::support) fn active_resource_lane_for_pair(
    snapshot: &store::ProjectionSnapshot,
    run_id: &RunId,
    pair_id: &SideEffectPairId,
) -> Option<(store::ResourceLaneKey, store::ResourceLaneProjection)> {
    let holder = store::SideEffectPairLedgerRef::new(run_id.clone(), pair_id.clone());
    snapshot
        .resource_lanes()
        .find(|(_, projection)| projection.holder == holder)
        .map(|(key, projection)| (key.clone(), projection.clone()))
}

pub(in crate::tests::support) fn append_erased_runner_output(
    store: &mut TestTypedRunStore,
    run_id: &RunId,
    commit_key: &str,
    output: ErasedRunnerOutput,
    preconditions: store::CommitPreconditions,
) {
    let required_artifacts = output
        .staged_artifacts()
        .iter()
        .map(|artifact| artifact.evidence().clone())
        .collect::<Vec<_>>();
    let payloads = output
        .payloads()
        .iter()
        .cloned()
        .map(events::KernelEventPayload::from)
        .collect::<Vec<_>>();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: payloads,
            required_artifacts: required_artifacts,
            preconditions: preconditions,
        })
        .expect("append runner output");
}

pub(in crate::tests::support) fn side_effect_evidence(
    node: &spec::NodeSpec,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    }
}

pub(in crate::tests::support) fn append_synthetic_ambiguous(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let evidence_hash = content(0xd5);
    let evidence_artifact_id = artifact(0xd6);
    let evidence = side_effect_evidence(
        node,
        evidence_artifact_id.clone(),
        evidence_hash.clone(),
        events::ArtifactRole::AmbiguityEvidence,
    );
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    let payloads = vec![
        events::KernelEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            ledger_key: ledger.clone(),
            ledger_purpose,
            pair_id,
            pair_role,
            invocation_epoch: 1,
            ambiguity_code: events::AmbiguityCode::new("unknown").expect("ambiguity"),
            evidence_schema_id: node.config_ref.schema_id.clone(),
            evidence_hash,
            evidence_artifact_id,
            evidence_artifact_evidence_hash: evidence
                .evidence_hash()
                .expect("ambiguity evidence hash"),
        }),
        events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            retryable: false,
            error: side_effect_error(false),
        }),
    ];
    side_effect.append(
        store,
        commit_key,
        payloads,
        vec![evidence],
        store::RequiredSideEffectState::InvocationStarted,
        false,
    );
}
