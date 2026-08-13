CREATE TABLE mfm_store_schema (
    schema_contract TEXT PRIMARY KEY,
    installed_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO mfm_store_schema (schema_contract)
VALUES ('mfm.structured-run-history-postgres.v8')
ON CONFLICT (schema_contract) DO NOTHING;

CREATE TABLE mfm_run_frames (
    store_scope_id TEXT NOT NULL,
    store_epoch BIGINT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    run_sequence BIGINT NOT NULL CHECK (run_sequence > 0),
    append_request_id TEXT NOT NULL,
    frame_bytes BYTEA NOT NULL,
    frame_digest TEXT NOT NULL,
    head_digest TEXT NOT NULL,
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence),
    UNIQUE (store_scope_id, store_epoch, tenant_scope_id, run_id, append_request_id)
);

CREATE TABLE mfm_run_heads (
    store_scope_id TEXT NOT NULL,
    store_epoch BIGINT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    head_sequence BIGINT NOT NULL CHECK (head_sequence > 0),
    head_digest TEXT NOT NULL,
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id, run_id),
    FOREIGN KEY (
        store_scope_id, store_epoch, tenant_scope_id, run_id, head_sequence
    ) REFERENCES mfm_run_frames (
        store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence
    )
);

CREATE INDEX mfm_run_frames_tenant_run_order
    ON mfm_run_frames (store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence);

CREATE TABLE mfm_fact_heads (
    store_scope_id TEXT NOT NULL,
    store_epoch BIGINT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    publication_sequence BIGINT NOT NULL CHECK (publication_sequence >= 0),
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id)
);

CREATE TABLE mfm_fact_publications (
    store_scope_id TEXT NOT NULL,
    store_epoch BIGINT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    publication_sequence BIGINT NOT NULL CHECK (publication_sequence > 0),
    run_id TEXT NOT NULL,
    run_sequence BIGINT NOT NULL CHECK (run_sequence > 0),
    proposal_set_ref TEXT NOT NULL,
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id, publication_sequence),
    FOREIGN KEY (
        store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence
    ) REFERENCES mfm_run_frames (
        store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence
    )
);

CREATE TABLE mfm_configuration_revisions (
    store_scope_id TEXT NOT NULL,
    store_epoch BIGINT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    revision_sequence BIGINT NOT NULL CHECK (revision_sequence > 0),
    append_request_id TEXT NOT NULL,
    canonical_bytes BYTEA NOT NULL,
    content_ref TEXT NOT NULL,
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id, revision_sequence),
    UNIQUE (store_scope_id, store_epoch, tenant_scope_id, append_request_id)
);

CREATE TABLE mfm_configuration_heads (
    store_scope_id TEXT NOT NULL,
    store_epoch BIGINT NOT NULL,
    tenant_scope_id TEXT NOT NULL,
    head_sequence BIGINT NOT NULL CHECK (head_sequence >= 0),
    total_bytes BIGINT NOT NULL CHECK (total_bytes >= 0),
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id),
    FOREIGN KEY (
        store_scope_id, store_epoch, tenant_scope_id, head_sequence
    ) REFERENCES mfm_configuration_revisions (
        store_scope_id, store_epoch, tenant_scope_id, revision_sequence
    )
);
