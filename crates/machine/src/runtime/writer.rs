//! Event stream writer used by the runtime.

use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::debug;

use crate::errors::{RunError, StorageError};
use crate::events::{new_stream_record_for_event, Event, KernelEvent};
use crate::ids::RunId;
use crate::stores::{StreamAppend, StreamId, StreamStore};

pub(super) type SharedEventWriter = Arc<Mutex<EventWriter>>;

pub(super) struct EventWriter {
    run_id: RunId,
    stream_id: StreamId,
    store: Arc<dyn StreamStore>,
    next_seq: u64,
}

impl EventWriter {
    pub(super) async fn new(
        store: Arc<dyn StreamStore>,
        run_id: RunId,
    ) -> Result<Self, StorageError> {
        let stream_id = StreamId::run(run_id);
        let head = store.head_seq(&stream_id).await?;
        debug!(run_id = %run_id.0, head_seq = head, "initialized event writer");
        Ok(Self {
            run_id,
            stream_id,
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
        let mut records = Vec::with_capacity(events.len());
        for event in events {
            records.push(new_stream_record_for_event(event, None)?);
        }

        let head = self
            .store
            .append(StreamAppend::new(
                self.stream_id.clone(),
                expected_seq,
                records,
            ))
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
