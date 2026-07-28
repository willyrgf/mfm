use super::*;

#[tokio::test]
async fn commit_key_sequence_and_projection_rebuild_contract() {
    let (store, schema) = test_store().await;
    let run = run_id(7);
    let artifact = artifact_id(8);
    let digest = content_digest(8);

    append_prepared(
        &store,
        run_start_request(run.clone(), "run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "attempt-start",
            vec![state_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt start");
    let terminal_request = request(
        run.clone(),
        3,
        "terminal",
        vec![
            cell_produced(artifact.clone(), digest.clone()),
            state_attempt_completed(),
        ],
    );
    let appended = append_prepared(
        &store,
        terminal_request.clone(),
        vec![store_artifact_ref(
            artifact.clone(),
            digest.clone(),
            ArtifactRole::StateOutput,
        )],
    )
    .await
    .expect("terminal commit");
    let CommitOutcome::Appended(appended_batch) = appended else {
        panic!("terminal commit should append");
    };
    let appended_fingerprint = appended_batch.fingerprint().clone();

    let stale_retry =
        terminal_request.with_expected_next_seq(StreamSeq::new(1).expect("stale seq"));
    let idempotent = append_prepared(
        &store,
        stale_retry,
        vec![store_artifact_ref(
            artifact,
            digest,
            ArtifactRole::StateOutput,
        )],
    )
    .await
    .expect("idempotent retry before stale seq");
    let CommitOutcome::Idempotent(idempotent_batch) = idempotent else {
        panic!("terminal retry should be idempotent");
    };
    assert_eq!(idempotent_batch.fingerprint(), &appended_fingerprint);
    assert_eq!(
        expected_next_sequence(&store, &run).await,
        StreamSeq::new(4).expect("seq")
    );

    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    assert!(matches!(
        before.cell_terminal(&cell_id(21)),
        Some(CellTerminalProjection::Produced { .. })
    ));
    let journal = store
        .load_committed_journal(&run)
        .await
        .expect("committed journal");
    assert_eq!(journal.current_run_sequence(), Some(3));

    assert_eq!(
        store
            .status_projection_snapshot(&run)
            .await
            .expect("stream-authoritative projection rebuilds from events"),
        before
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn interrupted_attempt_projection_rebuilds_from_events() {
    let (store, schema) = test_store().await;
    let run = run_id(17);

    append_prepared(
        &store,
        run_start_request(run.clone(), "run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "attempt-start",
            vec![state_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt start");
    append_prepared(
        &store,
        request(
            run.clone(),
            3,
            "attempt-interrupted",
            vec![state_attempt_interrupted()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt interrupted");

    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let attempt = before
        .attempt(&node_id(20), &attempt_id(23))
        .expect("attempt projection");
    assert!(matches!(&attempt.status, AttemptStatus::Interrupted));
    assert!(before.saga_engagement(&run).is_none());

    let rebuilt = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection rebuilds from events");
    assert_eq!(rebuilt, before);
    assert!(matches!(
        &rebuilt
            .attempt(&node_id(20), &attempt_id(23))
            .expect("stream-derived attempt projection")
            .status,
        AttemptStatus::Interrupted
    ));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn resource_lane_projection_rebuilds_from_events() {
    let (store, schema) = test_store().await;
    let run = run_id(21);
    let intent_artifact = artifact_id(22);
    let intent_digest = content_digest(22);
    let lane_key = resource_lane_key("wallet-1");

    append_prepared(
        &store,
        run_start_request(run.clone(), "resource-run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        certified_request(
            run.clone(),
            2,
            "resource-attempt-start",
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
            "resource-prepare",
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                resource_lane_claim_intent(resource_key("wallet-1", 201)),
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
    .expect("prepare with resource key");

    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let lane = before.resource_lane(&lane_key).expect("resource lane");
    assert_eq!(lane.holder.run_id, run);
    assert_eq!(lane.holder.pair_id, side_effect_pair_id());
    assert_eq!(lane.ledger_key, side_effect_ledger_key());

    let journal = store
        .load_committed_journal(&run)
        .await
        .expect("committed journal");
    assert_eq!(journal.current_run_sequence(), Some(3));

    let peer_run = run_id(24);
    append_prepared(
        &store,
        run_start_request(peer_run.clone(), "resource-peer-run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("peer run start");
    let peer_before = store
        .status_projection_snapshot(&peer_run)
        .await
        .expect("peer projection");
    let peer_lane = peer_before
        .resource_lane(&lane_key)
        .expect("peer snapshot includes cross-run lane");
    assert_eq!(&peer_lane.holder.run_id, &run);
    assert_eq!(peer_before.resource_lanes().count(), 1);

    let peer_after_reload = store
        .status_projection_snapshot(&peer_run)
        .await
        .expect("peer stream-authoritative projection");
    let peer_lane = peer_after_reload
        .resource_lane(&lane_key)
        .expect("peer snapshot includes cross-run lane from stream");
    assert_eq!(&peer_lane.holder.run_id, &run);
    assert_eq!(&peer_lane.holder.pair_id, &side_effect_pair_id());
    assert_eq!(&peer_lane.ledger_key, &side_effect_ledger_key());
    assert_eq!(peer_after_reload.run_state(&peer_run), RunState::Started);
    assert_eq!(
        store
            .status_projection_snapshot(&run)
            .await
            .expect("holder stream-authoritative projection"),
        before
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn resource_lane_append_admission_uses_stream_authority() {
    let (store, schema) = test_store().await;
    let holder_run = run_id(25);
    let contender = run_id(26);
    let lane_value = "wallet-admission";
    let lane_key = resource_lane_key(lane_value);

    append_run_start(&store, &holder_run, "admission-holder-run-start")
        .await
        .expect("holder run start");
    append_resource_lane_attempt_start(&store, &holder_run, "admission-holder-attempt-start")
        .await
        .expect("holder attempt start");
    append_resource_lane_prepare(
        &store,
        &holder_run,
        "admission-holder-prepare",
        lane_value,
        28,
    )
    .await
    .expect("holder resource lane prepare");
    let holder_projection = store
        .status_projection_snapshot(&holder_run)
        .await
        .expect("holder projection");
    assert_eq!(
        holder_projection
            .resource_lane(&lane_key)
            .expect("holder resource lane")
            .holder
            .run_id,
        holder_run
    );

    append_run_start(&store, &contender, "contender-run-start")
        .await
        .expect("contender run start");
    append_resource_lane_attempt_start(&store, &contender, "contender-attempt-start")
        .await
        .expect("contender attempt start");
    let sequence_before = store
        .load_committed_journal(&contender)
        .await
        .expect("contender journal before conflict")
        .current_run_sequence();
    let outcome =
        append_resource_lane_prepare(&store, &contender, "contender-prepare", lane_value, 30)
            .await
            .expect("resource lane stream authority blocks contender");
    assert_resource_lane_blocked(outcome, &lane_key, &holder_run);
    let sequence_after = store
        .load_committed_journal(&contender)
        .await
        .expect("contender journal after conflict")
        .current_run_sequence();
    assert_eq!(sequence_after, sequence_before);
    assert_eq!(
        expected_next_sequence(&store, &contender).await,
        StreamSeq::new(3).expect("contender prepare seq")
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn admission_waiters_enforce_single_lane_fifo_after_release() {
    let (store, schema) = test_store().await;
    let holder_run = run_id(32);
    let first_waiter = run_id(33);
    let second_waiter = run_id(34);
    let lane_value = "wallet-fifo-admission";
    let lane_key = resource_lane_key(lane_value);

    for (run, run_key, attempt_key) in [
        (
            &holder_run,
            "fifo-holder-run-start",
            "fifo-holder-attempt-start",
        ),
        (&first_waiter, "fifo-b-run-start", "fifo-b-attempt-start"),
        (&second_waiter, "fifo-c-run-start", "fifo-c-attempt-start"),
    ] {
        append_run_start(&store, run, run_key)
            .await
            .expect("run start");
        append_resource_lane_attempt_start(&store, run, attempt_key)
            .await
            .expect("attempt start");
    }

    append_resource_lane_prepare(&store, &holder_run, "fifo-holder-prepare", lane_value, 32)
        .await
        .expect("holder resource lane prepare");

    let first_waiter_block = assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &first_waiter, "fifo-b-prepare", lane_value, 33)
            .await
            .expect("first waiter blocked by holder"),
        &lane_key,
        Some(&holder_run),
    );
    let second_waiter_block = assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &second_waiter, "fifo-c-prepare", lane_value, 34)
            .await
            .expect("second waiter blocked by holder"),
        &lane_key,
        Some(&holder_run),
    );
    assert!(first_waiter_block.lane_ticket < second_waiter_block.lane_ticket);

    append_resource_lane_release(&store, &holder_run, "fifo-holder-release", lane_value)
        .await
        .expect("holder release");

    let second_retry_block = assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &second_waiter, "fifo-c-retry-1", lane_value, 34)
            .await
            .expect("second waiter cannot bypass first waiter"),
        &lane_key,
        None,
    );
    assert_eq!(second_retry_block.waiter_id, second_waiter_block.waiter_id);
    assert_eq!(
        second_retry_block.lane_ticket,
        second_waiter_block.lane_ticket
    );

    append_resource_lane_prepare(&store, &first_waiter, "fifo-b-retry-1", lane_value, 33)
        .await
        .expect("first waiter claims after holder release");
    assert_eq!(
        store
            .status_projection_snapshot(&first_waiter)
            .await
            .expect("first waiter projection")
            .resource_lane(&lane_key)
            .expect("first waiter lane")
            .holder
            .run_id,
        first_waiter
    );

    assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &second_waiter, "fifo-c-retry-2", lane_value, 34)
            .await
            .expect("second waiter blocked by first waiter holder"),
        &lane_key,
        Some(&first_waiter),
    );

    drop_schema(&store, &schema).await;
}
