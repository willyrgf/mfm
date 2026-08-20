CREATE SCHEMA mfm_catalog;

CREATE TABLE mfm_catalog.mfm_catalog_schema (
    schema_contract TEXT COLLATE "C" NOT NULL,
    CONSTRAINT mfm_catalog_schema_pkey PRIMARY KEY (schema_contract),
    CONSTRAINT mfm_catalog_schema_contract_check
        CHECK (schema_contract = 'mfm.config-catalog-postgres.v1')
);

INSERT INTO mfm_catalog.mfm_catalog_schema (schema_contract)
VALUES ('mfm.config-catalog-postgres.v1');

CREATE TABLE mfm_catalog.config_entries (
    config_name   TEXT COLLATE "C" NOT NULL,
    config_digest TEXT COLLATE "C" NOT NULL,
    canonical     BYTEA NOT NULL,
    CONSTRAINT mfm_catalog_config_entries_pkey PRIMARY KEY (config_name),
    CONSTRAINT mfm_catalog_config_entries_name_length_check
        CHECK (octet_length(config_name) BETWEEN 1 AND 64),
    CONSTRAINT mfm_catalog_config_entries_name_grammar_check
        CHECK (config_name ~ '^[a-z0-9][a-z0-9-]*$' AND right(config_name, 1) <> '-'),
    CONSTRAINT mfm_catalog_config_entries_digest_check
        CHECK (config_digest ~ '^content:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT mfm_catalog_config_entries_bytes_check
        CHECK (octet_length(canonical) BETWEEN 1 AND 262144)
);

REVOKE ALL ON SCHEMA mfm_catalog FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_catalog.mfm_catalog_schema FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_catalog.config_entries FROM PUBLIC, mfm_runtime;
GRANT USAGE ON SCHEMA mfm_catalog TO mfm_runtime;
GRANT SELECT ON TABLE mfm_catalog.mfm_catalog_schema TO mfm_runtime;
GRANT SELECT, INSERT, DELETE ON TABLE mfm_catalog.config_entries TO mfm_runtime;
