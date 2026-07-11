use super::*;

#[test]
fn commit_request_rejects_empty_payloads() {
    assert!(matches!(
        CommitRequest::from_payloads(
            run_id(144),
            StreamSeq::new(1).expect("seq"),
            CommitKey::new("empty-request").expect("commit key"),
            Vec::new(),
            Vec::new(),
            CommitPreconditions::default(),
        ),
        Err(StoreError::EmptyCommit)
    ));
}

#[test]
fn prepared_commit_plan_mints_valid_run_start_authority() {
    let run_id = run_id(145);
    let request = run_start_request(run_id.clone(), "purpose-run-start");
    let artifacts = CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        request.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    let commit = PreparedCommit::<RunAdmission>::new(request.clone(), artifacts)
        .expect("run-start authority");
    let plan = PreparedCommitPlan::from(commit);

    assert_eq!(plan.request().run_id(), &run_id);
    assert_eq!(plan.request().payloads(), request.payloads());
    assert_eq!(
        plan.request().required_artifacts(),
        request.required_artifacts()
    );
}

#[test]
fn prepared_commit_authority_rejects_invalid_request_shapes() {
    let base_run_id = run_id(146);
    assert!(matches!(
        CommitRequest::from_payloads(
            base_run_id.clone(),
            StreamSeq::FIRST,
            CommitKey::new("purpose-empty").expect("commit key"),
            Vec::new(),
            Vec::new(),
            CommitPreconditions::default(),
        ),
        Err(StoreError::EmptyCommit)
    ));

    assert!(matches!(
        CommitRequest::from_payloads(
            base_run_id.clone(),
            StreamSeq::FIRST,
            CommitKey::new("purpose-mixed-run").expect("commit key"),
            vec![run_admitted(run_id(147))],
            Vec::new(),
            CommitPreconditions::default(),
        ),
        Err(StoreError::PayloadRunMismatch { .. })
    ));

    let mut foreign_spec_payload = side_effect_attempt_started();
    let KernelEventPayload::StateAttemptStarted(payload) = &mut foreign_spec_payload else {
        panic!("state-attempt-start payload")
    };
    payload.spec_hash = spec_hash(148);
    assert!(matches!(
        CommitRequest::from_payloads(
            base_run_id.clone(),
            StreamSeq::FIRST,
            CommitKey::new("purpose-mixed-spec").expect("commit key"),
            vec![run_admitted(base_run_id.clone()), foreign_spec_payload],
            Vec::new(),
            CommitPreconditions::default(),
        ),
        Err(StoreError::PayloadSpecHashMismatch { .. })
    ));

    let missing_artifact = CommitRequest::from_payloads(
        base_run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new("purpose-missing-artifact").expect("commit key"),
        vec![run_admitted(base_run_id.clone())],
        vec![spec_artifact_ref()],
        CommitPreconditions::default(),
    )
    .expect("missing artifact request");
    let missing_artifact_set = CommitArtifactEvidenceSet::new(
        missing_artifact.required_artifacts().to_vec(),
        missing_artifact.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    assert!(matches!(
        PreparedCommit::<RunAdmission>::new(missing_artifact, missing_artifact_set),
        Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "run_admission",
            ..
        })
    ));

    let wrong_purpose = run_start_request(base_run_id, "purpose-wrong-marker");
    let wrong_purpose_artifacts = CommitArtifactEvidenceSet::new(
        wrong_purpose.required_artifacts().to_vec(),
        wrong_purpose.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    assert!(matches!(
        PreparedCommit::<StateAttemptStarted>::new(wrong_purpose, wrong_purpose_artifacts),
        Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "state_attempt_started",
            ..
        })
    ));
}

#[test]
fn prepared_commit_plan_accepts_explicit_terminal_attempt_authority() {
    let artifact_id = artifact_id(149);
    let digest = content_digest(150);
    let payloads = terminal_cell_commit_payloads(artifact_id.clone(), digest.clone());
    let artifact = store_artifact_ref(artifact_id, digest);
    let request = typed_commit_request! {
        run_id: run_id(151),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("purpose-attempt-terminal").expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![artifact.clone()],
        preconditions: run_state_preconditions(RequiredRunState::NotCompleted),
    };
    let commit = PreparedCommit::<AttemptTerminal>::new(
        request,
        CommitArtifactEvidenceSet::new(vec![artifact.clone()], vec![artifact])
            .expect("artifact evidence set"),
    )
    .expect("attempt-terminal authority");
    let plan = PreparedCommitPlan::from(commit);

    assert!(matches!(plan, PreparedCommitPlan::AttemptTerminal(_)));
}

#[test]
fn attempt_terminal_rejects_unrelated_resource_lane_release() {
    let release = resource_lane_release_intent_for(side_effect_ledger_key());
    let request = default_commit_request(
        &run_id(153),
        StreamSeq::FIRST,
        "attempt-terminal-unrelated-release",
        vec![release, state_attempt_completed()],
        Vec::new(),
    );
    let error = PreparedCommit::<AttemptTerminal>::new(
        request,
        CommitArtifactEvidenceSet::new(Vec::new(), Vec::new()).expect("artifact evidence set"),
    )
    .expect_err("unrelated release rejects");
    assert_invalid_prepared_commit_contains(
        error,
        "resource lane release must precede a matching terminal attempt payload",
    );
}

#[test]
fn side_effect_terminal_rejects_same_commit_resource_lane_claim() {
    let run_id = run_id(154);
    let mut payloads = vec![
        resource_lane_claim_intent(resource_key("account:154", 154)),
        side_effect_failed(false),
        side_effect_attempt_failed(false),
    ];
    let preconditions =
        certify_payloads_for_policy(&run_id, SagaPolicySpec::NoSideEffects, &mut payloads);
    let request = typed_commit_request! {
        run_id: run_id,
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("side-effect-terminal-lane-claim").expect("commit key"),
        payloads: payloads,
        required_artifacts: Vec::new(),
        preconditions: preconditions,
    };
    let error = PreparedCommit::<SideEffectTerminal>::new(
        request,
        CommitArtifactEvidenceSet::new(Vec::new(), Vec::new()).expect("artifact evidence set"),
    )
    .expect_err("terminal lane claim rejects");
    assert_invalid_prepared_commit_contains(
        error,
        "terminal commits cannot acquire resource lanes",
    );
}

#[test]
fn commit_rejects_secret_shaped_persisted_error_message() {
    let mut store = StoreContractRunStore::new();
    let run_id = run_id(152);
    ensure_test_run_admitted(&mut store, &run_id, "public-diagnostic-secret-run-start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-secret-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(&run_id, compensate_saga_policy()),
        })
        .expect("append attempt start");

    let mut failure = fact_attempt_failed(false);
    let KernelEventPayload::StateAttemptFailed(payload) = &mut failure else {
        unreachable!("helper returns state attempt failure");
    };
    payload.error.safe_message = "provider returned bearer token=super-secret-value".to_owned();
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-secret-attempt-failed").expect("commit key"),
            payloads: vec![failure],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(&run_id, compensate_saga_policy()),
        })
        .expect_err("secret-shaped diagnostic rejects before append");

    assert_event_error_contains(error.clone(), "message resembles secret material");
    assert!(
        !error.to_string().contains("super-secret-value"),
        "rejection must not echo secret-shaped diagnostic text"
    );
}

#[test]
fn commit_rejects_non_redacted_diagnostic_artifact_ref() {
    let mut store = StoreContractRunStore::new();
    let run_id = run_id(153);
    ensure_test_run_admitted(&mut store, &run_id, "public-diagnostic-artifact-run-start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-artifact-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(&run_id, compensate_saga_policy()),
        })
        .expect("append attempt start");

    let artifact_id = artifact_id(154);
    let digest = content_digest(155);
    let schema_id = schema_id("mfm.test.diagnostic", 156);
    let required_artifact = ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: digest.clone(),
        byte_len: 64,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node_id(90)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::SideEffectIntent,
    };
    let diagnostic_ref = events::ArtifactEvidenceRef {
        artifact_id,
        role: ArtifactRole::SideEffectIntent,
        schema_id,
        semantic_type_id: None,
        content_digest: digest,
        evidence_hash: required_artifact
            .evidence_hash()
            .expect("diagnostic evidence hash"),
        byte_len: 64,
        media_type: media_type("application/json"),
    };
    let mut failure = fact_attempt_failed(false);
    let KernelEventPayload::StateAttemptFailed(payload) = &mut failure else {
        unreachable!("helper returns state attempt failure");
    };
    payload.error.diagnostic_ref = Some(diagnostic_ref);

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-artifact-attempt-failed").expect("commit key"),
            payloads: vec![failure],
            required_artifacts: vec![required_artifact],
            preconditions: saga_preconditions(&run_id, compensate_saga_policy()),
        })
        .expect_err("non-redacted diagnostic artifact rejects before append");

    assert_event_error_contains(
        error,
        "diagnostic artifact role must be redacted_diagnostic",
    );
}

#[test]
fn run_admission_rejects_schema_less_launch_artifacts() {
    let run_id = run_id(176);
    let mut payload = run_admitted(run_id.clone());
    let KernelEventPayload::RunAdmitted(run_admitted) = &mut payload else {
        unreachable!("helper returns run admission");
    };
    run_admitted.spec_artifact.schema_id = None;

    let request = CommitRequest::from_payloads(
        run_id,
        StreamSeq::FIRST,
        CommitKey::new("schema-less-launch-artifact").expect("commit key"),
        vec![payload],
        vec![spec_artifact_ref(), certificate_artifact_ref()],
        run_state_preconditions(RequiredRunState::Absent),
    )
    .expect("run start request");

    let mut store = StoreContractRunStore::new();
    let error = store
        .append_prepared_commit(request)
        .expect_err("schema-less launch artifact must reject");
    assert_invalid_prepared_commit_contains(error, "schema_id");
}

#[test]
fn in_memory_store_rejects_unbacked_existing_artifact_admission() {
    let store = AsyncInMemoryRunStore::new();
    let request = run_start_request(run_id(39), "missing-existing-artifact-run-start");
    let plan = test_prepared_commit_plan(request.clone(), request.required_artifacts().to_vec())
        .expect("prepare run start");
    let bundle = test_bundle_from_plan(plan).expect("existing artifact bundle");

    let error = poll_ready_store_future(store.append_prepared_commit_bundle(bundle))
        .expect_err("missing existing artifact admission is rejected");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
}

#[test]
fn commit_key_replay_classification_precedes_stale_expected_next_seq() {
    enum Case {
        SameFingerprint,
        DifferentFingerprint,
    }

    for (case, run_id) in [
        (Case::SameFingerprint, run_id(40)),
        (Case::DifferentFingerprint, run_id(41)),
    ] {
        let mut store = StoreContractRunStore::new();
        let request = run_start_request(run_id.clone(), "run-start");
        let appended = store
            .append_prepared_commit(request.clone())
            .expect("append run start");
        assert!(matches!(appended, CommitOutcome::Appended(_)));

        let stale_seq = StreamSeq::new(99).expect("stale seq");
        match case {
            Case::SameFingerprint => {
                let retry = request.with_expected_next_seq(stale_seq);
                let outcome = store
                    .append_prepared_commit(retry)
                    .expect("idempotent retry");

                let CommitOutcome::Idempotent(batch) = outcome else {
                    panic!("same commit key and fingerprint should be idempotent");
                };
                assert_eq!(batch.seq(), StreamSeq::FIRST);
                assert_eq!(store.expected_next_seq(&run_id), StreamSeq::new(2).unwrap());
            }
            Case::DifferentFingerprint => {
                let conflicting = default_commit_request(
                    &run_id,
                    stale_seq,
                    "run-start",
                    vec![state_attempt_started()],
                    Vec::new(),
                );

                let error = store
                    .append_prepared_commit(conflicting)
                    .expect_err("same key different fingerprint conflicts");
                assert!(matches!(error, StoreError::CommitConflict { .. }));
            }
        }
    }
}

#[test]
fn async_in_memory_store_exposes_commit_stream_and_status_contract() {
    let store = AsyncInMemoryRunStore::new();
    let resource_run = run_id(43);
    let status_run = run_id(44);
    let lane_key = resource_lane_key_with_schema("async-wallet", 230);

    let resource_request = run_start_request(resource_run.clone(), "async-resource-run-start");
    let resource_plan = test_prepared_commit_plan(
        resource_request.clone(),
        resource_request.required_artifacts().to_vec(),
    )
    .expect("prepare resource run start");
    store
        .seed_artifact_evidence_for_test(resource_plan.admitted_artifacts())
        .expect("seed resource artifact authority");
    let resource_bundle = test_bundle_from_plan(resource_plan).expect("bundle resource run start");
    let resource_outcome =
        poll_ready_store_future(store.append_prepared_commit_bundle(resource_bundle))
            .expect("append resource run start");
    assert!(matches!(resource_outcome, CommitOutcome::Appended(_)));

    append_async_side_effect_prepare_for_ledger(
        &store,
        &resource_run,
        "async-resource-prepare",
        side_effect_ledger_key_with_suffix(30),
        resource_key("async-wallet", 230),
        170,
    );

    let status_request = run_start_request(status_run.clone(), "async-status-run-start");
    let status_plan = test_prepared_commit_plan(
        status_request.clone(),
        status_request.required_artifacts().to_vec(),
    )
    .expect("prepare status run start");
    store
        .seed_artifact_evidence_for_test(status_plan.admitted_artifacts())
        .expect("seed status artifact authority");
    let status_bundle = test_bundle_from_plan(status_plan).expect("bundle status run start");
    poll_ready_store_future(store.append_prepared_commit_bundle(status_bundle))
        .expect("append status run start");

    let stream =
        poll_ready_store_future(store.load_run_stream(&status_run)).expect("load status stream");
    assert_eq!(stream.len(), 1);
    assert_eq!(stream[0].run_id(), &status_run);
    assert_eq!(
        poll_ready_store_future(store.expected_next_seq(&status_run)).expect("status next seq"),
        StreamSeq::new(2).expect("next seq")
    );

    let status_projection = poll_ready_store_future(store.status_projection_snapshot(&status_run))
        .expect("status projection");
    assert_eq!(status_projection.run_state(&status_run), RunState::Started);
    assert_eq!(status_projection.run_state(&resource_run), RunState::Absent);
    let lane = status_projection
        .resource_lane(&lane_key)
        .expect("cross-run resource lane");
    assert_eq!(lane.holder.run_id, resource_run);
    assert_eq!(lane.holder.pair_id, side_effect_pair_id());
    assert_eq!(lane.ledger_key, side_effect_ledger_key_with_suffix(30));
}

#[test]
fn store_owns_envelope_sequence_ordinal_and_event_id() {
    let run_id = run_id(42);
    let mut store = StoreContractRunStore::new();
    let first = store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    let CommitOutcome::Appended(first_batch) = first else {
        panic!("run start should append");
    };
    let first_event = &first_batch.events()[0];
    assert_eq!(first_event.seq(), StreamSeq::FIRST);
    assert_eq!(first_event.store_commit_order().as_u64(), 1);
    assert_eq!(first_event.ordinal().as_u32(), 0);
    assert_eq!(first_event.commit_key().as_str(), "run-start");
    assert_eq!(first_event.logical_key().as_str(), "run:admission");

    let second = append_run_state_commit(
        &mut store,
        &run_id,
        "attempt-start",
        vec![state_attempt_started()],
        Vec::new(),
        RequiredRunState::NotCompleted,
    )
    .expect("append attempt start");
    let CommitOutcome::Appended(second_batch) = second else {
        panic!("attempt start should append");
    };
    let second_event = &second_batch.events()[0];
    assert_eq!(second_event.seq(), StreamSeq::new(2).unwrap());
    assert_eq!(second_event.store_commit_order().as_u64(), 2);
    assert_eq!(second_event.ordinal().as_u32(), 0);
    assert_ne!(first_event.event_id(), second_event.event_id());

    let other_run = run_id_with_saga_policy(142, &SagaPolicySpec::NoSideEffects);
    let other_request = run_start_request(other_run.clone(), "other-run-start");
    let other_plan = test_prepared_commit_plan(
        other_request.clone(),
        other_request.required_artifacts().to_vec(),
    )
    .expect("prepare second run start");
    let CommitOutcome::Appended(other_batch) = store
        .append_test_commit_plan(other_plan)
        .expect("append second run start")
    else {
        panic!("second run start should append");
    };
    assert_eq!(other_batch.events()[0].seq(), StreamSeq::FIRST);
    assert_eq!(other_batch.events()[0].store_commit_order().as_u64(), 3);
}

#[test]
fn required_artifact_precondition_is_atomic_with_append() {
    let artifact_id = artifact_id(50);
    let artifact_digest = content_digest(51);
    let mut store = StoreContractRunStore::new();
    let run_id = run_id(52);
    let evidence = event_artifact_ref(artifact_id, artifact_digest);
    let request = default_commit_request(
        &run_id,
        StreamSeq::FIRST,
        "artifact-ref",
        vec![KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: spec_hash(1),
                node_id: Some(node_id(20)),
                attempt_id: Some(attempt_id(23)),
                artifact_ref: evidence.clone(),
            },
        )],
        vec![store_artifact_ref(
            evidence.artifact_id.clone(),
            evidence.content_digest.clone(),
        )],
    );

    let artifacts =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), Vec::new())
            .expect("missing artifact evidence set");
    let commit = PreparedCommit::<AttemptTerminal>::new(request, artifacts)
        .expect("prepare missing artifact");
    let error = store
        .append_test_commit_plan(commit.into())
        .expect_err("missing artifact rejects commit");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
    assert!(store.load_run_stream(&run_id).is_empty());
    assert_eq!(store.expected_next_seq(&run_id), StreamSeq::FIRST);
}

#[test]
fn prepared_commit_rejects_unreferenced_admitted_artifact_without_persisting_it() {
    let run_id = run_id(53);
    let artifact_id = artifact_id(54);
    let artifact_digest = content_digest(55);
    let mut store = StoreContractRunStore::new();
    let admitted = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());

    let error = store
        .append_prepared_commit_with_artifacts(
            default_commit_request(
                &run_id,
                StreamSeq::FIRST,
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
    assert!(store.load_run_stream(&run_id).is_empty());

    let missing_evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let error = store
        .append_prepared_commit_with_artifacts(
            default_commit_request(
                &run_id,
                StreamSeq::FIRST,
                "artifact-still-missing",
                vec![KernelEventPayload::ArtifactReferenced(
                    events::ArtifactReferenced {
                        spec_hash: spec_hash(1),
                        node_id: Some(node_id(20)),
                        attempt_id: Some(attempt_id(23)),
                        artifact_ref: event_artifact_ref(artifact_id, artifact_digest),
                    },
                )],
                vec![missing_evidence],
            ),
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

    let first_requirement = events::EventArtifactRequirement {
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
    let second_requirement = events::EventArtifactRequirement {
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
    let swapped_requirement = events::EventArtifactRequirement {
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
    let mut store = StoreContractRunStore::new();
    let evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());

    let error = store
        .append_prepared_commit(default_commit_request(
            &run_id,
            StreamSeq::FIRST,
            "cell-without-attempt-complete",
            vec![cell_produced(artifact_id.clone(), artifact_digest.clone())],
            vec![evidence],
        ))
        .expect_err("projection failure rejects commit after artifact validation");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
    assert!(store.load_run_stream(&run_id).is_empty());

    let missing_evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let error = store
        .append_prepared_commit_with_artifacts(
            default_commit_request(
                &run_id,
                StreamSeq::FIRST,
                "artifact-not-leaked",
                vec![KernelEventPayload::ArtifactReferenced(
                    events::ArtifactReferenced {
                        spec_hash: spec_hash(1),
                        node_id: Some(node_id(20)),
                        attempt_id: Some(attempt_id(23)),
                        artifact_ref: event_artifact_ref(artifact_id, artifact_digest),
                    },
                )],
                vec![missing_evidence],
            ),
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
