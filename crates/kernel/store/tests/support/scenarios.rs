use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{DigestBytes, RunId};
use mfm_journal::{seal_frame, EncodedRunFrame};
use mfm_store::{AppendResult, Store};
use std::sync::Arc;

pub fn run(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

pub fn genesis(run: &RunId, context: &[u8]) -> EncodedRunFrame {
    let payload = PlainCanonicalJsonBytes::from_canonical_json_slice(context).expect("payload");
    seal_frame(run, 1, None, &payload).expect("genesis")
}

fn successor(previous: &EncodedRunFrame, value: u8) -> EncodedRunFrame {
    let payload =
        PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":{value}}}"#)).unwrap();
    seal_frame(
        previous.run_id(),
        previous.run_sequence() + 1,
        Some(previous.head_digest()),
        &payload,
    )
    .unwrap()
}

pub async fn exercise_store(store: &dyn Store, run: &RunId) {
    assert!(store.load_run(run, None).await.expect("absent").is_none());
    let first = genesis(run, br#"{"value":1}"#);
    let competing = genesis(run, br#"{"value":2}"#);
    assert_eq!(
        store.append_run(&first).await.expect("insert"),
        AppendResult::Inserted
    );
    assert_eq!(
        store.append_run(&first).await.expect("exact retry"),
        AppendResult::NotInserted
    );
    assert_eq!(
        store.append_run(&competing).await.expect("competing"),
        AppendResult::NotInserted
    );
    let admission = store.load_run(run, Some(1)).await.unwrap().unwrap();
    assert!(Arc::ptr_eq(admission.admission(), admission.latest()));
    assert!(Arc::ptr_eq(
        admission.admission(),
        admission.probe().unwrap()
    ));
    assert_eq!(admission.admission().as_ref(), first.canonical_bytes());

    let second = successor(&first, 3);
    assert_eq!(
        store.append_run(&second).await.expect("second append"),
        AppendResult::Inserted
    );
    assert_eq!(
        store.append_run(&first).await.expect("historical retry"),
        AppendResult::NotInserted
    );
    let left = successor(&second, 4);
    let right = successor(&second, 5);
    let (left_result, right_result) =
        tokio::join!(store.append_run(&left), store.append_run(&right));
    let outcomes = [
        left_result.expect("left append"),
        right_result.expect("right append"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == AppendResult::Inserted)
            .count(),
        1,
        "exactly one same-predecessor candidate wins"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == AppendResult::NotInserted)
            .count(),
        1
    );

    let retained = store
        .load_run(run, Some(2))
        .await
        .expect("bounded load")
        .expect("present");
    assert_eq!(retained.head().head_sequence(), 3);
    let winner = if outcomes[0] == AppendResult::Inserted {
        &left
    } else {
        &right
    };
    assert_eq!(retained.head().head_digest(), winner.head_digest());
    assert_eq!(retained.latest().as_ref(), winner.canonical_bytes());
    assert_eq!(retained.admission().as_ref(), first.canonical_bytes());
    assert_eq!(retained.probe().unwrap().as_ref(), second.canonical_bytes());
    assert_eq!(
        retained.head().total_bytes(),
        (first.canonical_bytes().len()
            + second.canonical_bytes().len()
            + winner.canonical_bytes().len()) as u64
    );
    let future = store.load_run(run, Some(4)).await.unwrap().unwrap();
    assert!(future.probe().is_none());
    assert_eq!(future.head().head_sequence(), 3);
    assert!(store.load_run(run, Some(0)).await.is_err());
    assert!(store
        .load_run(run, Some(mfm_journal::MAX_RUN_FRAMES + 1))
        .await
        .is_err());
}
