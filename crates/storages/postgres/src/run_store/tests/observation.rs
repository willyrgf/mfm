use super::*;

#[tokio::test]
async fn prepared_commit_idempotency_fingerprint_includes_admitted_artifacts() {
    let (store, schema) = test_store().await;
    let run = run_id(120);
    let artifact = artifact_id(121);
    let digest = content_digest(121);
    let evidence = store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let mut conflicting_evidence = evidence.clone();
    conflicting_evidence.media_type = media_type("application/octet-stream");
    append_prepared(
        &store,
        run_start_request(run.clone(), "run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    let request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        store.expected_next_seq(&run).await.expect("next seq"),
        CommitKey::new("prepared-fingerprint").expect("commit key"),
        vec![retention_refs_appended(
            run.clone(),
            artifact,
            digest,
            ArtifactRole::StateOutput,
        )],
        vec![evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("typed commit request");

    append_prepared(&store, request.clone(), vec![evidence])
        .await
        .expect("append initial prepared commit");
    let retry = request.with_expected_next_seq(StreamSeq::new(99).expect("stale seq"));
    let error = append_prepared(&store, retry, vec![conflicting_evidence])
        .await
        .expect_err("same request with different admitted evidence is not idempotent");
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::CommitConflict { .. })
    ));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn store_commit_orders_are_global_and_monotonic() {
    let (store, schema) = test_store().await;
    let first_run = run_id(125);
    append_run_start(&store, &first_run, "store-order-run-start")
        .await
        .expect("run start");
    append_resource_lane_attempt_start(&store, &first_run, "store-order-attempt-start")
        .await
        .expect("attempt start");
    let second_run = run_id(126);
    append_run_start(&store, &second_run, "store-order-second-run-start")
        .await
        .expect("second run start");

    let rows = sqlx::query(
        "SELECT run_id, seq, store_commit_order \
         FROM commits \
         ORDER BY store_commit_order",
    )
    .fetch_all(&store.pool)
    .await
    .expect("query store commit orders");
    assert_eq!(rows.len(), 3);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            row.try_get::<i64, _>("store_commit_order")
                .expect("store commit order"),
            i64::try_from(index + 1).expect("index fits i64")
        );
    }
    assert_eq!(
        rows[0].try_get::<String, _>("run_id").expect("run id"),
        first_run.as_str()
    );
    assert_eq!(
        rows[1].try_get::<String, _>("run_id").expect("run id"),
        first_run.as_str()
    );
    assert_eq!(
        rows[2].try_get::<String, _>("run_id").expect("run id"),
        second_run.as_str()
    );
    assert_eq!(rows[0].try_get::<i64, _>("seq").expect("seq"), 1);
    assert_eq!(rows[1].try_get::<i64, _>("seq").expect("seq"), 2);
    assert_eq!(rows[2].try_get::<i64, _>("seq").expect("seq"), 1);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn strict_load_rejects_corrupt_commit_fingerprint() {
    let (store, schema) = test_store().await;
    let run = run_id(126);
    append_run_start(&store, &run, "strict-canonical-run-start")
        .await
        .expect("run start");

    sqlx::query("ALTER TABLE commits DISABLE TRIGGER commits_no_update")
        .execute(&store.pool)
        .await
        .expect("disable commit mutation guard");
    sqlx::query(
        "UPDATE commits SET prepared_commit_plan_fingerprint = $1 WHERE run_id = $2 AND seq = 1",
    )
    .bind(content_digest(251).as_str())
    .bind(run.as_str())
    .execute(&store.pool)
    .await
    .expect("corrupt commit fingerprint");

    let error = store
        .load_run_stream(&run)
        .await
        .expect_err("strict load rejects corrupt commit fingerprint");
    assert_corruption(error, "commit id does not match persisted fingerprint");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn strict_load_rejects_commit_batch_authority_rewritten_away_from_rows() {
    let (store, schema) = test_store().await;
    let run = run_id(144);
    let commit_key = CommitKey::new("strict-batch-authority-run-start").expect("commit key");
    append_run_start(&store, &run, commit_key.as_str())
        .await
        .expect("run start");

    let forged_batch = canonical_json(serde_json::json!({
        "domain": "mfm.commit.batch.v1",
        "tampered": true,
    }))
    .expect("canonical forged batch");
    let forged_batch_hash = forged_batch.content_digest().as_str().to_owned();
    sqlx::query("ALTER TABLE commits DISABLE TRIGGER commits_no_update")
        .execute(&store.pool)
        .await
        .expect("disable commit mutation guard");
    sqlx::query("UPDATE commits SET commit_batch_hash = $1 WHERE run_id = $2 AND seq = 1")
        .bind(&forged_batch_hash)
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("forge commit batch authority");

    let error = store
        .load_run_stream(&run)
        .await
        .expect_err("strict load rejects batch authority that no longer matches rows");
    assert_corruption(
        error,
        "commit batch authority does not match persisted event and artifact bindings",
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_change_ids_and_cursors_do_not_expose_internal_authority() {
    let (store, schema) = test_store().await;
    let run = run_id(134);
    append_run_start(&store, &run, "opaque-observation-run-start")
        .await
        .expect("run start");

    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let row = sqlx::query(
        "SELECT commit_id, store_commit_order \
         FROM commits WHERE run_id = $1 AND seq = 1",
    )
    .bind(run.as_str())
    .fetch_one(&store.pool)
    .await
    .expect("load commit cursor authority");
    let commit_id: String = row.try_get("commit_id").expect("commit id");
    let store_commit_order = u64::try_from(
        row.try_get::<i64, _>("store_commit_order")
            .expect("store commit order"),
    )
    .expect("positive store commit order");
    let public_change_id =
        observation_change_id(&metadata, &commit_id).expect("observation change id");
    let position = CursorPosition { store_commit_order };
    let public_cursor = encode_observation_cursor(&store.pool, &metadata, &position)
        .await
        .expect("encode observation cursor");
    let decoded = decode_observation_cursor(&store.pool, &public_cursor, &metadata)
        .await
        .expect("decode observation cursor");
    assert_eq!(decoded.store_commit_order, position.store_commit_order);
    assert_eq!(
        bytes_from_hex(&public_cursor)
            .expect("opaque cursor token is hex")
            .len(),
        32
    );
    assert_eq!(
        bytes_from_hex(&public_change_id)
            .expect("opaque change id token is hex")
            .len(),
        32
    );
    for public_value in [public_cursor.as_str(), public_change_id.as_str()] {
        assert!(!public_value.contains('|'));
        assert!(!public_value.contains(CURSOR_VERSION));
        assert!(!public_value.contains(commit_id.as_str()));
        assert!(!public_value.contains(metadata.store_epoch.as_str()));
    }
    assert_ne!(public_change_id.as_str(), commit_id);

    let cursor_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_observation_cursors")
        .fetch_one(&store.pool)
        .await
        .expect("count cursor rows");
    assert_eq!(cursor_rows, 1);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_frontier_cursor_excludes_the_frontier_commit() {
    let (store, schema) = test_store().await;
    let run = run_id(143);
    append_run_start(&store, &run, "frontier-cursor-run-start")
        .await
        .expect("run start");

    let first = store
        .read_run_observations(RunObservationQuery::new(None, 10, 0))
        .await
        .expect("initial observation page");
    assert_eq!(first.runs.len(), 1);
    let second = store
        .read_run_observations(RunObservationQuery::new(Some(first.next_cursor), 10, 0))
        .await
        .expect("observation page after frontier");
    assert!(second.runs.is_empty());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_lifecycle_rejects_malformed_unknown_and_missing_tokens() {
    let (store, schema) = test_store().await;
    let run = run_id(145);
    append_run_start(&store, &run, "cursor-lifecycle-run-start")
        .await
        .expect("run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");

    let malformed = decode_observation_cursor(&store.pool, "not-hex", &metadata)
        .await
        .expect_err("non-hex cursor is invalid");
    assert_invalid_cursor(malformed, "hex");
    let unknown = decode_observation_cursor(&store.pool, &"00".repeat(32), &metadata)
        .await
        .expect_err("unknown cursor is invalid");
    assert_invalid_cursor(unknown, "unknown cursor");

    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    disable_observation_cursor_mutation_guard(&store.pool).await;
    sqlx::query("DELETE FROM run_observation_cursors WHERE token_hash = $1")
        .bind(observation_cursor_token_hash(&cursor))
        .execute(&store.pool)
        .await
        .expect("delete cursor row");
    let missing = decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect_err("missing cursor row is invalid");
    assert_invalid_cursor(missing, "unknown cursor");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_lifecycle_is_epoch_bound_without_ttl() {
    let (store, schema) = test_store().await;
    let run = run_id(146);
    append_run_start(&store, &run, "cursor-ttl-run-start")
        .await
        .expect("run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    let token_hash = observation_cursor_token_hash(&cursor);

    disable_observation_cursor_mutation_guard(&store.pool).await;
    sqlx::query(
        "UPDATE run_observation_cursors SET issued_at = '2000-01-01T00:00:00Z'::timestamptz \
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&store.pool)
    .await
    .expect("age cursor");
    decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect("aged issued_at does not expire current-epoch cursor");

    sqlx::query(
        "UPDATE run_observation_cursors \
         SET store_epoch = 'mfm.store.epoch.v1:prior' \
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&store.pool)
    .await
    .expect("move cursor to prior epoch");
    let expired = decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect_err("prior epoch cursor expires");
    assert_cursor_expired(expired);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_lifecycle_rejects_stale_format() {
    let (store, schema) = test_store().await;
    let run = run_id(147);
    append_run_start(&store, &run, "cursor-format-run-start")
        .await
        .expect("run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    let token_hash = observation_cursor_token_hash(&cursor);

    disable_observation_cursor_mutation_guard(&store.pool).await;
    sqlx::query(
        "ALTER TABLE run_observation_cursors DROP CONSTRAINT run_observation_cursors_version_v1",
    )
    .execute(&store.pool)
    .await
    .expect("drop cursor version constraint for stale-format fixture");
    sqlx::query(
        "UPDATE run_observation_cursors \
         SET cursor_version = 'mfm.run_observation.cursor.unsupported' \
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&store.pool)
    .await
    .expect("stale cursor format");
    let stale = decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect_err("stale cursor format is invalid");
    assert_invalid_cursor(stale, "stale cursor format");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_uses_durable_metadata_across_store_restarts() {
    let (store, schema) = test_store().await;
    let run = run_id(148);
    append_run_start(&store, &run, "cursor-restart-run-start")
        .await
        .expect("run start");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    let restarted = PostgresStore {
        pool: store.pool.clone(),
        authority: store.store_authority().clone(),
    };
    let page = restarted
        .read_run_observations(RunObservationQuery::new(Some(cursor), 10, 0))
        .await
        .expect("restarted store decodes durable cursor metadata");
    assert!(page.runs.is_empty());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_watch_cursor_pages_without_skipping_rows() {
    let (store, schema) = test_store().await;
    let run = run_id(149);
    append_run_start(&store, &run, "cursor-noskip-run-start")
        .await
        .expect("run start");
    append_retention_commit(&store, &run, 2, "cursor-noskip-retention-a", 150)
        .await
        .expect("append second observation row");
    append_retention_commit(&store, &run, 3, "cursor-noskip-retention-b", 151)
        .await
        .expect("append third observation row");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;

    let first = store
        .read_run_observations(RunObservationQuery::new(Some(cursor), 1, 5_000))
        .await
        .expect("first watch page");
    assert_eq!(first.runs.len(), 1);
    assert_eq!(first.runs[0].head_seq.as_u64(), 2);
    let second = store
        .read_run_observations(RunObservationQuery::new(Some(first.next_cursor), 1, 5_000))
        .await
        .expect("second watch page");
    assert_eq!(second.runs.len(), 1);
    assert_eq!(second.runs[0].head_seq.as_u64(), 3);
    let third = store
        .read_run_observations(RunObservationQuery::new(Some(second.next_cursor), 1, 0))
        .await
        .expect("third watch page");
    assert!(third.runs.is_empty());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_watch_polls_durable_rows_before_waiting_for_notify() {
    let (store, schema) = test_store().await;
    let run = run_id(152);
    append_run_start(&store, &run, "cursor-missed-notify-run-start")
        .await
        .expect("run start");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    append_retention_commit(&store, &run, 2, "cursor-missed-notify-retention", 153)
        .await
        .expect("append change before watcher starts");

    let started = Instant::now();
    let page = store
        .read_run_observations(RunObservationQuery::new(Some(cursor), 10, 5_000))
        .await
        .expect("watch polls durable rows before waiting");
    assert_eq!(page.runs.len(), 1);
    assert_eq!(page.runs[0].head_seq.as_u64(), 2);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "watch should return durable rows without waiting for notify"
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn append_commit_emits_observation_notification() {
    let (store, schema) = test_store().await;
    let mut listener = observation_change_listener(&store.pool)
        .await
        .expect("listen for observation changes");
    let run = run_id(141);

    append_run_start(&store, &run, "notify-run-start")
        .await
        .expect("append run start");

    let notification = tokio::time::timeout(Duration::from_secs(2), listener.recv())
        .await
        .expect("append should notify before timeout")
        .expect("receive observation notification");
    assert_eq!(notification.channel(), OBSERVATION_NOTIFY_CHANNEL);
    assert_eq!(notification.payload(), OBSERVATION_NOTIFY_PAYLOAD);
    drop(listener);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_watch_wakes_on_notification_before_timeout() {
    let (store, schema) = test_store().await;
    let run = run_id(142);
    append_run_start(&store, &run, "watch-notify-run-start")
        .await
        .expect("append run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let row = sqlx::query(
        "SELECT store_commit_order \
         FROM commits WHERE run_id = $1 AND seq = 1",
    )
    .bind(run.as_str())
    .fetch_one(&store.pool)
    .await
    .expect("load initial commit cursor position");
    let cursor = encode_observation_cursor(
        &store.pool,
        &metadata,
        &CursorPosition {
            store_commit_order: u64::try_from(
                row.try_get::<i64, _>("store_commit_order")
                    .expect("store commit order"),
            )
            .expect("positive store commit order"),
        },
    )
    .await
    .expect("encode row cursor");
    let watcher_store = store.clone();
    let watcher = tokio::spawn(async move {
        let started = Instant::now();
        let page = watcher_store
            .read_run_observations(RunObservationQuery::new(Some(cursor), 10, 5_000))
            .await
            .expect("watch observations");
        (page, started.elapsed())
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let artifact = artifact_id(143);
    let digest = content_digest(143);
    let evidence = store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let change_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        StreamSeq::new(2).expect("seq"),
        CommitKey::new("watch-notify-retention").expect("commit key"),
        vec![retention_refs_appended(
            run.clone(),
            artifact,
            digest,
            ArtifactRole::StateOutput,
        )],
        vec![evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("retention request");
    append_prepared(&store, change_request, vec![evidence])
        .await
        .expect("append observed change");
    let notify_pool = store.pool.clone();
    let notifier = tokio::spawn(async move {
        for _ in 0..20 {
            let _ = sqlx::query("SELECT pg_notify($1, $2)")
                .bind(OBSERVATION_NOTIFY_CHANNEL)
                .bind(OBSERVATION_NOTIFY_PAYLOAD)
                .execute(&notify_pool)
                .await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });

    let (page, elapsed) = watcher.await.expect("watch task joins");
    notifier.abort();
    let _ = notifier.await;
    assert!(
        elapsed < Duration::from_secs(2),
        "watch should wake from notification before long-poll timeout; elapsed={elapsed:?}"
    );
    assert_eq!(page.runs.len(), 1);
    assert_eq!(page.runs[0].run_id, run);
    assert_eq!(page.runs[0].head_seq.as_u64(), 2);

    drop_schema(&store, &schema).await;
}
