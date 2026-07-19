use super::*;

#[tokio::test]
async fn artifact_authority_accepts_distinct_evidence_for_same_artifact_id() {
    let (store, schema) = test_store().await;
    let run = run_id(122);
    let artifact = artifact_id(123);
    let digest = content_digest(123);
    let first_evidence =
        store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let mut second_evidence = first_evidence.clone();
    second_evidence.schema_id = Some(schema_id("mfm.test.alternate_position", 124));
    assert_ne!(
        first_evidence.evidence_hash().expect("first evidence hash"),
        second_evidence
            .evidence_hash()
            .expect("second evidence hash")
    );

    append_prepared(
        &store,
        run_start_request(run.clone(), "same-id-run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");

    let first_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        store.expected_next_seq(&run).await.expect("next seq"),
        CommitKey::new("same-id-first-evidence").expect("commit key"),
        vec![retention_refs_appended(
            run.clone(),
            artifact.clone(),
            digest.clone(),
            ArtifactRole::StateOutput,
        )],
        vec![first_evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("typed first request");
    append_prepared(&store, first_request, vec![first_evidence])
        .await
        .expect("append first evidence");

    let second_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        store.expected_next_seq(&run).await.expect("next seq"),
        CommitKey::new("same-id-second-evidence").expect("commit key"),
        vec![retention_refs_appended_for_evidence(
            run.clone(),
            &second_evidence,
            events::RetentionReason::PublicOutput,
        )],
        vec![second_evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("typed second request");
    append_prepared(&store, second_request, vec![second_evidence])
        .await
        .expect("append second evidence for same artifact id");

    let rows = sqlx::query(
        "SELECT evidence_hash FROM artifact_admissions WHERE artifact_id = $1 ORDER BY evidence_hash",
    )
    .bind(artifact.as_str())
    .fetch_all(&store.pool)
    .await
    .expect("retained evidence rows");
    assert_eq!(rows.len(), 2);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn artifact_precondition_failure_rolls_back_blob_insert() {
    let (store, schema) = test_store().await;
    let run = run_id(135);
    append_run_start(&store, &run, "blob-rollback-run-start")
        .await
        .expect("run start");
    let artifact =
        prepared_artifact_bytes_from_bytes(vec![0x5a; 1024 * 1024 + 1], ArtifactRole::StateOutput);
    let evidence = artifact.evidence().clone();
    let bundle = retention_artifact_bundle(
        run.clone(),
        2,
        "blob-rollback-retention",
        artifact,
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            ..CommitPreconditions::default()
        },
    )
    .expect("large retention bundle");

    let error = store
        .append_prepared_commit_bundle(bundle)
        .await
        .expect_err("precondition fails before blob commit");
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::RunStatePreconditionFailed { .. })
    ));
    let blob_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_blobs WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count rolled-back blob");
    assert_eq!(blob_count, 0);
    let admission_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_admissions WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count rolled-back admissions");
    assert_eq!(admission_count, 0);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn artifact_blob_success_is_admitted_in_append_transaction() {
    let (store, schema) = test_store().await;
    let run = run_id(136);
    append_run_start(&store, &run, "blob-success-run-start")
        .await
        .expect("run start");
    let artifact =
        prepared_artifact_bytes_from_bytes(vec![0x6b; 1024 * 1024 + 1], ArtifactRole::StateOutput);
    let evidence = artifact.evidence().clone();
    let bundle = retention_artifact_bundle(
        run.clone(),
        2,
        "blob-success-retention",
        artifact,
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("large retention bundle");

    let outcome = store
        .append_prepared_commit_bundle(bundle)
        .await
        .expect("large blob append");
    assert!(matches!(outcome, CommitOutcome::Appended(_)));
    let blob_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_blobs WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count committed blob");
    assert_eq!(blob_count, 1);
    let admission_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_admissions WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count committed admissions");
    assert_eq!(admission_count, 1);
    sqlx::query("DELETE FROM artifact_blobs WHERE artifact_id = $1")
        .bind(evidence.artifact_id.as_str())
        .execute(&store.pool)
        .await
        .expect_err("direct artifact blob delete is blocked");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn oversized_artifact_blob_is_rejected_before_authority_rows() {
    let (store, schema) = test_store().await;
    let run = run_id(137);
    append_run_start(&store, &run, "oversize-run-start")
        .await
        .expect("run start");
    let artifact = prepared_artifact_bytes_from_bytes(
        vec![0x7c; MAX_ARTIFACT_BLOB_BYTES as usize + 1],
        ArtifactRole::StateOutput,
    );
    let evidence = artifact.evidence().clone();
    let bundle = retention_artifact_bundle(
        run.clone(),
        2,
        "oversize-retention",
        artifact,
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("oversize retention bundle");

    let error = store
        .append_prepared_commit_bundle(bundle)
        .await
        .expect_err("oversized artifact rejected");
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::ArtifactEvidenceMismatch {
            field: "byte_len",
            ..
        })
    ));
    let blob_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_blobs WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count oversized blob rows");
    assert_eq!(blob_count, 0);
    let commit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM commits WHERE run_id = $1")
        .bind(run.as_str())
        .fetch_one(&store.pool)
        .await
        .expect("count run commits");
    assert_eq!(commit_count, 1);

    drop_schema(&store, &schema).await;
}
