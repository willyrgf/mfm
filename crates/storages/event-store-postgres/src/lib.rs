//! PostgreSQL `EventStore` (parity lane).
//!
//! This backend is intended for parity/integration testing and real deployments.
//! Fast-lane tests should use `mfm-event-store-mem`.

use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::events::{Event, EventEnvelope};
use mfm_machine::ids::{ErrorCode, RunId};
use mfm_machine::stores::EventStore;
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls, Transaction};

#[derive(Clone)]
pub struct PostgresEventStore {
    client: Arc<Mutex<Client>>,
}

impl PostgresEventStore {
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let (client, connection) = tokio_postgres::connect(database_url, NoTls)
            .await
            .map_err(|_| StorageError::Other(Self::info("pg_connect_failed", "connect failed")))?;

        // Drive the connection in the background.
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let store = Self {
            client: Arc::new(Mutex::new(client)),
        };
        store.init().await?;
        Ok(store)
    }

    pub async fn connect_env() -> Result<Self, StorageError> {
        let database_url = std::env::var("DATABASE_URL").map_err(|_| {
            StorageError::Other(Self::info("pg_missing_env", "missing DATABASE_URL"))
        })?;
        Self::connect(&database_url).await
    }

    async fn init(&self) -> Result<(), StorageError> {
        // Minimal schema: per-run head + per-run append-only events.
        let ddl = r#"
CREATE TABLE IF NOT EXISTS mfm_runs (
  run_id UUID PRIMARY KEY,
  head_seq BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS mfm_events (
  run_id UUID NOT NULL,
  seq BIGINT NOT NULL,
  ts_millis BIGINT NULL,
  event JSONB NOT NULL,
  PRIMARY KEY (run_id, seq),
  CONSTRAINT mfm_events_run_fk FOREIGN KEY (run_id) REFERENCES mfm_runs(run_id) ON DELETE CASCADE
);
"#;

        self.client
            .lock()
            .await
            .batch_execute(ddl)
            .await
            .map_err(|_| StorageError::Other(Self::info("pg_init_failed", "init failed")))?;
        Ok(())
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

    fn other(code: &'static str, message: impl Into<String>) -> StorageError {
        StorageError::Other(Self::info(code, message))
    }

    fn validate_append(
        run_id: RunId,
        expected_seq: u64,
        events: &[EventEnvelope],
    ) -> Result<(), StorageError> {
        for (idx, e) in events.iter().enumerate() {
            if e.run_id != run_id {
                return Err(Self::other(
                    "pg_append_invalid",
                    "event run_id did not match append run_id",
                ));
            }
            let want_seq = expected_seq + (idx as u64) + 1;
            if e.seq != want_seq {
                return Err(Self::other(
                    "pg_append_invalid",
                    "event seq did not match expected contiguous sequence",
                ));
            }
        }
        Ok(())
    }

    async fn read_head_for_update(
        tx: &Transaction<'_>,
        run_id: RunId,
    ) -> Result<u64, StorageError> {
        let row = tx
            .query_one(
                "SELECT head_seq FROM mfm_runs WHERE run_id = $1 FOR UPDATE",
                &[&run_id.0],
            )
            .await
            .map_err(|_| Self::other("pg_query_failed", "failed to read head_seq"))?;

        let head: i64 = row.get(0);
        Ok(head.max(0) as u64)
    }
}

#[async_trait]
impl EventStore for PostgresEventStore {
    async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError> {
        let row = self
            .client
            .lock()
            .await
            .query_opt(
                "SELECT head_seq FROM mfm_runs WHERE run_id = $1",
                &[&run_id.0],
            )
            .await
            .map_err(|_| Self::other("pg_query_failed", "failed to query head_seq"))?;

        let Some(row) = row else {
            return Ok(0);
        };

        let head: i64 = row.get(0);
        Ok(head.max(0) as u64)
    }

    async fn append(
        &self,
        run_id: RunId,
        expected_seq: u64,
        events: Vec<EventEnvelope>,
    ) -> Result<u64, StorageError> {
        Self::validate_append(run_id, expected_seq, &events)?;

        let mut client = self.client.lock().await;

        let tx = client
            .transaction()
            .await
            .map_err(|_| Self::other("pg_tx_failed", "failed to start transaction"))?;

        tx.execute(
            "INSERT INTO mfm_runs (run_id, head_seq) VALUES ($1, 0) ON CONFLICT (run_id) DO NOTHING",
            &[&run_id.0],
        )
        .await
        .map_err(|_| Self::other("pg_insert_failed", "failed to insert run"))?;

        let head = Self::read_head_for_update(&tx, run_id).await?;
        if head != expected_seq {
            return Err(Self::concurrency("head seq did not match expected seq"));
        }

        for e in events.iter() {
            let event_json = serde_json::to_value(&e.event)
                .map_err(|_| Self::other("pg_serde_failed", "failed to serialize event"))?;

            tx.execute(
                "INSERT INTO mfm_events (run_id, seq, ts_millis, event) VALUES ($1, $2, $3, $4)",
                &[
                    &run_id.0,
                    &(e.seq as i64),
                    &e.ts_millis.map(|v| v as i64),
                    &event_json,
                ],
            )
            .await
            .map_err(|_| Self::other("pg_insert_failed", "failed to insert event"))?;
        }

        let new_head = expected_seq + (events.len() as u64);
        tx.execute(
            "UPDATE mfm_runs SET head_seq = $2 WHERE run_id = $1",
            &[&run_id.0, &(new_head as i64)],
        )
        .await
        .map_err(|_| Self::other("pg_update_failed", "failed to update head_seq"))?;

        tx.commit()
            .await
            .map_err(|_| Self::other("pg_tx_failed", "failed to commit transaction"))?;

        Ok(new_head)
    }

    async fn read_range(
        &self,
        run_id: RunId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        let client = self.client.lock().await;
        let from = from_seq.max(1) as i64;
        let rows = if let Some(to) = to_seq {
            client
                .query(
                    "SELECT seq, ts_millis, event FROM mfm_events WHERE run_id = $1 AND seq >= $2 AND seq <= $3 ORDER BY seq ASC",
                    &[&run_id.0, &from, &(to as i64)],
                )
                .await
        } else {
            client
                .query(
                    "SELECT seq, ts_millis, event FROM mfm_events WHERE run_id = $1 AND seq >= $2 ORDER BY seq ASC",
                    &[&run_id.0, &from],
                )
                .await
        }
        .map_err(|_| Self::other("pg_query_failed", "failed to read range"))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i64 = row.get(0);
            let ts_millis: Option<i64> = row.get(1);
            let event_json: serde_json::Value = row.get(2);

            let event: Event = serde_json::from_value(event_json)
                .map_err(|_| Self::other("pg_serde_failed", "failed to deserialize event"))?;

            out.push(EventEnvelope {
                run_id,
                seq: seq.max(0) as u64,
                ts_millis: ts_millis.map(|v| v.max(0) as u64),
                event,
            });
        }

        Ok(out)
    }
}
