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

CREATE FUNCTION mfm_allow_orphan_artifact_blob_delete_only() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF current_setting('mfm.maintenance_artifact_blob_sweep', true) IS DISTINCT FROM 'on' THEN
    RAISE EXCEPTION 'artifact blob deletes require maintenance orphan sweep';
  END IF;
  IF EXISTS (
    SELECT 1
    FROM artifact_admissions a
    WHERE a.artifact_id = OLD.artifact_id
      AND a.digest = OLD.digest
      AND a.byte_len = OLD.byte_len
  ) THEN
    RAISE EXCEPTION 'referenced artifact blob cannot be deleted';
  END IF;
  RETURN OLD;
END;
$$;

CREATE TABLE store_metadata (
  singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  store_epoch TEXT NOT NULL,
  schema_contract_version TEXT NOT NULL,
  cursor_key_id TEXT NOT NULL,
  cursor_secret BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

INSERT INTO store_metadata (store_epoch, schema_contract_version, cursor_key_id, cursor_secret)
VALUES (
  'mfm.store.epoch.v1:' || encode(public.gen_random_bytes(16), 'hex'),
  'mfm.postgres.run_store.v1',
  'mfm.cursor.key.v1',
  public.gen_random_bytes(32)
);

CREATE TABLE commits (
  commit_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  commit_key TEXT NOT NULL,
  commit_purpose TEXT NOT NULL,
  commit_idempotency_hash TEXT NOT NULL,
  idempotency_canonical_json BYTEA NOT NULL,
  prepared_authority_hash TEXT NOT NULL,
  prepared_authority_canonical_json BYTEA NOT NULL,
  commit_batch_hash TEXT NOT NULL,
  commit_batch_canonical_json BYTEA NOT NULL,
  hash_domain_version TEXT NOT NULL,
  canonicalizer_identity TEXT NOT NULL,
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
  evidence_schema_version TEXT NOT NULL,
  media_type TEXT NOT NULL,
  schema_id TEXT NULL,
  semantic_type_id TEXT NULL,
  producer_node_id TEXT NULL,
  producer_seed_id TEXT NULL,
  producer_descriptor_hash TEXT NULL,
  certified_spec_hash TEXT NULL,
  requirement_provenance_hash TEXT NULL,
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

CREATE TABLE resource_lane_claim_events (
  claim_id TEXT PRIMARY KEY,
  lane_id BYTEA NOT NULL,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  ledger_key TEXT NOT NULL,
  ledger_purpose JSONB NOT NULL,
  invocation_epoch INTEGER NOT NULL,
  namespace TEXT NOT NULL,
  key_schema_id TEXT NOT NULL,
  key_canonical_json BYTEA NOT NULL,
  key_value TEXT NOT NULL,
  mode TEXT NOT NULL CHECK (mode = 'exclusive'),
  requirement_digest TEXT NOT NULL,
  resolved_by_capability_impl TEXT NOT NULL,
  fencing_token BIGINT NOT NULL,
  commit_id TEXT NOT NULL REFERENCES commits(commit_id) ON DELETE RESTRICT,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  source_event_type TEXT NOT NULL CHECK (source_event_type = 'ResourceLaneClaimed'),
  source_event_payload_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT resource_lane_claim_events_source_fk FOREIGN KEY (run_id, source_seq, source_ordinal) REFERENCES run_events(run_id, seq, ordinal) ON DELETE RESTRICT,
  CONSTRAINT resource_lane_claim_events_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT resource_lane_claim_events_invocation_epoch_nonnegative CHECK (invocation_epoch >= 0),
  CONSTRAINT resource_lane_claim_events_source_seq_positive CHECK (source_seq >= 1),
  CONSTRAINT resource_lane_claim_events_source_ordinal_nonnegative CHECK (source_ordinal >= 0),
  CONSTRAINT resource_lane_claim_events_fencing_token_positive CHECK (fencing_token >= 1),
  UNIQUE (lane_id, fencing_token)
);

CREATE TABLE resource_lane_release_events (
  release_id TEXT PRIMARY KEY,
  claim_id TEXT NOT NULL REFERENCES resource_lane_claim_events(claim_id) ON DELETE RESTRICT,
  lane_id BYTEA NOT NULL,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  ledger_key TEXT NOT NULL,
  ledger_purpose JSONB NOT NULL,
  invocation_epoch INTEGER NOT NULL,
  claim_fencing_token BIGINT NOT NULL,
  release_reason TEXT NOT NULL,
  commit_id TEXT NOT NULL REFERENCES commits(commit_id) ON DELETE RESTRICT,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  source_event_type TEXT NOT NULL CHECK (source_event_type = 'ResourceLaneReleased'),
  source_event_payload_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT resource_lane_release_events_source_fk FOREIGN KEY (run_id, source_seq, source_ordinal) REFERENCES run_events(run_id, seq, ordinal) ON DELETE RESTRICT,
  CONSTRAINT resource_lane_release_events_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT resource_lane_release_events_invocation_epoch_nonnegative CHECK (invocation_epoch >= 0),
  CONSTRAINT resource_lane_release_events_source_seq_positive CHECK (source_seq >= 1),
  CONSTRAINT resource_lane_release_events_source_ordinal_nonnegative CHECK (source_ordinal >= 0),
  CONSTRAINT resource_lane_release_events_fencing_token_positive CHECK (claim_fencing_token >= 1),
  UNIQUE (claim_id)
);

CREATE TABLE resource_lane_transitions (
  lane_id BYTEA NOT NULL,
  lane_transition_seq BIGINT NOT NULL,
  transition_kind TEXT NOT NULL CHECK (transition_kind IN ('claim', 'release')),
  claim_id TEXT NOT NULL REFERENCES resource_lane_claim_events(claim_id) ON DELETE RESTRICT,
  release_id TEXT NULL REFERENCES resource_lane_release_events(release_id) ON DELETE RESTRICT,
  claim_fencing_token BIGINT NOT NULL,
  previous_transition_hash TEXT NULL,
  transition_hash TEXT NOT NULL,
  run_id TEXT NOT NULL,
  commit_id TEXT NOT NULL REFERENCES commits(commit_id) ON DELETE RESTRICT,
  source_seq BIGINT NOT NULL,
  source_ordinal INTEGER NOT NULL,
  source_event_id TEXT NOT NULL,
  source_event_type TEXT NOT NULL,
  source_event_payload_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  PRIMARY KEY (lane_id, lane_transition_seq),
  CONSTRAINT resource_lane_transitions_source_fk FOREIGN KEY (run_id, source_seq, source_ordinal) REFERENCES run_events(run_id, seq, ordinal) ON DELETE RESTRICT,
  CONSTRAINT resource_lane_transitions_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT resource_lane_transitions_seq_positive CHECK (lane_transition_seq >= 1),
  CONSTRAINT resource_lane_transitions_fencing_token_positive CHECK (claim_fencing_token >= 1),
  CONSTRAINT resource_lane_transitions_kind_shape CHECK (
    (transition_kind = 'claim' AND release_id IS NULL AND source_event_type = 'ResourceLaneClaimed')
    OR
    (transition_kind = 'release' AND release_id IS NOT NULL AND source_event_type = 'ResourceLaneReleased')
  ),
  UNIQUE (transition_hash),
  UNIQUE (claim_id, transition_kind),
  UNIQUE (release_id)
);

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

CREATE TABLE run_observation_change_summaries (
  projection_version TEXT NOT NULL,
  commit_id TEXT NOT NULL REFERENCES run_commit_log(commit_id) ON DELETE RESTRICT,
  summary_kind TEXT NOT NULL CHECK (summary_kind = 'run'),
  run_id TEXT NOT NULL,
  head_seq BIGINT NOT NULL,
  observed_status TEXT NOT NULL CHECK (observed_status IN ('started', 'completed')),
  started_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  completed_at TIMESTAMPTZ NULL,
  source_authority_hash TEXT NOT NULL,
  source_event_count BIGINT NOT NULL CHECK (source_event_count >= 1),
  summary_row_hash TEXT NOT NULL,
  summary_row_canonical_json BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  PRIMARY KEY (projection_version, commit_id, summary_kind),
  CONSTRAINT run_observation_change_summaries_commit_fk FOREIGN KEY (run_id, head_seq) REFERENCES commits(run_id, seq) ON DELETE RESTRICT
);

CREATE VIEW current_run_observations AS
SELECT DISTINCT ON (run_id)
  projection_version,
  run_id,
  head_seq,
  observed_status,
  started_at,
  updated_at,
  completed_at,
  commit_id
FROM run_observation_change_summaries
WHERE projection_version = 'mfm.run_observation.v1' AND summary_kind = 'run'
ORDER BY run_id, head_seq DESC;

CREATE TABLE run_observation_cursors (
  token_hash TEXT PRIMARY KEY,
  cursor_key_id TEXT NOT NULL,
  store_epoch TEXT NOT NULL,
  cursor_kind TEXT NOT NULL CHECK (cursor_kind IN ('frontier', 'row')),
  append_xid XID8 NOT NULL,
  commit_sort_key BYTEA NOT NULL,
  issued_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
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

CREATE TRIGGER artifact_blobs_no_update_or_truncate
BEFORE UPDATE OR TRUNCATE ON artifact_blobs
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER artifact_blobs_orphan_delete_only
BEFORE DELETE ON artifact_blobs
FOR EACH ROW EXECUTE FUNCTION mfm_allow_orphan_artifact_blob_delete_only();

CREATE TRIGGER artifact_admissions_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON artifact_admissions
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER commit_artifact_evidence_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON commit_artifact_evidence
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_artifact_admissions_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_artifact_admissions
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER resource_lane_claim_events_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON resource_lane_claim_events
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER resource_lane_release_events_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON resource_lane_release_events
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER resource_lane_transitions_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON resource_lane_transitions
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_commit_log_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_commit_log
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_observation_change_summaries_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_observation_change_summaries
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER run_observation_cursors_no_update
BEFORE UPDATE OR DELETE OR TRUNCATE ON run_observation_cursors
FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();
