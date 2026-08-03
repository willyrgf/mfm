-- Sole current PostgreSQL authority for structured RunHistory.
--
-- This destructive pre-release baseline has no compatibility relation to the
-- retired graph journal. A database with any earlier 0001 checksum is rejected
-- and must be reset rather than migrated or reinterpreted.

DO $mfm_roles$
BEGIN
    IF current_user IN (
        'mfm_store_owner',
        'mfm_store_qualification',
        'mfm_store_application',
        'mfm_store_configuration_maintenance'
    ) THEN
        RAISE EXCEPTION 'the migration must not run as a runtime role';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_store_owner') THEN
        CREATE ROLE mfm_store_owner
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1;
    ELSE
        ALTER ROLE mfm_store_owner
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1 PASSWORD NULL;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_store_application') THEN
        CREATE ROLE mfm_store_application
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1;
    ELSE
        ALTER ROLE mfm_store_application
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1 PASSWORD NULL;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_store_qualification') THEN
        CREATE ROLE mfm_store_qualification
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1;
    ELSE
        ALTER ROLE mfm_store_qualification
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1 PASSWORD NULL;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles
        WHERE rolname = 'mfm_store_configuration_maintenance'
    ) THEN
        CREATE ROLE mfm_store_configuration_maintenance
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1;
    ELSE
        ALTER ROLE mfm_store_configuration_maintenance
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS
            CONNECTION LIMIT -1 PASSWORD NULL;
    END IF;

    EXECUTE format('GRANT mfm_store_owner TO %I', current_user);
    EXECUTE format('GRANT mfm_store_qualification TO %I', current_user);
    EXECUTE format('GRANT mfm_store_application TO %I', current_user);
    EXECUTE format('GRANT mfm_store_configuration_maintenance TO %I', current_user);
END
$mfm_roles$;

ALTER ROLE mfm_store_owner RESET ALL;
ALTER ROLE mfm_store_qualification RESET ALL;
ALTER ROLE mfm_store_application RESET ALL;
ALTER ROLE mfm_store_configuration_maintenance RESET ALL;

DO $mfm_schema_owner$
DECLARE
    schema_name text := current_schema();
BEGIN
    EXECUTE format('ALTER SCHEMA %I OWNER TO mfm_store_owner', schema_name);
END
$mfm_schema_owner$;

CREATE TABLE store_identity (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    store_scope_id TEXT NOT NULL UNIQUE,
    store_epoch NUMERIC(20, 0) NOT NULL,
    CONSTRAINT store_identity_singleton_v1 CHECK (singleton),
    CONSTRAINT store_identity_scope_v1 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT store_identity_epoch_v1 CHECK (
        store_epoch >= 1
        AND store_epoch <= 18446744073709551615::numeric
        AND trunc(store_epoch) = store_epoch
    )
);

CREATE TABLE store_schema_metadata (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    schema_contract_version TEXT NOT NULL,
    CONSTRAINT store_schema_metadata_singleton_v1 CHECK (singleton),
    CONSTRAINT store_schema_metadata_version_v1 CHECK (
        schema_contract_version = 'mfm.structured-run-history-postgres.v3'
    )
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
VALUES (TRUE, 'mfm.structured-run-history-postgres.v3');

CREATE TABLE run_history_heads (
    run_id TEXT PRIMARY KEY,
    store_scope_id TEXT NOT NULL,
    store_epoch NUMERIC(20, 0) NOT NULL,
    head_sequence NUMERIC(20, 0) NOT NULL,
    head_commit_digest TEXT NOT NULL,
    CONSTRAINT run_history_heads_run_id_v1 CHECK (length(run_id) BETWEEN 1 AND 512),
    CONSTRAINT run_history_heads_scope_v1 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT run_history_heads_epoch_v1 CHECK (
        store_epoch >= 1
        AND store_epoch <= 18446744073709551615::numeric
        AND trunc(store_epoch) = store_epoch
    ),
    CONSTRAINT run_history_heads_sequence_v1 CHECK (
        head_sequence >= 1
        AND head_sequence <= 18446744073709551615::numeric
        AND trunc(head_sequence) = head_sequence
    ),
    CONSTRAINT run_history_heads_digest_v1 CHECK (
        length(head_commit_digest) BETWEEN 1 AND 512
    )
);

CREATE TABLE run_history_batches (
    run_id TEXT NOT NULL,
    run_sequence NUMERIC(20, 0) NOT NULL,
    append_request_id TEXT NOT NULL,
    candidate_digest TEXT NOT NULL,
    predecessor_sequence NUMERIC(20, 0),
    predecessor_commit_digest TEXT,
    head_commit_digest TEXT NOT NULL,
    batch_envelope_json TEXT NOT NULL,
    CONSTRAINT run_history_batches_primary_v1 PRIMARY KEY (run_id, run_sequence),
    CONSTRAINT run_history_batches_append_v1 UNIQUE (run_id, append_request_id),
    CONSTRAINT run_history_batches_run_id_v1 CHECK (length(run_id) BETWEEN 1 AND 512),
    CONSTRAINT run_history_batches_sequence_v1 CHECK (
        run_sequence >= 1
        AND run_sequence <= 18446744073709551615::numeric
        AND trunc(run_sequence) = run_sequence
    ),
    CONSTRAINT run_history_batches_append_id_v1 CHECK (
        length(append_request_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT run_history_batches_candidate_digest_v1 CHECK (
        length(candidate_digest) BETWEEN 1 AND 512
    ),
    CONSTRAINT run_history_batches_predecessor_pair_v1 CHECK (
        (predecessor_sequence IS NULL) = (predecessor_commit_digest IS NULL)
    ),
    CONSTRAINT run_history_batches_predecessor_sequence_v1 CHECK (
        predecessor_sequence IS NULL
        OR (
            predecessor_sequence >= 1
            AND predecessor_sequence < run_sequence
            AND predecessor_sequence <= 18446744073709551615::numeric
            AND trunc(predecessor_sequence) = predecessor_sequence
        )
    ),
    CONSTRAINT run_history_batches_digest_v1 CHECK (
        length(head_commit_digest) BETWEEN 1 AND 512
        AND (predecessor_commit_digest IS NULL OR length(predecessor_commit_digest) BETWEEN 1 AND 512)
    ),
    CONSTRAINT run_history_batches_envelope_json_v1 CHECK (
        octet_length(batch_envelope_json) BETWEEN 2 AND 16777216
    )
);

CREATE TABLE run_history_batch_objects (
    run_id TEXT NOT NULL,
    run_sequence NUMERIC(20, 0) NOT NULL,
    object_ordinal INTEGER NOT NULL,
    object_type TEXT NOT NULL,
    content_schema_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    canonical_json TEXT NOT NULL,
    CONSTRAINT run_history_batch_objects_primary_v1 PRIMARY KEY (
        run_id,
        run_sequence,
        object_ordinal
    ),
    CONSTRAINT run_history_batch_objects_batch_v1 FOREIGN KEY (run_id, run_sequence)
        REFERENCES run_history_batches (run_id, run_sequence),
    CONSTRAINT run_history_batch_objects_ordinal_v1 CHECK (
        object_ordinal >= 0 AND object_ordinal < 65536
    ),
    CONSTRAINT run_history_batch_objects_type_v1 CHECK (
        octet_length(object_type) BETWEEN 1 AND 512
    ),
    CONSTRAINT run_history_batch_objects_schema_v1 CHECK (
        octet_length(content_schema_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT run_history_batch_objects_digest_v1 CHECK (
        octet_length(content_digest) BETWEEN 1 AND 512
    ),
    CONSTRAINT run_history_batch_objects_json_v1 CHECK (
        octet_length(canonical_json) BETWEEN 1 AND 16777216
    )
);

CREATE TABLE tenant_fact_heads (
    store_scope_id TEXT NOT NULL,
    store_epoch NUMERIC(20, 0) NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    fact_order NUMERIC(20, 0) NOT NULL,
    CONSTRAINT tenant_fact_heads_primary_v2 PRIMARY KEY (
        store_scope_id,
        store_epoch,
        tenant_scope_id
    ),
    CONSTRAINT tenant_fact_heads_scope_v2 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT tenant_fact_heads_epoch_v2 CHECK (
        store_epoch >= 1
        AND store_epoch <= 18446744073709551615::numeric
        AND trunc(store_epoch) = store_epoch
    ),
    CONSTRAINT tenant_fact_heads_tenant_v2 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT tenant_fact_heads_order_v2 CHECK (
        fact_order >= 0
        AND fact_order <= 18446744073709551615::numeric
        AND trunc(fact_order) = fact_order
    )
);

CREATE TABLE tenant_fact_publications (
    store_scope_id TEXT NOT NULL,
    store_epoch NUMERIC(20, 0) NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    fact_order NUMERIC(20, 0) NOT NULL,
    run_id TEXT NOT NULL,
    run_sequence NUMERIC(20, 0) NOT NULL,
    transition_ordinal INTEGER NOT NULL,
    transition_record_hash TEXT NOT NULL,
    CONSTRAINT tenant_fact_publications_primary_v2 PRIMARY KEY (
        store_scope_id,
        store_epoch,
        tenant_scope_id,
        fact_order
    ),
    CONSTRAINT tenant_fact_publications_transition_v2 UNIQUE (run_id, run_sequence),
    CONSTRAINT tenant_fact_publications_batch_v2 FOREIGN KEY (run_id, run_sequence)
        REFERENCES run_history_batches (run_id, run_sequence),
    CONSTRAINT tenant_fact_publications_scope_v2 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT tenant_fact_publications_epoch_v2 CHECK (
        store_epoch >= 1
        AND store_epoch <= 18446744073709551615::numeric
        AND trunc(store_epoch) = store_epoch
    ),
    CONSTRAINT tenant_fact_publications_tenant_v2 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT tenant_fact_publications_order_v2 CHECK (
        fact_order >= 1
        AND fact_order <= 18446744073709551615::numeric
        AND trunc(fact_order) = fact_order
    ),
    CONSTRAINT tenant_fact_publications_route_v2 CHECK (
        length(run_id) BETWEEN 1 AND 512
        AND run_sequence >= 1
        AND run_sequence <= 18446744073709551615::numeric
        AND trunc(run_sequence) = run_sequence
        AND transition_ordinal >= 0
        AND transition_ordinal < 2
        AND length(transition_record_hash) BETWEEN 1 AND 512
    )
);

CREATE TABLE configuration_revisions (
    store_scope_id TEXT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    entry_point_operation_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    revision_sequence NUMERIC(20, 0) NOT NULL,
    predecessor_schema_id TEXT,
    predecessor_digest TEXT,
    append_request_id TEXT NOT NULL,
    value_contract_schema_id TEXT NOT NULL,
    value_contract_digest TEXT NOT NULL,
    value_schema_id TEXT NOT NULL,
    value_digest TEXT NOT NULL,
    revision_schema_id TEXT NOT NULL,
    revision_digest TEXT NOT NULL,
    canonical_revision_json TEXT NOT NULL,
    CONSTRAINT configuration_revisions_primary_v1 PRIMARY KEY (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        revision_sequence
    ),
    CONSTRAINT configuration_revisions_append_v1 UNIQUE (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        append_request_id
    ),
    CONSTRAINT configuration_revisions_ref_v1 UNIQUE (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        revision_schema_id,
        revision_digest
    ),
    CONSTRAINT configuration_revisions_head_target_v1 UNIQUE (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        revision_sequence,
        revision_schema_id,
        revision_digest
    ),
    CONSTRAINT configuration_revisions_predecessor_v1 FOREIGN KEY (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        predecessor_schema_id,
        predecessor_digest
    ) REFERENCES configuration_revisions (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        revision_schema_id,
        revision_digest
    ),
    CONSTRAINT configuration_revisions_scope_v1 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT configuration_revisions_tenant_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT configuration_revisions_route_v1 CHECK (
        octet_length(entry_point_operation_id) BETWEEN 1 AND 512
        AND octet_length(target_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT configuration_revisions_sequence_v1 CHECK (
        revision_sequence >= 1
        AND revision_sequence <= 18446744073709551615::numeric
        AND trunc(revision_sequence) = revision_sequence
    ),
    CONSTRAINT configuration_revisions_predecessor_pair_v1 CHECK (
        (predecessor_schema_id IS NULL) = (predecessor_digest IS NULL)
    ),
    CONSTRAINT configuration_revisions_append_id_v1 CHECK (
        octet_length(append_request_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT configuration_revisions_refs_v1 CHECK (
        octet_length(predecessor_schema_id) BETWEEN 1 AND 512
        OR predecessor_schema_id IS NULL
    ),
    CONSTRAINT configuration_revisions_digests_v1 CHECK (
        octet_length(predecessor_digest) BETWEEN 1 AND 512
        OR predecessor_digest IS NULL
    ),
    CONSTRAINT configuration_revisions_value_refs_v1 CHECK (
        octet_length(value_contract_schema_id) BETWEEN 1 AND 512
        AND octet_length(value_contract_digest) BETWEEN 1 AND 512
        AND octet_length(value_schema_id) BETWEEN 1 AND 512
        AND octet_length(value_digest) BETWEEN 1 AND 512
        AND octet_length(revision_schema_id) BETWEEN 1 AND 512
        AND octet_length(revision_digest) BETWEEN 1 AND 512
    ),
    CONSTRAINT configuration_revisions_json_v1 CHECK (
        octet_length(canonical_revision_json) BETWEEN 2 AND 16777216
    )
);

CREATE TABLE configuration_heads (
    store_scope_id TEXT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    entry_point_operation_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    revision_sequence NUMERIC(20, 0) NOT NULL,
    revision_schema_id TEXT NOT NULL,
    revision_digest TEXT NOT NULL,
    CONSTRAINT configuration_heads_primary_v1 PRIMARY KEY (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id
    ),
    CONSTRAINT configuration_heads_revision_v1 FOREIGN KEY (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        revision_sequence,
        revision_schema_id,
        revision_digest
    ) REFERENCES configuration_revisions (
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        target_id,
        revision_sequence,
        revision_schema_id,
        revision_digest
    ),
    CONSTRAINT configuration_heads_scope_v1 CHECK (
        store_scope_id ~ '^mfm[.]store_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT configuration_heads_tenant_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT configuration_heads_route_v1 CHECK (
        octet_length(entry_point_operation_id) BETWEEN 1 AND 512
        AND octet_length(target_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT configuration_heads_sequence_v1 CHECK (
        revision_sequence >= 1
        AND revision_sequence <= 18446744073709551615::numeric
        AND trunc(revision_sequence) = revision_sequence
    ),
    CONSTRAINT configuration_heads_refs_v1 CHECK (
        octet_length(revision_schema_id) BETWEEN 1 AND 512
        AND octet_length(revision_digest) BETWEEN 1 AND 512
    )
);

ALTER TABLE store_identity OWNER TO mfm_store_owner;
ALTER TABLE store_schema_metadata OWNER TO mfm_store_owner;
ALTER TABLE run_history_heads OWNER TO mfm_store_owner;
ALTER TABLE run_history_batches OWNER TO mfm_store_owner;
ALTER TABLE run_history_batch_objects OWNER TO mfm_store_owner;
ALTER TABLE tenant_fact_heads OWNER TO mfm_store_owner;
ALTER TABLE tenant_fact_publications OWNER TO mfm_store_owner;
ALTER TABLE configuration_revisions OWNER TO mfm_store_owner;
ALTER TABLE configuration_heads OWNER TO mfm_store_owner;
ALTER TABLE _sqlx_migrations OWNER TO mfm_store_owner;

SET LOCAL ROLE mfm_store_owner;

DO $mfm_schema_acl$
DECLARE
    schema_name text := current_schema();
BEGIN
    EXECUTE format('REVOKE ALL ON SCHEMA %I FROM PUBLIC', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_store_qualification', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_store_application', schema_name);
    EXECUTE format(
        'GRANT USAGE ON SCHEMA %I TO mfm_store_configuration_maintenance',
        schema_name
    );
END
$mfm_schema_acl$;

REVOKE ALL ON TABLE store_identity FROM PUBLIC;
REVOKE ALL ON TABLE store_schema_metadata FROM PUBLIC;
REVOKE ALL ON TABLE run_history_heads FROM PUBLIC;
REVOKE ALL ON TABLE run_history_batches FROM PUBLIC;
REVOKE ALL ON TABLE run_history_batch_objects FROM PUBLIC;
REVOKE ALL ON TABLE tenant_fact_heads FROM PUBLIC;
REVOKE ALL ON TABLE tenant_fact_publications FROM PUBLIC;
REVOKE ALL ON TABLE configuration_revisions FROM PUBLIC;
REVOKE ALL ON TABLE configuration_heads FROM PUBLIC;
REVOKE ALL ON TABLE _sqlx_migrations FROM PUBLIC;

GRANT SELECT ON TABLE store_identity TO mfm_store_qualification;
GRANT SELECT ON TABLE store_schema_metadata TO mfm_store_qualification;
GRANT SELECT ON TABLE run_history_heads TO mfm_store_qualification;
GRANT SELECT ON TABLE run_history_batches TO mfm_store_qualification;
GRANT SELECT ON TABLE run_history_batch_objects TO mfm_store_qualification;
GRANT SELECT ON TABLE tenant_fact_heads TO mfm_store_qualification;
GRANT SELECT ON TABLE tenant_fact_publications TO mfm_store_qualification;
GRANT SELECT ON TABLE configuration_revisions TO mfm_store_qualification;
GRANT SELECT ON TABLE configuration_heads TO mfm_store_qualification;
GRANT SELECT ON TABLE _sqlx_migrations TO mfm_store_qualification;

GRANT SELECT ON TABLE store_identity TO mfm_store_application;
GRANT SELECT ON TABLE store_schema_metadata TO mfm_store_application;
GRANT SELECT, INSERT, UPDATE ON TABLE run_history_heads TO mfm_store_application;
GRANT SELECT, INSERT ON TABLE run_history_batches TO mfm_store_application;
GRANT SELECT, INSERT ON TABLE run_history_batch_objects TO mfm_store_application;
GRANT SELECT, INSERT, UPDATE ON TABLE tenant_fact_heads TO mfm_store_application;
GRANT SELECT, INSERT ON TABLE tenant_fact_publications TO mfm_store_application;
GRANT SELECT ON TABLE store_identity TO mfm_store_configuration_maintenance;
GRANT SELECT ON TABLE store_schema_metadata TO mfm_store_configuration_maintenance;
GRANT SELECT, INSERT ON TABLE configuration_revisions TO mfm_store_configuration_maintenance;
GRANT SELECT, INSERT, UPDATE ON TABLE configuration_heads TO mfm_store_configuration_maintenance;
GRANT SELECT ON TABLE configuration_revisions TO mfm_store_application;
GRANT SELECT ON TABLE configuration_heads TO mfm_store_application;

RESET ROLE;
