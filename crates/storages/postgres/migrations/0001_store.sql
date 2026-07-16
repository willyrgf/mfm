CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;

CREATE FUNCTION mfm_reject_authority_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  RAISE EXCEPTION 'mfm authority tables are append-only';
END;
$$;

CREATE TABLE store_metadata (
  singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  store_epoch TEXT NOT NULL,
  store_scope_id TEXT NOT NULL,
  schema_contract_version TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT store_metadata_store_scope_id_v1 CHECK (
    store_scope_id ~ '^mfm\.store_scope\.v1:[0-9a-f]{32}$'
  )
);

INSERT INTO store_metadata (store_epoch, store_scope_id, schema_contract_version)
VALUES (
  'mfm.store.epoch.v1:' || encode(public.gen_random_bytes(16), 'hex'),
  'mfm.store_scope.v1:' || encode(public.gen_random_bytes(16), 'hex'),
  'mfm.postgres.store.v3'
);

CREATE TABLE store_commit_order (
  singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  current_order BIGINT NOT NULL DEFAULT 0,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT store_commit_order_nonnegative CHECK (current_order >= 0)
);

INSERT INTO store_commit_order (current_order) VALUES (0);

CREATE TABLE commits (
  commit_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  commit_key TEXT NOT NULL,
  commit_purpose TEXT NOT NULL,
  prepared_commit_plan_fingerprint TEXT NOT NULL,
  commit_batch_hash TEXT NOT NULL,
  store_commit_order BIGINT NOT NULL,
  event_count INTEGER NOT NULL,
  committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  PRIMARY KEY (run_id, seq),
  UNIQUE (commit_id),
  UNIQUE (run_id, commit_key),
  UNIQUE (store_commit_order),
  CONSTRAINT commits_seq_positive CHECK (seq >= 1),
  CONSTRAINT commits_event_count_positive CHECK (event_count >= 1),
  CONSTRAINT commits_store_commit_order_positive CHECK (store_commit_order >= 1)
);

CREATE TABLE run_events (
  run_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  ordinal INTEGER NOT NULL,
  commit_id TEXT NOT NULL,
  event_id TEXT NOT NULL,
  event_schema_id TEXT NOT NULL,
  spec_hash TEXT NOT NULL,
  commit_key TEXT NOT NULL,
  logical_key TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  payload_canonical_json BYTEA NOT NULL,
  PRIMARY KEY (run_id, seq, ordinal),
  CONSTRAINT run_events_commit_fk FOREIGN KEY (run_id, seq) REFERENCES commits(run_id, seq) ON DELETE RESTRICT,
  CONSTRAINT run_events_commit_id_fk FOREIGN KEY (commit_id) REFERENCES commits(commit_id) ON DELETE RESTRICT,
  CONSTRAINT run_events_seq_positive CHECK (seq >= 1),
  CONSTRAINT run_events_ordinal_nonnegative CHECK (ordinal >= 0),
  UNIQUE (commit_id, ordinal),
  UNIQUE (run_id, event_id)
);

CREATE TABLE artifact_blobs (
  artifact_id TEXT PRIMARY KEY,
  digest TEXT NOT NULL,
  byte_len BIGINT NOT NULL,
  bytes BYTEA NOT NULL,
  inserted_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT artifact_blobs_byte_len_nonnegative CHECK (byte_len >= 0),
  CONSTRAINT artifact_blobs_byte_len_max CHECK (byte_len <= 16777216),
  CONSTRAINT artifact_blobs_byte_len_matches CHECK (octet_length(bytes) = byte_len),
  UNIQUE (digest),
  UNIQUE (artifact_id, digest),
  UNIQUE (artifact_id, digest, byte_len)
);

CREATE TABLE artifact_admissions (
  evidence_hash TEXT PRIMARY KEY,
  artifact_id TEXT NOT NULL,
  digest TEXT NOT NULL,
  byte_len BIGINT NOT NULL,
  media_type TEXT NOT NULL,
  schema_id TEXT NULL,
  semantic_type_id TEXT NULL,
  producer_node_id TEXT NULL,
  producer_seed_id TEXT NULL,
  artifact_role TEXT NOT NULL,
  evidence_canonical_json BYTEA NOT NULL,
  inserted_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT artifact_admissions_blob_fk FOREIGN KEY (artifact_id, digest, byte_len) REFERENCES artifact_blobs(artifact_id, digest, byte_len) ON DELETE RESTRICT,
  CONSTRAINT artifact_admissions_byte_len_nonnegative CHECK (byte_len >= 0),
  CONSTRAINT artifact_admissions_single_producer CHECK (
    producer_node_id IS NULL OR producer_seed_id IS NULL
  ),
  UNIQUE (artifact_id, evidence_hash),
  UNIQUE (evidence_hash, artifact_id, digest, byte_len)
);

CREATE TABLE commit_artifact_evidence (
  run_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  commit_key TEXT NOT NULL,
  commit_id TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  evidence_hash TEXT NOT NULL,
  binding_kind TEXT NOT NULL CHECK (binding_kind IN ('required', 'admitted')),
  requirement_source TEXT NULL,
  PRIMARY KEY (run_id, seq, binding_kind, artifact_id, evidence_hash),
  CONSTRAINT commit_artifact_evidence_commit_fk FOREIGN KEY (run_id, seq) REFERENCES commits(run_id, seq) ON DELETE RESTRICT,
  CONSTRAINT commit_artifact_evidence_commit_id_fk FOREIGN KEY (commit_id) REFERENCES commits(commit_id) ON DELETE RESTRICT,
  CONSTRAINT commit_artifact_evidence_artifact_fk FOREIGN KEY (artifact_id, evidence_hash) REFERENCES artifact_admissions(artifact_id, evidence_hash) ON DELETE RESTRICT,
  CONSTRAINT commit_artifact_evidence_seq_positive CHECK (seq >= 1),
  UNIQUE (run_id, seq, commit_key, commit_id, binding_kind, artifact_id, evidence_hash)
);

CREATE TABLE run_artifact_admissions (
  run_id TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  evidence_hash TEXT NOT NULL,
  first_commit_id TEXT NOT NULL,
  first_seq BIGINT NOT NULL,
  first_commit_key TEXT NOT NULL,
  first_binding_kind TEXT NOT NULL CHECK (first_binding_kind = 'admitted'),
  PRIMARY KEY (run_id, artifact_id, evidence_hash),
  CONSTRAINT run_artifact_admissions_commit_fk FOREIGN KEY (first_commit_id) REFERENCES commits(commit_id) ON DELETE RESTRICT,
  CONSTRAINT run_artifact_admissions_run_seq_fk FOREIGN KEY (run_id, first_seq) REFERENCES commits(run_id, seq) ON DELETE RESTRICT,
  CONSTRAINT run_artifact_admissions_artifact_fk FOREIGN KEY (artifact_id, evidence_hash) REFERENCES artifact_admissions(artifact_id, evidence_hash) ON DELETE RESTRICT,
  CONSTRAINT run_artifact_admissions_admitted_fk FOREIGN KEY (run_id, first_seq, first_commit_key, first_commit_id, first_binding_kind, artifact_id, evidence_hash)
    REFERENCES commit_artifact_evidence(run_id, seq, commit_key, commit_id, binding_kind, artifact_id, evidence_hash) ON DELETE RESTRICT,
  CONSTRAINT run_artifact_admissions_seq_positive CHECK (first_seq >= 1)
);

CREATE TABLE fact_descriptor_index (
  descriptor_hash TEXT PRIMARY KEY,
  descriptor_artifact_id TEXT NOT NULL,
  descriptor_artifact_evidence_hash TEXT NOT NULL,
  fact_kind TEXT NOT NULL,
  descriptor_schema_id TEXT NOT NULL,
  subject_schema_id TEXT NOT NULL,
  response_schema_id TEXT NOT NULL,
  fact_subject_namespace_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT fact_descriptor_index_artifact_fk FOREIGN KEY (descriptor_artifact_id, descriptor_artifact_evidence_hash) REFERENCES artifact_admissions(artifact_id, evidence_hash) ON DELETE RESTRICT,
  CONSTRAINT fact_descriptor_index_artifact_unique UNIQUE (descriptor_artifact_id, descriptor_artifact_evidence_hash),
  CONSTRAINT fact_descriptor_index_descriptor_artifact_unique UNIQUE (descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash)
);

CREATE TABLE run_fact_descriptor_admissions (
  run_id TEXT NOT NULL,
  descriptor_hash TEXT NOT NULL,
  descriptor_artifact_id TEXT NOT NULL,
  descriptor_artifact_evidence_hash TEXT NOT NULL,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  commit_id TEXT NOT NULL,
  PRIMARY KEY (run_id, descriptor_hash),
  CONSTRAINT run_fact_descriptor_admissions_descriptor_artifact_fk FOREIGN KEY (descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash) REFERENCES fact_descriptor_index(descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash) ON DELETE RESTRICT,
  CONSTRAINT run_fact_descriptor_admissions_event_fk FOREIGN KEY (run_id, source_seq, source_ordinal) REFERENCES run_events(run_id, seq, ordinal) ON DELETE RESTRICT,
  CONSTRAINT run_fact_descriptor_admissions_event_id_fk FOREIGN KEY (run_id, source_event_id) REFERENCES run_events(run_id, event_id) ON DELETE RESTRICT,
  CONSTRAINT run_fact_descriptor_admissions_commit_fk FOREIGN KEY (commit_id) REFERENCES commits(commit_id) ON DELETE RESTRICT,
  CONSTRAINT run_fact_descriptor_admissions_run_artifact_fk FOREIGN KEY (run_id, descriptor_artifact_id, descriptor_artifact_evidence_hash) REFERENCES run_artifact_admissions(run_id, artifact_id, evidence_hash) ON DELETE RESTRICT,
  CONSTRAINT run_fact_descriptor_admissions_seq_positive CHECK (source_seq >= 1),
  CONSTRAINT run_fact_descriptor_admissions_ordinal_nonnegative CHECK (source_ordinal >= 0)
);

CREATE TABLE fact_index (
  source_run_id TEXT NOT NULL,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  producer_node_id TEXT NOT NULL,
  commit_id TEXT NOT NULL,
  commit_key TEXT NOT NULL,
  store_commit_order BIGINT NOT NULL,
  recorded_at TEXT NOT NULL,
  observed_at TEXT NULL,
  audience TEXT NOT NULL,
  visibility_scope TEXT NOT NULL,
  fact_kind TEXT NOT NULL,
  fact_descriptor_hash TEXT NOT NULL,
  fact_subject_namespace_hash TEXT NOT NULL,
  fact_key TEXT NOT NULL,
  subject_material_hash TEXT NOT NULL,
  request_schema_id TEXT NULL,
  request_hash TEXT NULL,
  response_schema_id TEXT NOT NULL,
  response_hash TEXT NOT NULL,
  response_artifact_id TEXT NOT NULL,
  response_artifact_evidence_hash TEXT NOT NULL,
  capability_kind TEXT NOT NULL,
  capability_version TEXT NOT NULL,
  adapter_kind TEXT NOT NULL,
  adapter_version TEXT NOT NULL,
  PRIMARY KEY (source_run_id, source_seq, source_ordinal),
  CONSTRAINT fact_index_event_fk FOREIGN KEY (source_run_id, source_seq, source_ordinal) REFERENCES run_events(run_id, seq, ordinal) ON DELETE RESTRICT,
  CONSTRAINT fact_index_event_id_fk FOREIGN KEY (source_run_id, source_event_id) REFERENCES run_events(run_id, event_id) ON DELETE RESTRICT,
  CONSTRAINT fact_index_commit_fk FOREIGN KEY (commit_id) REFERENCES commits(commit_id) ON DELETE RESTRICT,
  CONSTRAINT fact_index_descriptor_fk FOREIGN KEY (fact_descriptor_hash) REFERENCES fact_descriptor_index(descriptor_hash) ON DELETE RESTRICT,
  CONSTRAINT fact_index_response_artifact_fk FOREIGN KEY (response_artifact_id, response_artifact_evidence_hash) REFERENCES artifact_admissions(artifact_id, evidence_hash) ON DELETE RESTRICT,
  CONSTRAINT fact_index_seq_positive CHECK (source_seq >= 1),
  CONSTRAINT fact_index_ordinal_nonnegative CHECK (source_ordinal >= 0),
  CONSTRAINT fact_index_store_commit_order_positive CHECK (store_commit_order >= 1),
  CONSTRAINT fact_index_audience_v1 CHECK (audience IN ('control', 'platform')),
  CONSTRAINT fact_index_visibility_scope_v1 CHECK (visibility_scope = 'default'),
  CONSTRAINT fact_index_request_pair CHECK ((request_schema_id IS NULL) = (request_hash IS NULL)),
  CONSTRAINT fact_index_response_artifact_unique UNIQUE (response_artifact_id, response_artifact_evidence_hash)
);

CREATE TABLE fact_index_terms (
  source_run_id TEXT NOT NULL,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  fact_descriptor_hash TEXT NOT NULL,
  field_id TEXT NOT NULL,
  source TEXT NOT NULL,
  value_type TEXT NOT NULL,
  value_text TEXT NULL,
  value_bool BOOLEAN NULL,
  value_i64 BIGINT NULL,
  value_u64 TEXT NULL,
  value_decimal TEXT NULL,
  value_timestamp TEXT NULL,
  value_digest TEXT NULL,
  unit TEXT NULL,
  scale INTEGER NULL,
  PRIMARY KEY (source_run_id, source_seq, source_ordinal, field_id),
  CONSTRAINT fact_index_terms_claim_fk FOREIGN KEY (source_run_id, source_seq, source_ordinal) REFERENCES fact_index(source_run_id, source_seq, source_ordinal) ON DELETE CASCADE,
  CONSTRAINT fact_index_terms_descriptor_fk FOREIGN KEY (fact_descriptor_hash) REFERENCES fact_descriptor_index(descriptor_hash) ON DELETE RESTRICT,
  CONSTRAINT fact_index_terms_seq_positive CHECK (source_seq >= 1),
  CONSTRAINT fact_index_terms_ordinal_nonnegative CHECK (source_ordinal >= 0),
  CONSTRAINT fact_index_terms_source_v1 CHECK (source IN ('subject', 'result', 'metadata')),
  CONSTRAINT fact_index_terms_value_type_v1 CHECK (
    value_type IN ('string', 'boolean', 'signed_integer', 'unsigned_integer', 'timestamp', 'decimal_string', 'digest')
  ),
  CONSTRAINT fact_index_terms_u64_range CHECK (
    value_u64 IS NULL
    OR (
      value_u64 ~ '^[0-9]+$'
      AND (
        length(value_u64) < 20
        OR (length(value_u64) = 20 AND value_u64 <= '18446744073709551615')
      )
    )
  ),
  CONSTRAINT fact_index_terms_value_shape CHECK (
    (value_type = 'string' AND value_text IS NOT NULL AND value_bool IS NULL AND value_i64 IS NULL AND value_u64 IS NULL AND value_decimal IS NULL AND value_timestamp IS NULL AND value_digest IS NULL)
    OR (value_type = 'boolean' AND value_text IS NULL AND value_bool IS NOT NULL AND value_i64 IS NULL AND value_u64 IS NULL AND value_decimal IS NULL AND value_timestamp IS NULL AND value_digest IS NULL)
    OR (value_type = 'signed_integer' AND value_text IS NULL AND value_bool IS NULL AND value_i64 IS NOT NULL AND value_u64 IS NULL AND value_decimal IS NULL AND value_timestamp IS NULL AND value_digest IS NULL)
    OR (value_type = 'unsigned_integer' AND value_text IS NULL AND value_bool IS NULL AND value_i64 IS NULL AND value_u64 IS NOT NULL AND value_decimal IS NULL AND value_timestamp IS NULL AND value_digest IS NULL)
    OR (value_type = 'timestamp' AND value_text IS NULL AND value_bool IS NULL AND value_i64 IS NULL AND value_u64 IS NULL AND value_decimal IS NULL AND value_timestamp IS NOT NULL AND value_digest IS NULL)
    OR (value_type = 'decimal_string' AND value_text IS NULL AND value_bool IS NULL AND value_i64 IS NULL AND value_u64 IS NULL AND value_decimal IS NOT NULL AND value_timestamp IS NULL AND value_digest IS NULL)
    OR (value_type = 'digest' AND value_text IS NULL AND value_bool IS NULL AND value_i64 IS NULL AND value_u64 IS NULL AND value_decimal IS NULL AND value_timestamp IS NULL AND value_digest IS NOT NULL)
  )
);

CREATE TABLE fact_projection_metadata (
  singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  projection_generation BIGINT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT fact_projection_metadata_generation_positive CHECK (projection_generation >= 1)
);

INSERT INTO fact_projection_metadata (projection_generation) VALUES (1);

CREATE INDEX fact_descriptor_index_kind_idx
ON fact_descriptor_index (fact_kind, descriptor_hash);

CREATE INDEX fact_index_descriptor_scope_idx
ON fact_index (fact_descriptor_hash, audience, visibility_scope, store_commit_order DESC, source_run_id, source_seq, source_ordinal);

CREATE INDEX fact_index_fact_key_idx
ON fact_index (fact_descriptor_hash, fact_key);

CREATE INDEX fact_index_response_artifact_idx
ON fact_index (response_artifact_id, response_artifact_evidence_hash);

CREATE INDEX fact_index_terms_text_idx
ON fact_index_terms (fact_descriptor_hash, field_id, value_text, source_run_id, source_seq, source_ordinal)
WHERE value_type = 'string';

CREATE INDEX fact_index_terms_bool_idx
ON fact_index_terms (fact_descriptor_hash, field_id, value_bool, source_run_id, source_seq, source_ordinal)
WHERE value_type = 'boolean';

CREATE INDEX fact_index_terms_i64_idx
ON fact_index_terms (fact_descriptor_hash, field_id, value_i64, source_run_id, source_seq, source_ordinal)
WHERE value_type = 'signed_integer';

CREATE INDEX fact_index_terms_u64_idx
ON fact_index_terms (fact_descriptor_hash, field_id, value_u64, source_run_id, source_seq, source_ordinal)
WHERE value_type = 'unsigned_integer';

CREATE INDEX fact_index_terms_decimal_idx
ON fact_index_terms (fact_descriptor_hash, field_id, value_decimal, source_run_id, source_seq, source_ordinal)
WHERE value_type = 'decimal_string';

CREATE INDEX fact_index_terms_timestamp_idx
ON fact_index_terms (fact_descriptor_hash, field_id, value_timestamp, source_run_id, source_seq, source_ordinal)
WHERE value_type = 'timestamp';

CREATE INDEX fact_index_terms_digest_idx
ON fact_index_terms (fact_descriptor_hash, field_id, value_digest, source_run_id, source_seq, source_ordinal)
WHERE value_type = 'digest';

CREATE TABLE admission_lane (
  class TEXT NOT NULL,
  lane_id BYTEA NOT NULL,
  mode TEXT NOT NULL,
  next_ticket BIGINT NULL,
  holder_token TEXT NULL,
  lease_expires_at TIMESTAMPTZ NULL,
  execution_run_id TEXT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  PRIMARY KEY (class, lane_id),
  CONSTRAINT admission_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT admission_lane_class_v1 CHECK (class IN ('resource_lane', 'execution_claim')),
  CONSTRAINT admission_lane_mode_v1 CHECK (mode IN ('wait_fifo', 'nowait_skip')),
  CONSTRAINT admission_lane_class_mode_v1 CHECK (
    (class = 'resource_lane' AND mode = 'wait_fifo')
    OR (class = 'execution_claim' AND mode = 'nowait_skip')
  ),
  CONSTRAINT admission_lane_wait_fifo_shape CHECK (
    mode <> 'wait_fifo'
    OR (
      next_ticket IS NOT NULL
      AND next_ticket >= 1
      AND holder_token IS NULL
      AND lease_expires_at IS NULL
    )
  ),
  CONSTRAINT admission_lane_nowait_skip_shape CHECK (
    mode <> 'nowait_skip'
    OR (
      next_ticket IS NULL
      AND ((holder_token IS NULL AND lease_expires_at IS NULL)
        OR (holder_token IS NOT NULL AND lease_expires_at IS NOT NULL))
    )
  ),
  CONSTRAINT admission_lane_execution_run_id_shape CHECK (
    (class = 'execution_claim' AND execution_run_id IS NOT NULL)
    OR (class <> 'execution_claim' AND execution_run_id IS NULL)
  ),
  CONSTRAINT admission_lane_execution_run_id_nonempty CHECK (
    execution_run_id IS NULL OR execution_run_id <> ''
  )
);

CREATE TABLE admission_waiter (
  class TEXT NOT NULL,
  lane_id BYTEA NOT NULL,
  lane_ticket BIGINT NOT NULL,
  waiter_id TEXT NOT NULL,
  token TEXT NOT NULL,
  status TEXT NOT NULL,
  enqueued_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  lease_expires_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (class, lane_id, lane_ticket),
  CONSTRAINT admission_waiter_lane_fk FOREIGN KEY (class, lane_id) REFERENCES admission_lane(class, lane_id) ON DELETE RESTRICT,
  CONSTRAINT admission_waiter_resource_wait_fifo_v1 CHECK (class = 'resource_lane'),
  CONSTRAINT admission_waiter_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT admission_waiter_lane_ticket_positive CHECK (lane_ticket >= 1),
  CONSTRAINT admission_waiter_id_nonempty CHECK (waiter_id <> ''),
  CONSTRAINT admission_waiter_token_nonempty CHECK (token <> ''),
  CONSTRAINT admission_waiter_status_v1 CHECK (status IN ('waiting', 'admitted', 'expired')),
  UNIQUE (waiter_id),
  UNIQUE (class, lane_id, token)
);

CREATE INDEX admission_waiter_live_fifo_idx
ON admission_waiter (class, lane_id, lane_ticket)
WHERE status = 'waiting';

CREATE INDEX admission_waiter_waiting_expiry_idx
ON admission_waiter (class, lane_id, lease_expires_at)
WHERE status = 'waiting';

CREATE INDEX admission_lane_expired_execution_claim_idx
ON admission_lane (lease_expires_at, execution_run_id)
WHERE class = 'execution_claim' AND holder_token IS NOT NULL;

CREATE TABLE run_observation_cursors (
  token_hash TEXT PRIMARY KEY,
  cursor_version TEXT NOT NULL,
  store_epoch TEXT NOT NULL,
  store_commit_order BIGINT NOT NULL,
  issued_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT run_observation_cursors_version_v3 CHECK (cursor_version = 'mfm.run_observation.cursor.v3'),
  CONSTRAINT run_observation_cursors_store_commit_order_nonnegative CHECK (store_commit_order >= 0)
);

CREATE TABLE configured_values (
  target TEXT COLLATE "C" PRIMARY KEY,
  schema_id TEXT COLLATE "C" NOT NULL,
  digest TEXT COLLATE "C" NOT NULL,
  canonical_json BYTEA NOT NULL,
  CONSTRAINT configured_values_target_bounds CHECK (
    octet_length(target) BETWEEN 1 AND 256
  ),
  CONSTRAINT configured_values_schema_id_bounds CHECK (
    octet_length(schema_id) BETWEEN 1 AND 1024
    AND schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:[a-z0-9][a-z0-9._/-]*:sha256-jcs-v1:[0-9a-f]{64}$'
  ),
  CONSTRAINT configured_values_digest_bounds CHECK (
    digest ~ '^content:sha256-jcs-v1:[0-9a-f]{64}$'
  ),
  CONSTRAINT configured_values_canonical_json_bounds CHECK (
    octet_length(canonical_json) BETWEEN 1 AND 262144
  )
);

CREATE TRIGGER store_metadata_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON store_metadata
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER commits_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON commits
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_events_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_events
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER artifact_blobs_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON artifact_blobs
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER artifact_admissions_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON artifact_admissions
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER commit_artifact_evidence_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON commit_artifact_evidence
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_artifact_admissions_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_artifact_admissions
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_observation_cursors_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_observation_cursors
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();
