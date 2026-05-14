#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! PostgreSQL-backed control-plane storage.
//!
//! This crate owns correctness-critical control-plane persistence that must share the same physical
//! PostgreSQL database and SQL transaction boundary as the shared append-only stream substrate.
//! It implements durable SQL appends and projection table updates for the backend-neutral
//! `rpc_source:*` and `source_pool:*` models from `mfm-control-plane-model`.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_control_plane_model::RpcSourceRef;
//! use mfm_control_plane_postgres::ControlPlanePostgresStore;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), mfm_machine::errors::StorageError> {
//! let store =
//!     ControlPlanePostgresStore::connect("postgres://postgres:postgres@localhost/mfm").await?;
//! let source =
//!     RpcSourceRef::new("shared", "eth-mainnet", "primary").expect("valid source ref");
//! let _state = store.rpc_source_state(&source).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls, Row, Transaction};
use tracing::{debug, info};

use mfm_control_plane_model::{
    rebuild_rpc_source_state, rebuild_source_pool_state, validate_source_id_snapshot,
    RpcSourceRecord, RpcSourceRef, RpcSourceState, SourcePoolCatalogSnapshot, SourcePoolRecord,
    SourcePoolRef, SourcePoolState,
};
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::stores::{StreamId, StreamRecord};

fn storage_info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode::must_new(code),
        category: ErrorCategory::Storage,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

fn storage_other(code: &'static str, message: impl Into<String>) -> StorageError {
    StorageError::Other(storage_info(code, message))
}

fn storage_corruption(code: &'static str, message: impl Into<String>) -> StorageError {
    StorageError::Corruption(storage_info(code, message))
}

fn storage_concurrency(message: impl Into<String>) -> StorageError {
    StorageError::Concurrency(storage_info("control_plane_concurrency", message))
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| {
        storage_other(
            "control_plane_value_out_of_range",
            format!("{field} exceeded the supported PostgreSQL bigint range"),
        )
    })
}

fn i64_to_u64(value: i64, field: &'static str) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| {
        storage_corruption(
            "control_plane_projection_invalid",
            format!("{field} contained a negative bigint value"),
        )
    })
}

fn opt_i64_to_u64(value: Option<i64>, field: &'static str) -> Result<Option<u64>, StorageError> {
    value.map(|value| i64_to_u64(value, field)).transpose()
}

/// Correctness-critical Postgres storage for control-plane stream families.
#[derive(Clone)]
pub struct ControlPlanePostgresStore {
    client: Arc<Mutex<Client>>,
}

impl ControlPlanePostgresStore {
    /// Connects to PostgreSQL, ensures the control-plane schema, and returns a ready store.
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        info!("connecting postgres control-plane store");
        let (client, connection) = tokio_postgres::connect(database_url, NoTls)
            .await
            .map_err(|_| storage_other("control_plane_pg_connect_failed", "connect failed"))?;

        tokio::spawn(async move {
            let _ = connection.await;
        });

        let store = Self {
            client: Arc::new(Mutex::new(client)),
        };
        store.init().await?;
        info!("postgres control-plane store connected");
        Ok(store)
    }

    /// Connects using the `DATABASE_URL` environment variable.
    pub async fn connect_env() -> Result<Self, StorageError> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| storage_other("control_plane_pg_missing_env", "missing DATABASE_URL"))?;
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

CREATE TABLE IF NOT EXISTS mfm_rpc_source_state (
  control_scope TEXT NOT NULL,
  network_id TEXT NOT NULL,
  source_id TEXT NOT NULL,
  stream_id TEXT NOT NULL UNIQUE,
  head_seq BIGINT NOT NULL,
  last_recorded_at_ms BIGINT NULL,
  last_observed_at_ms BIGINT NULL,
  last_probed_at_ms BIGINT NULL,
  last_observed_head BIGINT NULL,
  last_latency_ms BIGINT NULL,
  last_probe_latency_ms BIGINT NULL,
  supports_get_proof BOOLEAN NULL,
  success_count BIGINT NOT NULL,
  failure_count BIGINT NOT NULL,
  consecutive_failures BIGINT NOT NULL,
  cooldown_until_ms BIGINT NULL,
  last_error_code TEXT NULL,
  PRIMARY KEY (control_scope, network_id, source_id),
  CONSTRAINT mfm_rpc_source_state_stream_fk FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS mfm_source_pool_state (
  control_scope TEXT NOT NULL,
  network_id TEXT NOT NULL,
  pool_kind TEXT NOT NULL,
  stream_id TEXT NOT NULL UNIQUE,
  head_seq BIGINT NOT NULL,
  last_recorded_at_ms BIGINT NULL,
  last_catalog_declared_at_ms BIGINT NULL,
  last_membership_declared_at_ms BIGINT NULL,
  last_ranked_at_ms BIGINT NULL,
  catalog_fingerprint TEXT NULL,
  catalog_snapshot JSONB NULL,
  member_source_ids JSONB NOT NULL,
  ranked_source_ids JSONB NOT NULL,
  PRIMARY KEY (control_scope, network_id, pool_kind),
  CONSTRAINT mfm_source_pool_state_stream_fk FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);
"#;

        self.client
            .lock()
            .await
            .batch_execute(ddl)
            .await
            .map_err(|_| storage_other("control_plane_pg_init_failed", "init failed"))?;
        debug!("postgres control-plane schema ensured");
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
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to read control-plane stream head",
                )
            })?;

        i64_to_u64(row.get::<_, i64>(0), "mfm_streams.head_seq")
    }

    async fn load_rpc_source_state_tx(
        tx: &Transaction<'_>,
        source_ref: &RpcSourceRef,
    ) -> Result<Option<RpcSourceState>, StorageError> {
        let row = tx
            .query_opt(
                r#"
SELECT
  control_scope,
  network_id,
  source_id,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_observed_at_ms,
  last_probed_at_ms,
  last_observed_head,
  last_latency_ms,
  last_probe_latency_ms,
  supports_get_proof,
  success_count,
  failure_count,
  consecutive_failures,
  cooldown_until_ms,
  last_error_code
FROM mfm_rpc_source_state
WHERE control_scope = $1 AND network_id = $2 AND source_id = $3
"#,
                &[
                    &source_ref.control_scope(),
                    &source_ref.network_id(),
                    &source_ref.source_id(),
                ],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to load rpc_source projection row",
                )
            })?;

        row.map(Self::rpc_source_state_from_row).transpose()
    }

    fn rpc_source_state_from_row(row: Row) -> Result<RpcSourceState, StorageError> {
        let control_scope: String = row.get(0);
        let network_id: String = row.get(1);
        let source_id: String = row.get(2);
        let source_ref =
            RpcSourceRef::new(control_scope, network_id, source_id).map_err(|err| {
                storage_corruption(
                    "control_plane_projection_invalid",
                    format!("invalid rpc_source projection identity: {err}"),
                )
            })?;
        let stream_id = StreamId::new(row.get::<_, String>(3)).map_err(|err| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid rpc_source projection stream id: {err}"),
            )
        })?;
        Ok(RpcSourceState {
            source_ref,
            stream_id,
            head_seq: i64_to_u64(row.get::<_, i64>(4), "mfm_rpc_source_state.head_seq")?,
            last_recorded_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(5),
                "mfm_rpc_source_state.last_recorded_at_ms",
            )?,
            last_observed_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(6),
                "mfm_rpc_source_state.last_observed_at_ms",
            )?,
            last_probed_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(7),
                "mfm_rpc_source_state.last_probed_at_ms",
            )?,
            last_observed_head: opt_i64_to_u64(
                row.get::<_, Option<i64>>(8),
                "mfm_rpc_source_state.last_observed_head",
            )?,
            last_latency_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(9),
                "mfm_rpc_source_state.last_latency_ms",
            )?,
            last_probe_latency_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(10),
                "mfm_rpc_source_state.last_probe_latency_ms",
            )?,
            supports_get_proof: row.get(11),
            success_count: i64_to_u64(row.get::<_, i64>(12), "mfm_rpc_source_state.success_count")?,
            failure_count: i64_to_u64(row.get::<_, i64>(13), "mfm_rpc_source_state.failure_count")?,
            consecutive_failures: i64_to_u64(
                row.get::<_, i64>(14),
                "mfm_rpc_source_state.consecutive_failures",
            )?,
            cooldown_until_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(15),
                "mfm_rpc_source_state.cooldown_until_ms",
            )?,
            last_error_code: row.get(16),
        })
    }

    fn source_id_snapshot_to_json(
        source_ids: &[String],
        field: &'static str,
    ) -> Result<serde_json::Value, StorageError> {
        validate_source_id_snapshot(source_ids).map_err(|message| {
            storage_other(
                "control_plane_projection_invalid",
                format!("invalid {field}: {message}"),
            )
        })?;
        serde_json::to_value(source_ids).map_err(|err| {
            storage_other(
                "control_plane_record_encode_failed",
                format!("failed to encode {field}: {err}"),
            )
        })
    }

    fn source_id_snapshot_from_json(
        value: serde_json::Value,
        field: &'static str,
    ) -> Result<Vec<String>, StorageError> {
        let source_ids: Vec<String> = serde_json::from_value(value).map_err(|err| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid {field} JSON shape: {err}"),
            )
        })?;
        validate_source_id_snapshot(&source_ids).map_err(|message| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid {field}: {message}"),
            )
        })?;
        Ok(source_ids)
    }

    async fn load_source_pool_state_tx(
        tx: &Transaction<'_>,
        pool_ref: &SourcePoolRef,
    ) -> Result<Option<SourcePoolState>, StorageError> {
        let row = tx
            .query_opt(
                r#"
SELECT
  control_scope,
  network_id,
  pool_kind,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_catalog_declared_at_ms,
  last_membership_declared_at_ms,
  last_ranked_at_ms,
  catalog_fingerprint,
  catalog_snapshot,
  member_source_ids,
  ranked_source_ids
FROM mfm_source_pool_state
WHERE control_scope = $1 AND network_id = $2 AND pool_kind = $3
"#,
                &[
                    &pool_ref.control_scope(),
                    &pool_ref.network_id(),
                    &pool_ref.pool_kind(),
                ],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to load source_pool projection row",
                )
            })?;

        row.map(Self::source_pool_state_from_row).transpose()
    }

    fn source_pool_state_from_row(row: Row) -> Result<SourcePoolState, StorageError> {
        let control_scope: String = row.get(0);
        let network_id: String = row.get(1);
        let pool_kind: String = row.get(2);
        let pool_ref = SourcePoolRef::new(control_scope, network_id, pool_kind).map_err(|err| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid source_pool projection identity: {err}"),
            )
        })?;
        let stream_id = StreamId::new(row.get::<_, String>(3)).map_err(|err| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid source_pool projection stream id: {err}"),
            )
        })?;
        let catalog_fingerprint: Option<String> = row.get(9);
        let catalog_snapshot = row
            .get::<_, Option<serde_json::Value>>(10)
            .map(|value| {
                let snapshot: SourcePoolCatalogSnapshot =
                    serde_json::from_value(value).map_err(|err| {
                        storage_corruption(
                            "control_plane_projection_invalid",
                            format!(
                                "invalid mfm_source_pool_state.catalog_snapshot JSON shape: {err}"
                            ),
                        )
                    })?;
                snapshot.validate().map_err(|message| {
                    storage_corruption(
                        "control_plane_projection_invalid",
                        format!("invalid mfm_source_pool_state.catalog_snapshot: {message}"),
                    )
                })?;
                Ok(snapshot)
            })
            .transpose()?;
        if catalog_fingerprint.is_some() != catalog_snapshot.is_some() {
            return Err(storage_corruption(
                "control_plane_projection_invalid",
                "mfm_source_pool_state catalog fingerprint and snapshot must be present together",
            ));
        }
        if let (Some(catalog_fingerprint), Some(catalog_snapshot)) =
            (&catalog_fingerprint, &catalog_snapshot)
        {
            let expected = catalog_snapshot.fingerprint().map_err(|err| {
                storage_corruption(
                    "control_plane_projection_invalid",
                    format!(
                        "mfm_source_pool_state catalog snapshot was not canonical-json-hashable: {err}"
                    ),
                )
            })?;
            if catalog_fingerprint != &expected {
                return Err(storage_corruption(
                    "control_plane_projection_invalid",
                    format!(
                        "mfm_source_pool_state catalog fingerprint `{catalog_fingerprint}` did not match canonical snapshot fingerprint `{expected}`"
                    ),
                ));
            }
            if catalog_snapshot.control_scope != pool_ref.control_scope()
                || catalog_snapshot.network_id != pool_ref.network_id()
                || catalog_snapshot.pool_kind != pool_ref.pool_kind()
            {
                return Err(storage_corruption(
                    "control_plane_projection_invalid",
                    "mfm_source_pool_state catalog snapshot identity did not match pool identity",
                ));
            }
        }

        Ok(SourcePoolState {
            pool_ref,
            stream_id,
            head_seq: i64_to_u64(row.get::<_, i64>(4), "mfm_source_pool_state.head_seq")?,
            last_recorded_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(5),
                "mfm_source_pool_state.last_recorded_at_ms",
            )?,
            last_membership_declared_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(7),
                "mfm_source_pool_state.last_membership_declared_at_ms",
            )?,
            last_ranked_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(8),
                "mfm_source_pool_state.last_ranked_at_ms",
            )?,
            last_catalog_declared_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(6),
                "mfm_source_pool_state.last_catalog_declared_at_ms",
            )?,
            catalog_fingerprint,
            catalog_snapshot,
            member_source_ids: Self::source_id_snapshot_from_json(
                row.get::<_, serde_json::Value>(11),
                "mfm_source_pool_state.member_source_ids",
            )?,
            ranked_source_ids: Self::source_id_snapshot_from_json(
                row.get::<_, serde_json::Value>(12),
                "mfm_source_pool_state.ranked_source_ids",
            )?,
        })
    }

    async fn upsert_rpc_source_state_tx(
        tx: &Transaction<'_>,
        state: &RpcSourceState,
    ) -> Result<(), StorageError> {
        tx.execute(
            r#"
INSERT INTO mfm_rpc_source_state (
  control_scope,
  network_id,
  source_id,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_observed_at_ms,
  last_probed_at_ms,
  last_observed_head,
  last_latency_ms,
  last_probe_latency_ms,
  supports_get_proof,
  success_count,
  failure_count,
  consecutive_failures,
  cooldown_until_ms,
  last_error_code
) VALUES (
  $1, $2, $3, $4, $5, $6, $7, $8,
  $9, $10, $11, $12, $13, $14, $15, $16, $17
)
ON CONFLICT (control_scope, network_id, source_id) DO UPDATE SET
  stream_id = EXCLUDED.stream_id,
  head_seq = EXCLUDED.head_seq,
  last_recorded_at_ms = EXCLUDED.last_recorded_at_ms,
  last_observed_at_ms = EXCLUDED.last_observed_at_ms,
  last_probed_at_ms = EXCLUDED.last_probed_at_ms,
  last_observed_head = EXCLUDED.last_observed_head,
  last_latency_ms = EXCLUDED.last_latency_ms,
  last_probe_latency_ms = EXCLUDED.last_probe_latency_ms,
  supports_get_proof = EXCLUDED.supports_get_proof,
  success_count = EXCLUDED.success_count,
  failure_count = EXCLUDED.failure_count,
  consecutive_failures = EXCLUDED.consecutive_failures,
  cooldown_until_ms = EXCLUDED.cooldown_until_ms,
  last_error_code = EXCLUDED.last_error_code
"#,
            &[
                &state.source_ref.control_scope(),
                &state.source_ref.network_id(),
                &state.source_ref.source_id(),
                &state.stream_id.as_str(),
                &u64_to_i64(state.head_seq, "head_seq")?,
                &state
                    .last_recorded_at_ms
                    .map(|value| u64_to_i64(value, "last_recorded_at_ms"))
                    .transpose()?,
                &state
                    .last_observed_at_ms
                    .map(|value| u64_to_i64(value, "last_observed_at_ms"))
                    .transpose()?,
                &state
                    .last_probed_at_ms
                    .map(|value| u64_to_i64(value, "last_probed_at_ms"))
                    .transpose()?,
                &state
                    .last_observed_head
                    .map(|value| u64_to_i64(value, "last_observed_head"))
                    .transpose()?,
                &state
                    .last_latency_ms
                    .map(|value| u64_to_i64(value, "last_latency_ms"))
                    .transpose()?,
                &state
                    .last_probe_latency_ms
                    .map(|value| u64_to_i64(value, "last_probe_latency_ms"))
                    .transpose()?,
                &state.supports_get_proof,
                &u64_to_i64(state.success_count, "success_count")?,
                &u64_to_i64(state.failure_count, "failure_count")?,
                &u64_to_i64(state.consecutive_failures, "consecutive_failures")?,
                &state
                    .cooldown_until_ms
                    .map(|value| u64_to_i64(value, "cooldown_until_ms"))
                    .transpose()?,
                &state.last_error_code,
            ],
        )
        .await
        .map_err(|_| {
            storage_other(
                "control_plane_pg_update_failed",
                "failed to upsert rpc_source projection row",
            )
        })?;
        Ok(())
    }

    async fn upsert_source_pool_state_tx(
        tx: &Transaction<'_>,
        state: &SourcePoolState,
    ) -> Result<(), StorageError> {
        let member_source_ids =
            Self::source_id_snapshot_to_json(&state.member_source_ids, "member_source_ids")?;
        let ranked_source_ids =
            Self::source_id_snapshot_to_json(&state.ranked_source_ids, "ranked_source_ids")?;

        tx.execute(
            r#"
INSERT INTO mfm_source_pool_state (
  control_scope,
  network_id,
  pool_kind,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_catalog_declared_at_ms,
  last_membership_declared_at_ms,
  last_ranked_at_ms,
  catalog_fingerprint,
  catalog_snapshot,
  member_source_ids,
  ranked_source_ids
) VALUES (
  $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
)
ON CONFLICT (control_scope, network_id, pool_kind) DO UPDATE SET
  stream_id = EXCLUDED.stream_id,
  head_seq = EXCLUDED.head_seq,
  last_recorded_at_ms = EXCLUDED.last_recorded_at_ms,
  last_catalog_declared_at_ms = EXCLUDED.last_catalog_declared_at_ms,
  last_membership_declared_at_ms = EXCLUDED.last_membership_declared_at_ms,
  last_ranked_at_ms = EXCLUDED.last_ranked_at_ms,
  catalog_fingerprint = EXCLUDED.catalog_fingerprint,
  catalog_snapshot = EXCLUDED.catalog_snapshot,
  member_source_ids = EXCLUDED.member_source_ids,
  ranked_source_ids = EXCLUDED.ranked_source_ids
"#,
            &[
                &state.pool_ref.control_scope(),
                &state.pool_ref.network_id(),
                &state.pool_ref.pool_kind(),
                &state.stream_id.as_str(),
                &u64_to_i64(state.head_seq, "head_seq")?,
                &state
                    .last_recorded_at_ms
                    .map(|value| u64_to_i64(value, "last_recorded_at_ms"))
                    .transpose()?,
                &state
                    .last_catalog_declared_at_ms
                    .map(|value| u64_to_i64(value, "last_catalog_declared_at_ms"))
                    .transpose()?,
                &state
                    .last_membership_declared_at_ms
                    .map(|value| u64_to_i64(value, "last_membership_declared_at_ms"))
                    .transpose()?,
                &state
                    .last_ranked_at_ms
                    .map(|value| u64_to_i64(value, "last_ranked_at_ms"))
                    .transpose()?,
                &state.catalog_fingerprint,
                &state
                    .catalog_snapshot
                    .as_ref()
                    .map(|snapshot| {
                        snapshot.validate().map_err(|message| {
                            storage_other(
                                "control_plane_projection_invalid",
                                format!("invalid catalog_snapshot: {message}"),
                            )
                        })?;
                        serde_json::to_value(snapshot).map_err(|err| {
                            storage_other(
                                "control_plane_record_encode_failed",
                                format!("failed to encode catalog_snapshot: {err}"),
                            )
                        })
                    })
                    .transpose()?,
                &member_source_ids,
                &ranked_source_ids,
            ],
        )
        .await
        .map_err(|_| {
            storage_other(
                "control_plane_pg_update_failed",
                "failed to upsert source_pool projection row",
            )
        })?;
        Ok(())
    }

    /// Appends one atomic batch of `rpc_source:*` records and updates the projection in the same transaction.
    pub async fn append_rpc_source_records(
        &self,
        source_ref: &RpcSourceRef,
        expected_seq: u64,
        records: Vec<RpcSourceRecord>,
    ) -> Result<RpcSourceState, StorageError> {
        if records.is_empty() {
            return Err(storage_other(
                "control_plane_append_invalid",
                "rpc_source append must include at least one record",
            ));
        }

        let stream_id = source_ref.stream_id();
        let encoded_records = records
            .iter()
            .map(RpcSourceRecord::to_new_stream_record)
            .collect::<Result<Vec<_>, _>>()?;

        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to start transaction")
        })?;

        tx.execute(
            "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, 0) ON CONFLICT (stream_id) DO NOTHING",
            &[&stream_id.as_str()],
        )
        .await
        .map_err(|_| storage_other("control_plane_pg_insert_failed", "failed to insert stream head"))?;

        let head = Self::read_head_for_update(&tx, &stream_id).await?;
        if head != expected_seq {
            return Err(storage_concurrency(format!(
                "rpc_source head seq did not match expected seq for `{}`",
                stream_id
            )));
        }

        let existing = Self::load_rpc_source_state_tx(&tx, source_ref).await?;
        if let Some(existing) = &existing {
            if existing.head_seq != head {
                return Err(storage_corruption(
                    "control_plane_projection_desynced",
                    format!(
                        "rpc_source projection head {} did not match stream head {} for `{}`",
                        existing.head_seq, head, stream_id
                    ),
                ));
            }
        }

        let mut state = existing.unwrap_or_else(|| RpcSourceState::new(source_ref.clone()));
        let mut seq = expected_seq;
        for (record, encoded) in records.iter().zip(encoded_records.iter()) {
            seq = seq.saturating_add(1);
            tx.execute(
                "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                &[
                    &stream_id.as_str(),
                    &u64_to_i64(seq, "seq")?,
                    &encoded.ts_millis.map(|value| u64_to_i64(value, "ts_millis")).transpose()?,
                    &encoded.kind,
                    &encoded.payload,
                ],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_insert_failed",
                    "failed to insert rpc_source stream record",
                )
            })?;
            state.apply_record(seq, record);
        }

        Self::upsert_rpc_source_state_tx(&tx, &state).await?;
        tx.execute(
            "UPDATE mfm_streams SET head_seq = $2 WHERE stream_id = $1",
            &[&stream_id.as_str(), &u64_to_i64(seq, "head_seq")?],
        )
        .await
        .map_err(|_| {
            storage_other(
                "control_plane_pg_update_failed",
                "failed to update rpc_source stream head",
            )
        })?;

        tx.commit().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to commit transaction")
        })?;

        Ok(state)
    }

    /// Appends one atomic batch of `source_pool:*` records and updates the projection in the same transaction.
    pub async fn append_source_pool_records(
        &self,
        pool_ref: &SourcePoolRef,
        expected_seq: u64,
        records: Vec<SourcePoolRecord>,
    ) -> Result<SourcePoolState, StorageError> {
        if records.is_empty() {
            return Err(storage_other(
                "control_plane_append_invalid",
                "source_pool append must include at least one record",
            ));
        }

        let stream_id = pool_ref.stream_id();
        let encoded_records = records
            .iter()
            .map(SourcePoolRecord::to_new_stream_record)
            .collect::<Result<Vec<_>, _>>()?;

        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to start transaction")
        })?;

        tx.execute(
            "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, 0) ON CONFLICT (stream_id) DO NOTHING",
            &[&stream_id.as_str()],
        )
        .await
        .map_err(|_| storage_other("control_plane_pg_insert_failed", "failed to insert stream head"))?;

        let head = Self::read_head_for_update(&tx, &stream_id).await?;
        if head != expected_seq {
            return Err(storage_concurrency(format!(
                "source_pool head seq did not match expected seq for `{}`",
                stream_id
            )));
        }

        let existing = Self::load_source_pool_state_tx(&tx, pool_ref).await?;
        if let Some(existing) = &existing {
            if existing.head_seq != head {
                return Err(storage_corruption(
                    "control_plane_projection_desynced",
                    format!(
                        "source_pool projection head {} did not match stream head {} for `{}`",
                        existing.head_seq, head, stream_id
                    ),
                ));
            }
        }

        let mut state = existing.unwrap_or_else(|| SourcePoolState::new(pool_ref.clone()));
        let mut seq = expected_seq;
        for (record, encoded) in records.iter().zip(encoded_records.iter()) {
            seq = seq.saturating_add(1);
            tx.execute(
                "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                &[
                    &stream_id.as_str(),
                    &u64_to_i64(seq, "seq")?,
                    &encoded.ts_millis.map(|value| u64_to_i64(value, "ts_millis")).transpose()?,
                    &encoded.kind,
                    &encoded.payload,
                ],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_insert_failed",
                    "failed to insert source_pool stream record",
                )
            })?;
            state
                .apply_record(seq, record)
                .map_err(|err| storage_other("control_plane_record_invalid", err.to_string()))?;
        }

        Self::upsert_source_pool_state_tx(&tx, &state).await?;
        tx.execute(
            "UPDATE mfm_streams SET head_seq = $2 WHERE stream_id = $1",
            &[&stream_id.as_str(), &u64_to_i64(seq, "head_seq")?],
        )
        .await
        .map_err(|_| {
            storage_other(
                "control_plane_pg_update_failed",
                "failed to update source_pool stream head",
            )
        })?;

        tx.commit().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to commit transaction")
        })?;

        Ok(state)
    }

    /// Loads the current projection row for one RPC source.
    pub async fn rpc_source_state(
        &self,
        source_ref: &RpcSourceRef,
    ) -> Result<Option<RpcSourceState>, StorageError> {
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                r#"
SELECT
  control_scope,
  network_id,
  source_id,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_observed_at_ms,
  last_probed_at_ms,
  last_observed_head,
  last_latency_ms,
  last_probe_latency_ms,
  supports_get_proof,
  success_count,
  failure_count,
  consecutive_failures,
  cooldown_until_ms,
  last_error_code
FROM mfm_rpc_source_state
WHERE control_scope = $1 AND network_id = $2 AND source_id = $3
"#,
                &[
                    &source_ref.control_scope(),
                    &source_ref.network_id(),
                    &source_ref.source_id(),
                ],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to query rpc_source projection row",
                )
            })?;
        row.map(Self::rpc_source_state_from_row).transpose()
    }

    /// Lists all RPC-source projections for one network.
    pub async fn list_rpc_source_states(
        &self,
        control_scope: &str,
        network_id: &str,
    ) -> Result<Vec<RpcSourceState>, StorageError> {
        let client = self.client.lock().await;
        let rows = client
            .query(
                r#"
SELECT
  control_scope,
  network_id,
  source_id,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_observed_at_ms,
  last_probed_at_ms,
  last_observed_head,
  last_latency_ms,
  last_probe_latency_ms,
  supports_get_proof,
  success_count,
  failure_count,
  consecutive_failures,
  cooldown_until_ms,
  last_error_code
FROM mfm_rpc_source_state
WHERE control_scope = $1 AND network_id = $2
ORDER BY source_id ASC
"#,
                &[&control_scope, &network_id],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to list rpc_source projection rows",
                )
            })?;
        rows.into_iter()
            .map(Self::rpc_source_state_from_row)
            .collect()
    }

    /// Loads the current projection row for one source pool.
    pub async fn source_pool_state(
        &self,
        pool_ref: &SourcePoolRef,
    ) -> Result<Option<SourcePoolState>, StorageError> {
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                r#"
SELECT
  control_scope,
  network_id,
  pool_kind,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_catalog_declared_at_ms,
  last_membership_declared_at_ms,
  last_ranked_at_ms,
  catalog_fingerprint,
  catalog_snapshot,
  member_source_ids,
  ranked_source_ids
FROM mfm_source_pool_state
WHERE control_scope = $1 AND network_id = $2 AND pool_kind = $3
"#,
                &[
                    &pool_ref.control_scope(),
                    &pool_ref.network_id(),
                    &pool_ref.pool_kind(),
                ],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to query source_pool projection row",
                )
            })?;
        row.map(Self::source_pool_state_from_row).transpose()
    }

    /// Lists all source-pool projections for one network.
    pub async fn list_source_pool_states(
        &self,
        control_scope: &str,
        network_id: &str,
    ) -> Result<Vec<SourcePoolState>, StorageError> {
        let client = self.client.lock().await;
        let rows = client
            .query(
                r#"
SELECT
  control_scope,
  network_id,
  pool_kind,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_catalog_declared_at_ms,
  last_membership_declared_at_ms,
  last_ranked_at_ms,
  catalog_fingerprint,
  catalog_snapshot,
  member_source_ids,
  ranked_source_ids
FROM mfm_source_pool_state
WHERE control_scope = $1 AND network_id = $2
ORDER BY pool_kind ASC
"#,
                &[&control_scope, &network_id],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to list source_pool projection rows",
                )
            })?;
        rows.into_iter()
            .map(Self::source_pool_state_from_row)
            .collect()
    }

    /// Returns the effective ordered source refs for one pool.
    pub async fn source_pool_ordered_sources(
        &self,
        pool_ref: &SourcePoolRef,
    ) -> Result<Vec<RpcSourceRef>, StorageError> {
        let Some(state) = self.source_pool_state(pool_ref).await? else {
            return Ok(Vec::new());
        };
        state.ordered_source_refs().map_err(|err| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid source_pool ordered source refs: {err}"),
            )
        })
    }

    /// Rebuilds every `mfm_rpc_source_state` row from append-only `rpc_source:*` stream records.
    pub async fn rebuild_all_rpc_source_states(&self) -> Result<Vec<RpcSourceState>, StorageError> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to start transaction")
        })?;

        let rows = tx
            .query(
                r#"
SELECT stream_id, seq, ts_millis, kind, payload
FROM mfm_stream_records
WHERE stream_id LIKE 'rpc_source:%'
ORDER BY stream_id ASC, seq ASC
"#,
                &[],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to query rpc_source stream records",
                )
            })?;

        let mut by_stream = BTreeMap::<String, Vec<StreamRecord>>::new();
        for row in rows {
            let stream_id = StreamId::new(row.get::<_, String>(0)).map_err(|err| {
                storage_corruption(
                    "control_plane_stream_invalid",
                    format!("invalid rpc_source stream id in record table: {err}"),
                )
            })?;
            let record = StreamRecord {
                stream_id: stream_id.clone(),
                seq: i64_to_u64(row.get::<_, i64>(1), "mfm_stream_records.seq")?,
                ts_millis: opt_i64_to_u64(
                    row.get::<_, Option<i64>>(2),
                    "mfm_stream_records.ts_millis",
                )?,
                kind: row.get(3),
                payload: row.get(4),
            };
            by_stream
                .entry(stream_id.as_str().to_string())
                .or_default()
                .push(record);
        }

        let mut rebuilt = Vec::with_capacity(by_stream.len());
        for records in by_stream.into_values() {
            let source_ref =
                RpcSourceRef::from_stream_id(&records[0].stream_id).map_err(|err| {
                    storage_corruption(
                        "control_plane_stream_invalid",
                        format!("invalid rpc_source stream id in rebuild: {err}"),
                    )
                })?;
            let state = rebuild_rpc_source_state(source_ref, &records).map_err(|err| {
                storage_corruption(
                    "control_plane_projection_rebuild_failed",
                    format!("failed to rebuild rpc_source projection: {err}"),
                )
            })?;
            rebuilt.push(state);
        }

        tx.execute("DELETE FROM mfm_rpc_source_state", &[])
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_delete_failed",
                    "failed to clear rpc_source projection table during rebuild",
                )
            })?;

        for state in &rebuilt {
            Self::upsert_rpc_source_state_tx(&tx, state).await?;
        }

        tx.commit().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to commit transaction")
        })?;

        Ok(rebuilt)
    }

    /// Rebuilds every `mfm_source_pool_state` row from append-only `source_pool:*` stream records.
    pub async fn rebuild_all_source_pool_states(
        &self,
    ) -> Result<Vec<SourcePoolState>, StorageError> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to start transaction")
        })?;

        let rows = tx
            .query(
                r#"
SELECT stream_id, seq, ts_millis, kind, payload
FROM mfm_stream_records
WHERE stream_id LIKE 'source_pool:%'
ORDER BY stream_id ASC, seq ASC
"#,
                &[],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to query source_pool stream records",
                )
            })?;

        let mut by_stream = BTreeMap::<String, Vec<StreamRecord>>::new();
        for row in rows {
            let stream_id = StreamId::new(row.get::<_, String>(0)).map_err(|err| {
                storage_corruption(
                    "control_plane_stream_invalid",
                    format!("invalid source_pool stream id in record table: {err}"),
                )
            })?;
            let record = StreamRecord {
                stream_id: stream_id.clone(),
                seq: i64_to_u64(row.get::<_, i64>(1), "mfm_stream_records.seq")?,
                ts_millis: opt_i64_to_u64(
                    row.get::<_, Option<i64>>(2),
                    "mfm_stream_records.ts_millis",
                )?,
                kind: row.get(3),
                payload: row.get(4),
            };
            by_stream
                .entry(stream_id.as_str().to_string())
                .or_default()
                .push(record);
        }

        let mut rebuilt = Vec::with_capacity(by_stream.len());
        for records in by_stream.into_values() {
            let pool_ref = SourcePoolRef::from_stream_id(&records[0].stream_id).map_err(|err| {
                storage_corruption(
                    "control_plane_stream_invalid",
                    format!("invalid source_pool stream id in rebuild: {err}"),
                )
            })?;
            let state = rebuild_source_pool_state(pool_ref, &records).map_err(|err| {
                storage_corruption(
                    "control_plane_projection_rebuild_failed",
                    format!("failed to rebuild source_pool projection: {err}"),
                )
            })?;
            rebuilt.push(state);
        }

        tx.execute("DELETE FROM mfm_source_pool_state", &[])
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_delete_failed",
                    "failed to clear source_pool projection table during rebuild",
                )
            })?;

        for state in &rebuilt {
            Self::upsert_source_pool_state_tx(&tx, state).await?;
        }

        tx.commit().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to commit transaction")
        })?;

        Ok(rebuilt)
    }
}
