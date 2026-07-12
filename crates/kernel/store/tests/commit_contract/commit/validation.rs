use super::*;

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
