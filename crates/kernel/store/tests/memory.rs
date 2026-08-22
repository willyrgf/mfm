use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId};
use mfm_journal::EncodedRunFrame;
use mfm_store::{AppendResult, MemoryStore, RunIndex, RunPageLimit, Store};

#[path = "support/scenarios.rs"]
mod scenarios;

fn run_with_byte(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
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
async fn memory_matches_the_shared_mechanical_store_scenarios() {
    let store = MemoryStore::new();
    scenarios::exercise_store(&store, &scenarios::run(8)).await;
}

#[test]
fn run_page_limits_are_bounded() {
    assert!(RunPageLimit::new(0).is_err());
    assert!(RunPageLimit::new(mfm_store::MAX_RUN_PAGE_ITEMS + 1).is_err());
}

#[tokio::test]
async fn memory_run_index_pages_only_mechanical_heads_in_run_id_order() {
    let store = MemoryStore::new();
    for byte in [5, 1, 3] {
        let run_id = run_with_byte(byte);
        let program = b"{}";
        let context = b"[]";
        let frame = EncodedRunFrame::admission(
            &run_id,
            &reference("mfm.test.program", program),
            program,
            &reference("mfm.test.context", context),
            context,
        )
        .expect("genesis");
        assert_eq!(
            store.append_run(&frame).await.expect("append"),
            AppendResult::Inserted
        );
    }

    let limit = RunPageLimit::new(2).expect("limit");
    let first = store.list_runs(None, limit).await.expect("first page");
    assert_eq!(
        first
            .items()
            .iter()
            .map(|summary| summary.run_id().clone())
            .collect::<Vec<_>>(),
        [run_with_byte(1), run_with_byte(3)]
    );
    for summary in first.items() {
        assert_eq!(summary.head_sequence(), 1);
        assert!(summary.total_bytes() > 0);
        assert_eq!(summary.head_digest().algorithm(), DigestAlgorithm::Sha256V1);
    }
    let second = store
        .list_runs(first.next_after(), limit)
        .await
        .expect("second page");
    assert_eq!(second.items().len(), 1);
    assert_eq!(second.items()[0].run_id(), &run_with_byte(5));
    assert!(second.next_after().is_none());
}
