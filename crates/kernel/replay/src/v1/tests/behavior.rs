use super::fixtures::*;
use super::*;

use mfm_ids::{
    AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, DigestAlgorithm, DigestBytes,
    SpecHash,
};

#[test]
fn replay_broker_borrows_a_real_store_verified_view() {
    let fixture = loaded_fact_run(
        0x31,
        vec![ReplayStreamFact::new("account-a", 41)],
        QuerySource::None,
        false,
    );
    let authority = ReplayReadAuthority::from_verified_run_view_with_source_facts_and_artifacts(
        &fixture.view,
        Vec::new(),
        Vec::new(),
    )
    .expect("replay authority");
    let broker = ReplayBroker::from_read_authority(authority).expect("replay broker");

    assert_eq!(broker.certified_spec().spec_hash, *fixture.view.spec_hash());
    let frames = broker
        .produced_cell_frames_matching(|node, _, _| Ok(node.node_id == fixture.node_id))
        .expect("produced frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(
        broker
            .reject_live_capability_request()
            .expect_err("replay cannot mint live authority")
            .kind,
        ReplayErrorKind::LiveCapabilityRequest
    );
}

#[test]
fn replay_retained_artifact_lookup_uses_the_full_store_requirement() {
    let fixture = loaded_fact_run(0x32, Vec::new(), QuerySource::None, true);
    assert_eq!(fixture.exact_requirements.len(), 2);
    let first = fixture.exact_requirements[0].clone();
    let second = fixture.exact_requirements[1].clone();
    assert_eq!(first.artifact_id, second.artifact_id);
    assert_ne!(first.evidence_hash, second.evidence_hash);

    let authority = ReplayReadAuthority::from_verified_run_view_with_source_facts_and_artifacts(
        &fixture.view,
        Vec::new(),
        Vec::new(),
    )
    .expect("replay authority");
    let broker = ReplayBroker::from_read_authority(authority).expect("replay broker");
    let selected = broker
        .retained_artifact(&second)
        .expect("second exact object");
    assert_eq!(
        selected.artifact.schema_id.as_ref(),
        second.schema_id.as_ref()
    );

    let mut cross_wired = second;
    cross_wired.evidence_hash = first.evidence_hash;
    let error = broker
        .retained_artifact(&cross_wired)
        .expect_err("one object's key cannot authorize another requirement");
    assert_eq!(error.kind, ReplayErrorKind::ArtifactMissing);
}

#[test]
fn replay_artifact_expectations_check_artifact_id_even_with_a_matching_hash() {
    let fixture = loaded_fact_run(0x33, Vec::new(), QuerySource::None, true);
    let expected = &fixture.exact_requirements[0];
    let lifecycle = store::current_lifecycle::read(&fixture.view);
    let object = lifecycle
        .object_for_requirement(expected)
        .expect("exact object");
    let mut wrong = object.evidence().clone();
    wrong.artifact_id = ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x34; 32]),
    );

    let error = verify_artifact_expectation(
        &wrong,
        ArtifactEvidenceExpectation {
            artifact_id: &expected.artifact_id,
            evidence_hash: &wrong.evidence_hash().expect("evidence hash"),
            digest: expected.digest.as_ref().expect("digest"),
            schema_id: expected.schema_id.as_ref(),
            semantic_type_id: expected.semantic_type_id.as_ref(),
            role: expected.artifact_role.expect("role"),
            producer_node_id: expected.producer_node_id.as_ref(),
            producer_seed_id: expected.producer_seed_id.as_ref(),
        },
    )
    .expect_err("a different artifact id must reject");
    assert_eq!(error.kind, ReplayErrorKind::ArtifactMismatch);
}

#[test]
fn replay_fact_batch_requires_exact_count_order_and_identity() {
    let fixture = loaded_fact_run(
        0x35,
        vec![
            ReplayStreamFact::new("account-a", 41),
            ReplayStreamFact::new("account-b", 42),
        ],
        QuerySource::None,
        false,
    );
    let authority = ReplayReadAuthority::from_verified_run_view_with_source_facts_and_artifacts(
        &fixture.view,
        Vec::new(),
        Vec::new(),
    )
    .expect("replay authority");
    let broker = ReplayBroker::from_read_authority(authority).expect("replay broker");
    let mut frames = broker
        .produced_cell_frames_matching(|node, _, _| Ok(node.node_id == fixture.node_id))
        .expect("fact output frame");
    let frame = frames.pop().expect("one fact output frame");
    assert!(frames.is_empty());
    verify_recorded_fact_batch_evidence(&broker, &frame, &fixture.facts)
        .expect("exact reducer fact batch");

    let cases = [
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
        (
            "duplicate",
            vec![
                ReplayStreamFact::new("account-a", 41),
                ReplayStreamFact::new("account-a", 41),
            ],
        ),
    ];
    for (name, facts) in cases {
        let error = verify_recorded_fact_batch_evidence(&broker, &frame, &facts)
            .expect_err("non-exact reducer fact batch must reject");
        assert_eq!(
            error.kind,
            ReplayErrorKind::CertifiedEvidenceMismatch,
            "{name}"
        );
    }
}

#[test]
fn replay_verifies_primary_run_fact_query_evidence() {
    let fixture = loaded_fact_run(
        0x36,
        vec![ReplayStreamFact::new("account-a", 41)],
        QuerySource::FirstRecorded,
        false,
    );
    let authority = ReplayReadAuthority::from_verified_run_view_with_source_facts_and_artifacts(
        &fixture.view,
        Vec::new(),
        Vec::new(),
    )
    .expect("replay authority");

    ReplayBroker::from_read_authority(authority)
        .expect("primary-run fact query evidence must verify");
}

#[test]
fn replay_verifies_cross_run_source_fact_and_rejects_missing_source() {
    let source = loaded_fact_run(
        0x37,
        vec![ReplayStreamFact::new("account-a", 41)],
        QuerySource::None,
        false,
    );
    let source_ref = source.fact_refs[0].clone();
    let source_event = retained_source_fact(&source.view, source_ref.fact_claim_id());
    let source_response = verified_response_object(&source, 0);

    let consumer = loaded_fact_run(
        0x38,
        Vec::new(),
        QuerySource::external(source_ref.clone()),
        false,
    );
    let authority = ReplayReadAuthority::from_verified_run_view_with_source_facts_and_artifacts(
        &consumer.view,
        vec![source_event.clone()],
        vec![source_response.clone()],
    )
    .expect("cross-run authority");
    ReplayBroker::from_read_authority(authority).expect("cross-run source fact must verify");

    let missing_consumer =
        loaded_fact_run(0x39, Vec::new(), QuerySource::external(source_ref), false);
    let authority = ReplayReadAuthority::from_verified_run_view_with_source_facts_and_artifacts(
        &missing_consumer.view,
        Vec::new(),
        vec![source_response],
    )
    .expect("missing-source authority");
    let error =
        ReplayBroker::from_read_authority(authority).expect_err("missing source fact must reject");
    assert_eq!(error.kind, ReplayErrorKind::FactMissing);
}

#[test]
fn replay_rejects_cross_run_returned_ref_mismatches() {
    let source = loaded_fact_run(
        0x3a,
        vec![ReplayStreamFact::new("account-a", 41)],
        QuerySource::None,
        false,
    );
    let exact = source.fact_refs[0].clone();
    let source_event = retained_source_fact(&source.view, exact.fact_claim_id());
    let source_response = verified_response_object(&source, 0);

    for (index, (name, mismatch)) in [
        ("descriptor hash", ReturnedRefMismatch::DescriptorHash),
        ("response digest", ReturnedRefMismatch::ResponseDigest),
        ("fact kind", ReturnedRefMismatch::FactKind),
        ("producer node", ReturnedRefMismatch::ProducerNode),
        ("subject material", ReturnedRefMismatch::SubjectMaterial),
    ]
    .into_iter()
    .enumerate()
    {
        let mismatched = mismatched_fact_ref(&exact, mismatch);
        let consumer = loaded_fact_run(
            0x40 + u8::try_from(index).expect("small case index"),
            Vec::new(),
            QuerySource::external(mismatched),
            false,
        );
        let authority =
            ReplayReadAuthority::from_verified_run_view_with_source_facts_and_artifacts(
                &consumer.view,
                vec![source_event.clone()],
                vec![source_response.clone()],
            )
            .expect("mismatch authority");
        let error = ReplayBroker::from_read_authority(authority).expect_err(name);
        assert_eq!(error.kind, ReplayErrorKind::FactMismatch, "{name}");
    }
}

#[derive(Clone, Copy)]
enum ReturnedRefMismatch {
    DescriptorHash,
    ResponseDigest,
    FactKind,
    ProducerNode,
    SubjectMaterial,
}

fn mismatched_fact_ref(
    exact: &mfm_facts::InternalFactRef,
    mismatch: ReturnedRefMismatch,
) -> mfm_facts::InternalFactRef {
    let mut parts = mfm_facts::InternalFactRefParts {
        fact_claim_id: exact.fact_claim_id().clone(),
        source_event_id: exact.source_event_id().clone(),
        recorded_at: exact.recorded_at().to_owned(),
        producer_node_id: exact.producer_node_id().clone(),
        fact_kind: exact.fact_kind().clone(),
        fact_descriptor_hash: exact.fact_descriptor_hash().clone(),
        subject: mfm_facts::FactSubjectRef::new(
            exact.fact_subject_namespace_hash().clone(),
            exact.fact_key().clone(),
            exact.subject_material_hash().clone(),
        ),
        response: mfm_facts::FactResponseEvidence::new(
            exact.response_schema_id().clone(),
            exact.response_hash().clone(),
            exact.artifact_id().clone(),
            exact.artifact_evidence_hash().clone(),
        ),
    };
    match mismatch {
        ReturnedRefMismatch::DescriptorHash => {
            parts.fact_descriptor_hash = store::test_support::fixed_content_digest_for_test(0x51);
        }
        ReturnedRefMismatch::ResponseDigest => {
            parts.response = mfm_facts::FactResponseEvidence::new(
                exact.response_schema_id().clone(),
                store::test_support::fixed_content_digest_for_test(0x52),
                exact.artifact_id().clone(),
                exact.artifact_evidence_hash().clone(),
            );
        }
        ReturnedRefMismatch::FactKind => {
            parts.fact_kind =
                mfm_facts::FactKind::new("mfm.replay.test.other_fact").expect("other fact kind");
        }
        ReturnedRefMismatch::ProducerNode => {
            parts.producer_node_id = NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x53; 32]),
            );
        }
        ReturnedRefMismatch::SubjectMaterial => {
            parts.subject = mfm_facts::FactSubjectRef::new(
                exact.fact_subject_namespace_hash().clone(),
                exact.fact_key().clone(),
                store::test_support::fixed_content_digest_for_test(0x54),
            );
        }
    }
    mfm_facts::InternalFactRef::new(parts).expect("mismatched internal fact ref")
}

#[test]
fn side_effect_frame_requests_preserve_pair_epoch_and_recorded_evidence() {
    let pair_id = SideEffectPairId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x61; 32]),
    );
    let ledger_key = events::SideEffectLedgerKey::new("pair-keyed-ledger").expect("ledger key");
    let intent = intent_persisted(pair_id.clone(), ledger_key.clone());
    let submission = side_effect::SubmissionObserved {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        submission_schema_id: schema_id("mfm.replay.test.submission", 0x62),
        submission_hash: content_digest(0x63),
        submission_artifact_id: artifact_id(0x64),
        submission_artifact_evidence_hash: content_digest(0x65),
    };
    let unknown = side_effect::SubmissionUnknown {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        evidence_schema_id: schema_id("mfm.replay.test.submission_unknown", 0x66),
        evidence_hash: content_digest(0x67),
        evidence_artifact_id: artifact_id(0x68),
        evidence_artifact_evidence_hash: content_digest(0x69),
    };
    let proof = side_effect::NotSubmittedProven {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        proof_schema_id: schema_id("mfm.replay.test.not_submitted", 0x6a),
        proof_hash: content_digest(0x6b),
        proof_artifact_id: artifact_id(0x6c),
        proof_artifact_evidence_hash: content_digest(0x6d),
    };
    let replay_verifier_id =
        events::ReplayVerifierId::new("mfm.replay.test.receipt.verifier").expect("verifier id");
    let receipt = side_effect::ReceiptObserved {
        spec_hash: intent.spec_hash.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: pair_id.clone(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        receipt_schema_id: schema_id("mfm.replay.test.receipt", 0x6e),
        receipt_hash: content_digest(0x6f),
        receipt_artifact_id: artifact_id(0x70),
        receipt_artifact_evidence_hash: content_digest(0x71),
        replay_verifier_id: replay_verifier_id.clone(),
        resource_touched_set: None,
    };
    let frame = SideEffectReplayFrame {
        intent: &intent,
        certified_context: CertifiedSideEffectContext::no_context(),
        prepared: None,
        submission: Some(&submission),
        submission_unknown: Some(&unknown),
        not_submitted: Some(&proof),
        receipt: Some(&receipt),
        confirmation: None,
        ambiguity: None,
    };

    for (request, schema_id, evidence_hash) in [
        (
            frame.submission_request().expect("submission request"),
            &submission.submission_schema_id,
            &submission.submission_hash,
        ),
        (
            frame
                .submission_unknown_request()
                .expect("submission-unknown request"),
            &unknown.evidence_schema_id,
            &unknown.evidence_hash,
        ),
        (
            frame
                .not_submitted_request()
                .expect("not-submitted request"),
            &proof.proof_schema_id,
            &proof.proof_hash,
        ),
        (
            frame.receipt_request().expect("receipt request"),
            &receipt.receipt_schema_id,
            &receipt.receipt_hash,
        ),
    ] {
        assert_eq!(request.pair_id, pair_id);
        assert_eq!(request.invocation_epoch, 1);
        assert_eq!(&request.evidence_schema_id, schema_id);
        assert_eq!(&request.evidence_hash, evidence_hash);
    }
    let receipt_request = frame.receipt_request().expect("receipt request");
    assert_eq!(
        receipt_request.evidence_schema_id,
        receipt.receipt_schema_id
    );
    assert_eq!(receipt_request.evidence_hash, receipt.receipt_hash);
    assert_eq!(receipt_request.replay_verifier_id, Some(replay_verifier_id));
}

#[test]
fn replay_verifier_identity_must_match_recorded_evidence() {
    let recorded =
        events::ReplayVerifierId::new("mfm.replay.test.recorded").expect("recorded verifier");
    let wrong = events::ReplayVerifierId::new("mfm.replay.test.wrong").expect("wrong verifier");

    verify_replay_verifier(Some(&recorded), &recorded).expect("exact verifier");
    for requested in [None, Some(&wrong)] {
        let error = verify_replay_verifier(requested, &recorded)
            .expect_err("missing or different verifier must reject");
        assert_eq!(error.kind, ReplayErrorKind::ReplayVerifierMismatch);
    }
}

#[test]
fn replay_source_does_not_import_live_authorities() {
    let sources = [
        include_str!("../../lib.rs"),
        include_str!("../mod.rs"),
        include_str!("../broker.rs"),
        include_str!("../broker/artifacts.rs"),
        include_str!("../broker/facts.rs"),
        include_str!("../broker/queries.rs"),
        include_str!("../broker/side_effects.rs"),
        include_str!("../evidence.rs"),
        include_str!("../value_read.rs"),
        include_str!("../verification_helpers.rs"),
        include_str!("../../../Cargo.toml"),
    ];
    for forbidden in [
        concat!("std", "::", "env"),
        concat!("current", "_", "exe"),
        concat!("/proc", "/self", "/exe"),
        concat!("Executable", "Identity", "Template"),
        concat!("mfm", "_", "transports"),
        concat!("mfm", "-", "transports"),
        concat!("mfm", "-", "runtime"),
        concat!("mfm", "_", "manual", "_", "auth"),
        concat!("mfm", "-", "manual", "-", "auth"),
        concat!("ManualResolution", "AuthorizationProof"),
        concat!("ManualResolution", "ProofAuthority"),
        concat!("mfm", "_", "signing", "::", "SigningProvider"),
        concat!("mfm", "_", "core", "::", "keystore"),
        concat!("Current", "Config", "Provider"),
        concat!("Evm", "Json", "Rpc", "Client"),
    ] {
        assert!(
            sources.iter().all(|source| !source.contains(forbidden)),
            "replay source must not import live authority surface {forbidden}"
        );
    }
}

fn intent_persisted(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
) -> side_effect::IntentPersisted {
    side_effect::IntentPersisted {
        spec_hash: SpecHash::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x72; 32]),
        ),
        node_id: NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x73; 32]),
        ),
        scope_id: mfm_ids::ScopeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x74; 32]),
        ),
        attempt_id: AttemptId::from_digest(DigestBytes::from_array([0x75; 32])),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.replay.test.intent", 0x76),
        intent_hash: content_digest(0x77),
        intent_artifact_id: artifact_id(0x78),
        intent_artifact_evidence_hash: content_digest(0x79),
        idempotency_input_schema_id: schema_id("mfm.replay.test.idempotency", 0x7a),
        idempotency_input_hash: content_digest(0x7b),
        idempotency_key: events::IdempotencyKeyRef::new("idem-key").expect("idempotency key"),
        capability_kind: CapabilityKind::new(
            "mfm.replay.test",
            "mutation",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x7c; 32]),
        )
        .expect("capability kind"),
        capability_version: CapabilityVersion::new("mfm.replay.test.capability.v1")
            .expect("capability version"),
        adapter_kind: AdapterKind::new(
            "mfm.replay.test",
            "adapter",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x7d; 32]),
        )
        .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.replay.test.adapter.v1")
            .expect("adapter version"),
    }
}

fn schema_id(name: &str, discriminator: u8) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([discriminator; 32]),
    )
    .expect("schema id")
}

fn content_digest(discriminator: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([discriminator; 32]),
    )
}

fn artifact_id(discriminator: u8) -> ArtifactId {
    ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([discriminator; 32]),
    )
}
