use super::*;

#[tokio::test]
async fn saga_projection_rebuilds_from_events() {
    let (store, schema) = test_store().await;
    let saga_policy = manual_saga_policy(42);
    let run = run_id_with_saga_policy(41, &saga_policy);

    append_prepared(
        &store,
        run_start_request_with_saga_policy(run.clone(), "saga-run-start", &saga_policy),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            run.clone(),
            2,
            "saga-side-effect-attempt-start",
            vec![side_effect_attempt_started()],
            saga_policy.clone(),
        ),
        Vec::new(),
    )
    .await
    .expect("side-effect attempt start");
    let intent_artifact = artifact_id(40);
    let intent_digest = content_digest(40);
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            run.clone(),
            3,
            "saga-side-effect-prepare",
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                side_effect_prepared(),
            ],
            saga_policy.clone(),
        ),
        vec![
            side_effect_artifact_ref(
                intent_artifact,
                intent_digest,
                schema_id("mfm.test.side_effect_intent", 70),
                ArtifactRole::SideEffectIntent,
            ),
            prepared_artifact_ref(),
        ],
    )
    .await
    .expect("side-effect prepare");
    let ambiguity_artifact = artifact_id(140);
    let ambiguity_digest = content_digest(140);
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            run.clone(),
            4,
            "saga-side-effect-verify-attempt-start",
            vec![side_effect_verify_attempt_started()],
            saga_policy.clone(),
        ),
        Vec::new(),
    )
    .await
    .expect("side-effect verify attempt start");
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            run.clone(),
            5,
            "saga-side-effect-ambiguous",
            vec![
                side_effect_started(),
                side_effect_ambiguous(ambiguity_artifact.clone(), ambiguity_digest.clone()),
                side_effect_attempt_failed(),
            ],
            saga_policy.clone(),
        ),
        vec![side_effect_artifact_ref(
            ambiguity_artifact,
            ambiguity_digest,
            schema_id("mfm.test.ambiguity", 84),
            ArtifactRole::AmbiguityEvidence,
        )],
    )
    .await
    .expect("side-effect ambiguous");
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            run.clone(),
            6,
            "saga-side-effect-submit-attempt-completed",
            vec![
                side_effect_submit_boundary_output_skipped(),
                side_effect_submit_attempt_completed(),
            ],
            saga_policy.clone(),
        ),
        Vec::new(),
    )
    .await
    .expect("side-effect submit attempt completed");
    let verified = verified_manual_resolution_for_seq(&run, 7, 42);
    let manual_artifacts = manual_resolution_artifacts(&verified);
    let manual_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        StreamSeq::new(7).expect("manual resolution seq"),
        CommitKey::new("saga-manual-resolution").expect("commit key"),
        vec![manual_resolution_recorded(&verified)],
        manual_artifacts.clone(),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            ..saga_preconditions(&run, saga_policy)
        },
    )
    .expect("manual resolution request");
    let manual_commit = PreparedCommit::<ManualResolution>::new(
        manual_request,
        CommitArtifactEvidenceSet::new(manual_artifacts.clone(), manual_artifacts)
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect("proof-backed manual resolution prepared commit");
    store
        .append_prepared_commit_bundle(
            PreparedCommitBundle::new(
                manual_commit.into(),
                manual_resolution_prepared_artifact_bytes(&verified)
                    .expect("manual artifact bytes"),
                Vec::new(),
            )
            .expect("manual bundle"),
        )
        .await
        .expect("manual resolution");
    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let engagement = before.saga_engagement(&run).expect("saga engagement");
    assert!(matches!(
        engagement.reason,
        SagaEngagementReason::ForwardAmbiguous { .. }
    ));
    let manual = before
        .manual_resolution(&run)
        .expect("manual resolution projection");
    assert_eq!(
        manual.outcome,
        events::ManualResolutionOutcome::ConfirmRemediated
    );
    assert_eq!(
        manual
            .note
            .as_ref()
            .map(events::ManualResolutionNote::as_str),
        Some("reviewed evidence")
    );
    assert!(before.run_completion(&run).is_none());

    let stream = store.load_run_stream(&run).await.expect("typed run stream");
    assert_eq!(
        ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("payload rebuild"),
        before
    );

    let terminal_policy = SagaPolicySpec::FailWithoutAcdcClaim;
    let terminal_run = run_id_with_saga_policy(43, &terminal_policy);
    append_prepared(
        &store,
        run_start_request_with_saga_policy(
            terminal_run.clone(),
            "saga-terminal-run-start",
            &terminal_policy,
        ),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("terminal run start");
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            terminal_run.clone(),
            2,
            "saga-terminal-side-effect-attempt-start",
            vec![side_effect_attempt_started()],
            terminal_policy.clone(),
        ),
        Vec::new(),
    )
    .await
    .expect("terminal side-effect attempt start");
    let terminal_intent_artifact = artifact_id(150);
    let terminal_intent_digest = content_digest(150);
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            terminal_run.clone(),
            3,
            "saga-terminal-side-effect-prepare",
            vec![
                side_effect_intent(
                    terminal_intent_artifact.clone(),
                    terminal_intent_digest.clone(),
                ),
                side_effect_claim(),
                side_effect_prepared(),
            ],
            terminal_policy.clone(),
        ),
        vec![
            side_effect_artifact_ref(
                terminal_intent_artifact,
                terminal_intent_digest,
                schema_id("mfm.test.side_effect_intent", 70),
                ArtifactRole::SideEffectIntent,
            ),
            prepared_artifact_ref(),
        ],
    )
    .await
    .expect("terminal side-effect prepare");
    let terminal_ambiguity_artifact = artifact_id(152);
    let terminal_ambiguity_digest = content_digest(152);
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            terminal_run.clone(),
            4,
            "saga-terminal-side-effect-verify-attempt-start",
            vec![side_effect_verify_attempt_started()],
            terminal_policy.clone(),
        ),
        Vec::new(),
    )
    .await
    .expect("terminal side-effect verify attempt start");
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            terminal_run.clone(),
            5,
            "saga-terminal-side-effect-ambiguous",
            vec![
                side_effect_started(),
                side_effect_ambiguous(
                    terminal_ambiguity_artifact.clone(),
                    terminal_ambiguity_digest.clone(),
                ),
                side_effect_attempt_failed(),
            ],
            terminal_policy.clone(),
        ),
        vec![side_effect_artifact_ref(
            terminal_ambiguity_artifact,
            terminal_ambiguity_digest,
            schema_id("mfm.test.ambiguity", 84),
            ArtifactRole::AmbiguityEvidence,
        )],
    )
    .await
    .expect("terminal side-effect ambiguous");
    append_prepared(
        &store,
        certified_request_with_saga_policy(
            terminal_run.clone(),
            6,
            "saga-terminal-side-effect-submit-attempt-completed",
            vec![
                side_effect_submit_boundary_output_skipped(),
                side_effect_submit_attempt_completed(),
            ],
            terminal_policy.clone(),
        ),
        Vec::new(),
    )
    .await
    .expect("terminal side-effect submit attempt completed");
    let terminal_before_completion = store
        .status_projection_snapshot(&terminal_run)
        .await
        .expect("terminal projection before completion");
    let terminal_policies =
        confirmation_terminal_policies_for_projection(&terminal_before_completion, &terminal_run);
    let terminal_saga = terminal_before_completion
        .derive_saga_projection(&terminal_run, &terminal_policy, &terminal_policies)
        .expect("terminal saga projection");
    let terminal_next_seq = store
        .expected_next_seq(&terminal_run)
        .await
        .expect("terminal next seq");
    let terminal_proof =
        SagaTerminalProof::new(&terminal_policy, &terminal_saga, terminal_next_seq, None)
            .expect("terminal proof");
    let terminal_spec_hash = saga_authority_spec(terminal_policy.clone())
        .spec_hash()
        .expect("terminal saga authority spec hash");
    let mut terminal_completion = run_completed(
        terminal_run.clone(),
        events::RunCompletionOutcome::FailedWithoutAcdcClaim,
    );
    let KernelEventPayload::RunCompleted(payload) = &mut terminal_completion else {
        unreachable!("run_completed helper returns RunCompleted payload");
    };
    payload.spec_hash = terminal_spec_hash;
    let terminal_request = request(
        terminal_run.clone(),
        terminal_next_seq.as_u64(),
        "saga-terminal-run-completed",
        vec![terminal_completion],
    );
    let terminal_request = terminal_request
        .with_preconditions(saga_preconditions(&terminal_run, terminal_policy.clone()));
    let terminal_commit = PreparedCommit::<SagaTerminal>::new(
        terminal_request,
        CommitArtifactEvidenceSet::empty(),
        &terminal_proof,
    )
    .expect("proof-backed terminal commit");
    store
        .append_prepared_commit_bundle(
            test_prepared_commit_bundle(terminal_commit.into()).expect("terminal bundle"),
        )
        .await
        .expect("terminal run completed");

    let terminal_before = store
        .status_projection_snapshot(&terminal_run)
        .await
        .expect("terminal projection");
    assert!(matches!(
        terminal_before
            .run_completion(&terminal_run)
            .map(|projection| &projection.outcome),
        Some(events::RunCompletionOutcome::FailedWithoutAcdcClaim)
    ));
    let terminal_stream = store
        .load_run_stream(&terminal_run)
        .await
        .expect("terminal typed run stream");
    assert_eq!(
        ProjectionSnapshot::rebuild_from_run_stream(&terminal_stream)
            .expect("terminal payload rebuild"),
        terminal_before
    );

    drop_schema(&store, &schema).await;
}
