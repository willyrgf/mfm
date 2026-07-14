use super::*;

pub(super) struct SideEffectPrepareFixture {
    pub(super) payloads: Vec<KernelEventPayload>,
    pub(super) required_artifacts: Vec<ArtifactEvidenceRef>,
}

pub(super) fn side_effect_prepare_fixture_for_ledger(
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
    node_id: NodeId,
    attempt_id: AttemptId,
) -> SideEffectPrepareFixture {
    let artifact_id = artifact_id(artifact_byte);
    let artifact_digest = content_digest(artifact_byte + 1);
    let intent_evidence = intent_artifact_ref_for_node(
        artifact_id.clone(),
        artifact_digest.clone(),
        node_id.clone(),
    );
    let purpose = events::SideEffectLedgerPurpose::Forward;
    let mut intent = side_effect_intent(artifact_id, artifact_digest);
    let mut claim = side_effect_claim();
    let mut lane = resource_lane_claim_intent(resource_key.clone());
    let mut prepared = side_effect_prepared_with_resource_key(1, "token-1", resource_key);
    for payload in [&mut intent, &mut claim, &mut lane, &mut prepared] {
        set_side_effect_ledger(payload, ledger_key.clone(), purpose.clone());
        set_side_effect_node_attempt(payload, node_id.clone(), attempt_id.clone());
    }
    if let KernelEventPayload::SideEffectIntentPersisted(payload) = &mut intent {
        payload.intent_artifact_evidence_hash = intent_evidence
            .evidence_hash()
            .expect("intent evidence hash for prepare fixture");
    }
    SideEffectPrepareFixture {
        payloads: vec![intent, claim, lane, prepared],
        required_artifacts: vec![intent_evidence],
    }
}

pub(super) fn append_async_side_effect_prepare_for_ledger(
    store: &AsyncInMemoryRunStore,
    run_id: &RunId,
    commit_key: &str,
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
) {
    let node_id = node_id(70);
    let attempt_id = attempt_id(72);
    append_async_prepared_commit(
        store,
        typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: async_expected_next_seq(store, run_id),
            commit_key: CommitKey::new(format!("{commit_key}-attempt-start"))
                .expect("commit key"),
            payloads: vec![side_effect_attempt_started_for(
                node_id.clone(),
                attempt_id.clone(),
            )],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(run_id, SagaPolicySpec::NoSideEffects),
        },
    )
    .expect("append async sidefx attempt start");

    let mut fixture = side_effect_prepare_fixture_for_ledger(
        ledger_key,
        resource_key,
        artifact_byte,
        node_id,
        attempt_id,
    );
    let preconditions =
        certify_payloads_for_policy(run_id, SagaPolicySpec::NoSideEffects, &mut fixture.payloads);

    append_async_prepared_commit(
        store,
        typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: async_expected_next_seq(store, run_id),
            commit_key: CommitKey::new(commit_key).expect("commit key"),
            payloads: fixture.payloads,
            required_artifacts: fixture.required_artifacts,
            preconditions: preconditions,
        },
    )
    .expect("append async sidefx prepare");
}

pub(super) fn side_effect_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key")
}

pub(super) fn side_effect_ledger_key_with_suffix(suffix: u8) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("ledger-key-{suffix}")).expect("ledger key")
}

pub(super) fn remediation_ledger_key(byte: u8) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("remediation-ledger-{byte}"))
        .expect("remediation ledger key")
}

pub(super) fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

pub(super) fn remediation_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: side_effect_pair_id(),
    }
}

pub(super) fn resource_namespace() -> ResourceNamespace {
    ResourceNamespace::new("mfm.test.account_nonce").expect("resource namespace")
}

pub(super) fn resource_key(value: &str, schema_byte: u8) -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace: resource_namespace(),
        key_schema_id: schema_id("mfm.test.resource_key", schema_byte),
        key: events::ResourceKey::new(value).expect("resource key"),
    }
}

pub(super) fn resource_lane_key(value: &str) -> ResourceLaneKey {
    resource_lane_key_with_schema(value, 201)
}

pub(super) fn resource_lane_key_with_schema(value: &str, schema_byte: u8) -> ResourceLaneKey {
    ResourceLaneKey::from_evidence(&resource_key(value, schema_byte))
}

pub(super) fn resource_touched_set(byte: u8) -> events::ResourceTouchedSetEvidence {
    let store = ArtifactEvidenceRef {
        artifact_id: artifact_id(byte),
        digest: content_digest(byte),
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.touched_set", byte)),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::StateOutput,
    };
    events::ResourceTouchedSetEvidence {
        namespace: resource_namespace(),
        evidence_schema_id: schema_id("mfm.test.touched_set", byte),
        evidence_hash: content_digest(byte),
        evidence_artifact_id: artifact_id(byte),
        evidence_artifact_evidence_hash: store.evidence_hash().expect("touched set evidence hash"),
    }
}

pub(super) fn resource_touched_set_artifact_ref(
    evidence: &events::ResourceTouchedSetEvidence,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: evidence.evidence_artifact_id.clone(),
        digest: evidence.evidence_hash.clone(),
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(evidence.evidence_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::StateOutput,
    }
}

pub(super) fn payload_json_value(payload: &KernelEventPayload) -> serde_json::Value {
    serde_json::from_str(
        payload_canonical_json(payload)
            .expect("payload canonical json")
            .as_str(),
    )
    .expect("payload json value")
}

pub(super) fn assert_projection_conflict_contains(error: StoreError, expected: &str) {
    assert!(
        matches!(
            &error,
            StoreError::ProjectionConflict { message, .. } if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

pub(super) fn assert_certified_side_effect_projection_conflict(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    payloads: Vec<KernelEventPayload>,
    required_artifacts: Vec<ArtifactEvidenceRef>,
    expected: &str,
) {
    let error = append_certified_side_effect_commit(
        store,
        run_id,
        commit_key,
        payloads,
        required_artifacts,
    )
    .expect_err("certified side-effect commit must reject");
    assert_projection_conflict_contains(error, expected);
}

pub(super) fn assert_invalid_prepared_commit_contains(error: StoreError, expected: &str) {
    assert!(
        matches!(
            &error,
            StoreError::InvalidPreparedCommitPurpose { message, .. } if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

pub(super) fn assert_event_error_contains(error: StoreError, expected: &str) {
    assert!(
        matches!(
            &error,
            StoreError::Event(message) if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

pub(super) fn assert_resource_lane_blocked(
    outcome: CommitOutcome,
    expected_lane_key: &ResourceLaneKey,
) {
    let CommitOutcome::AdmissionBlocked(block) = outcome else {
        panic!("unexpected outcome: {outcome:?}");
    };
    assert_eq!(&block.resource_lane_key, expected_lane_key);
}

pub(super) fn side_effect_attempt_started() -> KernelEventPayload {
    side_effect_attempt_started_for(submit_node_id(), submit_attempt_id())
}

pub(super) fn side_effect_verify_attempt_started() -> KernelEventPayload {
    side_effect_attempt_started_for(verify_node_id(), verify_attempt_id())
}

pub(super) fn side_effect_attempt_started_for(
    node_id: NodeId,
    attempt_id: AttemptId,
) -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        attempt_no: 1,
        state_kind: state_kind(70),
        state_version: StateVersion::new("mfm.test.side_effect_state.v1").expect("state version"),
    })
}

pub(super) fn side_effect_intent(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    let evidence = intent_artifact_ref(artifact_id.clone(), digest.clone());
    KernelEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        scope_id: scope_id(71),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.test.side_effect_intent", 70),
        intent_hash: digest,
        intent_artifact_id: artifact_id,
        intent_artifact_evidence_hash: evidence.evidence_hash().expect("intent evidence hash"),
        idempotency_input_schema_id: schema_id("mfm.test.idempotency_input", 73),
        idempotency_input_hash: content_digest(74),
        idempotency_key: events::IdempotencyKeyRef::new("idem-key-1").expect("idempotency key"),
        capability_kind: capability_kind(75),
        capability_version: CapabilityVersion::new("mfm.test.capability.v1")
            .expect("capability version"),
        adapter_kind: adapter_kind(76),
        adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
    })
}

pub(super) fn side_effect_claim() -> KernelEventPayload {
    side_effect_claim_for_epoch(1, 1, "token-1")
}

pub(super) fn side_effect_claim_for_epoch(
    invocation_epoch: u32,
    claim_generation: u32,
    token: &str,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    KernelEventPayload::SideEffectClaimed(side_effect::Claimed {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
    })
}

pub(super) fn side_effect_claim_taken_over() -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    KernelEventPayload::SideEffectClaimTakenOver(side_effect::ClaimTakenOver {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        previous_claim_owner: events::RunnerInvocationId::new("owner-1").expect("previous owner"),
        new_claim_owner: events::RunnerInvocationId::new("owner-2").expect("new owner"),
        invocation_epoch: 1,
        previous_claim_generation: 1,
        claim_generation: 2,
        claim_fencing_token: side_effect::ClaimFencingToken::new("token-2").expect("token"),
    })
}

pub(super) fn side_effect_claim_taken_over_with_token(token: &str) -> KernelEventPayload {
    side_effect_claim_taken_over_generation(1, 2, token)
}

pub(super) fn side_effect_claim_taken_over_generation(
    previous_claim_generation: u32,
    claim_generation: u32,
    token: &str,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    KernelEventPayload::SideEffectClaimTakenOver(side_effect::ClaimTakenOver {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        previous_claim_owner: events::RunnerInvocationId::new("owner-1").expect("previous owner"),
        new_claim_owner: events::RunnerInvocationId::new("owner-2").expect("new owner"),
        invocation_epoch: 1,
        previous_claim_generation,
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
    })
}

pub(super) fn side_effect_prepared(claim_generation: u32, token: &str) -> KernelEventPayload {
    side_effect_prepared_for_epoch(1, claim_generation, token)
}

pub(super) fn side_effect_prepared_for_epoch(
    invocation_epoch: u32,
    claim_generation: u32,
    token: &str,
) -> KernelEventPayload {
    side_effect_prepared_with_resource_key_for_epoch(
        invocation_epoch,
        claim_generation,
        token,
        None,
    )
}

pub(super) fn side_effect_prepared_with_resource_key(
    claim_generation: u32,
    token: &str,
    resource_key: events::ResourceKeyEvidence,
) -> KernelEventPayload {
    side_effect_prepared_with_resource_key_for_epoch(1, claim_generation, token, Some(resource_key))
}

pub(super) fn side_effect_prepared_with_resource_key_for_epoch(
    invocation_epoch: u32,
    claim_generation: u32,
    token: &str,
    resource_key: Option<events::ResourceKeyEvidence>,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    KernelEventPayload::SideEffectInvocationPrepared(side_effect::InvocationPrepared {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
        resource_key,
        prepared_artifact_id: None,
        prepared_hash: None,
        prepared_artifact_evidence_hash: None,
    })
}

pub(super) fn resource_lane_claim_intent(
    resource_key: events::ResourceKeyEvidence,
) -> KernelEventPayload {
    resource_lane_claim_intent_for_epoch(1, resource_key)
}

pub(super) fn resource_lane_claim_intent_for_epoch(
    invocation_epoch: u32,
    resource_key: events::ResourceKeyEvidence,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch,
        resource_key,
        requirement_digest: content_digest(210),
        resolved_by_capability_impl: events::RunnerFactoryId::new("mfm.test.store.runner")
            .expect("runner factory"),
    })
}

pub(super) fn resource_lane_release_intent_from_projection(
    store: &StoreContractRunStore,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
    reason: &str,
) -> KernelEventPayload {
    let holder = SideEffectPairLedgerRef::new(run_id.clone(), side_effect_pair_id());
    let (_, lane) = store
        .projection_snapshot()
        .resource_lanes()
        .find(|(_, projection)| projection.holder == holder)
        .expect("active resource lane");
    KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
        spec_hash: store.certified_spec_hash(run_id),
        ledger_key: ledger_key.clone(),
        ledger_purpose: lane.ledger_purpose.clone(),
        pair_id: lane.holder.pair_id.clone(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: lane.invocation_epoch,
        claim_id: lane.claim_id.clone(),
        release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
        release_reason: events::ResourceLaneReleaseReason::new(reason).expect("release reason"),
    })
}

pub(super) fn resource_lane_release_intent_for(
    ledger_key: events::SideEffectLedgerKey,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Verify);
    KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
        spec_hash: spec_hash(1),
        ledger_key,
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        claim_id: events::ResourceLaneClaimId::new("mfm.test.claim.1").expect("claim id"),
        release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
        release_reason: events::ResourceLaneReleaseReason::new("side_effect.terminal")
            .expect("release reason"),
    })
}

pub(super) fn side_effect_started(
    owner: &str,
    claim_generation: u32,
    token: &str,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    KernelEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        claim_owner: events::RunnerInvocationId::new(owner).expect("claim owner"),
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
    })
}

pub(super) fn submission_schema() -> SchemaId {
    schema_id("mfm.test.submission", 77)
}

pub(super) fn receipt_schema() -> SchemaId {
    schema_id("mfm.test.receipt", 78)
}

pub(super) fn confirmation_schema() -> SchemaId {
    schema_id("mfm.test.confirmation", 79)
}

pub(super) fn unknown_schema() -> SchemaId {
    schema_id("mfm.test.submission_unknown", 83)
}

pub(super) fn not_submitted_schema() -> SchemaId {
    schema_id("mfm.test.not_submitted", 80)
}

pub(super) fn side_effect_not_submitted(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        not_submitted_schema(),
        ArtifactRole::NotSubmittedProof,
    );
    KernelEventPayload::SideEffectNotSubmittedProven(side_effect::NotSubmittedProven {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        proof_schema_id: not_submitted_schema(),
        proof_hash: digest,
        proof_artifact_id: artifact_id,
        proof_artifact_evidence_hash: evidence.evidence_hash().expect("proof evidence hash"),
    })
}

pub(super) fn side_effect_submission_observed(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    KernelEventPayload::SideEffectSubmissionObserved(side_effect::SubmissionObserved {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        submission_schema_id: submission_schema(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
        submission_artifact_evidence_hash: evidence
            .evidence_hash()
            .expect("submission evidence hash"),
    })
}

pub(super) fn side_effect_ambiguous(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Verify);
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    KernelEventPayload::SideEffectAmbiguous(side_effect::Ambiguous {
        spec_hash: spec_hash(1),
        node_id: verify_node_id(),
        attempt_id: verify_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        ambiguity_code: events::AmbiguityCode::new("ambiguous").expect("ambiguity code"),
        evidence_schema_id: schema_id("mfm.test.ambiguity", 76),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
        evidence_artifact_evidence_hash: evidence.evidence_hash().expect("ambiguity evidence hash"),
    })
}

pub(super) fn side_effect_submission_unknown(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Submit);
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    KernelEventPayload::SideEffectSubmissionUnknown(side_effect::SubmissionUnknown {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        evidence_schema_id: unknown_schema(),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
        evidence_artifact_evidence_hash: evidence
            .evidence_hash()
            .expect("unknown submission evidence hash"),
    })
}

pub(super) fn side_effect_receipt(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Verify);
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        receipt_schema(),
        ArtifactRole::Receipt,
    );
    KernelEventPayload::SideEffectReceiptObserved(side_effect::ReceiptObserved {
        spec_hash: spec_hash(1),
        node_id: verify_node_id(),
        attempt_id: verify_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        receipt_schema_id: receipt_schema(),
        receipt_hash: digest,
        receipt_artifact_id: artifact_id,
        receipt_artifact_evidence_hash: evidence.evidence_hash().expect("receipt evidence hash"),
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
        resource_touched_set: None,
    })
}

pub(super) fn side_effect_confirmation(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Verify);
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        confirmation_schema(),
        ArtifactRole::Confirmation,
    );
    KernelEventPayload::SideEffectConfirmationObserved(side_effect::ConfirmationObserved {
        spec_hash: spec_hash(1),
        node_id: verify_node_id(),
        attempt_id: verify_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        confirmation_schema_id: confirmation_schema(),
        confirmation_hash: digest,
        confirmation_artifact_id: artifact_id,
        confirmation_artifact_evidence_hash: evidence
            .evidence_hash()
            .expect("confirmation evidence hash"),
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
        resource_touched_set: None,
    })
}

pub(super) fn side_effect_failed(retryable: bool) -> KernelEventPayload {
    let (pair_id, pair_role) = side_effect_pair_role(events::SideEffectPairRole::Verify);
    KernelEventPayload::SideEffectFailed(side_effect::Failed {
        spec_hash: spec_hash(1),
        node_id: verify_node_id(),
        attempt_id: verify_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id,
        pair_role,
        invocation_epoch: 1,
        failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
        retryable,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

pub(super) fn side_effect_attempt_failed(retryable: bool) -> KernelEventPayload {
    KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
        spec_hash: spec_hash(1),
        node_id: verify_node_id(),
        attempt_id: verify_attempt_id(),
        retryable,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}
