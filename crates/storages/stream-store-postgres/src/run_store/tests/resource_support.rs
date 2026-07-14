use super::*;

pub(super) async fn append_resource_lane_attempt_start(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> Result<CommitOutcome> {
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            2,
            commit_key,
            vec![side_effect_attempt_started()],
        ),
        Vec::new(),
    )
    .await
}

pub(super) async fn append_resource_lane_prepare(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
    lane_value: &str,
    artifact_byte: u8,
) -> Result<CommitOutcome> {
    let intent_artifact = artifact_id(artifact_byte);
    let intent_digest = content_digest(artifact_byte);
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            3,
            commit_key,
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                resource_lane_claim_intent(resource_key(lane_value, 201)),
            ],
        ),
        vec![side_effect_artifact_ref(
            intent_artifact,
            intent_digest,
            schema_id("mfm.test.side_effect_intent", 70),
            ArtifactRole::SideEffectIntent,
        )],
    )
    .await
}

pub(super) async fn append_resource_lane_release(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
    lane_value: &str,
) -> Result<CommitOutcome> {
    let projection = store
        .status_projection_snapshot(run_id)
        .await
        .expect("resource lane projection");
    let lane_key = resource_lane_key(lane_value);
    let lane = projection
        .resource_lane(&lane_key)
        .expect("active resource lane");
    let next_seq = store
        .expected_next_seq(run_id)
        .await
        .expect("release next seq")
        .as_u64();
    let verify_start_key = format!("{commit_key}-verify-attempt-start");
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            next_seq,
            verify_start_key.as_str(),
            vec![side_effect_verify_attempt_started()],
        ),
        Vec::new(),
    )
    .await?;
    let next_seq = store
        .expected_next_seq(run_id)
        .await
        .expect("release next seq after verify start")
        .as_u64();
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            next_seq,
            commit_key,
            vec![
                KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
                    spec_hash: spec_hash(1),
                    ledger_key: side_effect_ledger_key(),
                    ledger_purpose: lane.ledger_purpose.clone(),
                    pair_id: lane.holder.pair_id.clone(),
                    pair_role: events::SideEffectPairRole::Verify,
                    invocation_epoch: lane.invocation_epoch,
                    claim_id: lane.claim_id.clone(),
                    release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
                    release_reason: events::ResourceLaneReleaseReason::new("mfm.test.release")
                        .expect("release reason"),
                }),
                side_effect_failed(),
                side_effect_attempt_failed(),
            ],
        ),
        Vec::new(),
    )
    .await
}

pub(super) fn assert_resource_lane_blocked(
    outcome: CommitOutcome,
    expected_lane_key: &ResourceLaneKey,
    expected_holder_run: &RunId,
) {
    let CommitOutcome::AdmissionBlocked(block) = outcome else {
        panic!("expected typed resource lane block, got {outcome:?}");
    };
    assert_eq!(&block.resource_lane_key, expected_lane_key);
    let holder = block.holder.as_ref().expect("blocked holder");
    assert_eq!(&holder.run_id, expected_holder_run);
    assert_eq!(holder.pair_id, side_effect_pair_id());
}

pub(super) fn assert_wait_fifo_admission_blocked(
    outcome: CommitOutcome,
    expected_lane_key: &ResourceLaneKey,
    expected_holder_run: Option<&RunId>,
) -> AdmissionWaiter {
    let CommitOutcome::AdmissionBlocked(block) = outcome else {
        panic!("expected typed resource lane block, got {outcome:?}");
    };
    assert_eq!(&block.resource_lane_key, expected_lane_key);
    match (block.holder.as_ref(), expected_holder_run) {
        (Some(holder), Some(expected_holder_run)) => {
            assert_eq!(&holder.run_id, expected_holder_run);
            assert_eq!(holder.pair_id, side_effect_pair_id());
        }
        (None, None) => {}
        (actual, expected) => {
            panic!("unexpected block holder {actual:?}, expected {expected:?}")
        }
    }
    block.waiter.expect("wait-fifo admission waiter block")
}

pub(super) fn assert_corruption(error: PostgresStoreError, expected: &str) {
    let PostgresStoreError::Corruption(message) = error else {
        panic!("expected corruption error, got {error:?}");
    };
    assert!(
        message.contains(expected),
        "expected corruption containing {expected:?}, got {message:?}"
    );
}

pub(super) fn assert_invalid_cursor(error: PostgresStoreError, expected: &str) {
    let PostgresStoreError::Store(StoreError::InvalidCursor { message }) = error else {
        panic!("expected invalid cursor error, got {error:?}");
    };
    assert!(
        message.contains(expected),
        "expected invalid cursor containing {expected:?}, got {message:?}"
    );
}

pub(super) fn assert_cursor_expired(error: PostgresStoreError) {
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::CursorExpired)
    ));
}

pub(super) async fn disable_observation_cursor_mutation_guard(pool: &PgPool) {
    sqlx::query(
        "ALTER TABLE run_observation_cursors DISABLE TRIGGER \
         run_observation_cursors_no_update",
    )
    .execute(pool)
    .await
    .expect("disable cursor mutation guard");
}

pub(super) async fn observation_row_cursor_for_commit(
    store: &PostgresRunStore,
    run: &RunId,
    seq: u64,
) -> String {
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let row = sqlx::query("SELECT store_commit_order FROM commits WHERE run_id = $1 AND seq = $2")
        .bind(run.as_str())
        .bind(i64::try_from(seq).expect("seq fits i64"))
        .fetch_one(&store.pool)
        .await
        .expect("load commit cursor position");
    encode_observation_cursor(
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
    .expect("encode row cursor")
}

pub(super) async fn append_retention_commit(
    store: &PostgresRunStore,
    run: &RunId,
    seq: u64,
    commit_key: &str,
    artifact_byte: u8,
) -> Result<CommitOutcome> {
    let artifact = artifact_id(artifact_byte);
    let digest = content_digest(artifact_byte);
    let evidence = store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(commit_key).expect("commit key"),
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
    append_prepared(store, request, vec![evidence]).await
}
