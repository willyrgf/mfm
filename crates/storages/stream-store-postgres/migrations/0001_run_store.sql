CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;

CREATE FUNCTION mfm_set_append_xid() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  NEW.append_xid = pg_current_xact_id();
  RETURN NEW;
END;
$$;

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
  schema_contract_version TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

INSERT INTO store_metadata (store_epoch, schema_contract_version)
VALUES (
  'mfm.store.epoch.v1:' || encode(public.gen_random_bytes(16), 'hex'),
  'mfm.postgres.run_store.v1'
);

CREATE TABLE commits (
  commit_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  commit_key TEXT NOT NULL,
  commit_purpose TEXT NOT NULL,
  prepared_commit_plan_fingerprint TEXT NOT NULL,
  commit_batch_hash TEXT NOT NULL,
  event_count INTEGER NOT NULL,
  append_xid XID8 NOT NULL DEFAULT pg_current_xact_id(),
  committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  PRIMARY KEY (run_id, seq),
  UNIQUE (commit_id),
  UNIQUE (run_id, commit_key),
  CONSTRAINT commits_seq_positive CHECK (seq >= 1),
  CONSTRAINT commits_event_count_positive CHECK (event_count >= 1)
);

CREATE TRIGGER commits_set_append_xid
BEFORE INSERT ON commits
FOR EACH ROW EXECUTE FUNCTION mfm_set_append_xid();

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

CREATE TABLE resource_lane_waiter_counters (
  lane_id BYTEA PRIMARY KEY,
  next_ticket BIGINT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT resource_lane_waiter_counters_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT resource_lane_waiter_counters_next_ticket_positive CHECK (next_ticket >= 1)
);

CREATE TABLE resource_lane_waiters (
  waiter_id TEXT PRIMARY KEY,
  lane_id BYTEA NOT NULL,
  lane_ticket BIGINT NOT NULL,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  ledger_key TEXT NOT NULL,
  invocation_epoch INTEGER NOT NULL,
  claim_fingerprint TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('waiting', 'claimed', 'cancelled', 'expired')),
  enqueued_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  lease_expires_at TIMESTAMPTZ NOT NULL,
  CONSTRAINT resource_lane_waiters_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT resource_lane_waiters_lane_ticket_positive CHECK (lane_ticket >= 1),
  CONSTRAINT resource_lane_waiters_invocation_epoch_nonnegative CHECK (invocation_epoch >= 0),
  UNIQUE (lane_id, lane_ticket),
  UNIQUE (lane_id, claim_fingerprint)
);

CREATE INDEX resource_lane_waiters_live_fifo_idx
ON resource_lane_waiters (lane_id, status, lane_ticket)
WHERE status = 'waiting';

CREATE INDEX resource_lane_waiters_expiry_idx
ON resource_lane_waiters (status, lease_expires_at)
WHERE status = 'waiting';

CREATE TABLE run_commit_log (
  commit_id TEXT PRIMARY KEY REFERENCES commits(commit_id) ON DELETE RESTRICT,
  run_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  commit_key TEXT NOT NULL,
  first_ordinal INTEGER NOT NULL DEFAULT 0 CHECK (first_ordinal = 0),
  last_ordinal INTEGER NOT NULL,
  event_count INTEGER NOT NULL,
  commit_sort_key BYTEA NOT NULL,
  append_xid XID8 NOT NULL DEFAULT pg_current_xact_id(),
  committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT run_commit_log_commit_fk FOREIGN KEY (run_id, seq) REFERENCES commits(run_id, seq) ON DELETE RESTRICT,
  CONSTRAINT run_commit_log_seq_positive CHECK (seq >= 1),
  CONSTRAINT run_commit_log_event_count_positive CHECK (event_count >= 1),
  CONSTRAINT run_commit_log_last_ordinal_matches CHECK (last_ordinal = event_count - 1),
  CONSTRAINT run_commit_log_sort_key_v1_length CHECK (octet_length(commit_sort_key) = 32),
  CONSTRAINT run_commit_log_sort_key_v1_prefix CHECK (get_byte(commit_sort_key, 0) = 1),
  CONSTRAINT run_commit_log_sort_key_not_sentinel CHECK (commit_sort_key <> decode(repeat('00', 32), 'hex')),
  UNIQUE (append_xid, commit_sort_key)
);

CREATE TRIGGER run_commit_log_set_append_xid
BEFORE INSERT ON run_commit_log
FOR EACH ROW EXECUTE FUNCTION mfm_set_append_xid();

CREATE TABLE run_observation_cursors (
  token_hash TEXT PRIMARY KEY,
  cursor_version TEXT NOT NULL,
  store_epoch TEXT NOT NULL,
  cursor_kind TEXT NOT NULL CHECK (cursor_kind IN ('frontier', 'row')),
  append_xid XID8 NOT NULL,
  commit_sort_key BYTEA NOT NULL,
  issued_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT run_observation_cursors_version_v1 CHECK (cursor_version = 'mfm.run_observation.cursor.v1'),
  CONSTRAINT run_observation_cursors_sort_key_v1_length CHECK (octet_length(commit_sort_key) = 32)
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

CREATE TRIGGER run_commit_log_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_commit_log
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_observation_cursors_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_observation_cursors
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();
