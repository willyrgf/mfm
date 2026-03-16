BEGIN;

DROP TABLE IF EXISTS mfm_rpc_source_state;
DROP TABLE IF EXISTS mfm_source_pool_state;

DELETE FROM mfm_stream_records
WHERE stream_id LIKE 'rpc_source:%'
   OR stream_id LIKE 'source_pool:%';

DELETE FROM mfm_streams
WHERE stream_id LIKE 'rpc_source:%'
   OR stream_id LIKE 'source_pool:%';

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
  CONSTRAINT mfm_rpc_source_state_stream_fk
    FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
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
  CONSTRAINT mfm_source_pool_state_stream_fk
    FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);

COMMIT;
