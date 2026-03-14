#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! PostgreSQL `StreamStore` for parity tests and durable deployments.
//!
//! This backend persists append-only streams in PostgreSQL while preserving optimistic concurrency
//! and atomic multi-stream append semantics.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_event_store_postgres::PostgresEventStore;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), mfm_machine::errors::StorageError> {
//! let _store = PostgresEventStore::connect("postgres://postgres:postgres@localhost/mfm").await?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::stores::{AppendBatchResult, StreamAppend, StreamId, StreamRecord, StreamStore};
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls, Transaction};
use tracing::{debug, info, warn};

/// PostgreSQL-backed append-only event store.
#[derive(Clone)]
pub struct PostgresEventStore {
    client: Arc<Mutex<Client>>,
}

impl PostgresEventStore {
    /// Connects to PostgreSQL, initializes the schema, and returns a ready store.
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        info!("connecting postgres event store");
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
        info!("postgres event store connected");
        Ok(store)
    }

    /// Connects using the `DATABASE_URL` environment variable.
    pub async fn connect_env() -> Result<Self, StorageError> {
        let database_url = std::env::var("DATABASE_URL").map_err(|_| {
            StorageError::Other(Self::info("pg_missing_env", "missing DATABASE_URL"))
        })?;
        Self::connect(&database_url).await
    }

    async fn init(&self) -> Result<(), StorageError> {
        let ddl = r#"
CREATE TABLE IF NOT EXISTS mfm_streams (
  stream_id TEXT PRIMARY KEY,
  head_seq BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS mfm_stream_records (
  stream_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  ts_millis BIGINT NULL,
  kind TEXT NOT NULL,
  payload JSONB NOT NULL,
  PRIMARY KEY (stream_id, seq),
  CONSTRAINT mfm_stream_records_stream_fk FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);
"#;

        self.client
            .lock()
            .await
            .batch_execute(ddl)
            .await
            .map_err(|_| StorageError::Other(Self::info("pg_init_failed", "init failed")))?;
        debug!("postgres event store schema ensured");
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

    fn validate_append(append: &StreamAppend) -> Result<(), StorageError> {
        for record in &append.records {
            if record.kind.is_empty() {
                return Err(Self::other(
                    "pg_append_invalid",
                    "stream record kind must not be empty",
                ));
            }
        }
        Ok(())
    }

    fn validate_batch(appends: &[StreamAppend]) -> Result<(), StorageError> {
        let mut seen = std::collections::HashSet::new();
        for append in appends {
            Self::validate_append(append)?;
            if !seen.insert(append.stream_id.clone()) {
                return Err(Self::other(
                    "pg_append_invalid",
                    "append_batch contained duplicate stream ids",
                ));
            }
        }
        Ok(())
    }

    async fn read_head_for_update(
        tx: &Transaction<'_>,
        stream_id: &StreamId,
    ) -> Result<u64, StorageError> {
        let row = tx
            .query_one(
                "SELECT head_seq FROM mfm_streams WHERE stream_id = $1 FOR UPDATE",
                &[&stream_id.as_str()],
            )
            .await
            .map_err(|_| Self::other("pg_query_failed", "failed to read head_seq"))?;

        let head: i64 = row.get(0);
        Ok(head.max(0) as u64)
    }
}

#[async_trait]
impl StreamStore for PostgresEventStore {
    async fn head_seq(&self, stream_id: &StreamId) -> Result<u64, StorageError> {
        let row = self
            .client
            .lock()
            .await
            .query_opt(
                "SELECT head_seq FROM mfm_streams WHERE stream_id = $1",
                &[&stream_id.as_str()],
            )
            .await
            .map_err(|_| Self::other("pg_query_failed", "failed to query head_seq"))?;

        let Some(row) = row else {
            debug!(stream_id = %stream_id, head_seq = 0, "head_seq resolved");
            return Ok(0);
        };

        let head: i64 = row.get(0);
        debug!(stream_id = %stream_id, head_seq = head.max(0) as u64, "head_seq resolved");
        Ok(head.max(0) as u64)
    }

    async fn append(&self, append: StreamAppend) -> Result<u64, StorageError> {
        let result = self.append_batch(vec![append.clone()]).await?;
        result.head_for(&append.stream_id).ok_or_else(|| {
            Self::other(
                "pg_append_failed",
                "append_batch result did not contain the appended stream head",
            )
        })
    }

    async fn append_batch(
        &self,
        mut appends: Vec<StreamAppend>,
    ) -> Result<AppendBatchResult, StorageError> {
        Self::validate_batch(&appends)?;
        appends.sort_by(|left, right| left.stream_id.as_str().cmp(right.stream_id.as_str()));

        debug!(stream_count = appends.len(), "append_batch called");

        let mut client = self.client.lock().await;
        let tx = client
            .transaction()
            .await
            .map_err(|_| Self::other("pg_tx_failed", "failed to start transaction"))?;

        for append in &appends {
            tx.execute(
                "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, 0) ON CONFLICT (stream_id) DO NOTHING",
                &[&append.stream_id.as_str()],
            )
            .await
            .map_err(|_| Self::other("pg_insert_failed", "failed to insert stream"))?;
        }

        for append in &appends {
            let head = Self::read_head_for_update(&tx, &append.stream_id).await?;
            if head != append.expected_seq {
                warn!(
                    stream_id = %append.stream_id,
                    expected_seq = append.expected_seq,
                    actual_head = head,
                    "append_batch concurrency conflict"
                );
                return Err(Self::concurrency("head seq did not match expected seq"));
            }
        }

        let mut stream_heads = Vec::with_capacity(appends.len());
        for append in appends {
            let mut seq = append.expected_seq + 1;
            for record in append.records {
                tx.execute(
                    "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                    &[
                        &append.stream_id.as_str(),
                        &(seq as i64),
                        &record.ts_millis.map(|value| value as i64),
                        &record.kind,
                        &record.payload,
                    ],
                )
                .await
                .map_err(|_| Self::other("pg_insert_failed", "failed to insert stream record"))?;
                seq += 1;
            }

            let new_head = append.expected_seq + (seq - append.expected_seq - 1);
            tx.execute(
                "UPDATE mfm_streams SET head_seq = $2 WHERE stream_id = $1",
                &[&append.stream_id.as_str(), &(new_head as i64)],
            )
            .await
            .map_err(|_| Self::other("pg_update_failed", "failed to update head_seq"))?;
            stream_heads.push((append.stream_id, new_head));
        }

        tx.commit()
            .await
            .map_err(|_| Self::other("pg_tx_failed", "failed to commit transaction"))?;
        debug!(stream_count = stream_heads.len(), "append_batch committed");

        Ok(AppendBatchResult { stream_heads })
    }

    async fn read_range(
        &self,
        stream_id: &StreamId,
        from_seq: u64,
        to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, StorageError> {
        let client = self.client.lock().await;
        let from = from_seq.max(1) as i64;
        let rows = if let Some(to) = to_seq {
            client
                .query(
                    "SELECT seq, ts_millis, kind, payload FROM mfm_stream_records WHERE stream_id = $1 AND seq >= $2 AND seq <= $3 ORDER BY seq ASC",
                    &[&stream_id.as_str(), &from, &(to as i64)],
                )
                .await
        } else {
            client
                .query(
                    "SELECT seq, ts_millis, kind, payload FROM mfm_stream_records WHERE stream_id = $1 AND seq >= $2 ORDER BY seq ASC",
                    &[&stream_id.as_str(), &from],
                )
                .await
        }
        .map_err(|_| Self::other("pg_query_failed", "failed to read range"))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i64 = row.get(0);
            let ts_millis: Option<i64> = row.get(1);
            let kind: String = row.get(2);
            let payload: serde_json::Value = row.get(3);

            out.push(StreamRecord {
                stream_id: stream_id.clone(),
                seq: seq.max(0) as u64,
                ts_millis: ts_millis.map(|v| v.max(0) as u64),
                kind,
                payload,
            });
        }
        debug!(
            stream_id = %stream_id,
            from_seq,
            to_seq = ?to_seq,
            record_count = out.len(),
            "read_range completed"
        );

        Ok(out)
    }
}
