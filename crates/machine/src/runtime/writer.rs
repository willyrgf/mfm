//! Event stream writer used by the runtime.

use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::debug;

use crate::errors::{RunError, StorageError};
use crate::events::{Event, EventEnvelope, KernelEvent};
use crate::ids::RunId;
use crate::stores::EventStore;

pub(super) type SharedEventWriter = Arc<Mutex<EventWriter>>;

pub(super) struct EventWriter {
    run_id: RunId,
    store: Arc<dyn EventStore>,
    next_seq: u64,
}

impl EventWriter {
    pub(super) async fn new(
        store: Arc<dyn EventStore>,
        run_id: RunId,
    ) -> Result<Self, StorageError> {
        let head = store.head_seq(run_id).await?;
        debug!(run_id = %run_id.0, head_seq = head, "initialized event writer");
        Ok(Self {
            run_id,
            store,
            next_seq: head + 1,
        })
    }

    pub(super) async fn append(&mut self, events: Vec<Event>) -> Result<u64, StorageError> {
        if events.is_empty() {
            return Ok(self.next_seq.saturating_sub(1));
        }

        let expected_seq = self.next_seq.saturating_sub(1);
        debug!(
            run_id = %self.run_id.0,
            expected_seq,
            event_count = events.len(),
            "appending events"
        );
        let mut envelopes = Vec::with_capacity(events.len());
        for (idx, event) in events.into_iter().enumerate() {
            envelopes.push(EventEnvelope {
                run_id: self.run_id,
                seq: expected_seq + (idx as u64) + 1,
                ts_millis: None,
                event,
            });
        }

        let head = self
            .store
            .append(self.run_id, expected_seq, envelopes)
            .await?;
        debug!(run_id = %self.run_id.0, new_head = head, "append completed");
        self.next_seq = head + 1;
        Ok(head)
    }

    pub(super) async fn append_kernel(&mut self, event: KernelEvent) -> Result<u64, StorageError> {
        self.append(vec![Event::Kernel(event)]).await
    }
}

pub(super) async fn append_kernel(
    writer: &SharedEventWriter,
    event: KernelEvent,
) -> Result<(), RunError> {
    writer
        .lock()
        .await
        .append_kernel(event)
        .await
        .map_err(RunError::Storage)?;
    Ok(())
}
