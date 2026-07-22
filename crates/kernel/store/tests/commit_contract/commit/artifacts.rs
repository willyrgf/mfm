use super::*;

#[test]
fn required_artifact_precondition_is_atomic_with_append() {
    let artifact_id = artifact_id(50);
    let artifact_digest = content_digest(51);
    let run_id = run_id(52);
    let mut store = admitted_store(&run_id, "artifact-precondition-run-start");
    let evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("artifact-ref").expect("commit key"),
        payloads: vec![retention_refs_appended_for_run(
            &run_id,
            artifact_id,
            artifact_digest,
            ArtifactRole::StateOutput,
        )],
        required_artifacts: vec![evidence],
        preconditions: run_state_preconditions(RequiredRunState::Started),
    };

    let artifacts =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), Vec::new())
            .expect("missing artifact evidence set");
    let commit =
        PreparedCommit::<Retention>::new(request, artifacts).expect("prepare missing artifact");
    let error = store
        .append_test_commit_plan(commit.into())
        .expect_err("missing artifact rejects commit");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
    assert_eq!(store.load_run_stream(&run_id).len(), 1);
    assert_eq!(store.expected_next_seq(&run_id).as_u64(), 2);
}

#[test]
fn prepared_commit_rejects_unreferenced_admitted_artifact_without_persisting_it() {
    let run_id = run_id(53);
    let artifact_id = artifact_id(54);
    let artifact_digest = content_digest(55);
    let mut store = admitted_store(&run_id, "unreferenced-artifact-run-start");
    let admitted = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());

    let error = store
        .append_prepared_commit_with_artifacts(
            default_commit_request(
                &run_id,
                store.expected_next_seq(&run_id),
                "unreferenced-artifact",
                vec![state_attempt_started()],
                Vec::new(),
            ),
            vec![admitted.clone()],
        )
        .expect_err("unreferenced admitted artifact rejects before append");
    assert!(matches!(
        error,
        StoreError::UnreferencedArtifactEvidence { .. }
    ));
    assert_eq!(store.load_run_stream(&run_id).len(), 1);

    let missing_evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let error = store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new("artifact-still-missing").expect("commit key"),
                payloads: vec![retention_refs_appended_for_run(
                    &run_id,
                    artifact_id,
                    artifact_digest,
                    ArtifactRole::StateOutput,
                )],
                required_artifacts: vec![missing_evidence],
                preconditions: run_state_preconditions(RequiredRunState::Started),
            },
            Vec::new(),
        )
        .expect_err("rejected unreferenced evidence must not leak into store");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
}

#[test]
fn prepared_commit_idempotency_fingerprint_includes_admitted_artifacts() {
    let run_id = run_id(120);
    let artifact_id = artifact_id(131);
    let artifact_digest = content_digest(132);
    let mut store = admitted_store(&run_id, "run-start");
    let evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let mut conflicting_evidence = evidence.clone();
    conflicting_evidence.byte_len += 1;
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("prepared-fingerprint").expect("commit key"),
        payloads: vec![retention_refs_appended(
            artifact_id,
            artifact_digest,
            ArtifactRole::StateOutput,
        )],
        required_artifacts: vec![evidence.clone()],
        preconditions: run_state_preconditions(RequiredRunState::Started),
    };

    store
        .append_prepared_commit_with_artifacts(request.clone(), vec![evidence])
        .expect("append initial prepared commit");
    let retry = request.with_expected_next_seq(StreamSeq::new(99).expect("stale seq"));
    let error = store
        .append_prepared_commit_with_artifacts(retry, vec![conflicting_evidence])
        .expect_err("same request with different admitted evidence is not idempotent");
    assert!(matches!(error, StoreError::CommitConflict { .. }));
}

#[test]
fn artifact_authority_accepts_distinct_evidence_for_same_artifact_id() {
    let run_id = run_id(121);
    let artifact_id = artifact_id(122);
    let digest = content_digest(122);
    let first_evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
    let mut second_evidence = first_evidence.clone();
    second_evidence.schema_id = Some(schema_id("mfm.test.alternate_position", 124));
    assert_ne!(
        first_evidence.evidence_hash().expect("first evidence hash"),
        second_evidence
            .evidence_hash()
            .expect("second evidence hash")
    );

    let mut store = admitted_store(&run_id, "same-id-run-start");
    store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new("same-id-first-evidence").expect("commit key"),
                payloads: vec![KernelEventPayload::RetentionRefsAppended(
                    events::RetentionRefsAppended {
                        run_id: run_id.clone(),
                        spec_hash: spec_hash(1),
                        refs: vec![events::RetentionRef {
                            artifact_id: artifact_id.clone(),
                            role: ArtifactRole::StateOutput,
                            evidence_hash: first_evidence
                                .evidence_hash()
                                .expect("first evidence hash"),
                            content_digest: digest.clone(),
                        }],
                        reason: events::RetentionReason::RuntimeEvidence,
                    },
                )],
                required_artifacts: vec![first_evidence.clone()],
                preconditions: run_state_preconditions(RequiredRunState::Started),
            },
            vec![first_evidence.clone()],
        )
        .expect("append first evidence");

    store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new("same-id-second-evidence").expect("commit key"),
                payloads: vec![KernelEventPayload::RetentionRefsAppended(
                    events::RetentionRefsAppended {
                        run_id: run_id.clone(),
                        spec_hash: spec_hash(1),
                        refs: vec![events::RetentionRef {
                            artifact_id: artifact_id.clone(),
                            role: ArtifactRole::StateOutput,
                            evidence_hash: second_evidence
                                .evidence_hash()
                                .expect("second evidence hash"),
                            content_digest: digest,
                        }],
                        reason: events::RetentionReason::PublicOutput,
                    },
                )],
                required_artifacts: vec![second_evidence.clone()],
                preconditions: run_state_preconditions(RequiredRunState::Started),
            },
            vec![second_evidence.clone()],
        )
        .expect("append second evidence for same artifact id");

    let retention = store
        .projection_snapshot()
        .retention(&run_id)
        .expect("retention projection");
    assert_eq!(
        retention
            .refs
            .keys()
            .filter(|(id, _)| id == &artifact_id)
            .count(),
        2
    );
    let first_hash = first_evidence.evidence_hash().expect("first evidence hash");
    let second_hash = second_evidence
        .evidence_hash()
        .expect("second evidence hash");
    assert!(retention
        .refs
        .contains_key(&(artifact_id.clone(), first_hash.clone())));
    assert!(retention
        .refs
        .contains_key(&(artifact_id.clone(), second_hash.clone())));

    let first_requirement = EventArtifactRequirement {
        source: EventArtifactReferenceSource::RetentionRef,
        artifact_id: artifact_id.clone(),
        evidence_hash: first_hash.clone(),
        digest: Some(first_evidence.digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(ArtifactRole::StateOutput),
    };
    let second_requirement = EventArtifactRequirement {
        source: EventArtifactReferenceSource::RetentionRef,
        artifact_id: artifact_id.clone(),
        evidence_hash: second_hash.clone(),
        digest: Some(second_evidence.digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(ArtifactRole::StateOutput),
    };
    let swapped_requirement = EventArtifactRequirement {
        source: EventArtifactReferenceSource::RetentionRef,
        artifact_id: artifact_id.clone(),
        evidence_hash: second_hash,
        digest: Some(first_evidence.digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(ArtifactRole::StateOutput),
    };

    mfm_store::v1::validate_artifact_requirement_against_evidence(
        &first_requirement,
        &first_evidence,
    )
    .expect("exact first evidence key validates");
    mfm_store::v1::validate_artifact_requirement_against_evidence(
        &second_requirement,
        &second_evidence,
    )
    .expect("exact second evidence key validates");
    let mismatch = mfm_store::v1::validate_artifact_requirement_against_evidence(
        &swapped_requirement,
        &first_evidence,
    )
    .expect_err("wrong evidence hash must not validate against another variant");
    assert!(matches!(
        mismatch,
        StoreError::ArtifactEvidenceMismatch { .. }
    ));
}

#[test]
fn admitted_artifacts_are_rolled_back_when_commit_validation_fails() {
    let run_id = run_id(56);
    let artifact_id = artifact_id(57);
    let artifact_digest = content_digest(58);
    let mut store = admitted_store(&run_id, "artifact-rollback-run-start");
    let evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());

    let error = store
        .append_prepared_commit(default_commit_request(
            &run_id,
            store.expected_next_seq(&run_id),
            "cell-without-attempt-complete",
            vec![cell_produced(artifact_id.clone(), artifact_digest.clone())],
            vec![evidence],
        ))
        .expect_err("projection failure rejects commit after artifact validation");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
    assert_eq!(store.load_run_stream(&run_id).len(), 1);

    let missing_evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let error = store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new("artifact-not-leaked").expect("commit key"),
                payloads: vec![retention_refs_appended_for_run(
                    &run_id,
                    artifact_id,
                    artifact_digest,
                    ArtifactRole::StateOutput,
                )],
                required_artifacts: vec![missing_evidence],
                preconditions: run_state_preconditions(RequiredRunState::Started),
            },
            Vec::new(),
        )
        .expect_err("failed commit must not persist admitted artifact evidence");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
}

#[test]
fn payload_artifact_byte_len_media_type_and_producer_mismatches_are_rejected() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        ByteLen,
        MediaType,
        Producer,
    }

    for (case, run_byte, artifact_byte, digest_byte, commit_key, expected_field) in [
        (Case::ByteLen, 59, 60, 61, "artifact-byte-len", "byte_len"),
        (
            Case::MediaType,
            86,
            87,
            88,
            "artifact-media-type",
            "media_type",
        ),
        (
            Case::Producer,
            62,
            63,
            64,
            "artifact-producer",
            "producer_node_id",
        ),
    ] {
        let run_id = run_id(run_byte);
        let artifact_id = artifact_id(artifact_byte);
        let artifact_digest = content_digest(digest_byte);
        let mut stored_evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
        match case {
            Case::ByteLen => stored_evidence.byte_len = 129,
            Case::MediaType => {
                stored_evidence.media_type = media_type("application/octet-stream");
            }
            Case::Producer => stored_evidence.producer_node_id = Some(node_id(99)),
        }
        let evidence_hash = stored_evidence
            .evidence_hash()
            .expect("mutated evidence hash");
        let payloads = match case {
            Case::ByteLen | Case::MediaType => {
                let mut artifact_ref =
                    event_artifact_ref(artifact_id.clone(), artifact_digest.clone());
                // Pin the exact admitted evidence key while keeping payload field values
                // unmutated so validation fails on the mismatched field.
                artifact_ref.evidence_hash = evidence_hash;
                vec![KernelEventPayload::ArtifactReferenced(
                    events::ArtifactReferenced {
                        spec_hash: spec_hash(1),
                        node_id: Some(node_id(20)),
                        attempt_id: Some(attempt_id(23)),
                        artifact_ref,
                    },
                )]
            }
            Case::Producer => {
                let mut payloads =
                    terminal_cell_commit_payloads(artifact_id.clone(), artifact_digest.clone());
                for payload in &mut payloads {
                    if let KernelEventPayload::CellProduced(cell) = payload {
                        cell.evidence_hash = evidence_hash.clone();
                    }
                    if let KernelEventPayload::ArtifactReferenced(reference) = payload {
                        reference.artifact_ref.evidence_hash = evidence_hash.clone();
                    }
                }
                payloads
            }
        };
        let request = default_commit_request(
            &run_id,
            StreamSeq::FIRST,
            commit_key,
            payloads,
            vec![stored_evidence.clone()],
        );
        let mut store = StoreContractRunStore::new();
        let error = store
            .append_prepared_commit_with_artifacts(request, vec![stored_evidence])
            .expect_err("artifact evidence mismatch rejects commit");
        assert_invalid_prepared_commit_contains(error, expected_field);
        assert!(store.load_run_stream(&run_id).is_empty(), "{case:?}");
    }
}
