BEGIN;

CREATE TABLE IF NOT EXISTS typed_run_heads (
  run_id TEXT PRIMARY KEY,
  head_seq BIGINT NOT NULL,
  CONSTRAINT typed_run_heads_head_seq_nonnegative CHECK (head_seq >= 0)
);

CREATE TABLE IF NOT EXISTS typed_run_events (
  run_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  ordinal INTEGER NOT NULL,
  event_id TEXT NOT NULL,
  event_schema_id TEXT NOT NULL,
  spec_hash TEXT NOT NULL,
  commit_key TEXT NOT NULL,
  logical_key TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  payload_canonical_byte_len BIGINT NOT NULL,
  payload_json JSONB NOT NULL,
  PRIMARY KEY (run_id, seq, ordinal),
  CONSTRAINT typed_run_events_seq_positive CHECK (seq >= 1),
  CONSTRAINT typed_run_events_ordinal_nonnegative CHECK (ordinal >= 0),
  CONSTRAINT typed_run_events_payload_len_nonnegative CHECK (payload_canonical_byte_len >= 0),
  CONSTRAINT typed_run_events_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE,
  UNIQUE (run_id, event_id)
);

CREATE TABLE IF NOT EXISTS typed_commit_keys (
  run_id TEXT NOT NULL,
  commit_key TEXT NOT NULL,
  commit_fingerprint TEXT NOT NULL,
  seq BIGINT NOT NULL,
  event_count INTEGER NOT NULL,
  PRIMARY KEY (run_id, commit_key),
  CONSTRAINT typed_commit_keys_seq_positive CHECK (seq >= 1),
  CONSTRAINT typed_commit_keys_event_count_positive CHECK (event_count >= 1)
);

CREATE TABLE IF NOT EXISTS typed_artifacts (
  artifact_id TEXT PRIMARY KEY,
  digest TEXT NOT NULL,
  byte_len BIGINT NOT NULL,
  media_type TEXT NOT NULL,
  schema_id TEXT NULL,
  semantic_type_id TEXT NULL,
  producer_node_id TEXT NULL,
  producer_seed_id TEXT NULL,
  artifact_role TEXT NOT NULL,
  CONSTRAINT typed_artifacts_byte_len_nonnegative CHECK (byte_len >= 0)
);

CREATE TABLE IF NOT EXISTS typed_logical_keys (
  run_id TEXT NOT NULL,
  logical_key TEXT NOT NULL,
  PRIMARY KEY (run_id, logical_key)
);

CREATE TABLE IF NOT EXISTS typed_unique_logical_payloads (
  run_id TEXT NOT NULL,
  logical_key TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  PRIMARY KEY (run_id, logical_key)
);

CREATE TABLE IF NOT EXISTS typed_run_projection (
  run_id TEXT PRIMARY KEY,
  run_state TEXT NOT NULL,
  projection_json JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS typed_attempt_projection (
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, node_id, attempt_id)
);

CREATE TABLE IF NOT EXISTS typed_cell_projection (
  run_id TEXT NOT NULL,
  cell_id TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, cell_id)
);

CREATE TABLE IF NOT EXISTS typed_fact_projection (
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  fact_key TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, node_id, attempt_id, fact_key)
);

CREATE TABLE IF NOT EXISTS typed_side_effect_projection (
  run_id TEXT NOT NULL,
  ledger_key TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, ledger_key)
);

CREATE TABLE IF NOT EXISTS typed_public_output_projection (
  run_id TEXT NOT NULL,
  public_schema_id TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, public_schema_id)
);

CREATE TABLE IF NOT EXISTS typed_retention_projection (
  run_id TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, artifact_id)
);

CREATE TABLE IF NOT EXISTS typed_retention_manifests (
  run_id TEXT NOT NULL,
  manifest_seq BIGINT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, manifest_seq),
  CONSTRAINT typed_retention_manifests_seq_positive CHECK (manifest_seq >= 1)
);

COMMIT;
