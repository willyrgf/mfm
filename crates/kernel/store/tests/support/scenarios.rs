use mfm_canonical::raw_content_digest;
use mfm_ids::{
    ContentRef, DigestAlgorithm, DigestBytes, ExecutionPosition, RunId, SchemaId, StatePosition,
    VisitId,
};
use mfm_journal::{
    DomainConclusion, DomainDecision, EncodedRunFrame, JournalHistory, JournalObject, StopCode,
};
use mfm_store::{AppendResult, Store};

pub fn run(byte: u8) -> RunId {
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

pub fn genesis(run: &RunId, context: &[u8]) -> EncodedRunFrame {
    let program = b"{}";
    EncodedRunFrame::admission(
        run,
        &reference("mfm.test.program", program),
        program,
        &reference("mfm.test.context", context),
        context,
    )
    .expect("genesis")
}

pub async fn exercise_store(store: &dyn Store, run: &RunId) {
    assert!(store.load_run(run).await.expect("absent").is_none());
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

    let retained = store.load_run(run).await.expect("load").expect("present");
    let mut history = JournalHistory::qualify(run, retained).expect("history");
    let output = br#"{"value":3}"#;
    let second = history
        .encode_pure_conclusion(
            ExecutionPosition {
                state: StatePosition::new(0).unwrap(),
                visit: VisitId::new(0),
            },
            DomainConclusion::Success {
                output: JournalObject::new(&reference("mfm.test.output", output), output).unwrap(),
            },
        )
        .expect("second");
    assert_eq!(
        store.append_run(&second).await.expect("second append"),
        AppendResult::Inserted
    );
    history.extend_inserted(second).expect("extend");
    assert_eq!(
        store.append_run(&first).await.expect("historical retry"),
        AppendResult::NotInserted
    );

    let left_bytes = br#"{"value":4}"#;
    let right_bytes = br#"{"value":5}"#;
    let left = history
        .encode_pure_conclusion(
            ExecutionPosition {
                state: StatePosition::new(1).unwrap(),
                visit: VisitId::new(1),
            },
            DomainConclusion::Success {
                output: JournalObject::new(&reference("mfm.test.output", left_bytes), left_bytes)
                    .unwrap(),
            },
        )
        .expect("left candidate");
    let right = history
        .encode_pure_conclusion(
            ExecutionPosition {
                state: StatePosition::new(1).unwrap(),
                visit: VisitId::new(1),
            },
            DomainConclusion::Failure {
                original: JournalObject::new(
                    &reference("mfm.test.failure", right_bytes),
                    right_bytes,
                )
                .unwrap(),
                decision: DomainDecision::Stop {
                    reason: StopCode::Nonrecoverable,
                    root: JournalObject::new(
                        &reference("mfm.test.failure", right_bytes),
                        right_bytes,
                    )
                    .unwrap(),
                },
            },
        )
        .expect("right candidate");
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
        .load_run(run)
        .await
        .expect("complete load")
        .expect("present");
    let history = JournalHistory::qualify(run, retained).expect("qualified");
    assert_eq!(history.head_sequence(), 3);
    assert!(
        history.head_digest() == left.head_digest() || history.head_digest() == right.head_digest()
    );
}
