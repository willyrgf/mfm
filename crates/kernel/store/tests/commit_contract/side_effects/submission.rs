use super::*;

#[test]
fn side_effect_submission_unknown_recovery_uses_one_submission_result_key() {
    let mut store = StoreContractRunStore::new();
    let run_id = run_id(113);
    append_side_effect_prepare(&mut store, &run_id);
    append_side_effect_started(&mut store, &run_id);

    let unknown_artifact = artifact_id(111);
    let unknown_digest = content_digest(112);
    let unknown_evidence = side_effect_evidence(
        unknown_artifact.clone(),
        unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let unknown_outcome = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-submission-unknown",
        vec![side_effect_submission_unknown(
            unknown_artifact,
            unknown_digest,
        )],
        vec![unknown_evidence],
    )
    .expect("append submission unknown");
    let CommitOutcome::Appended(unknown) = unknown_outcome else {
        panic!("submission unknown should append");
    };
    let submission_result_key = format!(
        "sidefx:forward:{}:invocation:1:submission_result",
        side_effect_pair_id()
    );
    assert_eq!(
        unknown.events()[0].logical_key().as_str(),
        submission_result_key
    );

    let refreshed_unknown_artifact = artifact_id(117);
    let refreshed_unknown_digest = content_digest(118);
    let refreshed_unknown_evidence = side_effect_evidence(
        refreshed_unknown_artifact.clone(),
        refreshed_unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let refreshed_unknown_outcome = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-submission-unknown-refresh",
        vec![side_effect_submission_unknown(
            refreshed_unknown_artifact,
            refreshed_unknown_digest,
        )],
        vec![refreshed_unknown_evidence],
    )
    .expect("refresh submission unknown");
    let CommitOutcome::Appended(refreshed_unknown) = refreshed_unknown_outcome else {
        panic!("refreshed submission unknown should append");
    };
    assert_eq!(
        refreshed_unknown.events()[0].logical_key().as_str(),
        submission_result_key
    );
    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        SideEffectPhase::SubmissionUnknown {
            invocation_epoch: 1
        }
    ));

    let submission_artifact = artifact_id(113);
    let submission_digest = content_digest(114);
    let submission_evidence = side_effect_evidence(
        submission_artifact.clone(),
        submission_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    let observed_outcome = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-submission-observed-after-unknown",
        vec![side_effect_submission_observed(
            submission_artifact,
            submission_digest,
        )],
        vec![submission_evidence],
    )
    .expect("recover observed submission");
    let CommitOutcome::Appended(observed) = observed_outcome else {
        panic!("observed submission should append");
    };
    assert_eq!(
        observed.events()[0].logical_key().as_str(),
        submission_result_key
    );
    let projection = store
        .projection_snapshot()
        .side_effect_for_pair(&run_id, &side_effect_pair_id())
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        SideEffectPhase::SubmissionObserved {
            invocation_epoch: 1
        }
    ));

    let duplicate_artifact = artifact_id(115);
    let duplicate_digest = content_digest(116);
    let duplicate_evidence = side_effect_evidence(
        duplicate_artifact.clone(),
        duplicate_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    let duplicate = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "sidefx-duplicate-observed-after-recovery",
        vec![side_effect_submission_observed(
            duplicate_artifact,
            duplicate_digest,
        )],
        vec![duplicate_evidence],
    )
    .expect_err("duplicate observed submission rejects after recovery");
    assert!(matches!(
        duplicate,
        StoreError::LogicalKeyConflict { .. } | StoreError::ProjectionConflict { .. }
    ));
}
