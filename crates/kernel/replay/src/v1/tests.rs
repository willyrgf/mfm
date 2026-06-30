use super::*;

#[test]
fn side_effect_frame_requests_are_pair_keyed() {
    let pair_id = pair_id(0x31);
    let ledger_key = events::SideEffectLedgerKey::new("ledger-key").expect("ledger key");
    let intent = intent_persisted(pair_id.clone(), ledger_key.clone());
    let submission_hash = content_digest(0x41);
    let submission = side_effect::SubmissionObserved {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        submission_schema_id: schema_id("mfm.replay.test.submission", 0x42),
        submission_hash: submission_hash.clone(),
        submission_artifact_id: artifact_id(0x43),
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        submission: Some(&submission),
        not_submitted: None,
        receipt: None,
        confirmation: None,
        ambiguity: None,
    };

    let request = frame.submission_request().expect("submission request");

    assert_eq!(request.pair_id, pair_id);
    assert_eq!(request.invocation_epoch, 1);
    assert_eq!(request.evidence_schema_id, submission.submission_schema_id);
    assert_eq!(request.evidence_hash, submission_hash);
}

#[test]
fn side_effect_frame_not_submitted_requests_are_pair_keyed() {
    let pair_id = pair_id(0x51);
    let ledger_key = events::SideEffectLedgerKey::new("not-submitted-ledger").expect("ledger key");
    let intent = intent_persisted(pair_id.clone(), ledger_key.clone());
    let proof_hash = content_digest(0x52);
    let proof = side_effect::NotSubmittedProven {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        proof_schema_id: schema_id("mfm.replay.test.not_submitted", 0x53),
        proof_hash: proof_hash.clone(),
        proof_artifact_id: artifact_id(0x54),
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        submission: None,
        not_submitted: Some(&proof),
        receipt: None,
        confirmation: None,
        ambiguity: None,
    };

    let request = frame.not_submitted_request().expect("proof request");

    assert_eq!(request.pair_id, pair_id);
    assert_eq!(request.invocation_epoch, 1);
    assert_eq!(request.evidence_schema_id, proof.proof_schema_id);
    assert_eq!(request.evidence_hash, proof_hash);
}

#[test]
fn side_effect_frame_receipt_request_uses_recorded_verify_evidence() {
    let pair_id = pair_id(0x61);
    let ledger_key =
        events::SideEffectLedgerKey::new("receipt-ledger").expect("receipt ledger key");
    let intent = intent_persisted(pair_id.clone(), ledger_key.clone());
    let receipt_hash = content_digest(0x62);
    let replay_verifier_id = events::ReplayVerifierId::new("mfm.replay.test.receipt.verifier")
        .expect("replay verifier id");
    let receipt = side_effect::ReceiptObserved {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        receipt_schema_id: schema_id("mfm.replay.test.receipt", 0x63),
        receipt_hash: receipt_hash.clone(),
        receipt_artifact_id: artifact_id(0x64),
        replay_verifier_id: replay_verifier_id.clone(),
        resource_touched_set: None,
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        submission: None,
        not_submitted: None,
        receipt: Some(&receipt),
        confirmation: None,
        ambiguity: None,
    };

    let request = frame.receipt_request().expect("receipt request");

    assert_eq!(request.pair_id, pair_id);
    assert_eq!(request.invocation_epoch, 1);
    assert_eq!(request.evidence_schema_id, receipt.receipt_schema_id);
    assert_eq!(request.evidence_hash, receipt_hash);
    assert_eq!(request.replay_verifier_id, Some(replay_verifier_id));
}

#[test]
fn side_effect_ambiguity_submit_claim_identity_matches_intent() {
    let pair_id = pair_id(0x71);
    let ledger_key =
        events::SideEffectLedgerKey::new("ambiguity-ledger").expect("ambiguity ledger key");
    let intent = intent_persisted(pair_id, ledger_key);

    verify_side_effect_submit_claim_identity(
        &intent,
        &intent.node_id,
        &intent.attempt_id,
        "side-effect ambiguity event does not match persisted intent",
    )
    .expect("verify-role ambiguity from submit claim is valid");

    let wrong_node = node_id(0x72);
    let error = verify_side_effect_submit_claim_identity(
        &intent,
        &wrong_node,
        &intent.attempt_id,
        "side-effect ambiguity event does not match persisted intent",
    )
    .expect_err("different submit node is invalid");
    assert_eq!(error.kind, ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn resource_lane_release_requires_later_verify_terminal_payload() {
    let pair_id = pair_id(0x81);
    let ledger_key = events::SideEffectLedgerKey::new("release-ledger").expect("ledger key");
    let terminal_policies = side_effect_terminal_policies(
        pair_id.clone(),
        store::SideEffectTerminalPolicy::Confirmation,
    );
    let release = resource_lane_released(
        pair_id.clone(),
        ledger_key.clone(),
        events::ResourceLaneReleaseAuthority::VerifyTerminal,
    );
    let confirmation = side_effect_confirmation(pair_id, ledger_key);

    verify_resource_lane_release_payload_adjacency(&terminal_policies, &[&release, &confirmation])
        .expect("release before matching terminal is valid");
    let error = verify_resource_lane_release_payload_adjacency(
        &terminal_policies,
        &[&confirmation, &release],
    )
    .expect_err("terminal before release does not authorize release");

    assert_eq!(error.kind, ReplayErrorKind::InvalidRunStream);
    assert!(error.message.contains("matching verify-terminal payload"));
}

#[test]
fn verify_release_accepts_submit_role_side_effect_failure_terminal() {
    let pair_id = pair_id(0x8d);
    let ledger_key =
        events::SideEffectLedgerKey::new("release-failure-ledger").expect("ledger key");
    let terminal_policies = side_effect_terminal_policies(
        pair_id.clone(),
        store::SideEffectTerminalPolicy::Confirmation,
    );
    let release = resource_lane_released(
        pair_id.clone(),
        ledger_key.clone(),
        events::ResourceLaneReleaseAuthority::VerifyTerminal,
    );
    let failed = side_effect_failed(
        pair_id.clone(),
        ledger_key.clone(),
        events::SideEffectPairRole::Submit,
    );

    verify_resource_lane_release_payload_adjacency(&terminal_policies, &[&release, &failed])
        .expect("verify-authority release can be backed by submit-role failure terminal");

    let submit_release = resource_lane_released_with_role(
        pair_id,
        ledger_key,
        events::SideEffectPairRole::Submit,
        events::ResourceLaneReleaseAuthority::VerifyTerminal,
    );
    let error = verify_resource_lane_release_payload_adjacency(
        &terminal_policies,
        &[&submit_release, &failed],
    )
    .expect_err("resource lane release must retain verify authority role");
    assert_eq!(error.kind, ReplayErrorKind::InvalidRunStream);
}

#[test]
fn resource_lane_release_terminal_policy_is_replayed() {
    let pair_id = pair_id(0x84);
    let ledger_key = events::SideEffectLedgerKey::new("release-policy-ledger").expect("ledger key");
    let release = resource_lane_released(
        pair_id.clone(),
        ledger_key.clone(),
        events::ResourceLaneReleaseAuthority::VerifyTerminal,
    );
    let receipt = side_effect_receipt(pair_id.clone(), ledger_key);
    let confirmation_policy = side_effect_terminal_policies(
        pair_id.clone(),
        store::SideEffectTerminalPolicy::Confirmation,
    );
    let receipt_policy =
        side_effect_terminal_policies(pair_id, store::SideEffectTerminalPolicy::Receipt);

    let error =
        verify_resource_lane_release_payload_adjacency(&confirmation_policy, &[&release, &receipt])
            .expect_err("receipt cannot release a confirmation-policy lane");
    assert_eq!(error.kind, ReplayErrorKind::InvalidRunStream);

    verify_resource_lane_release_payload_adjacency(&receipt_policy, &[&release, &receipt])
        .expect("receipt-policy lane can release on receipt");
}

#[test]
fn manual_resource_lane_release_requires_later_manual_resolution_record() {
    let pair_id = pair_id(0x87);
    let ledger_key = events::SideEffectLedgerKey::new("manual-release-ledger").expect("ledger key");
    let terminal_policies = store::SideEffectTerminalPolicies::new(BTreeMap::new());
    let release = resource_lane_released(
        pair_id,
        ledger_key,
        events::ResourceLaneReleaseAuthority::ManualResolution,
    );
    let manual = manual_resolution_recorded();

    verify_resource_lane_release_payload_adjacency(&terminal_policies, &[&release, &manual])
        .expect("manual release before manual record is valid");
    let error = verify_resource_lane_release_payload_adjacency(&terminal_policies, &[&release])
        .expect_err("manual release without manual record rejects");
    assert_eq!(error.kind, ReplayErrorKind::InvalidRunStream);

    let error =
        verify_resource_lane_release_payload_adjacency(&terminal_policies, &[&manual, &release])
            .expect_err("manual record before release does not authorize release");
    assert_eq!(error.kind, ReplayErrorKind::InvalidRunStream);
}

#[test]
fn replay_source_does_not_import_live_authorities() {
    let source = include_str!("../lib.rs");
    for forbidden in [
        concat!("std", "::", "env"),
        concat!("mfm", "_", "transports"),
        concat!("mfm", "_", "signing", "::", "SigningProvider"),
        concat!("mfm", "_", "core", "::", "keystore"),
        concat!("Evm", "Json", "Rpc", "Client"),
    ] {
        assert!(
            !source.contains(forbidden),
            "replay source must not import live authority surface {forbidden}"
        );
    }
}

fn intent_persisted(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
) -> side_effect::IntentPersisted {
    side_effect::IntentPersisted {
        spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x01)),
        node_id: node_id(0x02),
        scope_id: mfm_ids::ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x03)),
        attempt_id: attempt_id(0x04),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.replay.test.intent", 0x05),
        intent_hash: content_digest(0x06),
        intent_artifact_id: artifact_id(0x07),
        idempotency_input_schema_id: schema_id("mfm.replay.test.idempotency", 0x08),
        idempotency_input_hash: content_digest(0x09),
        idempotency_key: events::IdempotencyKeyRef::new("idem-key").expect("idempotency key"),
        capability_kind: CapabilityKind::new(
            "mfm.replay.test",
            "mutation",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0x0a),
        )
        .expect("capability kind"),
        capability_version: CapabilityVersion::new("mfm.replay.test.capability.v1")
            .expect("capability version"),
        adapter_kind: AdapterKind::new(
            "mfm.replay.test",
            "adapter",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0x0b),
        )
        .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.replay.test.adapter.v1")
            .expect("adapter version"),
    }
}

fn resource_lane_released(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
    release_authority: events::ResourceLaneReleaseAuthority,
) -> KernelEventPayload {
    resource_lane_released_with_role(
        pair_id,
        ledger_key,
        events::SideEffectPairRole::Verify,
        release_authority,
    )
}

fn resource_lane_released_with_role(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
    pair_role: events::SideEffectPairRole,
    release_authority: events::ResourceLaneReleaseAuthority,
) -> KernelEventPayload {
    KernelEventPayload::ResourceLaneReleased(events::ResourceLaneReleased {
        spec_hash: spec_hash(0x80),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role,
        invocation_epoch: 1,
        claim_id: events::ResourceLaneClaimId::new("mfm.replay.test.claim").expect("claim id"),
        release_id: events::ResourceLaneReleaseId::new("mfm.replay.test.release")
            .expect("release id"),
        claim_fencing_token: 1,
        release_authority,
        release_reason: events::ResourceLaneReleaseReason::new("side_effect.terminal")
            .expect("release reason"),
        lane_transition_seq: 2,
    })
}

fn side_effect_confirmation(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectConfirmationObserved(side_effect::ConfirmationObserved {
        spec_hash: spec_hash(0x80),
        node_id: node_id(0x82),
        attempt_id: attempt_id(0x83),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        confirmation_schema_id: schema_id("mfm.replay.test.confirmation", 0x84),
        confirmation_hash: content_digest(0x85),
        confirmation_artifact_id: artifact_id(0x86),
        replay_verifier_id: events::ReplayVerifierId::new("mfm.replay.test.confirmation.verifier")
            .expect("replay verifier id"),
        resource_touched_set: None,
    })
}

fn side_effect_failed(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
    pair_role: events::SideEffectPairRole,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectFailed(side_effect::Failed {
        spec_hash: spec_hash(0x80),
        node_id: node_id(0x8e),
        attempt_id: attempt_id(0x8f),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role,
        invocation_epoch: 1,
        failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
        retryable: false,
        error: events::MfmErrorInfo::new(
            events::ErrorCode::new("mfm.replay.test.side_effect_failed").expect("error code"),
            events::ErrorCategory::SideEffect,
            false,
            "side-effect failed",
        )
        .expect("error info"),
    })
}

fn side_effect_receipt(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectReceiptObserved(side_effect::ReceiptObserved {
        spec_hash: spec_hash(0x80),
        node_id: node_id(0x88),
        attempt_id: attempt_id(0x89),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        receipt_schema_id: schema_id("mfm.replay.test.receipt", 0x8a),
        receipt_hash: content_digest(0x8b),
        receipt_artifact_id: artifact_id(0x8c),
        replay_verifier_id: events::ReplayVerifierId::new("mfm.replay.test.receipt.verifier")
            .expect("replay verifier id"),
        resource_touched_set: None,
    })
}

fn manual_resolution_recorded() -> KernelEventPayload {
    KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id: run_id(0x90),
        spec_hash: spec_hash(0x80),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence_schema_id: schema_id("mfm.replay.test.manual_evidence", 0x91),
        evidence_hash: content_digest(0x92),
        evidence_artifact_id: artifact_id(0x93),
        authorization_schema_id: schema_id("mfm.replay.test.manual_authorization", 0x94),
        authorization_hash: content_digest(0x95),
        authorization_artifact_id: artifact_id(0x96),
        note: None,
    })
}

fn side_effect_terminal_policies(
    pair_id: SideEffectPairId,
    policy: store::SideEffectTerminalPolicy,
) -> store::SideEffectTerminalPolicies {
    store::SideEffectTerminalPolicies::new(BTreeMap::from([(pair_id, policy)]))
}

fn spec_hash(byte: u8) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn pair_id(byte: u8) -> SideEffectPairId {
    SideEffectPairId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn artifact_id(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn schema_id(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(byte)).expect("schema id")
}

fn digest_bytes(byte: u8) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array([byte; 32])
}
