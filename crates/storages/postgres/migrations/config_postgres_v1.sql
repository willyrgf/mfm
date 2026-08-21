CREATE SCHEMA mfm_config;

CREATE TABLE mfm_config.mfm_config_schema (
    schema_contract TEXT COLLATE "C" NOT NULL,
    CONSTRAINT mfm_config_schema_pkey PRIMARY KEY (schema_contract),
    CONSTRAINT mfm_config_schema_contract_check
        CHECK (schema_contract = 'mfm.config-postgres.v1')
);

INSERT INTO mfm_config.mfm_config_schema (schema_contract)
VALUES ('mfm.config-postgres.v1');

CREATE TABLE mfm_config.config_revisions (
    config_name   TEXT COLLATE "C" NOT NULL,
    config_digest TEXT COLLATE "C" NOT NULL,
    canonical     BYTEA NOT NULL,
    current       BOOLEAN NOT NULL,
    CONSTRAINT mfm_config_revisions_pkey PRIMARY KEY (config_name, config_digest),
    CONSTRAINT mfm_config_revisions_name_length_check
        CHECK (octet_length(config_name) BETWEEN 1 AND 64),
    CONSTRAINT mfm_config_revisions_name_grammar_check
        CHECK (config_name ~ '^[a-z0-9][a-z0-9-]*$' AND right(config_name, 1) <> '-'),
    CONSTRAINT mfm_config_revisions_digest_check
        CHECK (config_digest ~ '^content:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT mfm_config_revisions_bytes_check
        CHECK (octet_length(canonical) BETWEEN 1 AND 262144)
);

CREATE UNIQUE INDEX mfm_config_revisions_current_key
ON mfm_config.config_revisions (config_name) WHERE current;

REVOKE ALL ON SCHEMA mfm_config FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_config.mfm_config_schema FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_config.config_revisions FROM PUBLIC, mfm_runtime;
GRANT USAGE ON SCHEMA mfm_config TO mfm_runtime;
GRANT SELECT ON TABLE mfm_config.mfm_config_schema TO mfm_runtime;
GRANT SELECT, INSERT, UPDATE ON TABLE mfm_config.config_revisions TO mfm_runtime;
