CREATE TABLE typed_run_heads (
  run_id TEXT PRIMARY KEY,
  head_seq BIGINT NOT NULL,
  CONSTRAINT typed_run_heads_head_seq_nonnegative CHECK (head_seq >= 0)
);

CREATE TABLE typed_run_events (
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

CREATE TABLE typed_commit_keys (
  run_id TEXT NOT NULL,
  commit_key TEXT NOT NULL,
  commit_fingerprint TEXT NOT NULL,
  seq BIGINT NOT NULL,
  event_count INTEGER NOT NULL,
  PRIMARY KEY (run_id, commit_key),
  CONSTRAINT typed_commit_keys_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE,
  CONSTRAINT typed_commit_keys_seq_positive CHECK (seq >= 1),
  CONSTRAINT typed_commit_keys_event_count_positive CHECK (event_count >= 1)
);

CREATE TABLE typed_artifacts (
  artifact_id TEXT PRIMARY KEY,
  digest TEXT NOT NULL,
  byte_len BIGINT NOT NULL,
  media_type TEXT NOT NULL,
  schema_id TEXT NULL,
  semantic_type_id TEXT NULL,
  producer_node_id TEXT NULL,
  producer_seed_id TEXT NULL,
  artifact_role TEXT NOT NULL,
  CONSTRAINT typed_artifacts_byte_len_nonnegative CHECK (byte_len >= 0),
  CONSTRAINT typed_artifacts_single_producer CHECK (
    producer_node_id IS NULL OR producer_seed_id IS NULL
  )
);

CREATE TABLE typed_run_artifacts (
  run_id TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  commit_key TEXT NOT NULL,
  seq BIGINT NOT NULL,
  PRIMARY KEY (run_id, artifact_id),
  CONSTRAINT typed_run_artifacts_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE,
  CONSTRAINT typed_run_artifacts_artifact_fk FOREIGN KEY (artifact_id) REFERENCES typed_artifacts(artifact_id) ON DELETE RESTRICT,
  CONSTRAINT typed_run_artifacts_seq_positive CHECK (seq >= 1)
);

CREATE TABLE typed_logical_keys (
  run_id TEXT NOT NULL,
  logical_key TEXT NOT NULL,
  PRIMARY KEY (run_id, logical_key),
  CONSTRAINT typed_logical_keys_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_unique_logical_payloads (
  run_id TEXT NOT NULL,
  logical_key TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  PRIMARY KEY (run_id, logical_key),
  CONSTRAINT typed_unique_logical_payloads_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_run_projection (
  run_id TEXT PRIMARY KEY,
  run_state TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  CONSTRAINT typed_run_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_run_completion_projection (
  run_id TEXT PRIMARY KEY,
  projection_json JSONB NOT NULL,
  CONSTRAINT typed_run_completion_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_saga_engagement_projection (
  run_id TEXT PRIMARY KEY,
  projection_json JSONB NOT NULL,
  CONSTRAINT typed_saga_engagement_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_manual_resolution_projection (
  run_id TEXT PRIMARY KEY,
  projection_json JSONB NOT NULL,
  CONSTRAINT typed_manual_resolution_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_attempt_projection (
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, node_id, attempt_id),
  CONSTRAINT typed_attempt_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_cell_projection (
  run_id TEXT NOT NULL,
  cell_id TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, cell_id),
  CONSTRAINT typed_cell_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_fact_projection (
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  fact_key TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, node_id, attempt_id, fact_key),
  CONSTRAINT typed_fact_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_side_effect_projection (
  run_id TEXT NOT NULL,
  ledger_key TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, ledger_key),
  CONSTRAINT typed_side_effect_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE TABLE typed_resource_lane_projection (
  namespace TEXT NOT NULL,
  resource_key TEXT NOT NULL,
  run_id TEXT NOT NULL,
  ledger_key TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (namespace, resource_key),
  CONSTRAINT typed_resource_lane_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);

CREATE INDEX typed_resource_lane_projection_run_idx
  ON typed_resource_lane_projection (run_id);

CREATE TABLE typed_public_output_projection (
  run_id TEXT NOT NULL,
  public_schema_id TEXT NOT NULL,
  projection_json JSONB NOT NULL,
  PRIMARY KEY (run_id, public_schema_id),
  CONSTRAINT typed_public_output_projection_run_fk FOREIGN KEY (run_id) REFERENCES typed_run_heads(run_id) ON DELETE CASCADE
);
