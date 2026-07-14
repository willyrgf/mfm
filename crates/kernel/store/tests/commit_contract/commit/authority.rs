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
