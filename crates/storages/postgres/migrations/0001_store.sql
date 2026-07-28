-- MFM recoverability-v1 PostgreSQL authority.
--
-- This is a destructive pre-production baseline, not an upgrade from the retired event,
-- projection, global-order, lane, or fact-query schemas. A database carrying an earlier
-- 0001 checksum is rejected by the compiled migration ledger and must be exported or reset.

DO $mfm_roles$
BEGIN
    IF current_user = 'mfm_store_application' THEN
        RAISE EXCEPTION 'the migration must not run as the application role';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_store_owner') THEN
        CREATE ROLE mfm_store_owner
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    ELSE
        ALTER ROLE mfm_store_owner
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_store_application') THEN
        CREATE ROLE mfm_store_application
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    ELSE
        ALTER ROLE mfm_store_application
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    END IF;

    EXECUTE format('GRANT mfm_store_owner TO %I', current_user);
    EXECUTE format('GRANT mfm_store_application TO %I', current_user);
END
$mfm_roles$;

DO $mfm_schema_acl$
DECLARE
    schema_name text := current_schema();
BEGIN
    EXECUTE format('REVOKE ALL ON SCHEMA %I FROM PUBLIC', schema_name);
    EXECUTE format('GRANT USAGE, CREATE ON SCHEMA %I TO mfm_store_owner', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_store_application', schema_name);
END
$mfm_schema_acl$;

CREATE TABLE store_identity (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    store_scope_id TEXT NOT NULL UNIQUE,
    store_epoch NUMERIC(20, 0) NOT NULL,
    CONSTRAINT store_identity_scope_v1 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT store_identity_singleton_v1 CHECK (singleton),
    CONSTRAINT store_identity_epoch_v1 CHECK (
        store_epoch >= 0
        AND store_epoch <= 18446744073709551615::numeric
        AND trunc(store_epoch) = store_epoch
    )
);

CREATE TABLE store_schema_metadata (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    schema_contract_version TEXT NOT NULL,
    CONSTRAINT store_schema_metadata_version_v1 CHECK (
        schema_contract_version = 'mfm.recoverability-postgres.v1'
    ),
    CONSTRAINT store_schema_metadata_singleton_v1 CHECK (singleton)
);

WITH random_identity AS (
    SELECT
        replace(gen_random_uuid()::text, '-', '') AS scope_hex,
        replace(gen_random_uuid()::text, '-', '') AS epoch_hex
),
identity_parts AS (
    SELECT
        scope_hex,
        ('x' || substr(epoch_hex, 1, 8))::bit(32)::bigint::numeric AS epoch_high,
        ('x' || substr(epoch_hex, 9, 8))::bit(32)::bigint::numeric AS epoch_low
    FROM random_identity
)
INSERT INTO store_identity (singleton, store_scope_id, store_epoch)
SELECT
    TRUE,
    'mfm.store_scope.v1:' || scope_hex,
    CASE
        WHEN epoch_high = 0 AND epoch_low = 0 THEN 1::numeric
        ELSE epoch_high * 4294967296::numeric + epoch_low
    END
FROM identity_parts;

INSERT INTO store_schema_metadata (singleton, schema_contract_version)
VALUES (TRUE, 'mfm.recoverability-postgres.v1');

CREATE TABLE tenant_fact_order_heads (
    tenant_scope_id TEXT PRIMARY KEY,
    current_fact_order NUMERIC(20, 0) NOT NULL DEFAULT 0,
    CONSTRAINT tenant_fact_order_heads_tenant_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT tenant_fact_order_heads_order_v1 CHECK (
        current_fact_order >= 0
        AND current_fact_order <= 18446744073709551615::numeric
        AND trunc(current_fact_order) = current_fact_order
    )
);

CREATE TABLE journal_commits (
    run_id TEXT NOT NULL,
    run_sequence NUMERIC(20, 0) NOT NULL,
    append_request_id TEXT NOT NULL,
    candidate_digest TEXT NOT NULL,
    predecessor_kind TEXT NOT NULL,
    predecessor_run_sequence NUMERIC(20, 0),
    predecessor_commit_digest TEXT NOT NULL,
    commit_digest TEXT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    admission_entry_point_operation_id TEXT,
    admission_invocation_identity TEXT,
    tenant_fact_coordinate_kind TEXT NOT NULL,
    tenant_fact_order NUMERIC(20, 0),
    record_count SMALLINT NOT NULL,
    committed_at NUMERIC(20, 0) NOT NULL,
    PRIMARY KEY (run_id, run_sequence),
    CONSTRAINT journal_commits_append_request_key UNIQUE (run_id, append_request_id),
    CONSTRAINT journal_commits_commit_digest_key UNIQUE (commit_digest),
    CONSTRAINT journal_commits_run_id_v1 CHECK (
        run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_commits_sequence_v1 CHECK (
        run_sequence >= 1
        AND run_sequence <= 18446744073709551615::numeric
        AND trunc(run_sequence) = run_sequence
    ),
    CONSTRAINT journal_commits_append_request_id_v1 CHECK (
        octet_length(append_request_id) BETWEEN 1 AND 512
        AND append_request_id ~ '^[a-z0-9][a-z0-9._/-]*$'
    ),
    CONSTRAINT journal_commits_candidate_digest_v1 CHECK (
        candidate_digest ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_commits_predecessor_digest_v1 CHECK (
        predecessor_commit_digest ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_commits_commit_digest_v1 CHECK (
        commit_digest ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_commits_tenant_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT journal_commits_admission_key_shape_v1 CHECK (
        (
            predecessor_kind = 'genesis'
            AND admission_entry_point_operation_id IS NOT NULL
            AND octet_length(admission_entry_point_operation_id) BETWEEN 1 AND 512
            AND admission_entry_point_operation_id ~ '^[a-z0-9][a-z0-9._/-]*$'
            AND admission_invocation_identity IS NOT NULL
            AND admission_invocation_identity ~
                '^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
        )
        OR
        (
            predecessor_kind = 'journal_head'
            AND admission_entry_point_operation_id IS NULL
            AND admission_invocation_identity IS NULL
        )
    ),
    CONSTRAINT journal_commits_predecessor_shape_v1 CHECK (
        (
            predecessor_kind = 'genesis'
            AND run_sequence = 1
            AND predecessor_run_sequence IS NULL
        )
        OR
        (
            predecessor_kind = 'journal_head'
            AND run_sequence > 1
            AND predecessor_run_sequence IS NOT NULL
            AND predecessor_run_sequence = run_sequence - 1
        )
    ),
    CONSTRAINT journal_commits_coordinate_shape_v1 CHECK (
        (
            tenant_fact_coordinate_kind = 'none'
            AND tenant_fact_order IS NULL
        )
        OR
        (
            tenant_fact_coordinate_kind = 'fact_publication'
            AND tenant_fact_order IS NOT NULL
            AND tenant_fact_order BETWEEN 1 AND 18446744073709551615::numeric
            AND trunc(tenant_fact_order) = tenant_fact_order
        )
        OR
        (
            tenant_fact_coordinate_kind = 'fact_selection_barrier'
            AND tenant_fact_order IS NOT NULL
            AND tenant_fact_order BETWEEN 0 AND 18446744073709551615::numeric
            AND trunc(tenant_fact_order) = tenant_fact_order
        )
    ),
    CONSTRAINT journal_commits_record_count_v1 CHECK (record_count BETWEEN 1 AND 2),
    CONSTRAINT journal_commits_committed_at_v1 CHECK (
        committed_at >= 0
        AND committed_at <= 18446744073709551615::numeric
        AND trunc(committed_at) = committed_at
    )
);

CREATE UNIQUE INDEX journal_commits_tenant_publication_order_key
    ON journal_commits (tenant_scope_id, tenant_fact_order)
    WHERE tenant_fact_coordinate_kind = 'fact_publication';

CREATE UNIQUE INDEX journal_commits_admission_logical_key
    ON journal_commits (
        tenant_scope_id,
        admission_entry_point_operation_id,
        admission_invocation_identity
    )
    WHERE admission_entry_point_operation_id IS NOT NULL
      AND admission_invocation_identity IS NOT NULL;

CREATE INDEX journal_commits_tenant_barrier_idx
    ON journal_commits (tenant_scope_id, tenant_fact_order, run_id, run_sequence)
    WHERE tenant_fact_coordinate_kind = 'fact_selection_barrier';

CREATE TABLE journal_records (
    run_id TEXT NOT NULL,
    run_sequence NUMERIC(20, 0) NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    fact_order NUMERIC(20, 0),
    ordinal INTEGER NOT NULL,
    record_id TEXT NOT NULL UNIQUE,
    record_schema_id TEXT NOT NULL,
    spec_hash TEXT NOT NULL,
    logical_key BYTEA NOT NULL,
    record_hash TEXT NOT NULL,
    canonical_payload BYTEA NOT NULL,
    emits_facts BOOLEAN NOT NULL,
    PRIMARY KEY (run_id, run_sequence, ordinal),
    CONSTRAINT journal_records_commit_fk
        FOREIGN KEY (run_id, run_sequence)
        REFERENCES journal_commits (run_id, run_sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT journal_records_run_id_v1 CHECK (
        run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_records_sequence_v1 CHECK (
        run_sequence >= 1
        AND run_sequence <= 18446744073709551615::numeric
        AND trunc(run_sequence) = run_sequence
    ),
    CONSTRAINT journal_records_tenant_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT journal_records_fact_order_v1 CHECK (
        (
            emits_facts
            AND fact_order IS NOT NULL
            AND fact_order BETWEEN 1 AND 18446744073709551615::numeric
            AND trunc(fact_order) = fact_order
        )
        OR
        (NOT emits_facts AND fact_order IS NULL)
    ),
    CONSTRAINT journal_records_ordinal_v1 CHECK (ordinal BETWEEN 0 AND 1),
    CONSTRAINT journal_records_record_id_v1 CHECK (
        record_id ~ '^record:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_records_schema_id_v1 CHECK (
        octet_length(record_schema_id) BETWEEN 90 AND 512
        AND record_schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_records_spec_hash_v1 CHECK (
        spec_hash ~ '^spec:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_records_logical_key_v1 CHECK (
        octet_length(logical_key) BETWEEN 2 AND 4096
    ),
    CONSTRAINT journal_records_record_hash_v1 CHECK (
        record_hash ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT journal_records_payload_bounds_v1 CHECK (
        octet_length(canonical_payload) BETWEEN 2 AND 16777216
    ),
    CONSTRAINT journal_records_logical_key_key UNIQUE (run_id, logical_key)
);

CREATE INDEX journal_records_fact_scan_idx
    ON journal_records (tenant_scope_id, fact_order, run_id, run_sequence, ordinal)
    WHERE emits_facts AND fact_order IS NOT NULL;

CREATE TABLE artifact_blobs (
    content_digest TEXT PRIMARY KEY,
    byte_length NUMERIC(20, 0) NOT NULL,
    bytes BYTEA NOT NULL,
    CONSTRAINT artifact_blobs_content_digest_v1 CHECK (
        content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT artifact_blobs_byte_length_v1 CHECK (
        byte_length >= 0
        AND byte_length <= 16777216
        AND trunc(byte_length) = byte_length
        AND byte_length = octet_length(bytes)::numeric
    )
);

CREATE TABLE artifact_admissions (
    artifact_id TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    schema_id TEXT NOT NULL,
    semantic_type_id TEXT NOT NULL,
    role TEXT NOT NULL,
    byte_length NUMERIC(20, 0) NOT NULL,
    media_type TEXT NOT NULL,
    evidence_contract_schema_id TEXT NOT NULL,
    evidence_contract_content_digest TEXT NOT NULL,
    canonical_value_ref BYTEA NOT NULL,
    PRIMARY KEY (canonical_value_ref),
    CONSTRAINT artifact_admissions_identity_key
        UNIQUE (
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        ),
    CONSTRAINT artifact_admissions_blob_fk
        FOREIGN KEY (content_digest)
        REFERENCES artifact_blobs (content_digest)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT artifact_admissions_artifact_id_v1 CHECK (
        artifact_id ~ '^artifact:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT artifact_admissions_evidence_hash_v1 CHECK (
        evidence_hash ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT artifact_admissions_content_digest_v1 CHECK (
        content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT artifact_admissions_schema_id_v1 CHECK (
        octet_length(schema_id) BETWEEN 90 AND 512
        AND schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT artifact_admissions_semantic_type_v1 CHECK (
        octet_length(semantic_type_id) BETWEEN 78 AND 512
    ),
    CONSTRAINT artifact_admissions_role_v1 CHECK (
        octet_length(role) BETWEEN 1 AND 512
        AND role ~ '^[a-z0-9][a-z0-9._/-]*$'
    ),
    CONSTRAINT artifact_admissions_byte_length_v1 CHECK (
        byte_length >= 0
        AND byte_length <= 16777216
        AND trunc(byte_length) = byte_length
    ),
    CONSTRAINT artifact_admissions_media_type_v1 CHECK (
        octet_length(media_type) BETWEEN 3 AND 256
        AND media_type = lower(media_type)
        AND media_type ~ '^[a-z0-9!#$&^_.+-]+/[a-z0-9!#$&^_.+-]+$'
    ),
    CONSTRAINT artifact_admissions_contract_schema_id_v1 CHECK (
        octet_length(evidence_contract_schema_id) BETWEEN 90 AND 512
        AND evidence_contract_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT artifact_admissions_contract_digest_v1 CHECK (
        evidence_contract_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT artifact_admissions_value_ref_bounds_v1 CHECK (
        octet_length(canonical_value_ref) BETWEEN 2 AND 65536
    )
);

CREATE INDEX artifact_admissions_routing_idx
    ON artifact_admissions (artifact_id, evidence_hash, content_digest);

CREATE TABLE commit_object_authorities (
    run_id TEXT NOT NULL,
    run_sequence NUMERIC(20, 0) NOT NULL,
    artifact_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    canonical_value_ref BYTEA NOT NULL,
    admission_mode TEXT NOT NULL,
    PRIMARY KEY (run_id, run_sequence, canonical_value_ref),
    CONSTRAINT commit_object_authorities_identity_key UNIQUE (
        run_id,
        run_sequence,
        canonical_value_ref,
        artifact_id,
        evidence_hash,
        content_digest
    ),
    CONSTRAINT commit_object_authorities_commit_fk
        FOREIGN KEY (run_id, run_sequence)
        REFERENCES journal_commits (run_id, run_sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT commit_object_authorities_admission_fk
        FOREIGN KEY (
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        )
        REFERENCES artifact_admissions (
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        )
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT commit_object_authorities_run_id_v1 CHECK (
        run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT commit_object_authorities_sequence_v1 CHECK (
        run_sequence >= 1
        AND run_sequence <= 18446744073709551615::numeric
        AND trunc(run_sequence) = run_sequence
    ),
    CONSTRAINT commit_object_authorities_artifact_id_v1 CHECK (
        artifact_id ~ '^artifact:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT commit_object_authorities_content_digest_v1 CHECK (
        content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT commit_object_authorities_evidence_hash_v1 CHECK (
        evidence_hash ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT commit_object_authorities_value_ref_bounds_v1 CHECK (
        octet_length(canonical_value_ref) BETWEEN 2 AND 65536
    ),
    CONSTRAINT commit_object_authorities_admission_mode_v1 CHECK (
        admission_mode IN ('require_existing', 'admit_or_verify_exact')
    )
);

CREATE INDEX commit_object_authorities_identity_idx
    ON commit_object_authorities (artifact_id, evidence_hash, content_digest);

CREATE TABLE commit_artifact_bindings (
    run_id TEXT NOT NULL,
    run_sequence NUMERIC(20, 0) NOT NULL,
    record_ordinal INTEGER NOT NULL,
    field_path TEXT NOT NULL,
    authority_use TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    canonical_value_ref BYTEA NOT NULL,
    PRIMARY KEY (run_id, run_sequence, record_ordinal, field_path),
    CONSTRAINT commit_artifact_bindings_commit_fk
        FOREIGN KEY (run_id, run_sequence)
        REFERENCES journal_commits (run_id, run_sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT commit_artifact_bindings_record_fk
        FOREIGN KEY (run_id, run_sequence, record_ordinal)
        REFERENCES journal_records (run_id, run_sequence, ordinal)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT commit_artifact_bindings_authority_fk
        FOREIGN KEY (
            run_id,
            run_sequence,
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        )
        REFERENCES commit_object_authorities (
            run_id,
            run_sequence,
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        )
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT commit_artifact_bindings_field_path_v1 CHECK (
        octet_length(field_path) BETWEEN 1 AND 1024
    ),
    CONSTRAINT commit_artifact_bindings_authority_use_v1 CHECK (
        authority_use IN ('preexisting', 'produced_here')
    ),
    CONSTRAINT commit_artifact_bindings_artifact_id_v1 CHECK (
        artifact_id ~ '^artifact:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT commit_artifact_bindings_content_digest_v1 CHECK (
        content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT commit_artifact_bindings_evidence_hash_v1 CHECK (
        evidence_hash ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT commit_artifact_bindings_value_ref_bounds_v1 CHECK (
        octet_length(canonical_value_ref) BETWEEN 2 AND 65536
    )
);

CREATE INDEX commit_artifact_bindings_identity_idx
    ON commit_artifact_bindings (
        run_id,
        run_sequence,
        artifact_id,
        evidence_hash,
        content_digest
    );

CREATE TABLE qualified_support_members (
    qualification_scope_id TEXT NOT NULL,
    field_path TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    canonical_value_ref BYTEA NOT NULL,
    PRIMARY KEY (qualification_scope_id, field_path),
    CONSTRAINT qualified_support_members_object_fk
        FOREIGN KEY (canonical_value_ref, artifact_id, evidence_hash, content_digest)
        REFERENCES artifact_admissions (
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        )
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT qualified_support_members_scope_v1 CHECK (
        octet_length(qualification_scope_id) BETWEEN 90 AND 1024
        AND qualification_scope_id ~
            '^semantic:[a-z0-9][a-z0-9._/-]*:[a-z0-9][a-z0-9._/-]*:[a-z0-9][a-z0-9._/-]*:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT qualified_support_members_field_path_v1 CHECK (
        octet_length(field_path) BETWEEN 1 AND 1024
    ),
    CONSTRAINT qualified_support_members_artifact_id_v1 CHECK (
        artifact_id ~ '^artifact:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT qualified_support_members_content_digest_v1 CHECK (
        content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT qualified_support_members_evidence_hash_v1 CHECK (
        evidence_hash ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT qualified_support_members_value_ref_bounds_v1 CHECK (
        octet_length(canonical_value_ref) BETWEEN 2 AND 65536
    )
);

CREATE INDEX qualified_support_members_identity_idx
    ON qualified_support_members (artifact_id, evidence_hash, content_digest);

CREATE TABLE fact_scan_attestations (
    tenant_scope_id TEXT NOT NULL,
    consuming_run_id TEXT NOT NULL,
    containing_run_sequence NUMERIC(20, 0) NOT NULL,
    authorization_ref BYTEA PRIMARY KEY,
    attestation_artifact_id TEXT NOT NULL,
    attestation_content_digest TEXT NOT NULL,
    attestation_evidence_hash TEXT NOT NULL,
    attestation_ref BYTEA NOT NULL,
    observation_ref BYTEA NOT NULL UNIQUE,
    containing_journal_head BYTEA NOT NULL,
    CONSTRAINT fact_scan_attestations_commit_key
        UNIQUE (consuming_run_id, containing_run_sequence),
    CONSTRAINT fact_scan_attestations_commit_fk
        FOREIGN KEY (consuming_run_id, containing_run_sequence)
        REFERENCES journal_commits (run_id, run_sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT fact_scan_attestations_object_fk
        FOREIGN KEY (
            attestation_ref,
            attestation_artifact_id,
            attestation_evidence_hash,
            attestation_content_digest
        )
        REFERENCES artifact_admissions (
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        )
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT fact_scan_attestations_tenant_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT fact_scan_attestations_run_id_v1 CHECK (
        consuming_run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT fact_scan_attestations_sequence_v1 CHECK (
        containing_run_sequence >= 1
        AND containing_run_sequence <= 18446744073709551615::numeric
        AND trunc(containing_run_sequence) = containing_run_sequence
    ),
    CONSTRAINT fact_scan_attestations_artifact_id_v1 CHECK (
        attestation_artifact_id ~ '^artifact:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT fact_scan_attestations_content_digest_v1 CHECK (
        attestation_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT fact_scan_attestations_evidence_hash_v1 CHECK (
        attestation_evidence_hash ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT fact_scan_attestations_authorization_bounds_v1 CHECK (
        octet_length(authorization_ref) BETWEEN 2 AND 4096
    ),
    CONSTRAINT fact_scan_attestations_attestation_bounds_v1 CHECK (
        octet_length(attestation_ref) BETWEEN 2 AND 65536
    ),
    CONSTRAINT fact_scan_attestations_observation_bounds_v1 CHECK (
        octet_length(observation_ref) BETWEEN 2 AND 4096
    ),
    CONSTRAINT fact_scan_attestations_head_bounds_v1 CHECK (
        octet_length(containing_journal_head) BETWEEN 2 AND 2048
    )
);

CREATE INDEX fact_scan_attestations_run_idx
    ON fact_scan_attestations (
        tenant_scope_id,
        consuming_run_id,
        containing_run_sequence
    );

CREATE TABLE configured_values (
    store_scope_id TEXT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    entry_point_id TEXT NOT NULL,
    target TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    canonical_value_ref BYTEA NOT NULL,
    canonical_binding BYTEA NOT NULL,
    PRIMARY KEY (store_scope_id, tenant_scope_id, entry_point_id, target),
    CONSTRAINT configured_values_store_identity_fk
        FOREIGN KEY (store_scope_id)
        REFERENCES store_identity (store_scope_id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT configured_values_object_fk
        FOREIGN KEY (canonical_value_ref, artifact_id, evidence_hash, content_digest)
        REFERENCES artifact_admissions (
            canonical_value_ref,
            artifact_id,
            evidence_hash,
            content_digest
        )
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT configured_values_store_scope_v1 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT configured_values_tenant_scope_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT configured_values_entry_point_v1 CHECK (
        octet_length(entry_point_id) BETWEEN 7 AND 512
    ),
    CONSTRAINT configured_values_target_v1 CHECK (
        octet_length(target) BETWEEN 1 AND 512
        AND target ~ '^[a-z0-9][a-z0-9._/-]*$'
    ),
    CONSTRAINT configured_values_artifact_id_v1 CHECK (
        artifact_id ~ '^artifact:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT configured_values_content_digest_v1 CHECK (
        content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT configured_values_evidence_hash_v1 CHECK (
        evidence_hash ~ '^sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT configured_values_value_ref_bounds_v1 CHECK (
        octet_length(canonical_value_ref) BETWEEN 2 AND 65536
    ),
    CONSTRAINT configured_values_binding_bounds_v1 CHECK (
        octet_length(canonical_binding) BETWEEN 2 AND 131072
    )
);

CREATE FUNCTION mfm_reject_authority_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $mfm_function$
BEGIN
    RAISE EXCEPTION 'mfm authority tables are append-only';
END
$mfm_function$;

CREATE FUNCTION mfm_reject_store_identity_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $mfm_function$
BEGIN
    RAISE EXCEPTION 'mfm store identity and schema metadata are immutable';
END
$mfm_function$;

CREATE FUNCTION mfm_guard_tenant_fact_head_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $mfm_function$
BEGIN
    IF current_setting('mfm.internal_tenant_fact_head_mutation', TRUE)
        IS DISTINCT FROM 'enabled'
    THEN
        RAISE EXCEPTION 'tenant fact heads may change only through the sealed allocator';
    END IF;
    RETURN NULL;
END
$mfm_function$;

DO $mfm_allocator$
DECLARE
    schema_name text := current_schema();
BEGIN
    EXECUTE format(
        $mfm_ddl$
        CREATE FUNCTION %1$I.mfm_assign_tenant_fact_coordinate(
            requested_tenant_scope_id TEXT,
            requested_coordinate_kind TEXT
        )
        RETURNS NUMERIC(20, 0)
        LANGUAGE plpgsql
        SECURITY DEFINER
        SET search_path = pg_catalog
        AS $mfm_function$
        DECLARE
            previous_guard TEXT;
            assigned_order NUMERIC(20, 0);
        BEGIN
            IF requested_tenant_scope_id
                !~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
            THEN
                RAISE EXCEPTION 'invalid tenant scope id';
            END IF;
            IF requested_coordinate_kind
                NOT IN ('fact_publication', 'fact_selection_barrier')
            THEN
                RAISE EXCEPTION 'invalid tenant fact coordinate kind';
            END IF;

            previous_guard := current_setting(
                'mfm.internal_tenant_fact_head_mutation',
                TRUE
            );
            PERFORM set_config(
                'mfm.internal_tenant_fact_head_mutation',
                'enabled',
                TRUE
            );

            INSERT INTO %1$I.tenant_fact_order_heads (
                tenant_scope_id,
                current_fact_order
            )
            VALUES (requested_tenant_scope_id, 0)
            ON CONFLICT (tenant_scope_id) DO NOTHING;

            SELECT current_fact_order
              INTO STRICT assigned_order
              FROM %1$I.tenant_fact_order_heads
             WHERE tenant_scope_id = requested_tenant_scope_id
             FOR UPDATE;

            IF requested_coordinate_kind = 'fact_publication' THEN
                IF assigned_order = 18446744073709551615::numeric THEN
                    RAISE EXCEPTION 'tenant fact order overflow';
                END IF;
                assigned_order := assigned_order + 1;
                UPDATE %1$I.tenant_fact_order_heads
                   SET current_fact_order = assigned_order
                 WHERE tenant_scope_id = requested_tenant_scope_id;
            END IF;

            PERFORM set_config(
                'mfm.internal_tenant_fact_head_mutation',
                coalesce(previous_guard, ''),
                TRUE
            );
            RETURN assigned_order;
        END
        $mfm_function$
        $mfm_ddl$,
        schema_name
    );
END
$mfm_allocator$;

CREATE FUNCTION mfm_validate_journal_batch()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $mfm_function$
DECLARE
    checked_run_id TEXT := NEW.run_id;
    checked_run_sequence NUMERIC(20, 0) := NEW.run_sequence;
    expected_record_count SMALLINT;
    commit_tenant TEXT;
    coordinate_kind TEXT;
    coordinate_order NUMERIC(20, 0);
    actual_record_count BIGINT;
    fact_record_count BIGINT;
    min_ordinal INTEGER;
    max_ordinal INTEGER;
    mismatched_tenant_count BIGINT;
    mismatched_fact_order_count BIGINT;
BEGIN
    EXECUTE format(
        'SELECT record_count, tenant_scope_id, tenant_fact_coordinate_kind, tenant_fact_order
           FROM %I.journal_commits
          WHERE run_id = $1 AND run_sequence = $2',
        TG_TABLE_SCHEMA
    )
    INTO
        expected_record_count,
        commit_tenant,
        coordinate_kind,
        coordinate_order
    USING checked_run_id, checked_run_sequence;

    IF expected_record_count IS NULL THEN
        RAISE EXCEPTION 'journal record has no containing commit';
    END IF;

    EXECUTE format(
        'SELECT
             count(*),
             count(*) FILTER (WHERE emits_facts),
             min(ordinal),
             max(ordinal),
             count(*) FILTER (WHERE tenant_scope_id <> $3),
             count(*) FILTER (
                 WHERE (emits_facts AND fact_order IS DISTINCT FROM $4)
                    OR (NOT emits_facts AND fact_order IS NOT NULL)
             )
           FROM %I.journal_records
          WHERE run_id = $1 AND run_sequence = $2',
        TG_TABLE_SCHEMA
    )
    INTO
        actual_record_count,
        fact_record_count,
        min_ordinal,
        max_ordinal,
        mismatched_tenant_count,
        mismatched_fact_order_count
    USING checked_run_id, checked_run_sequence, commit_tenant, coordinate_order;

    IF actual_record_count <> expected_record_count
        OR min_ordinal <> 0
        OR max_ordinal <> expected_record_count - 1
    THEN
        RAISE EXCEPTION 'journal record count or ordinal routing mismatch';
    END IF;
    IF mismatched_tenant_count <> 0 THEN
        RAISE EXCEPTION 'journal record tenant routing mismatch';
    END IF;

    IF coordinate_kind = 'fact_publication' THEN
        IF fact_record_count <> 1 OR mismatched_fact_order_count <> 0 THEN
            RAISE EXCEPTION 'fact publication routing mismatch';
        END IF;
    ELSIF fact_record_count <> 0 OR mismatched_fact_order_count <> 0 THEN
        RAISE EXCEPTION 'non-publication commit contains fact routing';
    END IF;

    RETURN coalesce(NEW, OLD);
END
$mfm_function$;

CREATE FUNCTION mfm_validate_tenant_fact_heads()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $mfm_function$
DECLARE
    checked_tenant TEXT := NEW.tenant_scope_id;
    retained_head NUMERIC(20, 0);
    publication_count BIGINT;
    maximum_publication NUMERIC(20, 0);
    invalid_barrier_count BIGINT;
    coordinate_count BIGINT;
BEGIN
    EXECUTE format(
        'SELECT current_fact_order
           FROM %I.tenant_fact_order_heads
          WHERE tenant_scope_id = $1',
        TG_TABLE_SCHEMA
    )
    INTO retained_head
    USING checked_tenant;

    IF retained_head IS NULL THEN
        RAISE EXCEPTION 'tenant fact coordinate has no protected head';
    END IF;

    EXECUTE format(
        'SELECT
             count(*) FILTER (
                 WHERE tenant_fact_coordinate_kind = ''fact_publication''
             ),
             coalesce(max(tenant_fact_order) FILTER (
                 WHERE tenant_fact_coordinate_kind = ''fact_publication''
             ), 0),
             count(*) FILTER (
                 WHERE tenant_fact_coordinate_kind = ''fact_selection_barrier''
                   AND tenant_fact_order NOT BETWEEN 0 AND $2
             ),
             count(*) FILTER (
                 WHERE tenant_fact_coordinate_kind IN (
                     ''fact_publication'',
                     ''fact_selection_barrier''
                 )
             )
           FROM %I.journal_commits
          WHERE tenant_scope_id = $1',
        TG_TABLE_SCHEMA
    )
    INTO
        publication_count,
        maximum_publication,
        invalid_barrier_count,
        coordinate_count
    USING checked_tenant, retained_head;

    IF coordinate_count = 0 THEN
        RAISE EXCEPTION 'orphan tenant fact head';
    END IF;
    IF retained_head <> maximum_publication
        OR retained_head <> publication_count::numeric
        OR invalid_barrier_count <> 0
    THEN
        RAISE EXCEPTION 'tenant fact head, publication, or barrier mismatch';
    END IF;

    RETURN coalesce(NEW, OLD);
END
$mfm_function$;

CREATE TRIGGER store_identity_no_mutation
    BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON store_identity
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_store_identity_mutation();

CREATE TRIGGER store_schema_metadata_no_mutation
    BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON store_schema_metadata
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_store_identity_mutation();

CREATE TRIGGER tenant_fact_order_heads_guard
    BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON tenant_fact_order_heads
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_guard_tenant_fact_head_mutation();

CREATE TRIGGER journal_commits_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON journal_commits
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER journal_records_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON journal_records
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER artifact_blobs_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON artifact_blobs
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER artifact_admissions_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON artifact_admissions
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER commit_artifact_bindings_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON commit_artifact_bindings
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER commit_object_authorities_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON commit_object_authorities
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER fact_scan_attestations_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON fact_scan_attestations
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER qualified_support_members_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON qualified_support_members
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE TRIGGER configured_values_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON configured_values
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_reject_authority_mutation();

CREATE CONSTRAINT TRIGGER journal_commits_batch_contract
    AFTER INSERT ON journal_commits
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION mfm_validate_journal_batch();

CREATE CONSTRAINT TRIGGER journal_records_batch_contract
    AFTER INSERT ON journal_records
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION mfm_validate_journal_batch();

CREATE CONSTRAINT TRIGGER tenant_fact_order_heads_integrity
    AFTER INSERT OR UPDATE ON tenant_fact_order_heads
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION mfm_validate_tenant_fact_heads();

CREATE CONSTRAINT TRIGGER journal_commits_tenant_fact_integrity
    AFTER INSERT ON journal_commits
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (
        NEW.tenant_fact_coordinate_kind IN (
            'fact_publication',
            'fact_selection_barrier'
        )
    )
    EXECUTE FUNCTION mfm_validate_tenant_fact_heads();

DO $mfm_object_acl$
DECLARE
    schema_name text := current_schema();
    object_name text;
    function_arguments text;
BEGIN
    FOREACH object_name IN ARRAY ARRAY[
        'store_identity',
        'store_schema_metadata',
        'tenant_fact_order_heads',
        'journal_commits',
        'journal_records',
        'artifact_blobs',
        'artifact_admissions',
        'commit_object_authorities',
        'commit_artifact_bindings',
        'qualified_support_members',
        'fact_scan_attestations',
        'configured_values'
    ]
    LOOP
        EXECUTE format('REVOKE ALL ON TABLE %I.%I FROM PUBLIC', schema_name, object_name);
        EXECUTE format(
            'ALTER TABLE %I.%I OWNER TO mfm_store_owner',
            schema_name,
            object_name
        );
    END LOOP;

    FOREACH object_name IN ARRAY ARRAY[
        'mfm_reject_authority_mutation',
        'mfm_reject_store_identity_mutation',
        'mfm_guard_tenant_fact_head_mutation',
        'mfm_assign_tenant_fact_coordinate',
        'mfm_validate_journal_batch',
        'mfm_validate_tenant_fact_heads'
    ]
    LOOP
        SELECT pg_catalog.pg_get_function_identity_arguments(procedure.oid)
          INTO STRICT function_arguments
          FROM pg_catalog.pg_proc AS procedure
          JOIN pg_catalog.pg_namespace AS namespace
            ON namespace.oid = procedure.pronamespace
         WHERE namespace.nspname = schema_name
           AND procedure.proname = object_name;
        EXECUTE format(
            'REVOKE ALL ON FUNCTION %I.%I(%s) FROM PUBLIC',
            schema_name,
            object_name,
            function_arguments
        );
        EXECUTE format(
            'ALTER FUNCTION %I.%I(%s) OWNER TO mfm_store_owner',
            schema_name,
            object_name,
            function_arguments
        );
    END LOOP;

    EXECUTE format(
        'GRANT SELECT ON TABLE
             %1$I.store_identity,
             %1$I.store_schema_metadata,
             %1$I.tenant_fact_order_heads,
             %1$I.journal_commits,
             %1$I.journal_records,
             %1$I.artifact_blobs,
             %1$I.artifact_admissions,
             %1$I.commit_object_authorities,
             %1$I.commit_artifact_bindings,
             %1$I.qualified_support_members,
             %1$I.fact_scan_attestations,
             %1$I.configured_values
         TO mfm_store_application',
        schema_name
    );
    EXECUTE format(
        'GRANT SELECT ON TABLE %I._sqlx_migrations TO mfm_store_application',
        schema_name
    );
    EXECUTE format(
        'GRANT INSERT ON TABLE
             %1$I.journal_commits,
             %1$I.journal_records,
             %1$I.artifact_blobs,
             %1$I.artifact_admissions,
             %1$I.commit_object_authorities,
             %1$I.commit_artifact_bindings,
             %1$I.qualified_support_members,
             %1$I.fact_scan_attestations
         TO mfm_store_application',
        schema_name
    );
    EXECUTE format(
        'GRANT EXECUTE ON FUNCTION %I.mfm_assign_tenant_fact_coordinate(TEXT, TEXT)
         TO mfm_store_application',
        schema_name
    );
END
$mfm_object_acl$;
