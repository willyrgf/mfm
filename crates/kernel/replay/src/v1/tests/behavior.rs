use super::*;

#[test]
fn replay_retained_artifact_lookup_uses_exact_evidence_identity() {
    let first = fact_response_artifact(0xa1);
    let mut second = first.clone();
    second.producer_node_id = Some(node_id(0xd1));
    let first_hash = first.evidence_hash().expect("first evidence hash");
    let second_hash = second.evidence_hash().expect("second evidence hash");
    assert_ne!(first_hash, second_hash);

    let artifacts = artifact_map(vec![first.clone(), second.clone()]).expect("artifact map");
    let mut broker = replay_broker_with_facts(Vec::new());
    broker.retained_artifacts = artifacts.clone();
    broker.artifacts = artifacts;
    broker.artifact_bytes.insert(
        (first.artifact_id.clone(), first_hash.clone()),
        vec![b'{', b'}'],
    );
    broker.artifact_bytes.insert(
        (second.artifact_id.clone(), second_hash.clone()),
        vec![b'{', b'}'],
    );

    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::FactResponse,
        artifact_id: second.artifact_id.clone(),
        evidence_hash: second_hash,
        digest: Some(second.digest.clone()),
        byte_len: Some(second.byte_len),
        media_type: Some(second.media_type.clone()),
        schema_id: second.schema_id.clone(),
        semantic_type_id: second.semantic_type_id.clone(),
        producer_node_id: second.producer_node_id.clone(),
        producer_seed_id: second.producer_seed_id.clone(),
        artifact_role: Some(second.artifact_role),
    };
    let selected = broker
        .retained_artifact(&requirement)
        .expect("exact retained evidence");
    assert_eq!(selected.artifact.producer_node_id, second.producer_node_id);

    let mut wrong_key = requirement.clone();
    wrong_key.evidence_hash = first_hash;
    assert!(broker.retained_artifact(&wrong_key).is_err());
}

#[test]
fn replay_run_admission_artifacts_use_exact_evidence_identity() {
    let expected = run_artifact_ref(
        ArtifactRole::TypedConfig,
        schema_id("mfm.replay.test.config", 0xd2),
        content_digest(0xd3),
    );
    let mut wrong = StoredArtifactEvidenceRef::from_run_artifact(&expected);
    wrong.media_type = spec::MediaType::new("application/octet-stream").expect("media type");
    let wrong_key = replay_artifact_authority_key(&wrong).expect("wrong artifact key");
    let artifacts = BTreeMap::from([(wrong_key, wrong)]);

    let error = verify_run_artifact(&artifacts, &expected, ArtifactRole::TypedConfig)
        .expect_err("different artifact evidence must not satisfy run admission");
    assert_eq!(error.kind, ReplayErrorKind::ArtifactMissing);
}

#[test]
fn replay_artifact_expectations_check_artifact_id_without_hash() {
    let expected = run_artifact_ref(
        ArtifactRole::TypedConfig,
        schema_id("mfm.replay.test.config", 0xd4),
        content_digest(0xd5),
    );
    let expected_evidence = StoredArtifactEvidenceRef::from_run_artifact(&expected);
    let mut wrong = expected_evidence.clone();
    wrong.artifact_id = artifact_id(0xd6);
    let expected_evidence_hash = expected_evidence.evidence_hash().expect("evidence hash");

    let error = verify_artifact_expectation(
        &wrong,
        ArtifactEvidenceExpectation {
            artifact_id: &expected_evidence.artifact_id,
            evidence_hash: &expected_evidence_hash,
            digest: &expected_evidence.digest,
            schema_id: expected_evidence.schema_id.as_ref(),
            semantic_type_id: None,
            role: ArtifactRole::TypedConfig,
            producer_node_id: None,
            producer_seed_id: None,
        },
    )
    .expect_err("an artifact with a different id must not satisfy the expectation");
    assert_eq!(error.kind, ReplayErrorKind::ArtifactMismatch);
}

#[test]
fn replay_broker_rebuilds_fact_projection_with_retained_artifact_bytes() {
    let fixture = replay_fact_stream_fixture();

    let broker = ReplayBroker::from_read_authority(fixture.authority())
        .expect("fact-bearing replay authority rebuilds");

    assert!(
        broker
            .projection_snapshot()
            .fact_query_entry(&fixture.claim_id)
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
    let response_artifact_evidence_hash = mismatched_bytes
        .artifact_evidence
        .iter()
        .find(|artifact| artifact.artifact_id == fixture.response_artifact_id)
        .expect("response artifact evidence")
        .evidence_hash()
        .expect("response artifact evidence hash");
    mismatched_bytes.artifact_bytes.insert(
        (
            fixture.response_artifact_id.clone(),
            response_artifact_evidence_hash,
        ),
        b"{\"amount\":999}".to_vec(),
    );
    let error = ReplayBroker::from_read_authority(mismatched_bytes)
        .expect_err("mismatched retained fact response bytes must fail closed");
    assert_eq!(error.kind, ReplayErrorKind::ArtifactMismatch);
}

#[test]
fn replay_fact_batch_comparison_rejects_missing_extra_reordered_and_wrong_facts() {
    let fixture = replay_fact_stream_fixture_with_values(&[("account-a", 41), ("account-b", 42)]);
    let broker = ReplayBroker::from_read_authority(fixture.authority())
        .expect("two-fact replay authority rebuilds");
    let mut frames = broker
        .produced_cell_frames_matching(|node, _, _| Ok(node.node_id == fact_node_id()))
        .expect("fact output frame");
    assert_eq!(frames.len(), 1);
    let frame = frames.pop().expect("one fact output frame");
    let exact = vec![
        ReplayStreamFact::new("account-a", 41),
        ReplayStreamFact::new("account-b", 42),
    ];
    verify_recorded_fact_batch_evidence(&broker, &frame, &exact)
        .expect("exact reducer fact batch verifies");

    for (name, facts) in [
        ("missing", vec![ReplayStreamFact::new("account-a", 41)]),
        (
            "extra",
            vec![
                ReplayStreamFact::new("account-a", 41),
                ReplayStreamFact::new("account-b", 42),
                ReplayStreamFact::new("account-c", 43),
            ],
        ),
        (
            "reordered",
            vec![
                ReplayStreamFact::new("account-b", 42),
                ReplayStreamFact::new("account-a", 41),
            ],
        ),
        (
            "wrong",
            vec![
                ReplayStreamFact::new("account-a", 99),
                ReplayStreamFact::new("account-b", 42),
            ],
        ),
    ] {
        let error = verify_recorded_fact_batch_evidence(&broker, &frame, &facts)
            .expect_err("non-exact fact batch must reject");
        assert_eq!(
            error.kind,
            ReplayErrorKind::CertifiedEvidenceMismatch,
            "{name}"
        );
    }
}

#[test]
fn replay_rejects_duplicate_fact_keys_in_one_settlement() {
    let fixture =
        replay_fact_stream_fixture_with_values(&[("same-account", 41), ("same-account", 42)]);

    let error = ReplayBroker::from_read_authority(fixture.authority())
        .expect_err("duplicate fact keys must reject");
    assert_eq!(error.kind, ReplayErrorKind::InvalidRunStream);
    assert!(error.message.contains("fact keys must be unique"));
}

#[test]
fn replay_rejects_cell_terminal_context_mismatches() {
    for (terminal, message_fragment) in [
        (CellReplayTerminal::Produced, "produced cell"),
        (CellReplayTerminal::Skipped, "skipped cell"),
    ] {
        ReplayBroker::from_read_authority(cell_replay_authority(
            terminal,
            spec::CellContextSpec::no_context(),
        ))
        .expect("matching cell context verifies");

        let error = ReplayBroker::from_read_authority(cell_replay_authority(
            terminal,
            mismatched_cell_context(),
        ))
        .expect_err("mismatched cell context rejects");
        assert_eq!(error.kind, ReplayErrorKind::CertifiedEvidenceMismatch);
        assert!(
            error.message.contains(message_fragment),
            "{terminal:?}: {error}"
        );
    }
}

#[test]
fn replay_verifies_fact_query_evidence_from_retained_authority() {
    let fixture = replay_fact_stream_fixture();
    let query = fact_query_evidence_artifact(&fixture, |fact_ref| fact_ref);
    let mut authority = fixture.authority();
    append_fact_query_evidence(&mut authority, &fixture.run_id, 4, &query);

    ReplayBroker::from_read_authority(authority).expect("query evidence verifies");
}

#[test]
fn replay_verifies_fact_query_evidence_with_retained_cross_run_source_fact() {
    let fixture = replay_fact_stream_fixture();
    let source_run_id = run_id(0x77);
    let source_claim_id =
        mfm_facts::FactClaimId::new(source_run_id.clone(), 3, 0).expect("source claim id");
    // Cross-op source facts keep their producing program's SpecHash (not the consumer's).
    let mut source_payload = fixture.stream[2].payload().clone();
    if let KernelEventPayload::FactRecorded(payload) = &mut source_payload {
        payload.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0xef));
    }
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
    let unreferenced_output_keys = authority
        .artifact_evidence
        .iter()
        .filter(|artifact| artifact.artifact_role == ArtifactRole::StateOutput)
        .map(replay_artifact_authority_key)
        .collect::<Result<Vec<_>>>()
        .expect("state output artifact keys");
    authority
        .artifact_evidence
        .retain(|artifact| artifact.artifact_role != ArtifactRole::StateOutput);
    authority
        .artifact_bytes
        .retain(|key, _| !unreferenced_output_keys.contains(key));
    append_fact_query_evidence(&mut authority, &fixture.run_id, 3, &query);
    authority.source_fact_events.push(source_event);

    ReplayBroker::from_read_authority(authority).expect("cross-run source fact verifies");
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
    append_fact_query_evidence(&mut authority, &fixture.run_id, 4, &query);

    let error =
        ReplayBroker::from_read_authority(authority).expect_err("missing source fact rejects");
    assert_eq!(error.kind, ReplayErrorKind::FactMissing);
}

#[test]
fn replay_rejects_fact_query_evidence_with_mismatched_returned_refs() {
    enum ReturnedRefMismatch {
        DescriptorHash,
        ResponseDigest,
        FactKind,
        ProducerNodeId,
        SubjectMaterial,
    }

    for (name, mismatch) in [
        ("descriptor hash", ReturnedRefMismatch::DescriptorHash),
        ("response digest", ReturnedRefMismatch::ResponseDigest),
        ("fact kind", ReturnedRefMismatch::FactKind),
        ("producer node id", ReturnedRefMismatch::ProducerNodeId),
        ("subject material", ReturnedRefMismatch::SubjectMaterial),
    ] {
        let fixture = replay_fact_stream_fixture();
        let query = match mismatch {
            ReturnedRefMismatch::DescriptorHash => {
                fact_query_evidence_artifact(&fixture, |fact_ref| {
                    let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
                    parts.fact_descriptor_hash =
                        mfm_facts::fact_descriptor_hash(&replay_stream_other_fact_descriptor())
                            .expect("other descriptor hash");
                    mfm_facts::InternalFactRef::new(parts).expect("mismatched descriptor ref")
                })
            }
            ReturnedRefMismatch::ResponseDigest => {
                fact_query_evidence_artifact(&fixture, |fact_ref| {
                    let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
                    parts.response = mfm_facts::FactResponseEvidence::new(
                        parts.response.response_schema_id().clone(),
                        content_digest(0x44),
                        parts.response.artifact_id().clone(),
                        parts.response.artifact_evidence_hash().clone(),
                    );
                    mfm_facts::InternalFactRef::new(parts).expect("mismatched returned ref")
                })
            }
            ReturnedRefMismatch::FactKind => fact_query_evidence_artifact(&fixture, |fact_ref| {
                let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
                parts.fact_kind =
                    mfm_facts::FactKind::new("mfm.replay.test.other_fact").expect("fact kind");
                mfm_facts::InternalFactRef::new(parts).expect("mismatched fact kind ref")
            }),
            ReturnedRefMismatch::ProducerNodeId => {
                fact_query_evidence_artifact(&fixture, |fact_ref| {
                    let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
                    parts.producer_node_id = node_id(0x44);
                    mfm_facts::InternalFactRef::new(parts).expect("mismatched producer_node_id ref")
                })
            }
            ReturnedRefMismatch::SubjectMaterial => {
                fact_query_evidence_artifact(&fixture, |fact_ref| {
                    let mut parts = internal_fact_ref_parts_from_ref(&fact_ref);
                    parts.subject = mfm_facts::FactSubjectRef::new(
                        parts.subject.fact_subject_namespace_hash().clone(),
                        parts.subject.fact_key().clone(),
                        content_digest(0x45),
                    );
                    mfm_facts::InternalFactRef::new(parts).expect("mismatched subject material ref")
                })
            }
        };
        let mut authority = fixture.authority();
        append_fact_query_evidence(&mut authority, &fixture.run_id, 4, &query);

        let error = ReplayBroker::from_read_authority(authority).expect_err(name);
        assert_eq!(error.kind, ReplayErrorKind::FactMismatch, "{name}");
    }
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
    let descriptor_artifact_key =
        replay_artifact_authority_key(&descriptor_artifact).expect("descriptor artifact key");
    let other_descriptor_artifact_key = replay_artifact_authority_key(&other_descriptor_artifact)
        .expect("other descriptor artifact key");
    let response_artifact_key =
        replay_artifact_authority_key(&response_artifact).expect("response artifact key");
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
        attempt_id: attempt_id.clone(),
        claim: replay_stream_fact_claim(&other_descriptor, "same-subject", &response_artifact),
    };
    let (output_artifact, output_bytes, cell_produced) =
        replay_stream_state_output(&certified_spec, node, &attempt_id);
    let output_artifact_key =
        replay_artifact_authority_key(&output_artifact).expect("state output artifact key");
    let completed = events::StateAttemptCompleted {
        spec_hash: certified_spec.spec_hash.clone(),
        node_id: node.node_id.clone(),
        attempt_id,
        output_cell_id: node.output_cell.clone(),
    };
    let mut stream = vec![
        persisted_envelope(
            &run_id,
            1,
            KernelEventPayload::RunAdmitted(Box::new(run_admitted)),
        ),
        persisted_envelope(&run_id, 2, KernelEventPayload::StateAttemptStarted(started)),
    ];
    stream.extend(persisted_commit(
        &run_id,
        3,
        vec![
            KernelEventPayload::FactRecorded(fact),
            KernelEventPayload::CellProduced(cell_produced),
            KernelEventPayload::StateAttemptCompleted(completed),
        ],
    ));
    let authority = ReplayReadAuthority {
        certified_spec: certified_spec.clone(),
        stream,
        canonicalizer_identity: certified_spec
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        capability_implementations: Vec::new(),
        artifact_evidence: vec![
            stored_artifact_from_run_ref(&stream_run_admitted_spec_artifact_for_hash(
                &certified_spec.spec_hash,
            )),
            stored_artifact_from_run_ref(&stream_run_admitted_certificate_artifact()),
            replay_stream_config_artifact(&certified_spec),
            descriptor_artifact,
            other_descriptor_artifact,
            response_artifact.clone(),
            output_artifact,
        ],
        artifact_bytes: BTreeMap::from([
            (descriptor_artifact_key, descriptor_bytes.to_vec()),
            (
                other_descriptor_artifact_key,
                other_descriptor_bytes.to_vec(),
            ),
            (response_artifact_key, response_bytes),
            (output_artifact_key, output_bytes),
        ]),
        additional_artifact_evidence: Vec::new(),
        source_fact_events: Vec::new(),
    };

    let error = ReplayBroker::from_read_authority(authority)
        .expect_err("wrong-node descriptor must reject");
    assert_eq!(error.kind, ReplayErrorKind::CertifiedEvidenceMismatch);
    assert!(error
        .message
        .contains("not the exact read_external fact contract"));
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
        submission_artifact_evidence_hash: content_digest(0x44),
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        certified_context: CertifiedSideEffectContext::no_context(),
        prepared: None,
        submission: Some(&submission),
        submission_unknown: None,
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
fn side_effect_frame_submission_unknown_request_is_pair_keyed() {
    let pair_id = pair_id(0x45);
    let ledger_key = events::SideEffectLedgerKey::new("unknown-ledger").expect("ledger key");
    let intent = intent_persisted(pair_id.clone(), ledger_key.clone());
    let evidence_hash = content_digest(0x46);
    let unknown = side_effect::SubmissionUnknown {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        evidence_schema_id: schema_id("mfm.replay.test.submission_unknown", 0x47),
        evidence_hash: evidence_hash.clone(),
        evidence_artifact_id: artifact_id(0x48),
        evidence_artifact_evidence_hash: content_digest(0x49),
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        certified_context: CertifiedSideEffectContext::no_context(),
        prepared: None,
        submission: None,
        submission_unknown: Some(&unknown),
        not_submitted: None,
        receipt: None,
        confirmation: None,
        ambiguity: None,
    };

    let request = frame
        .submission_unknown_request()
        .expect("submission-unknown request");

    assert_eq!(request.pair_id, pair_id);
    assert_eq!(request.invocation_epoch, 1);
    assert_eq!(request.evidence_schema_id, unknown.evidence_schema_id);
    assert_eq!(request.evidence_hash, evidence_hash);
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
        proof_artifact_evidence_hash: content_digest(0x55),
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        certified_context: CertifiedSideEffectContext::no_context(),
        prepared: None,
        submission: None,
        submission_unknown: None,
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
        receipt_artifact_evidence_hash: content_digest(0x65),
        replay_verifier_id: replay_verifier_id.clone(),
        resource_touched_set: None,
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        certified_context: CertifiedSideEffectContext::no_context(),
        prepared: None,
        submission: None,
        submission_unknown: None,
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
    let source = include_str!("../../lib.rs");
    for forbidden in [
        concat!("std", "::", "env"),
        concat!("current", "_", "exe"),
        concat!("/proc", "/self", "/exe"),
        concat!("Executable", "Identity", "Template"),
        concat!("mfm", "_", "transports"),
        concat!("mfm", "_", "signing", "::", "SigningProvider"),
        concat!("mfm", "_", "core", "::", "keystore"),
        concat!("Current", "Config", "Provider"),
        concat!("Evm", "Json", "Rpc", "Client"),
    ] {
        assert!(
            !source.contains(forbidden),
            "replay source must not import live authority surface {forbidden}"
        );
    }
}
