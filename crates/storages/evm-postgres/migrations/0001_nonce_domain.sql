CREATE TABLE mfm_evm_nonce_schema (
    schema_contract TEXT PRIMARY KEY,
    installed_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO mfm_evm_nonce_schema (schema_contract)
VALUES ('mfm.single-trust-evm-nonce-postgres.v1')
ON CONFLICT (schema_contract) DO NOTHING;

CREATE TABLE mfm_evm_nonce_domains (
    tenant_scope_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    nonce_domain_id TEXT NOT NULL,
    next_nonce BIGINT NOT NULL CHECK (next_nonce >= 0),
    PRIMARY KEY (tenant_scope_id, sender_id, nonce_domain_id)
);

CREATE TABLE mfm_evm_nonce_operations (
    tenant_scope_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    nonce_domain_id TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    nonce BIGINT NOT NULL CHECK (nonce >= 0),
    completed BOOLEAN NOT NULL,
    PRIMARY KEY (tenant_scope_id, sender_id, nonce_domain_id, operation_key),
    UNIQUE (tenant_scope_id, sender_id, nonce_domain_id, nonce),
    FOREIGN KEY (tenant_scope_id, sender_id, nonce_domain_id)
        REFERENCES mfm_evm_nonce_domains (tenant_scope_id, sender_id, nonce_domain_id)
);
