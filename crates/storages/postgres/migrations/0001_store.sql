-- Sole current PostgreSQL authority for structured RunHistory.
--
-- This destructive pre-release baseline has no compatibility relation to earlier
-- structured-history baselines. A database with any earlier 0001 checksum is rejected
-- and must be reset rather than migrated or reinterpreted.
--
-- Roles are exact-target: each store schema owns a distinct non-login owner plus
-- qualification, run-reader, run-writer, configuration-reader, and configuration-writer
-- roles. Sibling schemas never share grants. Role names are derived from a private
-- 16-hex target key of md5(current_schema()).

DO $mfm_target_roles$
DECLARE
    schema_name text := current_schema();
    target_key text := substr(md5(schema_name), 1, 16);
    owner_role text := format('mfm_t_%s_own', target_key);
    qualification_role text := format('mfm_t_%s_qlf', target_key);
    run_reader_role text := format('mfm_t_%s_rrd', target_key);
    run_writer_role text := format('mfm_t_%s_rwr', target_key);
    configuration_reader_role text := format('mfm_t_%s_crd', target_key);
    configuration_writer_role text := format('mfm_t_%s_cwr', target_key);
    role_name text;
BEGIN
    IF current_user IN (
        owner_role,
        qualification_role,
        run_reader_role,
        run_writer_role,
        configuration_reader_role,
        configuration_writer_role
    ) THEN
        RAISE EXCEPTION 'the migration must not run as a runtime role';
    END IF;

    IF schema_name IS NULL OR schema_name = '' OR schema_name = 'pg_catalog' THEN
        RAISE EXCEPTION 'migration requires an explicit non-catalog search_path schema';
    END IF;

    FOREACH role_name IN ARRAY ARRAY[
        owner_role,
        qualification_role,
        run_reader_role,
        run_writer_role,
        configuration_reader_role,
        configuration_writer_role
    ]
    LOOP
        IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = role_name) THEN
            EXECUTE format(
                'CREATE ROLE %I NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1',
                role_name
            );
        ELSE
            EXECUTE format(
                'ALTER ROLE %I NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD NULL',
                role_name
            );
        END IF;
        EXECUTE format('ALTER ROLE %I RESET ALL', role_name);
    END LOOP;

    EXECUTE format('GRANT %I TO %I', owner_role, current_user);
    EXECUTE format('GRANT %I TO %I', qualification_role, current_user);
    EXECUTE format('ALTER SCHEMA %I OWNER TO %I', schema_name, owner_role);
END
$mfm_target_roles$;

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
        schema_contract_version = 'mfm.structured-run-history-postgres.v4'
    )
);

CREATE TABLE target_authority (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    target_key TEXT NOT NULL,
    fence_generation NUMERIC(20, 0) NOT NULL,
    release_epoch NUMERIC(20, 0) NOT NULL,
    owner_role TEXT NOT NULL,
    qualification_role TEXT NOT NULL,
    run_reader_role TEXT NOT NULL,
    run_writer_role TEXT NOT NULL,
    configuration_reader_role TEXT NOT NULL,
    configuration_writer_role TEXT NOT NULL,
    CONSTRAINT target_authority_singleton_v1 CHECK (singleton),
    CONSTRAINT target_authority_key_v1 CHECK (target_key ~ '^[0-9a-f]{16}$'),
    CONSTRAINT target_authority_generation_v1 CHECK (
        fence_generation >= 1
        AND fence_generation <= 18446744073709551615::numeric
        AND trunc(fence_generation) = fence_generation
    ),
    CONSTRAINT target_authority_release_v1 CHECK (
        release_epoch >= 1
        AND release_epoch <= 18446744073709551615::numeric
        AND trunc(release_epoch) = release_epoch
    ),
    CONSTRAINT target_authority_roles_v1 CHECK (
        owner_role = format('mfm_t_%s_own', target_key)
        AND qualification_role = format('mfm_t_%s_qlf', target_key)
        AND run_reader_role = format('mfm_t_%s_rrd', target_key)
        AND run_writer_role = format('mfm_t_%s_rwr', target_key)
        AND configuration_reader_role = format('mfm_t_%s_crd', target_key)
        AND configuration_writer_role = format('mfm_t_%s_cwr', target_key)
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
VALUES (TRUE, 'mfm.structured-run-history-postgres.v4');

INSERT INTO target_authority (
    singleton,
    target_key,
    fence_generation,
    release_epoch,
    owner_role,
    qualification_role,
    run_reader_role,
    run_writer_role,
    configuration_reader_role,
    configuration_writer_role
)
SELECT
    TRUE,
    target_key,
    1,
    1,
    format('mfm_t_%s_own', target_key),
    format('mfm_t_%s_qlf', target_key),
    format('mfm_t_%s_rrd', target_key),
    format('mfm_t_%s_rwr', target_key),
    format('mfm_t_%s_crd', target_key),
    format('mfm_t_%s_cwr', target_key)
FROM (SELECT substr(md5(current_schema()), 1, 16) AS target_key) AS keys;

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

DO $mfm_table_owners$
DECLARE
    owner_role text;
BEGIN
    SELECT authority.owner_role INTO owner_role FROM target_authority AS authority WHERE singleton;
    EXECUTE format('ALTER TABLE store_identity OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE store_schema_metadata OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE target_authority OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE run_history_heads OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE run_history_batches OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE run_history_batch_objects OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE tenant_fact_heads OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE tenant_fact_publications OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE configuration_revisions OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE configuration_heads OWNER TO %I', owner_role);
    EXECUTE format('ALTER TABLE _sqlx_migrations OWNER TO %I', owner_role);
END
$mfm_table_owners$;

DO $mfm_schema_acl$
DECLARE
    schema_name text := current_schema();
    owner_role text;
    qualification_role text;
    run_reader_role text;
    run_writer_role text;
    configuration_reader_role text;
    configuration_writer_role text;
BEGIN
    SELECT
        authority.owner_role,
        authority.qualification_role,
        authority.run_reader_role,
        authority.run_writer_role,
        authority.configuration_reader_role,
        authority.configuration_writer_role
    INTO
        owner_role,
        qualification_role,
        run_reader_role,
        run_writer_role,
        configuration_reader_role,
        configuration_writer_role
    FROM target_authority AS authority
    WHERE singleton;

    EXECUTE format('SET LOCAL ROLE %I', owner_role);

    EXECUTE format('REVOKE ALL ON SCHEMA %I FROM PUBLIC', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO %I', schema_name, qualification_role);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO %I', schema_name, run_reader_role);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO %I', schema_name, run_writer_role);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO %I', schema_name, configuration_reader_role);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO %I', schema_name, configuration_writer_role);

    EXECUTE 'REVOKE ALL ON TABLE store_identity FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE store_schema_metadata FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE target_authority FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE run_history_heads FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE run_history_batches FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE run_history_batch_objects FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE tenant_fact_heads FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE tenant_fact_publications FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE configuration_revisions FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE configuration_heads FROM PUBLIC';
    EXECUTE 'REVOKE ALL ON TABLE _sqlx_migrations FROM PUBLIC';

    EXECUTE format('GRANT SELECT ON TABLE store_identity TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE store_schema_metadata TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE target_authority TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE run_history_heads TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE run_history_batches TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE run_history_batch_objects TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE tenant_fact_heads TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE tenant_fact_publications TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE configuration_revisions TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE configuration_heads TO %I', qualification_role);
    EXECUTE format('GRANT SELECT ON TABLE _sqlx_migrations TO %I', qualification_role);

    EXECUTE format('GRANT SELECT ON TABLE store_identity TO %I', run_reader_role);
    EXECUTE format('GRANT SELECT ON TABLE store_schema_metadata TO %I', run_reader_role);
    EXECUTE format('GRANT SELECT ON TABLE target_authority TO %I', run_reader_role);
    EXECUTE format('GRANT SELECT ON TABLE run_history_heads TO %I', run_reader_role);
    EXECUTE format('GRANT SELECT ON TABLE run_history_batches TO %I', run_reader_role);
    EXECUTE format('GRANT SELECT ON TABLE run_history_batch_objects TO %I', run_reader_role);
    EXECUTE format('GRANT SELECT ON TABLE tenant_fact_heads TO %I', run_reader_role);
    EXECUTE format('GRANT SELECT ON TABLE tenant_fact_publications TO %I', run_reader_role);

    EXECUTE format('GRANT SELECT ON TABLE store_identity TO %I', run_writer_role);
    EXECUTE format('GRANT SELECT ON TABLE store_schema_metadata TO %I', run_writer_role);
    EXECUTE format('GRANT SELECT ON TABLE target_authority TO %I', run_writer_role);
    EXECUTE format(
        'GRANT SELECT, INSERT, UPDATE ON TABLE run_history_heads TO %I',
        run_writer_role
    );
    EXECUTE format('GRANT SELECT, INSERT ON TABLE run_history_batches TO %I', run_writer_role);
    EXECUTE format(
        'GRANT SELECT, INSERT ON TABLE run_history_batch_objects TO %I',
        run_writer_role
    );
    EXECUTE format(
        'GRANT SELECT, INSERT, UPDATE ON TABLE tenant_fact_heads TO %I',
        run_writer_role
    );
    EXECUTE format(
        'GRANT SELECT, INSERT ON TABLE tenant_fact_publications TO %I',
        run_writer_role
    );

    EXECUTE format('GRANT SELECT ON TABLE store_identity TO %I', configuration_reader_role);
    EXECUTE format(
        'GRANT SELECT ON TABLE store_schema_metadata TO %I',
        configuration_reader_role
    );
    EXECUTE format('GRANT SELECT ON TABLE target_authority TO %I', configuration_reader_role);
    EXECUTE format(
        'GRANT SELECT ON TABLE configuration_revisions TO %I',
        configuration_reader_role
    );
    EXECUTE format('GRANT SELECT ON TABLE configuration_heads TO %I', configuration_reader_role);

    EXECUTE format('GRANT SELECT ON TABLE store_identity TO %I', configuration_writer_role);
    EXECUTE format(
        'GRANT SELECT ON TABLE store_schema_metadata TO %I',
        configuration_writer_role
    );
    EXECUTE format('GRANT SELECT ON TABLE target_authority TO %I', configuration_writer_role);
    EXECUTE format(
        'GRANT SELECT, INSERT ON TABLE configuration_revisions TO %I',
        configuration_writer_role
    );
    EXECUTE format(
        'GRANT SELECT, INSERT, UPDATE ON TABLE configuration_heads TO %I',
        configuration_writer_role
    );

    RESET ROLE;
END
$mfm_schema_acl$;
