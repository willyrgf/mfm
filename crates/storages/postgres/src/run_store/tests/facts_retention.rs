use super::*;

async fn fixed_postgres_fact_snapshot_query(
    store: PostgresStore,
    plan: mfm_facts::CanonicalFactQueryPlan,
    snapshot_fixed: std::sync::Arc<tokio::sync::Barrier>,
    release_snapshot: std::sync::Arc<tokio::sync::Barrier>,
) -> (u64, mfm_facts::FactQueryResult) {
    let mut tx = store.pool.begin().await.expect("begin fact query");
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .expect("set repeatable-read snapshot");
    let fixed_order: i64 =
        sqlx::query_scalar("SELECT current_order FROM store_commit_order WHERE singleton")
            .fetch_one(&mut *tx)
            .await
            .expect("establish fact query snapshot");
    snapshot_fixed.wait().await;
    release_snapshot.wait().await;
    let mut results =
        super::super::fact_queries::execute_fact_queries_tx(&mut tx, std::slice::from_ref(&plan))
            .await
            .expect("execute fixed-snapshot query");
    tx.commit().await.expect("commit fact query");
    (
        u64::try_from(fixed_order).expect("nonnegative store order"),
        results.pop().expect("one aligned fact query result"),
    )
}

#[tokio::test]
async fn fact_queries_are_snapshot_aligned_across_descriptor_and_fact_appends() {
    let (store, schema) = test_store().await;
    let plan = fact_query_plan_with_limit(None);
    let run = fact_run_id(9);

    let snapshot_fixed = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let release_snapshot = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let query = tokio::spawn(fixed_postgres_fact_snapshot_query(
        store.clone(),
        plan.clone(),
        std::sync::Arc::clone(&snapshot_fixed),
        std::sync::Arc::clone(&release_snapshot),
    ));
    snapshot_fixed.wait().await;
    append_fact_run_start(&store, run.clone())
        .await
        .expect("descriptor-only admission");
    release_snapshot.wait().await;
    let (fixed_order, before_descriptor) = query.await.expect("descriptor query task");
    assert_eq!(fixed_order, 0);
    assert!(before_descriptor.rows().is_empty());
    assert_eq!(
        before_descriptor
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        fixed_order
    );

    let after_descriptor = store
        .execute_fact_query(&plan)
        .await
        .expect("post-descriptor query");
    assert!(after_descriptor.rows().is_empty());
    assert_eq!(
        after_descriptor
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        1
    );
    assert_eq!(
        store
            .fact_projection_snapshot()
            .await
            .expect("descriptor projection")
            .fact_descriptors()
            .count(),
        1
    );

    append_fact_attempt_start(&store, run.clone(), "snapshot-fact-attempt-start")
        .await
        .expect("fact attempt start");
    let snapshot_fixed = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let release_snapshot = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let query = tokio::spawn(fixed_postgres_fact_snapshot_query(
        store.clone(),
        plan.clone(),
        std::sync::Arc::clone(&snapshot_fixed),
        std::sync::Arc::clone(&release_snapshot),
    ));
    snapshot_fixed.wait().await;
    let response = fact_artifact_ref();
    append_fact_commit(
        &store,
        fact_commit_request(run.clone(), 3, "snapshot-fact", &response),
        &response,
    )
    .await
    .expect("fact-producing append");
    release_snapshot.wait().await;
    let (fixed_order, before_fact) = query.await.expect("fact query task");
    assert_eq!(fixed_order, 2);
    assert!(before_fact.rows().is_empty());
    assert_eq!(
        before_fact
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        fixed_order
    );

    let after_fact = store
        .execute_fact_query(&plan)
        .await
        .expect("post-fact query");
    assert_eq!(after_fact.rows().len(), 1);
    assert_eq!(
        after_fact
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        3
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn required_artifacts_and_fact_projection_are_atomic() {
    let (store, schema) = test_store().await;
    let run = fact_run_id(10);
    append_fact_run_start(&store, run.clone())
        .await
        .expect("run start");
    append_fact_attempt_start(&store, run.clone(), "fact-attempt-start")
        .await
        .expect("fact attempt start");

    let response_ref = fact_artifact_ref();
    let fact_request = fact_commit_request(run.clone(), 3, "fact", &response_ref);
    let err = append_fact_commit_with_missing_existing_artifact(
        &store,
        fact_request.clone(),
        &response_ref,
    )
    .await
    .expect_err("missing fact artifact");
    assert!(matches!(
        err,
        PostgresStoreError::Store(StoreError::MissingArtifact { .. })
    ));
    assert_eq!(
        expected_next_sequence(&store, &run).await,
        StreamSeq::new(3).expect("seq")
    );
    assert_fact_projection_table_counts(&store, &run, 1, 0, 0).await;
    let empty_plan = fact_query_plan_with_limit(None);
    assert_empty_fact_query(&store, &empty_plan).await;

    append_fact_commit(&store, fact_request.clone(), &response_ref)
        .await
        .expect("fact commit after artifact");
    let projection = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    assert_fact_projection_counts(&projection, 1, 1, 2);
    assert!(projection
        .fact_query_entries()
        .any(|(_, fact)| fact.producer_node_id() == &node_id(30)
            && fact.attempt_id() == &attempt_id(31)
            && fact.fact_key() == &fact_key()));
    assert_fact_projection_table_counts(&store, &run, 1, 1, 2).await;
    let exact_plan = fact_query_plan_with_limit(Some(10));
    let exact_query_result = assert_single_public_fact_query(&store, &exact_plan).await;
    assert_fact_query_receipt(
        &store,
        &exact_plan,
        &exact_query_result,
        mfm_facts::QueryResultCardinality::Exact(1),
    );
    let query_plan = fact_query_plan();
    let query_result = assert_single_public_fact_query(&store, &query_plan).await;
    let receipt = query_result.receipt();
    assert_fact_query_receipt(
        &store,
        &query_plan,
        &query_result,
        mfm_facts::QueryResultCardinality::AtLeast(1),
    );
    assert_eq!(receipt.returned_refs().len(), 1);
    assert_eq!(
        receipt.read_frontier().store_scope_id(),
        store.store_authority().store_scope_id()
    );
    assert_eq!(receipt.read_frontier().store_commit_order().as_u64(), 3);
    sqlx::query(
        "UPDATE fact_query_terms SET value_u64 = '1' \
         WHERE source_run_id = $1 AND field_id = 'result.height'",
    )
    .bind(run.as_str())
    .execute(&store.pool)
    .await
    .expect("tamper fact projection term");
    let query_after_projection_tamper = assert_single_public_fact_query(&store, &query_plan).await;
    assert_eq!(
        query_after_projection_tamper.rows(),
        query_result.rows(),
        "fact projection terms must not be semantic query authority"
    );
    sqlx::query("DELETE FROM fact_query_terms WHERE source_run_id = $1")
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("delete fact terms");
    sqlx::query("DELETE FROM fact_query_projection WHERE source_run_id = $1")
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("delete fact index");
    sqlx::query("DELETE FROM run_fact_descriptor_admissions WHERE run_id = $1")
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("delete run fact descriptor admissions");
    let query_after_projection_delete = store
        .execute_fact_query(&query_plan)
        .await
        .expect("fact query remains authoritative after projection deletion");
    assert_eq!(query_after_projection_delete.rows().len(), 1);
    assert_fact_query_receipt(
        &store,
        &query_plan,
        &query_after_projection_delete,
        mfm_facts::QueryResultCardinality::AtLeast(1),
    );
    let fact_projection_after_delete = store
        .fact_projection_snapshot()
        .await
        .expect("authoritative fact projection snapshot after cache deletion");
    assert_fact_projection_counts(&fact_projection_after_delete, 1, 1, 2);
    let err = store
        .status_projection_snapshot(&run)
        .await
        .expect_err("missing fact projection rows fail validation");
    assert!(matches!(err, PostgresStoreError::Corruption(_)));

    let rebuilt = rebuild_fact_projection_tables_client(&store.pool, &run)
        .await
        .expect("fact projection rebuild");
    assert_fact_projection_counts(&rebuilt, 1, 1, 2);
    let projection = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection after fact rebuild");
    assert_fact_projection_counts(&projection, 1, 1, 2);

    let retry = append_fact_commit(&store, fact_request, &response_ref)
        .await
        .expect("fact commit idempotent retry");
    assert!(matches!(retry, CommitOutcome::Idempotent(_)));
    assert_fact_projection_table_counts(&store, &run, 1, 1, 2).await;

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn fact_descriptor_catalog_and_response_artifacts_deduplicate_across_runs() {
    let (store, schema) = test_store().await;
    let first_run = fact_run_id(11);
    let second_run = fact_run_id(12);

    append_fact_run_start(&store, first_run.clone())
        .await
        .expect("first run start");
    append_fact_run_start(&store, second_run.clone())
        .await
        .expect("second run start with duplicate descriptor");
    assert_eq!(global_fact_descriptor_catalog_count(&store).await, 1);
    assert_fact_projection_table_counts(&store, &first_run, 1, 0, 0).await;
    assert_fact_projection_table_counts(&store, &second_run, 1, 0, 0).await;

    append_fact_attempt_start(&store, first_run.clone(), "first-fact-attempt-start")
        .await
        .expect("first fact attempt start");
    append_fact_attempt_start(&store, second_run.clone(), "second-fact-attempt-start")
        .await
        .expect("second fact attempt start");

    let first_response_ref = fact_artifact_ref_with_height(12_345);
    let first_fact_request =
        fact_commit_request(first_run.clone(), 3, "first-fact", &first_response_ref);
    append_fact_commit_with_response_bytes(
        &store,
        first_fact_request,
        &first_response_ref,
        fact_response_bytes_with_height(12_345),
    )
    .await
    .expect("first fact commit");

    let second_response_ref = fact_artifact_ref_with_height(12_345);
    let second_fact_request =
        fact_commit_request(second_run.clone(), 3, "second-fact", &second_response_ref);
    append_fact_commit_with_response_bytes(
        &store,
        second_fact_request,
        &second_response_ref,
        fact_response_bytes_with_height(12_345),
    )
    .await
    .expect("second fact commit");

    assert_eq!(global_fact_descriptor_catalog_count(&store).await, 1);
    assert_fact_projection_table_counts(&store, &first_run, 1, 1, 2).await;
    assert_fact_projection_table_counts(&store, &second_run, 1, 1, 2).await;

    let query_plan = fact_query_plan_with_limit(None);
    let query_result = store
        .execute_fact_query(&query_plan)
        .await
        .expect("fact query execution");
    assert_eq!(query_result.rows().len(), 2);
    let first_ref = query_result.rows()[0].fact_ref();
    let second_ref = query_result.rows()[1].fact_ref();
    assert_ne!(first_ref.fact_claim_id(), second_ref.fact_claim_id());
    assert_eq!(first_ref.artifact_id(), second_ref.artifact_id());
    assert_eq!(
        first_ref.artifact_evidence_hash(),
        second_ref.artifact_evidence_hash()
    );
    assert_fact_query_receipt(
        &store,
        &query_plan,
        &query_result,
        mfm_facts::QueryResultCardinality::Exact(2),
    );
    assert_eq!(
        query_result.receipt().read_frontier().store_scope_id(),
        store.store_authority().store_scope_id()
    );
    assert_eq!(
        query_result
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        6
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn side_effect_unknown_recovery_updates_submission_result_slot() {
    let (store, schema) = test_store().await;
    let run = run_id(13);

    append_prepared(
        &store,
        run_start_request(run.clone(), "run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");

    let intent_artifact = artifact_id(14);
    let intent_digest = content_digest(14);
    append_prepared(
        &store,
        certified_request(
            run.clone(),
            2,
            "sidefx-attempt-start",
            vec![side_effect_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt start");
    append_prepared(
        &store,
        certified_request(
            run.clone(),
            3,
            "sidefx-prepare",
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                side_effect_prepared(),
            ],
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
    .expect("prepare");
    append_prepared(
        &store,
        certified_request(
            run.clone(),
            4,
            "sidefx-started",
            vec![side_effect_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("started");

    let unknown_artifact = artifact_id(16);
    let unknown_digest = content_digest(16);
    let unknown_outcome = append_prepared(
        &store,
        certified_request(
            run.clone(),
            5,
            "sidefx-submission-unknown",
            vec![side_effect_submission_unknown(
                unknown_artifact.clone(),
                unknown_digest.clone(),
            )],
        ),
        vec![side_effect_artifact_ref(
            unknown_artifact,
            unknown_digest,
            unknown_schema(),
            ArtifactRole::SubmissionUnknownEvidence,
        )],
    )
    .await
    .expect("submission unknown");
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

    let submission_artifact = artifact_id(18);
    let submission_digest = content_digest(18);
    let observed_outcome = append_prepared(
        &store,
        certified_request(
            run.clone(),
            6,
            "sidefx-submission-observed-after-unknown",
            vec![side_effect_submission_observed(
                submission_artifact.clone(),
                submission_digest.clone(),
            )],
        ),
        vec![side_effect_artifact_ref(
            submission_artifact,
            submission_digest,
            submission_schema(),
            ArtifactRole::Submission,
        )],
    )
    .await
    .expect("submission observed recovery");
    let CommitOutcome::Appended(observed) = observed_outcome else {
        panic!("submission observed should append");
    };
    assert_eq!(
        observed.events()[0].logical_key().as_str(),
        submission_result_key
    );

    let projection = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let side_effect = projection
        .side_effect_for_pair(&run, &side_effect_pair_id())
        .expect("side-effect projection");
    assert!(matches!(
        side_effect.phase,
        SideEffectPhase::SubmissionObserved {
            invocation_epoch: 1
        }
    ));

    let stored_payload_hash = sqlx::query_scalar::<_, String>(
        "SELECT payload_hash FROM run_events \
         WHERE run_id = $1 AND logical_key = $2 \
         ORDER BY seq DESC, ordinal DESC \
         LIMIT 1",
    )
    .bind(run.as_str())
    .bind(submission_result_key.as_str())
    .fetch_one(&store.pool)
    .await
    .expect("run event payload row");
    assert_eq!(
        stored_payload_hash,
        observed.events()[0].payload_hash().as_str()
    );

    drop_schema(&store, &schema).await;
}
