use super::*;

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
