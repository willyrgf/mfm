//! In-memory `EventStore` (fast lane, service-free).
//!
//! This store provides per-run append-only event streams with optimistic concurrency.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::events::EventEnvelope;
use mfm_machine::ids::{ErrorCode, RunId};
use mfm_machine::stores::EventStore;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct MemEventStore {
    inner: Arc<Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
}

impl MemEventStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category: ErrorCategory::Storage,
            retryable: false,
            message: message.into(),
            details: None,
        }
    }

    fn concurrency(message: impl Into<String>) -> StorageError {
        StorageError::Concurrency(Self::info("event_store_concurrency", message))
    }

    fn other(message: impl Into<String>) -> StorageError {
        StorageError::Other(Self::info("event_store_mem", message))
    }

    fn validate_append(
        run_id: RunId,
        expected_seq: u64,
        events: &[EventEnvelope],
    ) -> Result<(), StorageError> {
        for (idx, e) in events.iter().enumerate() {
            if e.run_id != run_id {
                return Err(Self::other("event run_id did not match append run_id"));
            }
            let want_seq = expected_seq + (idx as u64) + 1;
            if e.seq != want_seq {
                return Err(Self::other(
                    "event seq did not match expected contiguous sequence",
                ));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl EventStore for MemEventStore {
    async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .get(&run_id)
            .and_then(|v| v.last())
            .map(|e| e.seq)
            .unwrap_or(0))
    }

    async fn append(
        &self,
        run_id: RunId,
        expected_seq: u64,
        events: Vec<EventEnvelope>,
    ) -> Result<u64, StorageError> {
        Self::validate_append(run_id, expected_seq, &events)?;

        let mut inner = self.inner.lock().await;
        let stream = inner.entry(run_id).or_default();

        let head = stream.last().map(|e| e.seq).unwrap_or(0);
        if head != expected_seq {
            return Err(Self::concurrency("head seq did not match expected seq"));
        }

        stream.extend(events);

        Ok(stream.last().map(|e| e.seq).unwrap_or(head))
    }

    async fn read_range(
        &self,
        run_id: RunId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        let inner = self.inner.lock().await;
        let Some(stream) = inner.get(&run_id) else {
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
