use super::*;
use ed25519_dalek::SigningKey;
use mfm_store::v1::test_support::{
    fact_query_receipt_trust_root_for_test,
    persisted_kernel_event_envelope_for_test as store_persisted_kernel_event_envelope_for_test,
    persisted_kernel_event_envelope_with_ordinal_for_test as store_persisted_kernel_event_envelope_with_ordinal_for_test,
    run_artifact_ref_from_store_artifact_for_test, signed_fact_query_receipt_for_test,
    SignedFactQueryReceiptFixtureInputForTest,
};

#[test]
fn recorded_fact_replay_uses_claim_id_not_fact_key() {
    let run_id = run_id(0xa0);
    let first_claim_id = mfm_facts::FactClaimId::new(run_id.clone(), 2, 0).expect("claim id");
    let second_claim_id = mfm_facts::FactClaimId::new(run_id, 3, 0).expect("claim id");
    let first_artifact = fact_response_artifact(0xa1);
    let second_artifact = fact_response_artifact(0xa2);
    let first_fact = fact_recorded(&first_artifact);
    let second_fact = fact_recorded(&second_artifact);
    assert_eq!(
        first_fact.claim.subject().fact_key(),
        second_fact.claim.subject().fact_key()
    );

    let broker = replay_broker_with_facts(vec![
        (first_claim_id.clone(), first_fact, first_artifact),
        (
            second_claim_id.clone(),
            second_fact.clone(),
            second_artifact.clone(),
        ),
    ]);
    let replay = broker
        .recorded_fact(&FactReplayRequest {
            node_id: fact_node_id(),
            attempt_id: fact_attempt_id(),
            fact_claim_id: second_claim_id.clone(),
            capability_kind: fact_capability_kind(),
            capability_version: fact_capability_version(),
            adapter_kind: fact_adapter_kind(),
            adapter_version: fact_adapter_version(),
            request_schema_id: fact_request_schema_id(),
            request_hash: fact_request_hash(),
            response_schema_id: fact_response_schema_id(),
        })
        .expect("replay second claim");

    assert_eq!(replay.fact_claim_id, second_claim_id);
    assert_eq!(replay.fact, second_fact);
    assert_eq!(replay.artifact.artifact_id, second_artifact.artifact_id);
}

#[test]
fn replay_broker_rebuilds_fact_projection_with_retained_artifact_bytes() {
    let fixture = replay_fact_stream_fixture();

    let broker = ReplayBroker::from_read_authority(fixture.authority())
        .expect("fact-bearing replay authority rebuilds");

    assert!(
        broker
            .projection_snapshot()
            .fact_record(&fixture.claim_id)
            .is_some(),
        "fact claim should be projected from retained descriptor and response bytes"
    );

    let mut missing_bytes = fixture.authority();
    missing_bytes.artifact_bytes.clear();
    let error = ReplayBroker::from_read_authority(missing_bytes)
        .expect_err("missing retained fact artifact bytes must fail closed");
    assert_eq!(error.kind, ReplayErrorKind::InvalidRunStream);
    assert!(
        error.message.contains("missing artifact")
            || error.message.contains("requires retained")
            || error.message.contains("MissingArtifact"),
        "unexpected error: {error}"
    );

    let mut mismatched_bytes = fixture.authority();
    mismatched_bytes.artifact_bytes.insert(
        fixture.response_artifact_id.clone(),
        b"{\"amount\":999}".to_vec(),
    );
    let error = ReplayBroker::from_read_authority(mismatched_bytes)
        .expect_err("mismatched retained fact response bytes must fail closed");
    assert_eq!(error.kind, ReplayErrorKind::ArtifactMismatch);
}

#[test]
fn replay_rejects_cell_produced_context_mismatch() {
    ReplayBroker::from_read_authority(cell_replay_authority(
        CellReplayTerminal::Produced,
        spec::CellContextSpec::no_context(),
    ))
    .expect("matching produced cell context verifies");

    let error = ReplayBroker::from_read_authority(cell_replay_authority(
        CellReplayTerminal::Produced,
        mismatched_cell_context(),
    ))
    .expect_err("mismatched produced cell context rejects");
    assert_eq!(error.kind, ReplayErrorKind::CertifiedEvidenceMismatch);
    assert!(error.message.contains("produced cell"));
}

#[test]
fn replay_rejects_cell_skipped_context_mismatch() {
    ReplayBroker::from_read_authority(cell_replay_authority(
        CellReplayTerminal::Skipped,
        spec::CellContextSpec::no_context(),
    ))
    .expect("matching skipped cell context verifies");

    let error = ReplayBroker::from_read_authority(cell_replay_authority(
        CellReplayTerminal::Skipped,
        mismatched_cell_context(),
    ))
    .expect_err("mismatched skipped cell context rejects");
    assert_eq!(error.kind, ReplayErrorKind::CertifiedEvidenceMismatch);
    assert!(error.message.contains("skipped cell"));
}

#[test]
fn replay_verifies_fact_query_evidence_from_retained_authority() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| fact_ref);
    let mut authority = fixture.authority();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        4,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);
    authority.fact_query_receipt_trust_root = Some(query.trust_root);

    ReplayBroker::from_read_authority(authority).expect("query evidence verifies");
}

#[test]
fn replay_verifies_fact_query_evidence_with_retained_cross_run_source_fact() {
    let fixture = replay_fact_stream_fixture();
    let source_run_id = run_id(0x77);
    let source_claim_id =
        mfm_facts::FactClaimId::new(source_run_id.clone(), 3, 0).expect("source claim id");
    let source_payload = fixture.stream[2].payload().clone();
    let source_envelope = persisted_envelope(&source_run_id, 3, source_payload);
    let source_event_id = source_envelope.event_id().clone();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| {
        let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
        parts.fact_claim_id = source_claim_id.clone();
        parts.source_event_id = source_event_id;
        mfm_facts::InternalFactRef::new(parts).expect("cross-run source ref")
    });
    let source_event = RetainedSourceFactReplayEvent::new(source_claim_id, source_envelope)
        .expect("retained source fact event");
    let mut authority = fixture.authority();
    authority.stream = fixture.stream[..2].to_vec();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        3,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);
    authority.fact_query_receipt_trust_root = Some(query.trust_root);
    authority.source_fact_events.push(source_event);

    ReplayBroker::from_read_authority(authority).expect("cross-run source fact verifies");
}

#[test]
fn replay_rejects_fact_query_evidence_without_trust_root() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| fact_ref);
    let mut authority = fixture.authority();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        4,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);

    let error = ReplayBroker::from_read_authority(authority)
        .expect_err("missing receipt trust root rejects");
    assert_eq!(error.kind, ReplayErrorKind::CertifiedEvidenceMismatch);
    assert!(error.message.contains("receipt trust root"));
}

#[test]
fn replay_rejects_fact_query_evidence_with_missing_source_fact() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| {
        let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
        parts.fact_claim_id =
            mfm_facts::FactClaimId::new(fixture.run_id.clone(), 9, 0).expect("claim id");
        mfm_facts::InternalFactRef::new(parts).expect("missing source ref")
    });
    let mut authority = fixture.authority();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        4,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);
    authority.fact_query_receipt_trust_root = Some(query.trust_root);

    let error =
        ReplayBroker::from_read_authority(authority).expect_err("missing source fact rejects");
    assert_eq!(error.kind, ReplayErrorKind::FactMissing);
}

#[test]
fn replay_rejects_fact_query_evidence_with_mismatched_returned_ref() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| {
        let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
        parts.response = mfm_facts::FactResponseEvidence::new(
            parts.response.response_schema_id().clone(),
            content_digest(0x44),
            parts.response.artifact_id().clone(),
            parts.response.artifact_evidence_hash().clone(),
        );
        mfm_facts::InternalFactRef::new(parts).expect("mismatched returned ref")
    });
    let mut authority = fixture.authority();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        4,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);
    authority.fact_query_receipt_trust_root = Some(query.trust_root);

    let error =
        ReplayBroker::from_read_authority(authority).expect_err("mismatched returned ref rejects");
    assert_eq!(error.kind, ReplayErrorKind::FactMismatch);
}

#[test]
fn replay_rejects_fact_query_evidence_with_mismatched_returned_ref_observed_at() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| {
        let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
        parts.observed_at = Some("2026-07-02T00:00:00Z".to_owned());
        mfm_facts::InternalFactRef::new(parts).expect("mismatched observed_at ref")
    });
    let mut authority = fixture.authority();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        4,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);
    authority.fact_query_receipt_trust_root = Some(query.trust_root);

    let error = ReplayBroker::from_read_authority(authority)
        .expect_err("mismatched returned ref observed_at rejects");
    assert_eq!(error.kind, ReplayErrorKind::FactMismatch);
}

#[test]
fn replay_rejects_fact_query_evidence_with_mismatched_returned_ref_producer_node_id() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| {
        let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
        parts.producer_node_id = node_id(0x44);
        mfm_facts::InternalFactRef::new(parts).expect("mismatched producer_node_id ref")
    });
    let mut authority = fixture.authority();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        4,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);
    authority.fact_query_receipt_trust_root = Some(query.trust_root);

    let error = ReplayBroker::from_read_authority(authority)
        .expect_err("mismatched returned ref producer_node_id rejects");
    assert_eq!(error.kind, ReplayErrorKind::FactMismatch);
}

#[test]
fn replay_rejects_fact_query_evidence_with_wrong_receipt_trust_root() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| fact_ref);
    let mut authority = fixture.authority();
    authority.stream.push(persisted_envelope(
        &fixture.run_id,
        4,
        KernelEventPayload::ArtifactReferenced(query.event.clone()),
    ));
    authority.artifact_evidence.push(query.artifact.clone());
    authority
        .artifact_bytes
        .insert(query.artifact.artifact_id.clone(), query.bytes);
    authority.fact_query_receipt_trust_root = Some(test_fact_query_trust_root(
        &SigningKey::from_bytes(&[8; 32]),
    ));

    let error =
        ReplayBroker::from_read_authority(authority).expect_err("wrong receipt trust root rejects");
    assert_eq!(error.kind, ReplayErrorKind::CertifiedEvidenceMismatch);
    assert!(error.message.contains("receipt authentication"));
}

#[test]
fn replay_rejects_fact_descriptor_allowed_only_for_other_node() {
    let descriptor = replay_stream_fact_descriptor();
    let other_descriptor = replay_stream_other_fact_descriptor();
    let descriptor_bytes =
        mfm_facts::canonical_fact_descriptor_bytes(&descriptor).expect("descriptor bytes");
    let descriptor_digest = mfm_facts::fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let descriptor_artifact = stored_artifact_ref(
        descriptor_digest.clone(),
        ArtifactRole::FactDescriptor,
        Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
        None,
        descriptor_bytes.as_bytes().len() as u64,
    );
    let other_descriptor_bytes =
        mfm_facts::canonical_fact_descriptor_bytes(&other_descriptor).expect("descriptor bytes");
    let other_descriptor_digest =
        mfm_facts::fact_descriptor_hash(&other_descriptor).expect("descriptor hash");
    let other_descriptor_artifact = stored_artifact_ref(
        other_descriptor_digest.clone(),
        ArtifactRole::FactDescriptor,
        Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
        None,
        other_descriptor_bytes.as_bytes().len() as u64,
    );

    let (response_artifact, response_bytes) =
        replay_stream_fact_response_artifact(&other_descriptor, 42);
    let certified_spec = hashed_fact_replay_spec_with_other_node_descriptor(&other_descriptor);
    let run_admitted = fact_run_admitted_for_stream_with_descriptors(
        &certified_spec,
        vec![
            run_artifact_ref_from_store_artifact_for_test(&descriptor_artifact),
            run_artifact_ref_from_store_artifact_for_test(&other_descriptor_artifact),
        ],
    );
    let run_id = run_admitted.run_id.clone();
    let node = certified_spec
        .spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact_node_id())
        .expect("fact node");
    let attempt_id = fact_attempt_id();
    let started = events::StateAttemptStarted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id: attempt_id.clone(),
        attempt_no: 1,
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
    };
    let fact = events::FactRecorded {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: fact_node_id(),
        attempt_id,
        claim: replay_stream_fact_claim(&other_descriptor, &response_artifact),
    };
    let authority = ReplayReadAuthority {
        certified_spec: certified_spec.clone(),
        stream: vec![
            persisted_envelope(
                &run_id,
                1,
                KernelEventPayload::RunAdmitted(Box::new(run_admitted)),
            ),
            persisted_envelope(&run_id, 2, KernelEventPayload::StateAttemptStarted(started)),
            persisted_envelope(&run_id, 3, KernelEventPayload::FactRecorded(fact)),
        ],
        canonicalizer_identity: certified_spec
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        artifact_evidence: vec![
            stored_artifact_from_run_ref(&stream_run_admitted_spec_artifact_for_hash(
                &certified_spec.spec_hash,
            )),
            stored_artifact_from_run_ref(&stream_run_admitted_certificate_artifact()),
            replay_stream_config_artifact(&certified_spec),
            descriptor_artifact,
            other_descriptor_artifact,
            response_artifact.clone(),
        ],
        artifact_bytes: BTreeMap::from([
            (
                descriptor_artifact_id(&descriptor_digest),
                descriptor_bytes.to_vec(),
            ),
            (
                descriptor_artifact_id(&other_descriptor_digest),
                other_descriptor_bytes.to_vec(),
            ),
            (response_artifact.artifact_id.clone(), response_bytes),
        ]),
        fact_query_receipt_trust_root: None,
        source_fact_events: Vec::new(),
    };

    let error = ReplayBroker::from_read_authority(authority)
        .expect_err("wrong-node descriptor must reject");
    assert_eq!(error.kind, ReplayErrorKind::CertifiedEvidenceMismatch);
    assert!(error.message.contains("not certified for producing node"));
}

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
        prepared: None,
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
        prepared: None,
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
        prepared: None,
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

fn replay_broker_with_facts(
    facts: Vec<(
        mfm_facts::FactClaimId,
        events::FactRecorded,
        StoredArtifactEvidenceRef,
    )>,
) -> ReplayBroker {
    let mut fact_map = BTreeMap::new();
    let mut artifact_map = BTreeMap::new();
    for (claim_id, fact, artifact) in facts {
        fact_map.insert(claim_id, fact);
        artifact_map.insert(
            (
                artifact.artifact_id.clone(),
                artifact.evidence_hash().expect("artifact evidence hash"),
            ),
            artifact,
        );
    }
    ReplayBroker {
        certified_spec: fact_replay_spec(),
        stream: Vec::new(),
        run_id: fact_run_admitted(),
        projection: ProjectionSnapshot::default(),
        retained_artifacts: artifact_map.clone(),
        artifact_bytes: BTreeMap::new(),
        artifact_byte_authority: BTreeMap::new(),
        fact_query_receipt_trust_root: None,
        artifacts: artifact_map,
        facts: fact_map,
        fact_events: BTreeMap::new(),
        intents: BTreeMap::new(),
        prepared_invocations: BTreeMap::new(),
        submissions: BTreeMap::new(),
        not_submitted: BTreeMap::new(),
        receipts: BTreeMap::new(),
        confirmations: BTreeMap::new(),
        ambiguities: BTreeMap::new(),
        manual_resolutions: BTreeMap::new(),
    }
}

fn fact_replay_spec() -> HashedSpecEnvelope {
    let descriptor_ref = spec::FactDescriptorRef {
        descriptor_hash: mfm_facts::fact_descriptor_hash(&replay_stream_fact_descriptor())
            .expect("descriptor hash"),
    };
    let capability = mfm_capabilities::CapabilityDescriptor::new(
        fact_capability_kind(),
        fact_capability_version(),
        mfm_capabilities::CapabilityRole::ReadExternal,
        "fact_read",
    )
    .expect("capability descriptor");
    let capability_bindings =
        CapabilitySetDescriptor::new(vec![capability]).expect("capability set");
    let config_ref = spec::ConfigRef {
        schema_id: schema_id("mfm.replay.test.config", 0xb0),
        artifact_id: artifact_id(0xb1),
        digest: content_digest(0xb2),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    };
    let input_bindings = spec::InputBindingSpec {
        input_schema_id: schema_id("mfm.replay.test.input", 0xb3),
        input_descriptor_id: descriptor_id(0xb4),
        root: spec::InputBindingNodeSpec::Unit,
        digest: content_digest(0xb5),
    };
    let public_schema_id = schema_id("mfm.replay.test.public", 0xb6);
    let renderer_descriptor = spec::RendererDescriptorIdentity {
        descriptor_id: descriptor_id(0xb7),
        renderer_kind: spec::RendererKind::new("replay-test-renderer").expect("renderer kind"),
        renderer_version: spec::RendererVersion::new("mfm.replay.test.renderer.v1")
            .expect("renderer version"),
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity: CanonicalizerIdentity::new("sha256-jcs-v1")
            .expect("canonicalizer identity"),
    };
    let node = spec::NodeSpec {
        node_id: fact_node_id(),
        stable_key: spec::StableAuthorKey::new("fact-node").expect("stable key"),
        scope_id: scope_id(0xb8),
        state_kind: state_kind(0xb9),
        state_version: mfm_ids::StateVersion::new("mfm.replay.test.fact_state.v1")
            .expect("state version"),
        descriptor_id: descriptor_id(0xba),
        context: spec::NodeContextSpec::no_context(),
        config_ref: config_ref.clone(),
        input_bindings,
        output_cell: cell_id(0xbb),
        effect_kind: effect_kind(0xbc),
        capability_bindings,
        adapter_bindings: vec![spec::AdapterBinding {
            adapter_kind: fact_adapter_kind(),
            adapter_version: fact_adapter_version(),
            binding_digest: None,
        }],
        fact_descriptor_allowlist: vec![descriptor_ref.clone()],
        side_effect: None,
        framework: None,
        planning_lineage: spec::PlanningLineage {
            active_operation_instances: Vec::new(),
            completed_operation_frames: Vec::new(),
            lineage_digest: content_digest(0xbd),
        },
        deterministic_predecessors: Vec::new(),
    };
    let state_identity = spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
        descriptor_id: node.descriptor_id.clone(),
        name: "mfm.replay.test.fact_state".to_owned(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        context: spec::StateContextDescriptorSpec::no_context(),
        input_context: spec::StateInputContextContractSpec::no_context(),
        output_context: spec::StateOutputContextContractSpec::no_context(),
        config_schema_id: node.config_ref.schema_id.clone(),
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        output_schema_id: schema_id("mfm.replay.test.output", 0xd5),
        output_semantic_type_id: semantic_type_id(0xd6),
        effect_kind: node.effect_kind.clone(),
        effect_class: "read".to_owned(),
        effect_name: "read".to_owned(),
        effect_version: mfm_ids::EffectVersion::new("mfm.replay.test.effect.v1")
            .expect("effect version"),
        capabilities: node.capability_bindings.clone(),
        emitted_fact_descriptors: vec![descriptor_ref],
        runner: "mfm.replay.test.runner".to_owned(),
        side_effect_contract_digest: None,
    }));
    let spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: descriptor_id(0xbe),
                name: "mfm.replay.test.fact_composition".to_owned(),
                version: "mfm.replay.test.fact_composition.v1".to_owned(),
            },
            config_hash: content_digest(0xbf),
        },
        saga: spec::SagaPolicySpec::NoSideEffects,
        contexts: Vec::new(),
        scopes: Vec::new(),
        seeds: Vec::new(),
        descriptor_identities: vec![state_identity],
        config_refs: vec![config_ref],
        nodes: vec![node],
        remediations: BTreeMap::new(),
        cells: Vec::new(),
        value_lineages: Vec::new(),
        planning_lineage: Vec::new(),
        public_outputs: spec::PublicOutputSpec {
            public_schema_id,
            outputs: Vec::new(),
            renderer_descriptor,
        },
    })
    .expect("typed spec");
    HashedSpecEnvelope {
        spec_hash: spec_hash(0xc0),
        spec,
        audit: spec::TypedExecutionSpecAudit::default(),
    }
}

fn fact_run_admitted() -> events::RunAdmitted {
    let spec_hash = spec_hash(0xc0);
    events::RunAdmitted {
        run_id: run_id(0xc1),
        identity_material: events::RunIdentityMaterialV1 {
            certified_spec_hash: spec_hash.clone(),
            trust_scope_id: mfm_ids::TrustScopeId::new(
                "mfm.trust_scope.v1:000000000000000000000000000000c1",
            )
            .expect("trust scope"),
            distinct_run_key_digest: None,
        },
        entry_point: events::EntryPointLaunchEvidence {
            resolved_op_id: events::EntryPointOpId::new("mfm.replay.test.fact")
                .expect("entry point"),
            entry_point_registry_digest: content_digest(0xc2),
        },
        spec_hash,
        spec_artifact: run_artifact_ref(
            ArtifactRole::TypedExecutionSpec,
            schema_id("mfm.replay.test.spec", 0xc4),
            content_digest(0xc5),
        ),
        certificate_artifact: run_artifact_ref(
            ArtifactRole::TypedSpecCertificate,
            schema_id("mfm.replay.test.certificate", 0xc7),
            content_digest(0xc8),
        ),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: mfm_ids::SpecVersion::new("1").expect("spec version"),
        lowering_version: mfm_ids::LoweringVersion::new("mfm.lowering.v1")
            .expect("lowering version"),
        public_output_schema_id: schema_id("mfm.replay.test.public", 0xc9),
        saga_policy_digest: content_digest(0xca),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(0xcb),
        canonicalizer_identity: CanonicalizerIdentity::new("sha256-jcs-v1")
            .expect("canonicalizer identity"),
        seed_cells: Vec::new(),
    }
}

fn run_artifact_ref(
    role: ArtifactRole,
    schema_id: SchemaId,
    content_digest: ContentDigest,
) -> events::RunArtifactEvidenceRef {
    let artifact_id = ArtifactId::from_digest(content_digest.algorithm(), *content_digest.digest());
    events::RunArtifactEvidenceRef {
        artifact_id,
        role,
        schema_id: Some(schema_id),
        semantic_type_id: None,
        content_digest,
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    }
}

fn fact_recorded(artifact: &StoredArtifactEvidenceRef) -> events::FactRecorded {
    events::FactRecorded {
        spec_hash: spec_hash(0xc0),
        node_id: fact_node_id(),
        attempt_id: fact_attempt_id(),
        claim: fact_claim(artifact),
    }
}

fn fact_claim(artifact: &StoredArtifactEvidenceRef) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new("mfm.replay.test.fact").expect("fact kind"),
        fact_descriptor_hash: content_digest(0xcc),
        subject: fact_subject_evidence(),
        observed_at: None,
        request: Some(mfm_facts::FactRequestEvidence::new(
            fact_request_schema_id(),
            fact_request_hash(),
        )),
        response: mfm_facts::FactResponseEvidence::new(
            fact_response_schema_id(),
            artifact.digest.clone(),
            artifact.artifact_id.clone(),
            artifact.evidence_hash().expect("artifact evidence hash"),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            fact_capability_kind(),
            fact_capability_version(),
            fact_adapter_kind(),
            fact_adapter_version(),
        ),
    })
    .expect("fact claim")
}

fn fact_subject_evidence() -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.account").expect("field id"),
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::string("same-subject"),
    )
    .expect("subject value")])
    .expect("subject material");
    mfm_facts::FactSubjectEvidence::from_material(content_digest(0xcd), &material)
        .expect("subject evidence")
}

#[derive(Clone)]
struct ReplayFactStreamFixture {
    certified_spec: HashedSpecEnvelope,
    stream: Vec<KernelEventEnvelope>,
    artifact_evidence: Vec<StoredArtifactEvidenceRef>,
    artifact_bytes: BTreeMap<ArtifactId, Vec<u8>>,
    run_id: RunId,
    claim_id: mfm_facts::FactClaimId,
    response_artifact_id: ArtifactId,
}

impl ReplayFactStreamFixture {
    fn authority(&self) -> ReplayReadAuthority {
        ReplayReadAuthority {
            certified_spec: self.certified_spec.clone(),
            stream: self.stream.clone(),
            canonicalizer_identity: self
                .certified_spec
                .spec
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            runner_executables: Vec::new(),
            adapter_executables: Vec::new(),
            artifact_evidence: self.artifact_evidence.clone(),
            artifact_bytes: self.artifact_bytes.clone(),
            fact_query_receipt_trust_root: None,
            source_fact_events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CellReplayTerminal {
    Produced,
    Skipped,
}

fn cell_replay_authority(
    terminal: CellReplayTerminal,
    event_context: spec::CellContextSpec,
) -> ReplayReadAuthority {
    let descriptor = replay_stream_fact_descriptor();
    let descriptor_bytes =
        mfm_facts::canonical_fact_descriptor_bytes(&descriptor).expect("descriptor bytes");
    let descriptor_digest = mfm_facts::fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let descriptor_artifact = stored_artifact_ref(
        descriptor_digest.clone(),
        ArtifactRole::FactDescriptor,
        Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
        None,
        descriptor_bytes.as_bytes().len() as u64,
    );
    let descriptor_run_ref = run_artifact_ref_from_store_artifact_for_test(&descriptor_artifact);

    let certified_spec = hashed_cell_replay_spec();
    let run_admitted = fact_run_admitted_for_stream(&certified_spec, descriptor_run_ref);
    let run_id = run_admitted.run_id.clone();
    let node = certified_spec
        .spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact_node_id())
        .expect("cell node");
    let cell = certified_spec
        .spec
        .cells
        .iter()
        .find(|cell| cell.cell_id == node.output_cell)
        .expect("output cell");
    let attempt_id = fact_attempt_id();
    let started = events::StateAttemptStarted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id: attempt_id.clone(),
        attempt_no: 1,
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
    };
    let mut artifact_evidence = vec![
        stored_artifact_from_run_ref(&stream_run_admitted_spec_artifact_for_hash(
            &certified_spec.spec_hash,
        )),
        stored_artifact_from_run_ref(&stream_run_admitted_certificate_artifact()),
        replay_stream_config_artifact(&certified_spec),
        descriptor_artifact,
    ];
    let terminal_payload = match terminal {
        CellReplayTerminal::Produced => {
            let output_digest = content_digest(0xe1);
            artifact_evidence.push(state_output_artifact_ref(cell, &output_digest));
            KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: certified_spec.spec_hash.clone(),
                node_id: node.node_id.clone(),
                cell_id: cell.cell_id.clone(),
                scope_id: cell.scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: cell.semantic_type_id.clone(),
                schema_id: cell.schema_id.clone(),
                value_lineage: cell.value_lineage.clone(),
                context: event_context,
                artifact_id: ArtifactId::from_digest(
                    output_digest.algorithm(),
                    *output_digest.digest(),
                ),
                content_digest: output_digest,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            })
        }
        CellReplayTerminal::Skipped => KernelEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: certified_spec.spec_hash.clone(),
            node_id: node.node_id.clone(),
            cell_id: cell.cell_id.clone(),
            scope_id: cell.scope_id.clone(),
            attempt_id: attempt_id.clone(),
            semantic_type_id: cell.semantic_type_id.clone(),
            schema_id: cell.schema_id.clone(),
            value_lineage: cell.value_lineage.clone(),
            context: event_context,
            skip_reason: events::SkipReason {
                code: events::ErrorCode::new("mfm.replay.test.skipped").expect("skip code"),
                safe_message: "skipped for replay context test".to_owned(),
            },
        }),
    };
    let completed = KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id: attempt_id.clone(),
        output_cell_id: cell.cell_id.clone(),
    });
    let terminal_commit_key = store::CommitKey::new("replay-test:3").expect("terminal commit key");
    ReplayReadAuthority {
        certified_spec: certified_spec.clone(),
        stream: vec![
            persisted_envelope(
                &run_id,
                1,
                KernelEventPayload::RunAdmitted(Box::new(run_admitted)),
            ),
            persisted_envelope(&run_id, 2, KernelEventPayload::StateAttemptStarted(started)),
            persisted_envelope_with_ordinal(&run_id, 3, 0, terminal_commit_key.clone(), completed),
            persisted_envelope_with_ordinal(&run_id, 3, 1, terminal_commit_key, terminal_payload),
        ],
        canonicalizer_identity: certified_spec
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        artifact_evidence,
        artifact_bytes: BTreeMap::from([(
            descriptor_artifact_id(&descriptor_digest),
            descriptor_bytes.to_vec(),
        )]),
        fact_query_receipt_trust_root: None,
        source_fact_events: Vec::new(),
    }
}

fn hashed_cell_replay_spec() -> HashedSpecEnvelope {
    let mut envelope = fact_replay_spec();
    let node = envelope
        .spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact_node_id())
        .expect("cell node")
        .clone();
    let lineage = spec::ValueLineageRef {
        lineage_digest: content_digest(0xe0),
    };
    envelope.spec.cells.push(spec::CellSpec {
        cell_id: node.output_cell.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        scope_id: node.scope_id.clone(),
        semantic_type_id: semantic_type_id(0xd6),
        schema_id: schema_id("mfm.replay.test.output", 0xd5),
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::MaybeSkipped,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    envelope.spec.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: node.scope_id.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        input_cells: Vec::new(),
        config_ref_digest: None,
        planning_lineage: node.planning_lineage,
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("hashed cell replay spec")
}

fn state_output_artifact_ref(
    cell: &spec::CellSpec,
    digest: &ContentDigest,
) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest: digest.clone(),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(cell.schema_id.clone()),
        semantic_type_id: Some(cell.semantic_type_id.clone()),
        producer_node_id: match &cell.producer {
            spec::CellProducer::Node(node_id) => Some(node_id.clone()),
            spec::CellProducer::Seed(_) => None,
        },
        producer_seed_id: None,
        artifact_role: ArtifactRole::StateOutput,
    }
}

fn mismatched_cell_context() -> spec::CellContextSpec {
    spec::CellContextSpec::Bound {
        context_ref: mfm_ids::ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0xe2),
        ),
        resource_kind: mfm_ids::ContextResourceKind::new("mfm.replay.test.resource")
            .expect("resource kind"),
        stage: mfm_ids::ContextStage::new("wrong_stage").expect("context stage"),
        producer: Box::new(spec::ContextProducerSpec {
            producer_descriptor_ids: vec![descriptor_id(0xe3)],
            seed_producers_allowed: false,
        }),
    }
}

fn replay_fact_stream_fixture() -> ReplayFactStreamFixture {
    let descriptor = replay_stream_fact_descriptor();
    let descriptor_bytes =
        mfm_facts::canonical_fact_descriptor_bytes(&descriptor).expect("descriptor bytes");
    let descriptor_digest = mfm_facts::fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let descriptor_artifact = stored_artifact_ref(
        descriptor_digest.clone(),
        ArtifactRole::FactDescriptor,
        Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
        None,
        descriptor_bytes.as_bytes().len() as u64,
    );
    let descriptor_run_ref = run_artifact_ref_from_store_artifact_for_test(&descriptor_artifact);

    let (response_artifact, response_bytes) = replay_stream_fact_response_artifact(&descriptor, 42);
    let certified_spec = hashed_fact_replay_spec();
    let run_admitted = fact_run_admitted_for_stream(&certified_spec, descriptor_run_ref);
    let run_id = run_admitted.run_id.clone();
    let attempt_id = fact_attempt_id();
    let node = certified_spec
        .spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact_node_id())
        .expect("fact node");
    let started = events::StateAttemptStarted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id: attempt_id.clone(),
        attempt_no: 1,
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
    };
    let fact = events::FactRecorded {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: fact_node_id(),
        attempt_id,
        claim: replay_stream_fact_claim(&descriptor, &response_artifact),
    };
    let stream = vec![
        persisted_envelope(
            &run_id,
            1,
            KernelEventPayload::RunAdmitted(Box::new(run_admitted)),
        ),
        persisted_envelope(&run_id, 2, KernelEventPayload::StateAttemptStarted(started)),
        persisted_envelope(&run_id, 3, KernelEventPayload::FactRecorded(fact)),
    ];
    let claim_id = mfm_facts::derive_fact_claim_id(run_id, 3, 0).expect("fact claim id");
    let response_artifact_id = response_artifact.artifact_id.clone();
    let config_artifact = replay_stream_config_artifact(&certified_spec);
    ReplayFactStreamFixture {
        certified_spec,
        stream,
        artifact_evidence: vec![
            stored_artifact_from_run_ref(&stream_run_admitted_spec_artifact()),
            stored_artifact_from_run_ref(&stream_run_admitted_certificate_artifact()),
            config_artifact,
            descriptor_artifact,
            response_artifact,
        ],
        artifact_bytes: BTreeMap::from([
            (
                descriptor_artifact_id(&descriptor_digest),
                descriptor_bytes.to_vec(),
            ),
            (response_artifact_id.clone(), response_bytes),
        ]),
        run_id: claim_id.source_run_id().clone(),
        claim_id,
        response_artifact_id,
    }
}

struct ReplayFactQueryEvidenceArtifact {
    artifact: StoredArtifactEvidenceRef,
    bytes: Vec<u8>,
    event: events::ArtifactReferenced,
    trust_root: store::FactQueryReceiptTrustRoot,
}

fn fact_query_evidence_artifact(
    fixture: &ReplayFactStreamFixture,
    mutate_ref: impl FnOnce(mfm_facts::InternalFactRef) -> mfm_facts::InternalFactRef,
) -> ReplayFactQueryEvidenceArtifact {
    let key = SigningKey::from_bytes(&[7; 32]);
    let trust_root = test_fact_query_trust_root(&key);
    let plan = replay_fact_query_plan();
    let plan_hash = mfm_facts::fact_query_plan_hash(&plan).expect("plan hash");
    let returned_ref = mutate_ref(internal_fact_ref_for_fixture(fixture));
    let rows = [mfm_facts::FactQueryResultRow::new(
        returned_ref,
        vec![mfm_facts::FactFieldValue::new(
            mfm_facts::FactFieldId::new("result.amount").expect("field"),
            mfm_facts::FactFieldValueType::UnsignedInteger,
            mfm_facts::FactCanonicalScalar::UnsignedInteger(42),
        )
        .expect("field summary")],
    )];
    let frontier = mfm_facts::StoreReadFrontier::new(
        mfm_facts::StoreScopeRef::new("default").expect("store scope"),
        mfm_facts::FactQueryScope::new(
            mfm_facts::FactAudience::Platform,
            mfm_facts::FactVisibilityScope::Default,
        ),
        mfm_facts::DescriptorCatalogWatermark::new(1),
        mfm_facts::FactProjectionGeneration::new(1),
        3,
        mfm_facts::StoreCommitWatermark::new(3),
    );
    let receipt = signed_fact_query_receipt_for_test(SignedFactQueryReceiptFixtureInputForTest {
        plan_hash: &plan_hash,
        key: &key,
        store_identity: trust_root.store_identity().clone(),
        key_id: trust_root.key_id().clone(),
        read_frontier: frontier,
        rows: &rows,
        include_returned_field_summaries: true,
        limit: None,
    });
    let selected_summaries_digest = mfm_facts::selected_returned_field_summaries_digest(
        receipt.returned_field_summaries().expect("summaries"),
        &[0],
    )
    .expect("selected summaries digest");
    let selection = mfm_facts::FactSelectionEvidence::new(
        content_digest(0x45),
        vec![0],
        Some(selected_summaries_digest),
    )
    .expect("selection");
    let evidence = mfm_facts::FactQueryEvidence::new(plan, receipt, selection);
    let bytes =
        mfm_facts::canonical_fact_query_evidence_bytes(&evidence).expect("query evidence bytes");
    let artifact = stored_artifact_ref(
        bytes.content_digest(),
        ArtifactRole::FactQueryEvidence,
        Some(mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema")),
        Some(fact_node_id()),
        bytes.as_bytes().len() as u64,
    );
    let event = events::ArtifactReferenced {
        spec_hash: fixture.certified_spec.spec_hash.clone(),
        node_id: Some(fact_node_id()),
        attempt_id: Some(fact_attempt_id()),
        artifact_ref: events::ArtifactEvidenceRef {
            artifact_id: artifact.artifact_id.clone(),
            role: ArtifactRole::FactQueryEvidence,
            schema_id: artifact.schema_id.clone().expect("schema id"),
            semantic_type_id: None,
            content_digest: artifact.digest.clone(),
            byte_len: artifact.byte_len,
            media_type: artifact.media_type.clone(),
        },
    };
    ReplayFactQueryEvidenceArtifact {
        artifact,
        bytes: bytes.to_vec(),
        event,
        trust_root,
    }
}

fn replay_fact_query_plan() -> mfm_facts::CanonicalFactQueryPlan {
    let input = mfm_facts::FactQueryInput::new(
        mfm_facts::StoreScopeRef::new("default").expect("store scope"),
        mfm_facts::FactQueryScope::new(
            mfm_facts::FactAudience::Platform,
            mfm_facts::FactVisibilityScope::Default,
        ),
        mfm_facts::ScopeDecisionEvidence::new(content_digest(0x46)),
        vec![mfm_facts::FactQueryPredicate::new(
            mfm_facts::FactFieldId::new("subject.account").expect("field"),
            mfm_facts::FactQueryOperator::Equal,
            mfm_facts::FactCanonicalScalar::string("same-subject"),
        )],
        vec![mfm_facts::FactFieldId::new("result.amount").expect("field")],
        mfm_facts::FactOrderingName::new("result.amount.desc").expect("ordering"),
        Some(1),
    )
    .expect("query input");
    mfm_facts::compile_fact_query_plan(&replay_stream_fact_descriptor(), input).expect("query plan")
}

fn internal_fact_ref_for_fixture(fixture: &ReplayFactStreamFixture) -> mfm_facts::InternalFactRef {
    let event = fixture.stream.get(2).expect("fact event");
    let KernelEventPayload::FactRecorded(fact) = event.payload() else {
        panic!("expected fact event");
    };
    mfm_facts::InternalFactRef::from_claim(
        fixture.claim_id.clone(),
        event.event_id().clone(),
        "2026-07-01T00:00:00Z".to_owned(),
        fact_node_id(),
        &fact.claim,
    )
    .expect("internal fact ref")
    .expect("indexed fact ref")
}

fn internal_fact_ref_parts_from_ref(
    fact_ref: &mfm_facts::InternalFactRef,
) -> mfm_facts::InternalFactRefParts {
    mfm_facts::InternalFactRefParts {
        fact_claim_id: fact_ref.fact_claim_id().clone(),
        source_event_id: fact_ref.source_event_id().clone(),
        recorded_at: fact_ref.recorded_at().to_owned(),
        producer_node_id: fact_ref.producer_node_id().clone(),
        observed_at: fact_ref.observed_at().map(ToOwned::to_owned),
        visibility: fact_ref.visibility().clone(),
        fact_kind: fact_ref.fact_kind().clone(),
        fact_descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
        subject: mfm_facts::FactSubjectRef::new(
            fact_ref.fact_subject_namespace_hash().clone(),
            fact_ref.fact_key().clone(),
            fact_ref.subject_material_hash().clone(),
        ),
        request: match (fact_ref.request_schema_id(), fact_ref.request_hash()) {
            (Some(schema_id), Some(hash)) => Some(mfm_facts::FactRequestEvidence::new(
                schema_id.clone(),
                hash.clone(),
            )),
            (None, None) => None,
            _ => unreachable!("internal fact refs cannot carry partial request evidence"),
        },
        response: mfm_facts::FactResponseEvidence::new(
            fact_ref.response_schema_id().clone(),
            fact_ref.response_hash().clone(),
            fact_ref.artifact_id().clone(),
            fact_ref.artifact_evidence_hash().clone(),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            fact_ref.capability_kind().clone(),
            fact_ref.capability_version().clone(),
            fact_ref.adapter_kind().clone(),
            fact_ref.adapter_version().clone(),
        ),
    }
}

fn test_fact_query_trust_root(key: &SigningKey) -> store::FactQueryReceiptTrustRoot {
    fact_query_receipt_trust_root_for_test(
        key,
        mfm_facts::StoreIdentity::new("store.default").expect("store identity"),
        mfm_facts::StoreKeyId::new("key.default").expect("key id"),
    )
}

fn replay_stream_fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.replay.test.fact").expect("fact kind"),
        mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
        schema_id("mfm.replay.test.fact_subject", 0xe0),
        fact_response_schema_id(),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("subject.account").expect("field id"),
                mfm_facts::FactFieldValueType::String,
                mfm_facts::FactFieldExtraction::Subject(
                    mfm_facts::CanonicalValuePath::new("account").expect("subject path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("result.amount").expect("field id"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Response(
                    mfm_facts::CanonicalValuePath::new("amount").expect("response path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .sortable()
                .required(),
            )
            .expect("result field"),
        ],
        vec![mfm_facts::FactOrderingPolicy::new(
            mfm_facts::FactOrderingName::new("result.amount.desc").expect("ordering"),
            vec![mfm_facts::FactOrderingTerm::new(
                mfm_facts::FactFieldId::new("result.amount").expect("field"),
                mfm_facts::SortDirection::Descending,
                mfm_facts::NullOrdering::Last,
                false,
            )],
        )
        .expect("ordering")],
    )
    .expect("fact descriptor")
}

fn replay_stream_other_fact_descriptor() -> mfm_facts::FactDescriptor {
    let descriptor = replay_stream_fact_descriptor();
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.replay.test.other_fact").expect("fact kind"),
        descriptor.descriptor_schema_id().clone(),
        descriptor.subject_schema_id().clone(),
        descriptor.response_schema_id().clone(),
        descriptor.fields().to_vec(),
        descriptor.orderings().to_vec(),
    )
    .expect("other fact descriptor")
}

fn replay_stream_fact_subject_evidence(
    descriptor: &mfm_facts::FactDescriptor,
) -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.account").expect("field id"),
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::string("same-subject"),
    )
    .expect("subject value")])
    .expect("subject material");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(descriptor).expect("subject namespace hash");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

fn replay_stream_fact_response_artifact(
    descriptor: &mfm_facts::FactDescriptor,
    amount: u64,
) -> (StoredArtifactEvidenceRef, Vec<u8>) {
    let json =
        serde_json::to_string(&serde_json::json!({ "amount": amount })).expect("response json");
    let bytes =
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical response");
    let digest = bytes.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    (
        StoredArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: bytes.as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media type"),
            schema_id: Some(descriptor.response_schema_id().clone()),
            semantic_type_id: None,
            producer_node_id: Some(fact_node_id()),
            producer_seed_id: None::<SeedId>,
            artifact_role: ArtifactRole::FactResponse,
        },
        bytes.to_vec(),
    )
}

fn replay_stream_fact_claim(
    descriptor: &mfm_facts::FactDescriptor,
    response_artifact: &StoredArtifactEvidenceRef,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: descriptor.fact_kind().clone(),
        fact_descriptor_hash: mfm_facts::fact_descriptor_hash(descriptor).expect("descriptor hash"),
        subject: replay_stream_fact_subject_evidence(descriptor),
        observed_at: None,
        request: Some(mfm_facts::FactRequestEvidence::new(
            fact_request_schema_id(),
            fact_request_hash(),
        )),
        response: mfm_facts::FactResponseEvidence::new(
            descriptor.response_schema_id().clone(),
            response_artifact.digest.clone(),
            response_artifact.artifact_id.clone(),
            response_artifact
                .evidence_hash()
                .expect("response evidence hash"),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            fact_capability_kind(),
            fact_capability_version(),
            fact_adapter_kind(),
            fact_adapter_version(),
        ),
    })
    .expect("fact claim")
}

fn hashed_fact_replay_spec() -> HashedSpecEnvelope {
    let envelope = fact_replay_spec();
    HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("hashed replay spec")
}

fn hashed_fact_replay_spec_with_other_node_descriptor(
    other_descriptor: &mfm_facts::FactDescriptor,
) -> HashedSpecEnvelope {
    let mut envelope = fact_replay_spec();
    let other_ref = spec::FactDescriptorRef {
        descriptor_hash: mfm_facts::fact_descriptor_hash(other_descriptor)
            .expect("other descriptor hash"),
    };
    let template = envelope.spec.nodes[0].clone();
    let other_descriptor_id = descriptor_id(0xdd);
    let other_node = spec::NodeSpec {
        node_id: node_id(0xde),
        stable_key: spec::StableAuthorKey::new("other-fact-node").expect("stable key"),
        scope_id: template.scope_id.clone(),
        state_kind: template.state_kind.clone(),
        state_version: template.state_version.clone(),
        descriptor_id: other_descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: template.config_ref.clone(),
        input_bindings: template.input_bindings.clone(),
        output_cell: cell_id(0xdf),
        effect_kind: template.effect_kind.clone(),
        capability_bindings: template.capability_bindings.clone(),
        adapter_bindings: template.adapter_bindings.clone(),
        fact_descriptor_allowlist: vec![other_ref.clone()],
        side_effect: None,
        framework: None,
        planning_lineage: template.planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    };
    let mut other_state_identity = envelope
        .spec
        .descriptor_identities
        .iter()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) => Some((**state).clone()),
            _ => None,
        })
        .expect("state descriptor identity");
    other_state_identity.descriptor_id = other_descriptor_id;
    other_state_identity.name = "mfm.replay.test.other_fact_state".to_owned();
    other_state_identity.emitted_fact_descriptors = vec![other_ref];
    envelope
        .spec
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            other_state_identity,
        )));
    envelope.spec.nodes.push(other_node);
    HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("hashed replay spec")
}

fn fact_run_admitted_for_stream(
    certified_spec: &HashedSpecEnvelope,
    descriptor: events::RunArtifactEvidenceRef,
) -> events::RunAdmitted {
    fact_run_admitted_for_stream_with_descriptors(certified_spec, vec![descriptor])
}

fn fact_run_admitted_for_stream_with_descriptors(
    certified_spec: &HashedSpecEnvelope,
    descriptors: Vec<events::RunArtifactEvidenceRef>,
) -> events::RunAdmitted {
    let spec_hash = certified_spec.spec_hash.clone();
    let identity_material = events::RunIdentityMaterialV1 {
        certified_spec_hash: spec_hash.clone(),
        trust_scope_id: mfm_ids::TrustScopeId::new(
            "mfm.trust_scope.v1:000000000000000000000000000000c1",
        )
        .expect("trust scope"),
        distinct_run_key_digest: None,
    };
    let run_id = identity_material.derive_run_id().expect("run id");
    events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: events::EntryPointLaunchEvidence {
            resolved_op_id: events::EntryPointOpId::new("mfm.replay.test.fact")
                .expect("entry point"),
            entry_point_registry_digest: content_digest(0xc2),
        },
        spec_hash,
        spec_artifact: stream_run_admitted_spec_artifact_for_hash(&certified_spec.spec_hash),
        certificate_artifact: stream_run_admitted_certificate_artifact(),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: descriptors,
        spec_version: certified_spec.spec.spec_version.clone(),
        lowering_version: certified_spec.spec.lowering_version.clone(),
        public_output_schema_id: certified_spec.spec.public_outputs.public_schema_id.clone(),
        saga_policy_digest: content_digest(0xca),
        descriptor_identities: certified_spec.spec.descriptor_identities.clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: admitted_binding_digest(&[], &[]).expect("binding digest"),
        canonicalizer_identity: certified_spec
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        seed_cells: Vec::new(),
    }
}

fn stream_run_admitted_spec_artifact() -> events::RunArtifactEvidenceRef {
    stream_run_admitted_spec_artifact_for_hash(&hashed_fact_replay_spec().spec_hash)
}

fn stream_run_admitted_spec_artifact_for_hash(
    spec_hash: &SpecHash,
) -> events::RunArtifactEvidenceRef {
    run_artifact_ref(
        ArtifactRole::TypedExecutionSpec,
        spec::typed_execution_spec_schema_id().expect("spec schema"),
        spec_digest(spec_hash),
    )
}

fn stream_run_admitted_certificate_artifact() -> events::RunArtifactEvidenceRef {
    run_artifact_ref(
        ArtifactRole::TypedSpecCertificate,
        mfm_certify::typed_spec_certificate_schema_id().expect("cert schema"),
        content_digest(0xc8),
    )
}

fn stored_artifact_from_run_ref(
    artifact: &events::RunArtifactEvidenceRef,
) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: artifact.role,
    }
}

fn replay_stream_config_artifact(certified_spec: &HashedSpecEnvelope) -> StoredArtifactEvidenceRef {
    let config = certified_spec.spec.config_refs.first().expect("config ref");
    StoredArtifactEvidenceRef {
        artifact_id: config.artifact_id.clone(),
        digest: config.digest.clone(),
        byte_len: config.byte_len,
        media_type: config.media_type.clone(),
        schema_id: Some(config.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedConfig,
    }
}

fn stored_artifact_ref(
    digest: ContentDigest,
    role: ArtifactRole,
    schema_id: Option<SchemaId>,
    producer_node_id: Option<NodeId>,
    byte_len: u64,
) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id,
        semantic_type_id: None,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn descriptor_artifact_id(digest: &ContentDigest) -> ArtifactId {
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
}

fn persisted_envelope(
    run_id: &RunId,
    seq: u64,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    store_persisted_kernel_event_envelope_for_test(
        run_id,
        seq,
        store::CommitKey::new(format!("replay-test:{seq}")).expect("commit key"),
        payload,
    )
}

fn persisted_envelope_with_ordinal(
    run_id: &RunId,
    seq: u64,
    ordinal: u32,
    commit_key: store::CommitKey,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    store_persisted_kernel_event_envelope_with_ordinal_for_test(
        run_id, seq, ordinal, commit_key, payload,
    )
}

fn fact_response_artifact(byte: u8) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: artifact_id(byte),
        digest: content_digest(byte.wrapping_add(1)),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(fact_response_schema_id()),
        semantic_type_id: None,
        producer_node_id: Some(fact_node_id()),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::FactResponse,
    }
}

fn fact_node_id() -> NodeId {
    node_id(0xce)
}

fn fact_attempt_id() -> AttemptId {
    attempt_id(0xcf)
}

fn fact_capability_kind() -> CapabilityKind {
    CapabilityKind::new(
        "mfm.replay.test",
        "fact_read",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(0xd0),
    )
    .expect("capability kind")
}

fn fact_capability_version() -> CapabilityVersion {
    CapabilityVersion::new("mfm.replay.test.fact_read.v1").expect("capability version")
}

fn fact_adapter_kind() -> AdapterKind {
    AdapterKind::new(
        "mfm.replay.test",
        "fact_adapter",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(0xd1),
    )
    .expect("adapter kind")
}

fn fact_adapter_version() -> AdapterVersion {
    AdapterVersion::new("mfm.replay.test.fact_adapter.v1").expect("adapter version")
}

fn fact_request_schema_id() -> SchemaId {
    schema_id("mfm.replay.test.fact_request", 0xd2)
}

fn fact_request_hash() -> ContentDigest {
    content_digest(0xd3)
}

fn fact_response_schema_id() -> SchemaId {
    schema_id("mfm.replay.test.fact_response", 0xd4)
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

fn descriptor_id(byte: u8) -> mfm_ids::DescriptorId {
    mfm_ids::DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn scope_id(byte: u8) -> mfm_ids::ScopeId {
    mfm_ids::ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn state_kind(byte: u8) -> mfm_ids::StateKind {
    mfm_ids::StateKind::new(
        "mfm.replay.test",
        "state",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("state kind")
}

fn effect_kind(byte: u8) -> mfm_ids::EffectKind {
    mfm_ids::EffectKind::new(
        "mfm.replay.test",
        "effect",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("effect kind")
}

fn cell_id(byte: u8) -> mfm_ids::CellId {
    mfm_ids::CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn semantic_type_id(byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.replay.test",
        "value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("semantic type id")
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
