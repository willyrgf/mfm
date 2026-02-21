//! Test harnesses for `mfm-machine` trait contracts.
//!
//! This crate is intentionally tiny and depends only on `mfm-machine` so storage backends can
//! share correctness tests without creating dependency cycles.

use std::sync::Once;

use mfm_machine::errors::StorageError;
use mfm_machine::events::{Event, EventEnvelope, KernelEvent, RunStatus};
use mfm_machine::hashing::artifact_id_for_bytes;
use mfm_machine::ids::{ArtifactId, OpId, RunId, StateId};
use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};

const TEST_FILTER_DEFAULT: &str = "warn,mfm=debug";
const TEST_FILTER_VERBOSE: &str = "debug,mfm=trace";

fn resolve_test_filter<F>(mut lookup: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(filter) = lookup("MFM_TEST_LOG_FILTER") {
        return filter;
    }
    if let Some(filter) = lookup("MFM_LOG") {
        return filter;
    }
    if let Some(filter) = lookup("LOG_LEVEL") {
        return filter;
    }
    if let Some(filter) = lookup("RUST_LOG") {
        return filter;
    }

    if lookup("MFM_TEST_LOG").is_some() {
        TEST_FILTER_VERBOSE.to_string()
    } else {
        TEST_FILTER_DEFAULT.to_string()
    }
}

pub fn init_test_observability() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let filter = resolve_test_filter(|name| std::env::var(name).ok());

        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_new(filter)
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(TEST_FILTER_DEFAULT)),
            )
            .with_test_writer()
            .with_target(true)
            .try_init();
    });
}

pub async fn artifact_store_contract_tests(store: &dyn ArtifactStore) {
    put_get_roundtrip(store).await;
    content_addressed(store).await;
    exists_and_not_found(store).await;
}

pub async fn event_store_contract_tests(store: &dyn EventStore) {
    append_and_read(store).await;
    expected_seq_concurrency(store).await;
}

async fn put_get_roundtrip(store: &dyn ArtifactStore) {
    let bytes = b"hello artifact".to_vec();
    let id = store
        .put(ArtifactKind::Other("test".to_string()), bytes.clone())
        .await
        .expect("put must succeed");

    assert_eq!(id, artifact_id_for_bytes(&bytes));

    let got = store.get(&id).await.expect("get must succeed");
    assert_eq!(got, bytes);
}

async fn content_addressed(store: &dyn ArtifactStore) {
    let bytes = b"same bytes".to_vec();

    let id1 = store
        .put(ArtifactKind::Other("k1".to_string()), bytes.clone())
        .await
        .expect("put must succeed");

    let id2 = store
        .put(ArtifactKind::Other("k2".to_string()), bytes.clone())
        .await
        .expect("put must succeed");

    assert_eq!(id1, id2);
    assert_eq!(id1, artifact_id_for_bytes(&bytes));
}

async fn exists_and_not_found(store: &dyn ArtifactStore) {
    let missing = ArtifactId("0".repeat(64));
    assert!(!store.exists(&missing).await.expect("exists must succeed"));

    match store.get(&missing).await {
        Err(StorageError::NotFound(_)) => {}
        other => panic!("expected NotFound for missing artifact, got: {other:?}"),
    }
}

async fn append_and_read(store: &dyn EventStore) {
    let run_id = RunId(uuid::Uuid::new_v4());

    assert_eq!(store.head_seq(run_id).await.expect("head_seq"), 0);

    let e1 = EventEnvelope {
        run_id,
        seq: 1,
        ts_millis: Some(1),
        event: Event::Kernel(KernelEvent::RunStarted {
            op_id: OpId::must_new("op".to_string()),
            manifest_id: ArtifactId("0".repeat(64)),
            initial_snapshot_id: ArtifactId("1".repeat(64)),
        }),
    };

    let head = store
        .append(run_id, 0, vec![e1.clone()])
        .await
        .expect("append");
    assert_eq!(head, 1);
    assert_eq!(store.head_seq(run_id).await.expect("head_seq"), 1);

    let got = store.read_range(run_id, 1, None).await.expect("read_range");
    assert_eq!(got, vec![e1.clone()]);

    let e2 = EventEnvelope {
        run_id,
        seq: 2,
        ts_millis: Some(2),
        event: Event::Kernel(KernelEvent::StateEntered {
            state_id: StateId::must_new("machine.main.setup".to_string()),
            attempt: 0,
            base_snapshot_id: ArtifactId("2".repeat(64)),
        }),
    };
    let e3 = EventEnvelope {
        run_id,
        seq: 3,
        ts_millis: Some(3),
        event: Event::Kernel(KernelEvent::RunCompleted {
            status: RunStatus::Completed,
            final_snapshot_id: None,
        }),
    };

    let head = store
        .append(run_id, 1, vec![e2.clone(), e3.clone()])
        .await
        .expect("append");
    assert_eq!(head, 3);

    let got = store
        .read_range(run_id, 2, Some(2))
        .await
        .expect("read_range");
    assert_eq!(got, vec![e2]);
}

async fn expected_seq_concurrency(store: &dyn EventStore) {
    let run_id = RunId(uuid::Uuid::new_v4());

    let e1 = EventEnvelope {
        run_id,
        seq: 1,
        ts_millis: None,
        event: Event::Kernel(KernelEvent::RunStarted {
            op_id: OpId::must_new("op".to_string()),
            manifest_id: ArtifactId("0".repeat(64)),
            initial_snapshot_id: ArtifactId("1".repeat(64)),
        }),
    };

    let head = store
        .append(run_id, 0, vec![e1.clone()])
        .await
        .expect("append");
    assert_eq!(head, 1);

    // ExpectedSeq concurrency: appending with an old expected seq must fail and must not change head.
    let err = store
        .append(run_id, 0, vec![e1])
        .await
        .expect_err("append should fail");
    match err {
        StorageError::Concurrency(_) => {}
        other => panic!("expected Concurrency error, got: {other:?}"),
    }

    assert_eq!(store.head_seq(run_id).await.expect("head_seq"), 1);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{resolve_test_filter, TEST_FILTER_DEFAULT, TEST_FILTER_VERBOSE};

    fn lookup_from(entries: &[(&str, &str)]) -> impl FnMut(&str) -> Option<String> {
        let vars: HashMap<String, String> = entries
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn test_filter_prefers_test_specific_override() {
        let filter = resolve_test_filter(lookup_from(&[
            ("MFM_TEST_LOG_FILTER", "trace"),
            ("MFM_LOG", "warn"),
            ("LOG_LEVEL", "info"),
        ]));
        assert_eq!(filter, "trace");
    }

    #[test]
    fn test_filter_prefers_component_override_before_global_level() {
        let filter = resolve_test_filter(lookup_from(&[
            ("MFM_LOG", "debug,mfm=trace"),
            ("LOG_LEVEL", "info"),
            ("RUST_LOG", "warn"),
        ]));
        assert_eq!(filter, "debug,mfm=trace");
    }

    #[test]
    fn test_filter_uses_global_level_before_rust_log() {
        let filter =
            resolve_test_filter(lookup_from(&[("LOG_LEVEL", "debug"), ("RUST_LOG", "warn")]));
        assert_eq!(filter, "debug");
    }

    #[test]
    fn test_filter_uses_verbose_default_when_enabled() {
        let filter = resolve_test_filter(lookup_from(&[("MFM_TEST_LOG", "1")]));
        assert_eq!(filter, TEST_FILTER_VERBOSE);
    }

    #[test]
    fn test_filter_uses_quiet_default_when_not_enabled() {
        let filter = resolve_test_filter(lookup_from(&[]));
        assert_eq!(filter, TEST_FILTER_DEFAULT);
    }
}
