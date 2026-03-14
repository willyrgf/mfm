#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! Shared contract tests for `mfm-machine` storage traits.
//!
//! This crate stays intentionally small so storage backend crates can reuse the same
//! conformance checks without creating dependency cycles back into the runtime.
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::stores::ArtifactStore;
//! use mfm_machine_test_support::{artifact_store_contract_tests, init_test_observability};
//!
//! async fn assert_artifact_store(store: &dyn ArtifactStore) {
//!     init_test_observability();
//!     artifact_store_contract_tests(store).await;
//! }
//! ```

use std::sync::Once;

use mfm_machine::errors::StorageError;
use mfm_machine::hashing::artifact_id_for_bytes;
use mfm_machine::ids::ArtifactId;
use mfm_machine::stores::{
    AppendBatchResult, ArtifactKind, ArtifactStore, NewStreamRecord, StreamAppend, StreamId,
    StreamRecord, StreamStore,
};

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

/// Initializes a process-wide tracing subscriber for integration and contract tests.
///
/// The filter resolution order matches the repository test contract:
/// `MFM_TEST_LOG_FILTER`, `MFM_LOG`, `LOG_LEVEL`, `RUST_LOG`, then `MFM_TEST_LOG`.
/// Repeated calls are harmless and only the first invocation installs the subscriber.
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

/// Runs the shared `ArtifactStore` contract suite against a backend.
///
/// The suite currently verifies:
/// - content-addressed writes
/// - round-trip reads
/// - missing-artifact behavior for `exists` and `get`
///
/// Backend crates typically call this from their own async integration tests after provisioning
/// a clean store instance for the test case.
pub async fn artifact_store_contract_tests(store: &dyn ArtifactStore) {
    put_get_roundtrip(store).await;
    content_addressed(store).await;
    exists_and_not_found(store).await;
}

/// Runs the shared `StreamStore` contract suite against a backend.
///
/// The suite currently verifies:
/// - append/read round trips
/// - optimistic concurrency via `expected_seq`
/// - atomic multi-stream batch append semantics
///
/// The supplied store should start from an isolated test database or namespace so the sequence and
/// concurrency assertions do not interact with events written by other tests.
pub async fn stream_store_contract_tests(store: &dyn StreamStore) {
    append_and_read(store).await;
    expected_seq_concurrency(store).await;
    append_batch_atomicity(store).await;
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

fn test_stream_id(family: &str) -> StreamId {
    StreamId::must_new(format!("{family}:{}", uuid::Uuid::new_v4()))
}

fn record(kind: &str, seq: u64) -> NewStreamRecord {
    NewStreamRecord {
        ts_millis: Some(seq),
        kind: kind.to_string(),
        payload: serde_json::json!({ "seq": seq }),
    }
}

fn assert_record(record: &StreamRecord, stream_id: &StreamId, seq: u64, kind: &str) {
    assert_eq!(&record.stream_id, stream_id);
    assert_eq!(record.seq, seq);
    assert_eq!(record.ts_millis, Some(seq));
    assert_eq!(record.kind, kind);
    assert_eq!(record.payload, serde_json::json!({ "seq": seq }));
}

async fn append_and_read(store: &dyn StreamStore) {
    let stream_id = test_stream_id("contract");

    assert_eq!(store.head_seq(&stream_id).await.expect("head_seq"), 0);

    let head = store
        .append(StreamAppend::new(
            stream_id.clone(),
            0,
            vec![record("test", 1)],
        ))
        .await
        .expect("append");
    assert_eq!(head, 1);
    assert_eq!(store.head_seq(&stream_id).await.expect("head_seq"), 1);

    let got = store
        .read_range(&stream_id, 1, None)
        .await
        .expect("read_range");
    assert_eq!(got.len(), 1);
    assert_record(&got[0], &stream_id, 1, "test");

    let head = store
        .append(StreamAppend::new(
            stream_id.clone(),
            1,
            vec![record("test", 2), record("test", 3)],
        ))
        .await
        .expect("append");
    assert_eq!(head, 3);

    let got = store
        .read_range(&stream_id, 2, Some(2))
        .await
        .expect("read_range");
    assert_eq!(got.len(), 1);
    assert_record(&got[0], &stream_id, 2, "test");
}

async fn expected_seq_concurrency(store: &dyn StreamStore) {
    let stream_id = test_stream_id("concurrency");

    let head = store
        .append(StreamAppend::new(
            stream_id.clone(),
            0,
            vec![record("test", 1)],
        ))
        .await
        .expect("append");
    assert_eq!(head, 1);

    let err = store
        .append(StreamAppend::new(
            stream_id.clone(),
            0,
            vec![record("test", 2)],
        ))
        .await
        .expect_err("append should fail");
    match err {
        StorageError::Concurrency(_) => {}
        other => panic!("expected Concurrency error, got: {other:?}"),
    }

    assert_eq!(store.head_seq(&stream_id).await.expect("head_seq"), 1);
}

async fn append_batch_atomicity(store: &dyn StreamStore) {
    let left = test_stream_id("batch_left");
    let right = test_stream_id("batch_right");

    let success = store
        .append_batch(vec![
            StreamAppend::new(left.clone(), 0, vec![record("left", 1)]),
            StreamAppend::new(
                right.clone(),
                0,
                vec![record("right", 1), record("right", 2)],
            ),
        ])
        .await
        .expect("append_batch");
    assert_batch_head(&success, &left, 1);
    assert_batch_head(&success, &right, 2);

    let err = store
        .append_batch(vec![
            StreamAppend::new(left.clone(), 0, vec![record("left", 2)]),
            StreamAppend::new(right.clone(), 2, vec![record("right", 3)]),
        ])
        .await
        .expect_err("batch should fail");
    match err {
        StorageError::Concurrency(_) => {}
        other => panic!("expected Concurrency error, got: {other:?}"),
    }

    assert_eq!(store.head_seq(&left).await.expect("head_seq"), 1);
    assert_eq!(store.head_seq(&right).await.expect("head_seq"), 2);

    let left_records = store.read_range(&left, 1, None).await.expect("read_range");
    assert_eq!(left_records.len(), 1);
    assert_record(&left_records[0], &left, 1, "left");

    let right_records = store.read_range(&right, 1, None).await.expect("read_range");
    assert_eq!(right_records.len(), 2);
    assert_record(&right_records[0], &right, 1, "right");
    assert_record(&right_records[1], &right, 2, "right");
}

fn assert_batch_head(result: &AppendBatchResult, stream_id: &StreamId, expected: u64) {
    assert_eq!(result.head_for(stream_id), Some(expected));
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
