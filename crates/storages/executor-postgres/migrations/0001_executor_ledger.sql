-- Dedicated MFM executor-ledger authority.
--
-- This schema is intentionally independent from the run journal. Migrate it through a pool whose
-- current schema is reserved for the executor ledger. The application role can append and select
-- immutable records but cannot update, delete, truncate, migrate, or replace authority.

DO $mfm_executor_roles$
BEGIN
    IF current_user = 'mfm_executor_application' THEN
        RAISE EXCEPTION 'the executor migration must not run as the application role';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_executor_owner'
    ) THEN
        CREATE ROLE mfm_executor_owner
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    ELSE
        ALTER ROLE mfm_executor_owner
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_executor_application'
    ) THEN
        CREATE ROLE mfm_executor_application
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    ELSE
        ALTER ROLE mfm_executor_application
            NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS;
    END IF;

    EXECUTE format('GRANT mfm_executor_owner TO %I', current_user);
    EXECUTE format('GRANT mfm_executor_application TO %I', current_user);
END
$mfm_executor_roles$;

DO $mfm_executor_schema_acl$
DECLARE
    schema_name TEXT := current_schema();
BEGIN
    EXECUTE format('REVOKE ALL ON SCHEMA %I FROM PUBLIC', schema_name);
    EXECUTE format('GRANT USAGE, CREATE ON SCHEMA %I TO mfm_executor_owner', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_executor_application', schema_name);
END
$mfm_executor_schema_acl$;

CREATE TABLE executor_schema_metadata (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    schema_contract_version TEXT NOT NULL,
    CONSTRAINT executor_schema_metadata_singleton_v1 CHECK (singleton),
    CONSTRAINT executor_schema_metadata_version_v1 CHECK (
        schema_contract_version = 'mfm.executor-postgres.v1'
    )
);

INSERT INTO executor_schema_metadata (singleton, schema_contract_version)
VALUES (TRUE, 'mfm.executor-postgres.v1');

CREATE TABLE executor_bindings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    tenant_scope_id TEXT NOT NULL,
    executor_binding_schema_id TEXT NOT NULL,
    executor_binding_content_digest TEXT NOT NULL,
    durable_generation_schema_id TEXT NOT NULL,
    durable_generation_content_digest TEXT NOT NULL,
    evidence_authority_schema_id TEXT NOT NULL,
    evidence_authority_content_digest TEXT NOT NULL,
    resource_ownership_schema_id TEXT,
    resource_ownership_content_digest TEXT,
    CONSTRAINT executor_bindings_singleton_v1 CHECK (singleton),
    CONSTRAINT executor_bindings_tenant_v1 CHECK (
        tenant_scope_id ~ '^mfm[.]tenant_scope[.]v1:[0-9a-f]{32}$'
    ),
    CONSTRAINT executor_bindings_binding_schema_v1 CHECK (
        executor_binding_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_bindings_binding_digest_v1 CHECK (
        executor_binding_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_bindings_generation_schema_v1 CHECK (
        durable_generation_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_bindings_generation_digest_v1 CHECK (
        durable_generation_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_bindings_evidence_schema_v1 CHECK (
        evidence_authority_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_bindings_evidence_digest_v1 CHECK (
        evidence_authority_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_bindings_resource_shape_v1 CHECK (
        (
            resource_ownership_schema_id IS NULL
            AND resource_ownership_content_digest IS NULL
        )
        OR
        (
            resource_ownership_schema_id ~
                '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
            AND resource_ownership_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
        )
    )
);

CREATE TABLE executor_content_records (
    schema_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    canonical_bytes BYTEA NOT NULL,
    PRIMARY KEY (schema_id, content_digest),
    CONSTRAINT executor_content_records_schema_v1 CHECK (
        schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:[1-9][0-9]*:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_content_records_digest_v1 CHECK (
        content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_content_records_bytes_v1 CHECK (
        octet_length(canonical_bytes) BETWEEN 2 AND 16777216
    )
);

CREATE TABLE executor_effect_frontiers (
    effect_key TEXT NOT NULL,
    ordinal BIGINT NOT NULL,
    frontier_schema_id TEXT NOT NULL,
    frontier_content_digest TEXT NOT NULL,
    predecessor_schema_id TEXT,
    predecessor_content_digest TEXT,
    durable_payload BYTEA NOT NULL,
    PRIMARY KEY (effect_key, ordinal),
    CONSTRAINT executor_effect_frontiers_ref_key UNIQUE (
        effect_key,
        frontier_schema_id,
        frontier_content_digest
    ),
    CONSTRAINT executor_effect_frontiers_effect_v1 CHECK (
        effect_key ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_effect_frontiers_ordinal_v1 CHECK (ordinal >= 0),
    CONSTRAINT executor_effect_frontiers_schema_v1 CHECK (
        frontier_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_effect_frontiers_digest_v1 CHECK (
        frontier_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_effect_frontiers_predecessor_shape_v1 CHECK (
        (
            ordinal = 0
            AND predecessor_schema_id IS NULL
            AND predecessor_content_digest IS NULL
        )
        OR
        (
            ordinal > 0
            AND predecessor_schema_id ~
                '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
            AND predecessor_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
        )
    ),
    CONSTRAINT executor_effect_frontiers_payload_v1 CHECK (
        octet_length(durable_payload) BETWEEN 1 AND 16777216
    ),
    CONSTRAINT executor_effect_frontiers_predecessor_fk
        FOREIGN KEY (effect_key, predecessor_schema_id, predecessor_content_digest)
        REFERENCES executor_effect_frontiers (
            effect_key,
            frontier_schema_id,
            frontier_content_digest
        )
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE executor_resource_records (
    resource_ownership_schema_id TEXT NOT NULL,
    resource_ownership_content_digest TEXT NOT NULL,
    resource_key_schema_id TEXT NOT NULL,
    resource_key_content_digest TEXT NOT NULL,
    ordinal BIGINT NOT NULL,
    record_schema_id TEXT NOT NULL,
    record_content_digest TEXT NOT NULL,
    predecessor_schema_id TEXT,
    predecessor_content_digest TEXT,
    linked_effect_key TEXT NOT NULL,
    durable_payload BYTEA NOT NULL,
    PRIMARY KEY (
        resource_ownership_schema_id,
        resource_ownership_content_digest,
        resource_key_schema_id,
        resource_key_content_digest,
        ordinal
    ),
    CONSTRAINT executor_resource_records_ref_key UNIQUE (
        resource_ownership_schema_id,
        resource_ownership_content_digest,
        resource_key_schema_id,
        resource_key_content_digest,
        record_schema_id,
        record_content_digest
    ),
    CONSTRAINT executor_resource_records_owner_schema_v1 CHECK (
        resource_ownership_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_resource_records_owner_digest_v1 CHECK (
        resource_ownership_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_resource_records_key_schema_v1 CHECK (
        resource_key_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_resource_records_key_digest_v1 CHECK (
        resource_key_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_resource_records_ordinal_v1 CHECK (ordinal >= 0),
    CONSTRAINT executor_resource_records_record_schema_v1 CHECK (
        record_schema_id ~
            '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_resource_records_record_digest_v1 CHECK (
        record_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_resource_records_predecessor_shape_v1 CHECK (
        (
            ordinal = 0
            AND predecessor_schema_id IS NULL
            AND predecessor_content_digest IS NULL
        )
        OR
        (
            ordinal > 0
            AND predecessor_schema_id ~
                '^schema:[a-z0-9][a-z0-9._/-]*:1:sha256-jcs-v1:[0-9a-f]{64}$'
            AND predecessor_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'
        )
    ),
    CONSTRAINT executor_resource_records_effect_v1 CHECK (
        linked_effect_key ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_resource_records_payload_v1 CHECK (
        octet_length(durable_payload) BETWEEN 1 AND 16777216
    ),
    CONSTRAINT executor_resource_records_predecessor_fk
        FOREIGN KEY (
            resource_ownership_schema_id,
            resource_ownership_content_digest,
            resource_key_schema_id,
            resource_key_content_digest,
            predecessor_schema_id,
            predecessor_content_digest
        )
        REFERENCES executor_resource_records (
            resource_ownership_schema_id,
            resource_ownership_content_digest,
            resource_key_schema_id,
            resource_key_content_digest,
            record_schema_id,
            record_content_digest
        )
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE executor_effect_resource_links (
    effect_key TEXT PRIMARY KEY,
    effect_frontier_schema_id TEXT NOT NULL,
    effect_frontier_content_digest TEXT NOT NULL,
    resource_ownership_schema_id TEXT NOT NULL,
    resource_ownership_content_digest TEXT NOT NULL,
    resource_key_schema_id TEXT NOT NULL,
    resource_key_content_digest TEXT NOT NULL,
    resource_record_schema_id TEXT NOT NULL,
    resource_record_content_digest TEXT NOT NULL,
    CONSTRAINT executor_effect_resource_links_effect_v1 CHECK (
        effect_key ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'
    ),
    CONSTRAINT executor_effect_resource_links_effect_frontier_fk
        FOREIGN KEY (
            effect_key,
            effect_frontier_schema_id,
            effect_frontier_content_digest
        )
        REFERENCES executor_effect_frontiers (
            effect_key,
            frontier_schema_id,
            frontier_content_digest
        )
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT executor_effect_resource_links_resource_record_fk
        FOREIGN KEY (
            resource_ownership_schema_id,
            resource_ownership_content_digest,
            resource_key_schema_id,
            resource_key_content_digest,
            resource_record_schema_id,
            resource_record_content_digest
        )
        REFERENCES executor_resource_records (
            resource_ownership_schema_id,
            resource_ownership_content_digest,
            resource_key_schema_id,
            resource_key_content_digest,
            record_schema_id,
            record_content_digest
        )
        DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX executor_resource_records_linked_effect_idx
    ON executor_resource_records (linked_effect_key);

CREATE VIEW executor_effect_heads AS
SELECT DISTINCT ON (effect_key)
    effect_key,
    ordinal,
    frontier_schema_id,
    frontier_content_digest
FROM executor_effect_frontiers
ORDER BY effect_key, ordinal DESC;

CREATE VIEW executor_resource_heads AS
SELECT DISTINCT ON (
    resource_ownership_schema_id,
    resource_ownership_content_digest,
    resource_key_schema_id,
    resource_key_content_digest
)
    resource_ownership_schema_id,
    resource_ownership_content_digest,
    resource_key_schema_id,
    resource_key_content_digest,
    ordinal,
    record_schema_id,
    record_content_digest
FROM executor_resource_records
ORDER BY
    resource_ownership_schema_id,
    resource_ownership_content_digest,
    resource_key_schema_id,
    resource_key_content_digest,
    ordinal DESC;

CREATE FUNCTION mfm_executor_reject_authority_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $mfm_executor_function$
BEGIN
    RAISE EXCEPTION 'executor authority rows are immutable';
END
$mfm_executor_function$;

CREATE FUNCTION mfm_executor_reject_metadata_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $mfm_executor_function$
BEGIN
    RAISE EXCEPTION 'executor schema metadata is immutable';
END
$mfm_executor_function$;

CREATE TRIGGER executor_schema_metadata_no_mutation
    BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON executor_schema_metadata
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_executor_reject_metadata_mutation();

CREATE TRIGGER executor_bindings_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON executor_bindings
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_executor_reject_authority_mutation();

CREATE TRIGGER executor_content_records_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON executor_content_records
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_executor_reject_authority_mutation();

CREATE TRIGGER executor_effect_frontiers_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON executor_effect_frontiers
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_executor_reject_authority_mutation();

CREATE TRIGGER executor_resource_records_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON executor_resource_records
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_executor_reject_authority_mutation();

CREATE TRIGGER executor_effect_resource_links_no_update
    BEFORE UPDATE OR DELETE OR TRUNCATE ON executor_effect_resource_links
    FOR EACH STATEMENT EXECUTE FUNCTION mfm_executor_reject_authority_mutation();

DO $mfm_executor_object_acl$
DECLARE
    schema_name TEXT := current_schema();
    object_name TEXT;
    function_arguments TEXT;
BEGIN
    FOREACH object_name IN ARRAY ARRAY[
        'executor_schema_metadata',
        'executor_bindings',
        'executor_content_records',
        'executor_effect_frontiers',
        'executor_resource_records',
        'executor_effect_resource_links'
    ]
    LOOP
        EXECUTE format('REVOKE ALL ON TABLE %I.%I FROM PUBLIC', schema_name, object_name);
        EXECUTE format(
            'ALTER TABLE %I.%I OWNER TO mfm_executor_owner',
            schema_name,
            object_name
        );
    END LOOP;

    FOREACH object_name IN ARRAY ARRAY[
        'executor_effect_heads',
        'executor_resource_heads'
    ]
    LOOP
        EXECUTE format('REVOKE ALL ON TABLE %I.%I FROM PUBLIC', schema_name, object_name);
        EXECUTE format(
            'ALTER VIEW %I.%I OWNER TO mfm_executor_owner',
            schema_name,
            object_name
        );
    END LOOP;

    FOREACH object_name IN ARRAY ARRAY[
        'mfm_executor_reject_authority_mutation',
        'mfm_executor_reject_metadata_mutation'
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
            'ALTER FUNCTION %I.%I(%s) OWNER TO mfm_executor_owner',
            schema_name,
            object_name,
            function_arguments
        );
    END LOOP;

    EXECUTE format(
        'GRANT SELECT ON TABLE
             %1$I.executor_schema_metadata,
             %1$I.executor_bindings,
             %1$I.executor_content_records,
             %1$I.executor_effect_frontiers,
             %1$I.executor_resource_records,
             %1$I.executor_effect_resource_links,
             %1$I.executor_effect_heads,
             %1$I.executor_resource_heads
         TO mfm_executor_application',
        schema_name
    );
    EXECUTE format(
        'GRANT SELECT ON TABLE %I._sqlx_migrations TO mfm_executor_application',
        schema_name
    );
    EXECUTE format(
        'GRANT INSERT ON TABLE
             %1$I.executor_bindings,
             %1$I.executor_content_records,
             %1$I.executor_effect_frontiers,
             %1$I.executor_resource_records,
             %1$I.executor_effect_resource_links
         TO mfm_executor_application',
        schema_name
    );
END
$mfm_executor_object_acl$;
