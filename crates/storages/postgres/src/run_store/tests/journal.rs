use super::*;

fn bootstrap_observation(
    journal: &CommittedRunJournal,
) -> (
    RunId,
    Option<u64>,
    mfm_ids::ContentRef,
    Vec<u8>,
    mfm_ids::ContentRef,
    Vec<u8>,
) {
    let (spec_ref, spec_bytes) = journal.certified_spec_object();
    let (certificate_ref, certificate_bytes) = journal.certificate_object();
    (
        journal.run_id().clone(),
        journal.current_run_sequence(),
        spec_ref.clone(),
        spec_bytes.to_vec(),
        certificate_ref.clone(),
        certificate_bytes.to_vec(),
    )
}

#[tokio::test]
async fn memory_and_postgres_load_the_same_opaque_committed_journal() {
    let (postgres, schema) = test_store().await;
    let memory = mfm_store::v1::AsyncInMemoryRunStore::default();
    let run = run_id(246);
    let artifacts = vec![spec_artifact_ref(), certificate_artifact_ref()];

    let postgres_plan = test_prepared_commit_plan(
        run_start_request(run.clone(), "journal-parity"),
        artifacts.clone(),
    )
    .expect("postgres run-admission plan");
    postgres
        .append_prepared_commit_bundle(
            test_prepared_commit_bundle(postgres_plan).expect("postgres bundle"),
        )
        .await
        .expect("append postgres run admission");

    let memory_plan =
        test_prepared_commit_plan(run_start_request(run.clone(), "journal-parity"), artifacts)
            .expect("memory run-admission plan");
    memory
        .append_prepared_commit_bundle(
            test_prepared_commit_bundle(memory_plan).expect("memory bundle"),
        )
        .await
        .expect("append memory run admission");

    let postgres_journal = postgres
        .load_committed_journal(&run)
        .await
        .expect("postgres committed journal");
    let memory_journal = memory
        .load_committed_journal(&run)
        .await
        .expect("memory committed journal");

    assert_eq!(
        bootstrap_observation(&postgres_journal),
        bootstrap_observation(&memory_journal)
    );

    drop_schema(&postgres, &schema).await;
}

#[tokio::test]
async fn committed_journal_load_keeps_one_snapshot_across_an_interleaved_commit() {
    let (store, schema) = test_store().await;
    let run = run_id(247);
    append_prepared(
        &store,
        run_start_request(run.clone(), "journal-snapshot-run-start"),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("append run admission");

    let records_loaded = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let release_load = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let observed_head = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
    let loading_store = PostgresStore {
        pool: store.pool.clone(),
        authority: store.store_authority().clone(),
        committed_journal_load_test_barrier: Some(CommittedJournalLoadTestBarrier {
            run_id: run.clone(),
            records_loaded: std::sync::Arc::clone(&records_loaded),
            release_load: std::sync::Arc::clone(&release_load),
            observed_head: std::sync::Arc::clone(&observed_head),
        }),
    };
    let loading_run = run.clone();
    let load =
        tokio::spawn(async move { loading_store.load_committed_journal(&loading_run).await });

    tokio::time::timeout(Duration::from_secs(10), records_loaded.wait())
        .await
        .expect("journal load reached the records/object boundary");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "journal-snapshot-attempt-start",
            vec![state_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("commit interleaved after the journal record read");
    tokio::time::timeout(Duration::from_secs(10), release_load.wait())
        .await
        .expect("release fixed-snapshot journal load");

    let fixed_journal = tokio::time::timeout(Duration::from_secs(10), load)
        .await
        .expect("fixed-snapshot journal load completed")
        .expect("journal load task completed")
        .expect("fixed-snapshot committed journal");
    assert_eq!(fixed_journal.current_run_sequence(), Some(1));
    assert_eq!(
        observed_head.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the object-read transaction must retain the record-read snapshot"
    );

    let fresh_journal = store
        .load_committed_journal(&run)
        .await
        .expect("fresh committed journal");
    assert_eq!(fresh_journal.current_run_sequence(), Some(2));

    drop_schema(&store, &schema).await;
}
