use super::*;

struct SyntheticSideEffectAppend<'a> {
    fixture: &'a Fixture,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
}

impl<'a> SyntheticSideEffectAppend<'a> {
    fn new(
        fixture: &'a Fixture,
        run_id: &'a RunId,
        node: &'a spec::NodeSpec,
        attempt_id: &'a AttemptId,
    ) -> Self {
        Self {
            fixture,
            run_id,
            node,
            attempt_id,
        }
    }

    fn ledger_purpose(&self) -> events::SideEffectLedgerPurpose {
        events::SideEffectLedgerPurpose::Forward
    }

    fn pair_fields(
        &self,
        node: &spec::NodeSpec,
        role: events::SideEffectPairRole,
    ) -> (SideEffectPairId, events::SideEffectPairRole) {
        side_effect_pair_fields_for_purpose(
            &self.fixture.runtime_spec,
            &node.node_id,
            &self.ledger_purpose(),
            role,
        )
    }

    fn append(
        &self,
        store: &mut TestTypedRunStore,
        commit_key: &str,
        payloads: Vec<events::KernelEventPayload>,
        required_artifacts: Vec<store::ArtifactEvidenceRef>,
        required_side_effect_state: store::RequiredSideEffectState,
        require_attempt_started: bool,
    ) {
        let required_present_logical_keys = require_attempt_started
            .then(|| {
                store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    self.node.node_id, self.attempt_id
                ))
                .expect("attempt logical key")
            })
            .into_iter()
            .collect::<Vec<_>>();
        store
            .append_prepared_commit(store_typed_commit_request! {
                run_id: self.run_id.clone(),
                expected_next_seq: store.expected_next_seq(self.run_id),
                commit_key: store::CommitKey::new(commit_key).expect("commit key"),
                payloads: payloads,
                required_artifacts: required_artifacts,
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys,
                    required_side_effect_states: vec![store::SideEffectStatePrecondition {
                        pair_id: fixture_side_effect_pair_id(self.fixture, self.node),
                        required: required_side_effect_state,
                    }],
                    certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                        self.run_id.clone(),
                        self.fixture.runtime_spec.spec(),
                    )
                    .expect("certified run authority")),
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append synthetic side-effect commit");
    }
}

pub(super) fn append_synthetic_exclusive_prepare(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> (AttemptId, events::SideEffectLedgerKey) {
    let attempt_id =
        attempt_id(run_id, fixture.runtime_spec.spec_hash(), &node.node_id, 1).expect("attempt id");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(format!("{commit_key}-attempt-start"))
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no: 1,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic attempt start");

    let ledger =
        events::SideEffectLedgerKey::new(format!("holder-{commit_key}")).expect("holder ledger");
    let intent_hash = content_digest_json(serde_json::json!({
        "key": key,
        "ledger": ledger.as_str(),
        "run": run_id.as_str(),
    }))
    .expect("intent digest");
    let resource_key = exclusive_resource_key(fixture, key);
    let resource_lane_requirement_digest = content_digest_json(serde_json::json!({
        "acquisition": "pre_state_invocation",
        "hold": "until_side_effect_terminal",
        "key_schema_id": resource_key.key_schema_id.as_str(),
        "mode": "exclusive",
        "namespace": resource_key.namespace.as_str(),
    }))
    .expect("resource lane requirement digest");
    let intent_artifact_id =
        ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
    let intent_artifact = store::ArtifactEvidenceRef {
        artifact_id: intent_artifact_id.clone(),
        digest: intent_hash.clone(),
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::SideEffectIntent,
    };
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, &attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append(
        store,
        commit_key,
        vec![
            events::KernelEventPayload::SideEffectIntentPersisted(
                events::side_effect::IntentPersisted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: ledger_purpose.clone(),
                    pair_id: pair_id.clone(),
                    pair_role,
                    invocation_epoch: 1,
                    intent_schema_id: node.config_ref.schema_id.clone(),
                    intent_hash: intent_hash.clone(),
                    intent_artifact_id,
                    intent_artifact_evidence_hash: intent_artifact
                        .evidence_hash()
                        .expect("intent evidence hash"),
                    idempotency_input_schema_id: node.config_ref.schema_id.clone(),
                    idempotency_input_hash: content(0xc3),
                    idempotency_key: events::IdempotencyKeyRef::new(format!("idem-{commit_key}"))
                        .expect("idempotency key"),
                    capability_kind: side_effect_capability_kind(),
                    capability_version: side_effect_capability_version(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                },
            ),
            events::KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role,
                claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                    .expect("fencing token"),
            }),
            events::KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role,
                invocation_epoch: 1,
                resource_key: resource_key.clone(),
                requirement_digest: resource_lane_requirement_digest,
                resolved_by_capability_impl: events::RunnerFactoryId::new(
                    "mfm.test.side_effect_driver",
                )
                .expect("runner factory"),
            }),
            events::KernelEventPayload::SideEffectInvocationPrepared(
                events::side_effect::InvocationPrepared {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose,
                    pair_id: pair_id.clone(),
                    pair_role,
                    invocation_epoch: 1,
                    claim_generation: 1,
                    claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                        .expect("fencing token"),
                    resource_key: Some(resource_key),
                    prepared_artifact_id: None,
                    prepared_hash: None,
                    prepared_artifact_evidence_hash: None,
                },
            ),
        ],
        vec![intent_artifact],
        store::RequiredSideEffectState::Absent,
        true,
    );
    (attempt_id, ledger)
}

pub(super) fn append_synthetic_invocation_started(
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

pub(super) fn append_synthetic_exclusive_started(
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

pub(super) fn append_synthetic_submission_observed(
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

pub(super) fn append_synthetic_receipt_observed(
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

pub(super) fn append_synthetic_exclusive_receipt_phase(
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

pub(super) fn append_synthetic_submit_boundary_skipped(
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

pub(super) fn append_synthetic_verify_receipt_observed(
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

pub(super) fn append_synthetic_verify_confirmation_observed(
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

pub(super) fn synthetic_resource_lane_release(
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

pub(super) fn active_resource_lane_for_pair(
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

pub(super) fn append_erased_runner_output(
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

pub(super) fn side_effect_evidence(
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

pub(super) fn append_synthetic_ambiguous(
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

pub(super) async fn append_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    outcome: events::ManualResolutionOutcome,
) {
    let manual = match &fixture.runtime_spec.spec().saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual,
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manual,
        _ => panic!("fixture does not carry manual resolution schemas"),
    };
    let evidence_bytes = br#"{"operator_note":"reviewed"}"#.to_vec();
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&evidence_bytes),
    );
    let evidence_artifact_id =
        ArtifactId::from_digest(evidence_hash.algorithm(), *evidence_hash.digest());
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        content_hash: evidence_hash,
        artifact_id: evidence_artifact_id,
    };
    let prefix = build_manual_resolution_prefix_authority_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        store,
        manual.clone(),
    )
    .expect("manual prefix authority");
    let claim = prefix
        .authorization_claim(outcome, evidence)
        .expect("manual claim");
    let operator = manual.authorization.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: manual.authorization.verifier_id.clone(),
        signing_scheme: manual.authorization.signing_scheme.clone(),
        claim: claim.clone(),
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    };
    let proof_bytes = proof
        .canonical_json()
        .expect("canonical manual proof")
        .to_vec();
    record_manual_resolution(
        scheduler,
        store,
        &fixture.runtime_spec,
        &fixture.run_id,
        ManualResolutionRequest {
            outcome,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes,
            note: None,
        },
    )
    .await
    .expect("append manual resolution");
}

pub(super) fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

pub(super) fn sign_manual_claim_digest(
    signing_key: &k256::ecdsa::SigningKey,
    digest: &[u8; 32],
) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

pub(super) fn append_fact(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    subject_amount: u64,
    response_amount: u64,
) {
    let fact_key = test_fact_key(subject_amount);
    let (evidence, response_bytes) = test_fact_response_artifact(node, response_amount);
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new(format!(
            "manual-fact:{}:{}:{}:{}",
            node.node_id, attempt_id, fact_key, response_amount
        ))
        .expect("commit key"),
        payloads: vec![events::KernelEventPayload::FactRecorded(
            events::FactRecorded {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                claim: test_fact_claim(
                    subject_amount,
                    node.config_ref.schema_id.clone(),
                    content(0xd4),
                    &evidence,
                    fixture.cap_kind.clone(),
                    fixture.cap_version.clone(),
                    fixture.adapter_kind.clone(),
                    fixture.adapter_version.clone(),
                ),
            },
        )],
        required_artifacts: vec![evidence.clone()],
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("attempt logical key")],
            certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                fixture.run_id.clone(),
                fixture.runtime_spec.spec(),
            )
            .expect("certified run authority")),
            ..store::CommitPreconditions::default()
        },
    };
    let plan =
        test_prepared_commit_plan(request, vec![evidence.clone()]).expect("fact commit plan");
    let bundle = store::PreparedCommitBundle::new(
        plan,
        vec![store::PreparedArtifactBytes::new(response_bytes, evidence)
            .expect("fact response bytes")],
        Vec::new(),
    )
    .expect("fact commit bundle");
    block_on_ready(store.append_prepared_commit_bundle(bundle)).expect("append fact");
}

pub(super) fn append_terminal(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    artifact_id: ArtifactId,
    output_digest: ContentDigest,
) {
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let evidence =
        state_output_artifact(node, descriptor, artifact_id.clone(), output_digest.clone());
    let evidence_hash = evidence
        .evidence_hash()
        .expect("manual terminal state output evidence hash");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-terminal:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    cell_id: node.output_cell.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: descriptor.output_semantic_type_id.clone(),
                    schema_id: descriptor.output_schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    context: output_cell.context.clone(),
                    artifact_id,
                    content_digest: output_digest,
                    evidence_hash,
                    producer_state_kind: Some(node.state_kind.clone()),
                    producer_state_version: Some(node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    output_cell_id: node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![evidence],
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
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append terminal");
}

pub(super) fn append_public_output_render_failure(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        panic!("expected public-output render node");
    };
    let error = public_output_error();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-public-output-failure:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::PublicOutputRenderFailed(
                    events::PublicOutputRenderFailed {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        public_schema_id: render.public_schema_id.clone(),
                        renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
                        error: error.clone(),
                    },
                ),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable: true,
                    error,
                }),
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
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append public output failure");
}

pub(super) fn append_not_submitted_proven(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) {
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        &fixture.run_id,
        &projection_snapshot,
        node,
        attempt_id,
    )
    .expect("side-effect projection lookup")
    .expect("side-effect projection");
    let ledger_key = projection.ledger_key.clone();
    let ledger_purpose = projection.ledger_purpose.clone();
    let pair_id = projection.pair_id.clone();
    let pair_role = events::SideEffectPairRole::Submit;
    let proof_artifact = artifact(0xd5);
    let proof_hash = content(0xd6);
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: proof_artifact.clone(),
        digest: proof_hash.clone(),
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::NotSubmittedProof,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-not-submitted:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::SideEffectNotSubmittedProven(
                events::side_effect::NotSubmittedProven {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key,
                    ledger_purpose,
                    pair_id,
                    pair_role,
                    invocation_epoch,
                    proof_schema_id: node.config_ref.schema_id.clone(),
                    proof_hash,
                    proof_artifact_id: proof_artifact,
                    proof_artifact_evidence_hash: evidence
                        .evidence_hash()
                        .expect("proof evidence hash"),
                },
            )],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                    fixture.run_id.clone(),
                    fixture.runtime_spec.spec(),
                )
                .expect("certified run authority")),
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append not-submitted proof");
}
