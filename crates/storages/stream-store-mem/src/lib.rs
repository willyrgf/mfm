#![warn(missing_docs)]
//! In-memory `StreamStore` implementation for tests and local development.
//!
//! This backend keeps append-only streams in process memory and enforces the same
//! optimistic-concurrency and batch-atomicity contract as the durable stores.
//!
//! # Examples
//!
//! ```rust
//! use mfm_stream_store_mem::MemStreamStore;
//!
//! let _store = MemStreamStore::new();
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::stores::{
    validate_stream_append_records, AppendBatchResult, StreamAppend, StreamId, StreamRecord,
    StreamStore,
};
use tokio::sync::Mutex;

/// In-memory append-only stream store keyed by [`StreamId`].
#[derive(Clone, Default)]
pub struct MemStreamStore {
    inner: Arc<Mutex<HashMap<StreamId, Vec<StreamRecord>>>>,
}

impl MemStreamStore {
    /// Creates an empty in-memory stream store.
    pub fn new() -> Self {
        Self::default()
    }

    fn info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode::must_new(code),
            category: ErrorCategory::Storage,
            retryable: false,
            message: message.into(),
            details: None,
        }
    }

    fn concurrency(message: impl Into<String>) -> StorageError {
        StorageError::Concurrency(Self::info("stream_store_concurrency", message))
    }

    fn other(message: impl Into<String>) -> StorageError {
        StorageError::Other(Self::info("stream_store_mem", message))
    }

    fn validate_append(append: &StreamAppend) -> Result<(), StorageError> {
        validate_stream_append_records(append)
    }

    fn validate_batch(appends: &[StreamAppend]) -> Result<(), StorageError> {
        let mut seen = std::collections::HashSet::new();
        for append in appends {
            Self::validate_append(append)?;
            if !seen.insert(append.stream_id.clone()) {
                return Err(Self::other("append_batch contained duplicate stream ids"));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl StreamStore for MemStreamStore {
    async fn head_seq(&self, stream_id: &StreamId) -> Result<u64, StorageError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .get(stream_id)
            .and_then(|v| v.last())
            .map(|record| record.seq)
            .unwrap_or(0))
    }

    async fn append(&self, append: StreamAppend) -> Result<u64, StorageError> {
        let result = self.append_batch(vec![append.clone()]).await?;
        result.head_for(&append.stream_id).ok_or_else(|| {
            Self::other("append_batch result did not contain the appended stream head")
        })
    }

    async fn append_batch(
        &self,
        appends: Vec<StreamAppend>,
    ) -> Result<AppendBatchResult, StorageError> {
        Self::validate_batch(&appends)?;

        let mut inner = self.inner.lock().await;
        let mut stream_heads = Vec::with_capacity(appends.len());

        for append in &appends {
            let head = inner
                .get(&append.stream_id)
                .and_then(|records| records.last())
                .map(|record| record.seq)
                .unwrap_or(0);
            if head != append.expected_seq {
                return Err(Self::concurrency("head seq did not match expected seq"));
            }
        }

        for append in appends {
            let stream = inner.entry(append.stream_id.clone()).or_default();
            let mut next_seq = append.expected_seq + 1;
            for record in append.records {
                stream.push(StreamRecord {
                    stream_id: append.stream_id.clone(),
                    seq: next_seq,
                    ts_millis: record.ts_millis,
                    kind: record.kind,
                    payload: record.payload,
                });
                next_seq += 1;
            }
            let head = stream
                .last()
                .map(|record| record.seq)
                .unwrap_or(append.expected_seq);
            stream_heads.push((append.stream_id, head));
        }

        Ok(AppendBatchResult { stream_heads })
    }

    async fn read_range(
        &self,
        stream_id: &StreamId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, StorageError> {
        let inner = self.inner.lock().await;
        let Some(stream) = inner.get(stream_id) else {
            return Ok(Vec::new());
        };

        let from = from_seq.max(1);
        let to = to_seq.unwrap_or(u64::MAX);
        Ok(stream
            .iter()
            .filter(|e| e.seq >= from && e.seq <= to)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_machine::stores::NewStreamRecord;

    fn append(stream_id: StreamId, payload: serde_json::Value) -> StreamAppend {
        StreamAppend::new(
            stream_id,
            0,
            vec![NewStreamRecord {
                ts_millis: None,
                kind: "domain_event".to_string(),
                payload,
            }],
        )
    }

    #[tokio::test]
    async fn rejects_float_payloads_before_append() {
        let store = MemStreamStore::new();
        let stream_id = StreamId::must_new("test:float");
        let err = store
            .append(append(
                stream_id.clone(),
                serde_json::json!({ "value": 1.5 }),
            ))
            .await
            .expect_err("float payload must fail");
        assert!(matches!(err, StorageError::Other(_)));
        assert_eq!(store.head_seq(&stream_id).await.expect("head"), 0);
    }

    #[tokio::test]
    async fn rejects_secret_shaped_payloads_before_append() {
        let store = MemStreamStore::new();
        let stream_id = StreamId::must_new("test:secret");
        let err = store
            .append(append(
                stream_id.clone(),
                serde_json::json!({ "private_key": "do-not-persist" }),
            ))
            .await
            .expect_err("secret-shaped payload must fail");
        assert!(matches!(err, StorageError::Other(_)));
        assert_eq!(store.head_seq(&stream_id).await.expect("head"), 0);
    }
}
