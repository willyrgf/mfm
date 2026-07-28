use super::*;

use mfm_canonical::sha256_digest_bytes;

struct BootstrapJournalFixture {
    run_id: RunId,
    spec_hash: SpecHash,
    records: Vec<KernelEventEnvelope>,
    objects: mfm_store::v1::ArtifactByteAuthorityMap,
    spec_bytes: Vec<u8>,
    certificate_bytes: Vec<u8>,
    spec_ref: events::RunArtifactEvidenceRef,
    certificate_ref: events::RunArtifactEvidenceRef,
}

fn bootstrap_artifact(
    bytes: &[u8],
    role: ArtifactRole,
    schema_id: SchemaId,
    media_type_value: &str,
) -> ArtifactEvidenceRef {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: media_type(media_type_value),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn object_key(evidence: &ArtifactEvidenceRef) -> mfm_store::v1::ArtifactAuthorityKey {
    (
        evidence.artifact_id.clone(),
        evidence.evidence_hash().expect("evidence hash"),
    )
}

fn bootstrap_journal_fixture() -> BootstrapJournalFixture {
    let template_run_id = run_id(246);
    let spec_bytes = br#"{"kind":"test-certified-spec"}"#.to_vec();
    let certificate_bytes = br#"{"kind":"test-certificate"}"#.to_vec();
    let spec_evidence = bootstrap_artifact(
        &spec_bytes,
        ArtifactRole::TypedExecutionSpec,
        spec::typed_execution_spec_schema_id().expect("typed spec schema"),
        SPEC_MEDIA_TYPE,
    );
    let certificate_evidence = bootstrap_artifact(
        &certificate_bytes,
        ArtifactRole::TypedSpecCertificate,
        mfm_certify::typed_spec_certificate_schema_id().expect("certificate schema"),
        mfm_certify::CERTIFICATE_MEDIA_TYPE,
    );
    let spec_hash = SpecHash::from_digest(
        spec_evidence.digest.algorithm(),
        *spec_evidence.digest.digest(),
    );
    let identity_material =
        run_identity_material_for_test(spec_hash.clone(), &store_scope_hex(246));
    let run_id = identity_material.derive_run_id().expect("run id");
    let spec_ref =
        mfm_store::v1::test_support::run_artifact_ref_from_store_artifact_for_test(&spec_evidence);
    let certificate_ref =
        mfm_store::v1::test_support::run_artifact_ref_from_store_artifact_for_test(
            &certificate_evidence,
        );
    let mut admitted = match run_admitted(template_run_id) {
        KernelEventPayload::RunAdmitted(admitted) => admitted,
        _ => unreachable!("run-admitted helper returned another payload"),
    };
    admitted.run_id = run_id.clone();
    admitted.identity_material = identity_material;
    admitted.spec_hash = spec_hash.clone();
    admitted.spec_artifact = spec_ref.clone();
    admitted.certificate_artifact = certificate_ref.clone();

    let record = mfm_store::v1::test_support::persisted_kernel_event_envelope_for_test(
        &run_id,
        1,
        1,
        CommitKey::new("journal-bootstrap").expect("commit key"),
        KernelEventPayload::RunAdmitted(admitted),
    );
    let objects = BTreeMap::from([
        (
            object_key(&spec_evidence),
            (spec_bytes.clone(), spec_evidence),
        ),
        (
            object_key(&certificate_evidence),
            (certificate_bytes.clone(), certificate_evidence),
        ),
    ]);

    BootstrapJournalFixture {
        run_id,
        spec_hash,
        records: vec![record],
        objects,
        spec_bytes,
        certificate_bytes,
        spec_ref,
        certificate_ref,
    }
}

fn load_journal(
    run_id: RunId,
    records: Vec<KernelEventEnvelope>,
    objects: mfm_store::v1::ArtifactByteAuthorityMap,
) -> std::result::Result<mfm_store::v1::CommittedRunJournal, StoreError> {
    let backend = StaticRunJournalBackendForTest::new(run_id.clone(), records, objects);
    poll_ready_store_future(backend.load_committed_journal(&run_id))
}

#[test]
fn committed_journal_accepts_one_exact_bootstrap_and_exposes_no_projection_authority() {
    let fixture = bootstrap_journal_fixture();
    let journal = load_journal(fixture.run_id.clone(), fixture.records, fixture.objects)
        .expect("committed journal");

    assert_eq!(journal.run_id(), &fixture.run_id);
    assert_eq!(journal.current_run_sequence(), Some(1));
    let (spec_ref, spec_bytes) = journal.certified_spec_object();
    assert_eq!(
        spec_ref.schema_id(),
        fixture.spec_ref.schema_id.as_ref().unwrap()
    );
    assert_eq!(
        spec_ref.content_digest(),
        &ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(&fixture.spec_bytes)
        )
    );
    assert_eq!(spec_bytes, fixture.spec_bytes);
    let (certificate_ref, certificate_bytes) = journal.certificate_object();
    assert_eq!(
        certificate_ref.schema_id(),
        fixture.certificate_ref.schema_id.as_ref().unwrap()
    );
    assert_eq!(
        certificate_ref.content_digest(),
        &ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(&fixture.certificate_bytes)
        )
    );
    assert_eq!(certificate_bytes, fixture.certificate_bytes);
}

#[test]
fn committed_journal_rejects_corrupt_record_order_before_minting_authority() {
    let mut fixture = bootstrap_journal_fixture();
    fixture.records.push(
        mfm_store::v1::test_support::persisted_kernel_event_envelope_for_test(
            &fixture.run_id,
            3,
            2,
            CommitKey::new("journal-gap").expect("commit key"),
            KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: fixture.spec_hash,
                node_id: node_id(247),
                attempt_id: attempt_id(248),
                attempt_no: 1,
                state_kind: state_kind(249),
                state_version: StateVersion::new("mfm.test.journal_state.v1")
                    .expect("state version"),
            }),
        ),
    );

    let error = load_journal(fixture.run_id, fixture.records, fixture.objects)
        .expect_err("sequence gap must reject");

    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch { field: "seq", .. }
    ));
}

#[test]
fn committed_journal_rejects_missing_and_tampered_exact_objects() {
    for case in ["missing", "tampered"] {
        let mut fixture = bootstrap_journal_fixture();
        let spec_key = (
            fixture.spec_ref.artifact_id.clone(),
            fixture.spec_ref.evidence_hash.clone(),
        );
        match case {
            "missing" => {
                fixture.objects.remove(&spec_key);
            }
            "tampered" => {
                fixture
                    .objects
                    .get_mut(&spec_key)
                    .expect("spec object")
                    .0
                    .push(b'!');
            }
            _ => unreachable!(),
        }

        let error = load_journal(fixture.run_id, fixture.records, fixture.objects).expect_err(case);
        match case {
            "missing" => assert!(matches!(error, StoreError::MissingArtifact { .. })),
            "tampered" => {
                assert!(matches!(error, StoreError::ArtifactEvidenceMismatch { .. }))
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn committed_journal_revalidates_every_requirement_for_one_exact_object_key() {
    let mut fixture = bootstrap_journal_fixture();
    fixture.records.push(
        mfm_store::v1::test_support::persisted_kernel_event_envelope_for_test(
            &fixture.run_id,
            2,
            2,
            CommitKey::new("journal-conflicting-object-role").expect("commit key"),
            KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                run_id: fixture.run_id.clone(),
                spec_hash: fixture.spec_hash.clone(),
                refs: vec![events::RetentionRef {
                    artifact_id: fixture.spec_ref.artifact_id.clone(),
                    role: ArtifactRole::StateOutput,
                    evidence_hash: fixture.spec_ref.evidence_hash.clone(),
                    content_digest: fixture.spec_ref.content_digest.clone(),
                }],
                reason: events::RetentionReason::RuntimeEvidence,
            }),
        ),
    );

    let error = load_journal(fixture.run_id, fixture.records, fixture.objects)
        .expect_err("later conflicting requirement for the same object key must reject");

    assert!(matches!(
        error,
        StoreError::ArtifactEvidenceMismatch {
            field: "artifact_role",
            ..
        }
    ));
}

#[test]
fn committed_journal_ignores_an_unrelated_corrupt_retained_object_row() {
    let mut fixture = bootstrap_journal_fixture();
    let unrelated_evidence = bootstrap_artifact(
        br#"{"unrelated":"original"}"#,
        ArtifactRole::RedactedDiagnostic,
        schema_id("mfm.test.unrelated_object", 245),
        "application/json",
    );
    fixture.objects.insert(
        object_key(&unrelated_evidence),
        (br#"{"unrelated":"corrupt"}"#.to_vec(), unrelated_evidence),
    );

    let journal = load_journal(fixture.run_id.clone(), fixture.records, fixture.objects)
        .expect("unrelated corrupt retained row is outside this journal");

    assert_eq!(journal.run_id(), &fixture.run_id);
    assert_eq!(journal.current_run_sequence(), Some(1));
}

#[test]
fn committed_journal_distinguishes_absence_from_an_invalid_nonempty_root() {
    let absent_run = run_id(250);
    let error = load_journal(absent_run.clone(), Vec::new(), BTreeMap::new())
        .expect_err("empty journal must report absence");
    assert_eq!(error, StoreError::RunNotFound { run_id: absent_run });

    let fixture = bootstrap_journal_fixture();
    let non_root = mfm_store::v1::test_support::persisted_kernel_event_envelope_for_test(
        &fixture.run_id,
        1,
        1,
        CommitKey::new("not-an-admission-root").expect("commit key"),
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: fixture.spec_hash,
            node_id: node_id(251),
            attempt_id: attempt_id(252),
            attempt_no: 1,
            state_kind: state_kind(253),
            state_version: StateVersion::new("mfm.test.journal_state.v1").expect("state version"),
        }),
    );
    let error = load_journal(fixture.run_id, vec![non_root], fixture.objects)
        .expect_err("nonempty invalid root must not report absence");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "run_admission",
            ..
        }
    ));
}
