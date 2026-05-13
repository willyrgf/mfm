#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use mfm_integration_tests::parity_run_ids::read_parity_evm_run_id;
use mfm_machine::errors::StorageError;
use mfm_machine::events::{event_envelopes_from_stream_records, Event, KernelEvent, RunStatus};
use mfm_machine::ids::RunId;
use mfm_machine::stores::{StreamId, StreamStore};
use mfm_machine_test_support::init_test_observability;
use mfm_stream_store_postgres::PostgresStreamStore;
use tracing::info;

const EVM_REQUIRED_STATES: &[&str] = &[
    "evm_reth_pipeline.fetch.run",
    "evm_reth_pipeline.adapt.adapt",
    "evm_reth_pipeline.deploy.deploy",
    "evm_reth_pipeline.configure.configure",
    "evm_reth_pipeline.validate.validate",
];

async fn connect_postgres_with_retry(max_attempts: u32, delay_ms: u64) -> PostgresStreamStore {
    let mut last_err: Option<StorageError> = None;
    for _ in 0..max_attempts {
        match PostgresStreamStore::connect_env().await {
            Ok(pg) => return pg,
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "<missing>".to_string());
    panic!(
        "postgres config after retries (DATABASE_URL={}): {:?}",
        db_url, last_err
    );
}

fn assert_required_state_order(
    machine_id: &str,
    entered_state_ids: &[String],
    required_states: &[&str],
) {
    let mut cursor = 0usize;
    for required in required_states {
        let relative = entered_state_ids[cursor..]
            .iter()
            .position(|state| state == required)
            .unwrap_or_else(|| {
                panic!(
                    "missing required state `{required}` for `{machine_id}`; entered states={:?}",
                    entered_state_ids
                )
            });
        cursor += relative + 1;
    }
}

async fn audit_run_events(
    streams: Arc<dyn StreamStore>,
    run_id: RunId,
    machine_id: &str,
    required_states: &[&str],
    min_state_count: usize,
) {
    let head_seq = streams
        .head_seq(&StreamId::run(run_id))
        .await
        .expect("head seq");
    assert!(
        head_seq > 0,
        "run `{machine_id}` must emit at least one event"
    );

    let stream = streams
        .read_range(&StreamId::run(run_id), 1, None)
        .await
        .and_then(|records| event_envelopes_from_stream_records(run_id, records))
        .expect("event stream");
    assert_eq!(
        stream.len() as u64,
        head_seq,
        "stream length must equal head seq for `{machine_id}`"
    );
    assert_eq!(stream.first().map(|e| e.seq), Some(1));
    assert_eq!(stream.last().map(|e| e.seq), Some(head_seq));
    for (idx, envelope) in stream.iter().enumerate() {
        assert_eq!(
            envelope.seq,
            (idx + 1) as u64,
            "event seq continuity broken for `{machine_id}` at index {idx}"
        );
    }

    let mut entered_state_ids: Vec<String> = Vec::new();
    let mut active_states: HashSet<String> = HashSet::new();
    let mut entered_count = 0usize;
    let mut terminal_count = 0usize;
    let mut run_started_idx: Option<usize> = None;
    let mut run_completed_idx: Option<usize> = None;
    let mut run_completed_status: Option<RunStatus> = None;

    for (idx, envelope) in stream.iter().enumerate() {
        let Event::Kernel(kernel) = &envelope.event else {
            continue;
        };

        match kernel {
            KernelEvent::RunStarted { .. } => {
                assert!(
                    run_started_idx.is_none(),
                    "run `{machine_id}` emitted multiple RunStarted events"
                );
                run_started_idx = Some(idx);
            }
            KernelEvent::StateEntered { state_id, .. } => {
                let state = state_id.as_str().to_string();
                assert!(
                    state.starts_with(machine_id),
                    "run `{machine_id}` saw unexpected state id `{state}`"
                );
                assert!(
                    active_states.insert(state.clone()),
                    "state `{state}` re-entered before terminal event in `{machine_id}`"
                );
                entered_state_ids.push(state);
                entered_count += 1;
            }
            KernelEvent::StateCompleted { state_id, .. }
            | KernelEvent::StateFailed { state_id, .. } => {
                let state = state_id.as_str().to_string();
                assert!(
                    active_states.remove(&state),
                    "state `{state}` terminal event without matching entry in `{machine_id}`"
                );
                terminal_count += 1;
            }
            KernelEvent::RunCompleted { status, .. } => {
                assert!(
                    run_completed_idx.is_none(),
                    "run `{machine_id}` emitted multiple RunCompleted events"
                );
                run_completed_idx = Some(idx);
                run_completed_status = Some(status.clone());
            }
        }
    }

    let started = run_started_idx.expect("missing RunStarted");
    let completed = run_completed_idx.expect("missing RunCompleted");
    assert!(
        started < completed,
        "RunStarted must happen before RunCompleted for `{machine_id}`"
    );
    assert_eq!(
        run_completed_status,
        Some(RunStatus::Completed),
        "run `{machine_id}` must complete successfully"
    );
    assert_eq!(
        entered_count, terminal_count,
        "run `{machine_id}` has unmatched state entry/terminal counts"
    );
    assert!(
        active_states.is_empty(),
        "run `{machine_id}` left active states without terminal events: {:?}",
        active_states
    );
    assert!(
        entered_count >= min_state_count,
        "run `{machine_id}` must emit at least {min_state_count} StateEntered events"
    );

    assert_required_state_order(machine_id, &entered_state_ids, required_states);

    let report = serde_json::json!({
        "kind": "parity_postgres_state_events_audit_report_v2",
        "machine_id": machine_id,
        "run_id": run_id.0.to_string(),
        "head_seq": head_seq,
        "state_entered_count": entered_count,
        "state_terminal_count": terminal_count,
        "required_states": required_states,
    });
    info!(
        report = %serde_json::to_string(&report).expect("serialize report"),
        "postgres state events audit passed"
    );
}

#[tokio::test]
async fn parity_postgres_state_events_audit_for_multi_state_pipelines() {
    init_test_observability();

    let pg = connect_postgres_with_retry(20, 250).await;
    let streams: Arc<dyn StreamStore> = Arc::new(pg);

    let evm_run_id = read_parity_evm_run_id();

    audit_run_events(
        streams,
        evm_run_id,
        "evm_reth_pipeline",
        EVM_REQUIRED_STATES,
        5,
    )
    .await;
}
