use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId};
use mfm_journal::{
    EncodedRunFrame, JournalHistory, OutcomeKind, MAX_FRAME_BYTES, MAX_RUN_BYTES, MAX_RUN_FRAMES,
};
use mfm_store::{AppendResult, MemoryStore, Store};

#[path = "support/scenarios.rs"]
mod scenarios;

fn run() -> RunId {
    RunId::from_digest(DigestBytes::from_array([7; 32]))
}

fn reference(name: &str, bytes: &[u8]) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema"),
        raw_content_digest(bytes),
    )
    .expect("reference")
}

#[tokio::test]
async fn memory_store_is_atomic_idempotent_and_loads_complete_prefix() {
    let store = MemoryStore::new();
    let run = run();
    assert!(store.load_run(&run).await.expect("load").is_none());

    let program = b"{}";
    let context = br#"{"value":1}"#;
    let genesis = EncodedRunFrame::admission(
        &run,
        &reference("mfm.test.program", program),
        program,
        &reference("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    assert_eq!(
        store.append_run(&genesis).await.expect("insert"),
        AppendResult::Inserted
    );
    assert_eq!(
        store.append_run(&genesis).await.expect("retry"),
        AppendResult::NotInserted
    );

    let retained = store.load_run(&run).await.expect("load").expect("present");
    let mut history = JournalHistory::qualify(&run, retained).expect("history");
    assert_eq!(history.head_sequence(), 1);

    let outcome = br#"{"value":2}"#;
    let successor = history
        .encode_pure_conclusion(
            OutcomeKind::Success,
            &reference("mfm.test.output", outcome),
            outcome,
        )
        .expect("successor");
    assert_eq!(
        store.append_run(&successor).await.expect("append"),
        AppendResult::Inserted
    );
    history.extend_inserted(successor).expect("local extend");

    let conflicting = history
        .encode_pure_conclusion(
            OutcomeKind::Failure,
            &reference("mfm.test.failure", b"null"),
            b"null",
        )
        .expect("later candidate");
    assert_eq!(
        store.append_run(&conflicting).await.expect("append later"),
        AppendResult::Inserted
    );

    assert_eq!(
        store.append_run(&genesis).await.expect("historical retry"),
        AppendResult::NotInserted
    );
    let retained = store.load_run(&run).await.expect("load").expect("present");
    let history = JournalHistory::qualify(&run, retained).expect("history");
    assert_eq!(history.head_sequence(), 3);
    assert_eq!(history.head_digest(), conflicting.head_digest());
}

#[tokio::test]
async fn memory_matches_the_shared_mechanical_store_scenarios() {
    let store = MemoryStore::new();
    scenarios::exercise_store(&store, &scenarios::run(8)).await;
}

#[test]
fn store_capacity_constants_are_the_frozen_format_bounds() {
    assert_eq!(MAX_FRAME_BYTES, 3 * 8_388_608 + 65_536);
    assert_eq!(MAX_RUN_FRAMES, 65_536);
    assert_eq!(MAX_RUN_BYTES, 512 * 1024 * 1024);
}
