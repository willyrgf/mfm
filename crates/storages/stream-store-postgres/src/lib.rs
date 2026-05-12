#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! PostgreSQL `StreamStore` for parity tests and durable deployments.
//!
//! This backend persists append-only streams in PostgreSQL while preserving optimistic concurrency
//! and atomic multi-stream append semantics.
//!
//! PostgreSQL stores stream sequence and timestamp fields in `BIGINT` columns. This backend
//! therefore supports values up to `i64::MAX`; larger `u64` values are rejected before write, and
//! negative persisted values are treated as store corruption on read.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_stream_store_postgres::PostgresStreamStore;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), mfm_machine::errors::StorageError> {
//! let _store = PostgresStreamStore::connect("postgres://postgres:postgres@localhost/mfm").await?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::stores::{
    validate_stream_append_records, AppendBatchResult, StreamAppend, StreamId, StreamRecord,
    StreamStore,
};
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls, Transaction};
use tracing::{debug, info, warn};

const POSTGRES_BIGINT_MAX_U64: u64 = i64::MAX as u64;

/// PostgreSQL-backed append-only stream store.
#[derive(Clone)]
pub struct PostgresStreamStore {
    client: Arc<Mutex<Client>>,
}

impl PostgresStreamStore {
    /// Connects to PostgreSQL, initializes the schema, and returns a ready store.
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        info!("connecting postgres stream store");
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
        info!("postgres stream store connected");
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
  head_seq BIGINT NOT NULL,
  CONSTRAINT mfm_streams_head_seq_nonnegative CHECK (head_seq >= 0)
);

CREATE TABLE IF NOT EXISTS mfm_stream_records (
  stream_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  ts_millis BIGINT NULL,
  kind TEXT NOT NULL,
  payload JSONB NOT NULL,
  PRIMARY KEY (stream_id, seq),
  CONSTRAINT mfm_stream_records_seq_positive CHECK (seq >= 1),
  CONSTRAINT mfm_stream_records_ts_millis_nonnegative CHECK (ts_millis IS NULL OR ts_millis >= 0),
  CONSTRAINT mfm_stream_records_stream_fk FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);

BEGIN;
ALTER TABLE mfm_streams DROP CONSTRAINT IF EXISTS mfm_streams_head_seq_nonnegative;
ALTER TABLE mfm_streams
  ADD CONSTRAINT mfm_streams_head_seq_nonnegative CHECK (head_seq >= 0);
ALTER TABLE mfm_stream_records DROP CONSTRAINT IF EXISTS mfm_stream_records_seq_positive;
ALTER TABLE mfm_stream_records
  ADD CONSTRAINT mfm_stream_records_seq_positive CHECK (seq >= 1);
ALTER TABLE mfm_stream_records DROP CONSTRAINT IF EXISTS mfm_stream_records_ts_millis_nonnegative;
ALTER TABLE mfm_stream_records
  ADD CONSTRAINT mfm_stream_records_ts_millis_nonnegative CHECK (ts_millis IS NULL OR ts_millis >= 0);
COMMIT;
"#;

        self.client
            .lock()
            .await
            .batch_execute(ddl)
            .await
            .map_err(|_| StorageError::Other(Self::info("pg_init_failed", "init failed")))?;
        debug!("postgres stream store schema ensured");
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
        StorageError::Concurrency(Self::info("stream_store_concurrency", message))
    }

    fn other(code: &'static str, message: impl Into<String>) -> StorageError {
        StorageError::Other(Self::info(code, message))
    }

    fn corruption(code: &'static str, message: impl Into<String>) -> StorageError {
        StorageError::Corruption(Self::info(code, message))
    }

    fn u64_to_i64(value: u64, field: &'static str) -> Result<i64, StorageError> {
        i64::try_from(value).map_err(|_| {
            Self::other(
                "pg_value_out_of_range",
                format!("{field} exceeded the supported PostgreSQL bigint range"),
            )
        })
    }

    fn i64_to_nonnegative_u64(value: i64, field: &'static str) -> Result<u64, StorageError> {
        u64::try_from(value).map_err(|_| {
            Self::corruption(
                "pg_stream_corrupt",
                format!("{field} contained a negative bigint value"),
            )
        })
    }

    fn i64_to_positive_u64(value: i64, field: &'static str) -> Result<u64, StorageError> {
        let value = Self::i64_to_nonnegative_u64(value, field)?;
        if value == 0 {
            return Err(Self::corruption(
                "pg_stream_corrupt",
                format!("{field} contained a non-positive sequence value"),
            ));
        }
        Ok(value)
    }

    fn opt_i64_to_nonnegative_u64(
        value: Option<i64>,
        field: &'static str,
    ) -> Result<Option<u64>, StorageError> {
        value
            .map(|value| Self::i64_to_nonnegative_u64(value, field))
            .transpose()
    }

    fn checked_next_seq(seq: u64) -> Result<u64, StorageError> {
        seq.checked_add(1)
            .filter(|value| *value <= POSTGRES_BIGINT_MAX_U64)
            .ok_or_else(|| {
                Self::other(
                    "pg_value_out_of_range",
                    "next stream sequence exceeded the supported PostgreSQL bigint range",
                )
            })
    }

    fn validate_append(append: &StreamAppend) -> Result<(), StorageError> {
        Self::u64_to_i64(append.expected_seq, "expected_seq")?;
        validate_stream_append_records(append)
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
        Self::i64_to_nonnegative_u64(head, "mfm_streams.head_seq")
    }

    async fn validate_stream_record_integrity(
        client: &Client,
        stream_id: &StreamId,
    ) -> Result<(), StorageError> {
        let corrupt = client
            .query_opt(
                "SELECT seq, ts_millis FROM mfm_stream_records WHERE stream_id = $1 AND (seq < 1 OR ts_millis < 0) ORDER BY seq ASC LIMIT 1",
                &[&stream_id.as_str()],
            )
            .await
            .map_err(|_| Self::other("pg_query_failed", "failed to validate stream records"))?;

        let Some(row) = corrupt else {
            return Ok(());
        };

        let seq: i64 = row.get(0);
        if seq < 1 {
            return Err(Self::corruption(
                "pg_stream_corrupt",
                "mfm_stream_records.seq contained a non-positive sequence value",
            ));
        }

        let ts_millis: Option<i64> = row.get(1);
        if ts_millis.is_some_and(|value| value < 0) {
            return Err(Self::corruption(
                "pg_stream_corrupt",
                "mfm_stream_records.ts_millis contained a negative bigint value",
            ));
        }

        Err(Self::corruption(
            "pg_stream_corrupt",
            "mfm_stream_records contained invalid sequence metadata",
        ))
    }
}

#[async_trait]
impl StreamStore for PostgresStreamStore {
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
        let head_seq = Self::i64_to_nonnegative_u64(head, "mfm_streams.head_seq")?;
        debug!(stream_id = %stream_id, head_seq, "head_seq resolved");
        Ok(head_seq)
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
            let mut seq = append.expected_seq;
            for record in append.records {
                seq = Self::checked_next_seq(seq)?;
                tx.execute(
                    "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                    &[
                        &append.stream_id.as_str(),
                        &Self::u64_to_i64(seq, "mfm_stream_records.seq")?,
                        &record
                            .ts_millis
                            .map(|value| Self::u64_to_i64(value, "mfm_stream_records.ts_millis"))
                            .transpose()?,
                        &record.kind,
                        &record.payload,
                    ],
                )
                .await
                .map_err(|_| Self::other("pg_insert_failed", "failed to insert stream record"))?;
            }

            let new_head = seq;
            tx.execute(
                "UPDATE mfm_streams SET head_seq = $2 WHERE stream_id = $1",
                &[
                    &append.stream_id.as_str(),
                    &Self::u64_to_i64(new_head, "mfm_streams.head_seq")?,
                ],
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
        Self::validate_stream_record_integrity(&client, stream_id).await?;

        let from = Self::u64_to_i64(from_seq.max(1), "read_range.from_seq")?;
        let rows = if let Some(to) = to_seq {
            let to = Self::u64_to_i64(to, "read_range.to_seq")?;
            client
                .query(
                    "SELECT seq, ts_millis, kind, payload FROM mfm_stream_records WHERE stream_id = $1 AND seq >= $2 AND seq <= $3 ORDER BY seq ASC",
                    &[&stream_id.as_str(), &from, &to],
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
                seq: Self::i64_to_positive_u64(seq, "mfm_stream_records.seq")?,
                ts_millis: Self::opt_i64_to_nonnegative_u64(
                    ts_millis,
                    "mfm_stream_records.ts_millis",
                )?,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_corruption(err: StorageError) {
        assert!(
            matches!(err, StorageError::Corruption(_)),
            "expected corruption error, got {err:?}"
        );
    }

    fn assert_out_of_range(err: StorageError) {
        match err {
            StorageError::Other(info) => assert_eq!(info.code.0, "pg_value_out_of_range"),
            other => panic!("expected pg_value_out_of_range, got {other:?}"),
        }
    }

    #[test]
    fn conversion_helpers_reject_negative_and_overflow_values() {
        assert_eq!(
            PostgresStreamStore::i64_to_nonnegative_u64(0, "head_seq").expect("zero"),
            0
        );
        assert_eq!(
            PostgresStreamStore::i64_to_positive_u64(1, "seq").expect("positive"),
            1
        );
        assert_eq!(
            PostgresStreamStore::u64_to_i64(i64::MAX as u64, "seq").expect("i64 max"),
            i64::MAX
        );

        assert_corruption(
            PostgresStreamStore::i64_to_nonnegative_u64(-1, "head_seq").expect_err("negative"),
        );
        assert_corruption(
            PostgresStreamStore::i64_to_positive_u64(0, "seq").expect_err("zero seq"),
        );
        assert_corruption(
            PostgresStreamStore::opt_i64_to_nonnegative_u64(Some(-1), "ts_millis")
                .expect_err("negative timestamp"),
        );
        assert_out_of_range(
            PostgresStreamStore::u64_to_i64(POSTGRES_BIGINT_MAX_U64 + 1, "seq")
                .expect_err("overflow"),
        );
        assert_out_of_range(
            PostgresStreamStore::checked_next_seq(POSTGRES_BIGINT_MAX_U64)
                .expect_err("next overflow"),
        );
    }

    #[cfg(feature = "parity-tests")]
    mod postgres {
        use super::*;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        use tokio_postgres::error::SqlState;

        static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

        fn unique_schema() -> String {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos();
            let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
            format!("p16_{}_{}_{}", std::process::id(), nanos, counter)
        }

        async fn test_store(init: bool) -> (PostgresStreamStore, String) {
            let database_url =
                std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
            let (client, connection) = tokio_postgres::connect(&database_url, NoTls)
                .await
                .expect("connect postgres");
            tokio::spawn(async move {
                let _ = connection.await;
            });

            let schema = unique_schema();
            client
                .batch_execute(&format!(
                    "CREATE SCHEMA {schema}; SET search_path TO {schema};"
                ))
                .await
                .expect("create test schema");
            let store = PostgresStreamStore {
                client: Arc::new(Mutex::new(client)),
            };
            if init {
                store.init().await.expect("init schema");
            }
            (store, schema)
        }

        async fn create_legacy_schema(store: &PostgresStreamStore) {
            store
                .client
                .lock()
                .await
                .batch_execute(
                    r#"
CREATE TABLE mfm_streams (
  stream_id TEXT PRIMARY KEY,
  head_seq BIGINT NOT NULL
);

CREATE TABLE mfm_stream_records (
  stream_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  ts_millis BIGINT NULL,
  kind TEXT NOT NULL,
  payload JSONB NOT NULL,
  PRIMARY KEY (stream_id, seq),
  CONSTRAINT mfm_stream_records_stream_fk FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);
"#,
                )
                .await
                .expect("create legacy schema");
        }

        async fn drop_schema(store: &PostgresStreamStore, schema: &str) {
            store
                .client
                .lock()
                .await
                .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE;"))
                .await
                .expect("drop test schema");
        }

        fn assert_check_violation(err: tokio_postgres::Error) {
            let db_error = err.as_db_error().expect("database error");
            assert_eq!(db_error.code(), &SqlState::CHECK_VIOLATION);
        }

        #[tokio::test]
        async fn ddl_constraints_reject_negative_values() {
            let (store, schema) = test_store(true).await;
            let stream_id = format!("p16:{schema}");
            let payload = serde_json::json!({});

            let client = store.client.lock().await;
            let err = client
                .execute(
                    "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, $2)",
                    &[&stream_id, &-1_i64],
                )
                .await
                .expect_err("negative head_seq must fail");
            assert_check_violation(err);

            client
                .execute(
                    "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, $2)",
                    &[&stream_id, &0_i64],
                )
                .await
                .expect("insert valid stream");

            let err = client
                .execute(
                    "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                    &[&stream_id, &0_i64, &None::<i64>, &"bad", &payload],
                )
                .await
                .expect_err("non-positive seq must fail");
            assert_check_violation(err);

            let err = client
                .execute(
                    "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                    &[&stream_id, &1_i64, &Some(-1_i64), &"bad", &payload],
                )
                .await
                .expect_err("negative ts_millis must fail");
            assert_check_violation(err);
            drop(client);

            drop_schema(&store, &schema).await;
        }

        #[tokio::test]
        async fn corrupt_head_seq_fails_closed_on_read() {
            let (store, schema) = test_store(false).await;
            create_legacy_schema(&store).await;
            let stream_id = StreamId::must_new(format!("p16:{schema}_head"));

            {
                let client = store.client.lock().await;
                client
                    .execute(
                        "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, $2)",
                        &[&stream_id.as_str(), &-1_i64],
                    )
                    .await
                    .expect("insert corrupt head");
            }

            let err = store.head_seq(&stream_id).await.expect_err("negative head");
            assert_corruption(err);
            drop_schema(&store, &schema).await;
        }

        #[tokio::test]
        async fn corrupt_stream_record_metadata_fails_closed_on_read_range() {
            let (store, schema) = test_store(false).await;
            create_legacy_schema(&store).await;
            let stream_id = StreamId::must_new(format!("p16:{schema}_records"));
            let payload = serde_json::json!({});

            {
                let client = store.client.lock().await;
                client
                    .execute(
                        "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, $2)",
                        &[&stream_id.as_str(), &1_i64],
                    )
                    .await
                    .expect("insert stream");
                client
                    .execute(
                        "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                        &[&stream_id.as_str(), &-1_i64, &None::<i64>, &"bad", &payload],
                    )
                    .await
                    .expect("insert corrupt seq");
            }

            let err = store
                .read_range(&stream_id, 1, None)
                .await
                .expect_err("negative seq");
            assert_corruption(err);
            drop_schema(&store, &schema).await;
        }

        #[tokio::test]
        async fn corrupt_timestamp_fails_closed_on_read_range() {
            let (store, schema) = test_store(false).await;
            create_legacy_schema(&store).await;
            let stream_id = StreamId::must_new(format!("p16:{schema}_ts"));
            let payload = serde_json::json!({});

            {
                let client = store.client.lock().await;
                client
                    .execute(
                        "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, $2)",
                        &[&stream_id.as_str(), &1_i64],
                    )
                    .await
                    .expect("insert stream");
                client
                    .execute(
                        "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                        &[&stream_id.as_str(), &1_i64, &Some(-1_i64), &"bad", &payload],
                    )
                    .await
                    .expect("insert corrupt timestamp");
            }

            let err = store
                .read_range(&stream_id, 1, None)
                .await
                .expect_err("negative timestamp");
            assert_corruption(err);
            drop_schema(&store, &schema).await;
        }

        #[tokio::test]
        async fn normal_append_read_parity_remains_unchanged() {
            let (store, schema) = test_store(true).await;
            let stream_id = StreamId::must_new(format!("p16:{schema}_normal"));

            let head = store
                .append(StreamAppend::new(
                    stream_id.clone(),
                    0,
                    vec![mfm_machine::stores::NewStreamRecord {
                        ts_millis: Some(42),
                        kind: "test".to_string(),
                        payload: serde_json::json!({ "ok": true }),
                    }],
                ))
                .await
                .expect("append");
            assert_eq!(head, 1);
            assert_eq!(store.head_seq(&stream_id).await.expect("head_seq"), 1);

            let records = store
                .read_range(&stream_id, 1, None)
                .await
                .expect("read_range");
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].seq, 1);
            assert_eq!(records[0].ts_millis, Some(42));
            assert_eq!(records[0].kind, "test");
            assert_eq!(records[0].payload, serde_json::json!({ "ok": true }));

            drop_schema(&store, &schema).await;
        }

        #[tokio::test]
        async fn rejects_float_payloads_before_append() {
            let (store, schema) = test_store(true).await;
            let stream_id = StreamId::must_new(format!("p5:{schema}_float"));

            let err = store
                .append(StreamAppend::new(
                    stream_id.clone(),
                    0,
                    vec![mfm_machine::stores::NewStreamRecord {
                        ts_millis: None,
                        kind: "domain_event".to_string(),
                        payload: serde_json::json!({ "value": 1.5 }),
                    }],
                ))
                .await
                .expect_err("float payload must fail");
            assert!(matches!(err, StorageError::Other(_)));
            assert_eq!(store.head_seq(&stream_id).await.expect("head"), 0);

            drop_schema(&store, &schema).await;
        }

        #[tokio::test]
        async fn rejects_secret_shaped_payloads_before_append() {
            let (store, schema) = test_store(true).await;
            let stream_id = StreamId::must_new(format!("p5:{schema}_secret"));

            let err = store
                .append(StreamAppend::new(
                    stream_id.clone(),
                    0,
                    vec![mfm_machine::stores::NewStreamRecord {
                        ts_millis: None,
                        kind: "domain_event".to_string(),
                        payload: serde_json::json!({ "private_key": "do-not-persist" }),
                    }],
                ))
                .await
                .expect_err("secret-shaped payload must fail");
            assert!(matches!(err, StorageError::Other(_)));
            assert_eq!(store.head_seq(&stream_id).await.expect("head"), 0);

            drop_schema(&store, &schema).await;
        }
    }
}
