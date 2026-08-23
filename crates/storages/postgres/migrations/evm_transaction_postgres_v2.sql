CREATE SCHEMA mfm_evm_tx;

CREATE TABLE mfm_evm_tx.mfm_evm_tx_schema (
    schema_contract TEXT COLLATE "C" NOT NULL,
    authority_epoch BYTEA NOT NULL,
    CONSTRAINT mfm_evm_tx_schema_pkey PRIMARY KEY (schema_contract),
    CONSTRAINT mfm_evm_tx_schema_contract_check
        CHECK (schema_contract = 'mfm.evm-transaction-postgres.v2'),
    CONSTRAINT mfm_evm_tx_schema_epoch_check
        CHECK (octet_length(authority_epoch) = 32),
    CONSTRAINT mfm_evm_tx_schema_epoch_key UNIQUE (authority_epoch)
);

CREATE TABLE mfm_evm_tx.nonce_domains (
    authority_epoch       BYTEA NOT NULL,
    chain_id              NUMERIC(20,0) NOT NULL,
    genesis_hash          BYTEA NOT NULL,
    sender                BYTEA NOT NULL,
    CONSTRAINT nonce_domains_pkey
        PRIMARY KEY (authority_epoch, chain_id, genesis_hash, sender),
    CONSTRAINT nonce_domains_epoch_check
        CHECK (octet_length(authority_epoch) = 32),
    CONSTRAINT nonce_domains_chain_id_check
        CHECK (chain_id BETWEEN 1 AND 18446744073709551615),
    CONSTRAINT nonce_domains_genesis_hash_check
        CHECK (octet_length(genesis_hash) = 32),
    CONSTRAINT nonce_domains_sender_check
        CHECK (octet_length(sender) = 20),
    CONSTRAINT nonce_domains_epoch_fkey
        FOREIGN KEY (authority_epoch)
        REFERENCES mfm_evm_tx.mfm_evm_tx_schema (authority_epoch)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);

CREATE TABLE mfm_evm_tx.nonce_reservations (
    effect_id              TEXT COLLATE "C" NOT NULL,
    command_schema_id      TEXT COLLATE "C" NOT NULL,
    command_content_digest TEXT COLLATE "C" NOT NULL,
    authority_epoch        BYTEA NOT NULL,
    chain_id               NUMERIC(20,0) NOT NULL,
    genesis_hash           BYTEA NOT NULL,
    sender                 BYTEA NOT NULL,
    reserved_nonce         NUMERIC(20,0) NOT NULL,
    CONSTRAINT nonce_reservations_pkey PRIMARY KEY (effect_id),
    CONSTRAINT nonce_reservations_domain_nonce_key
        UNIQUE (authority_epoch, chain_id, genesis_hash, sender, reserved_nonce),
    CONSTRAINT nonce_reservations_effect_id_check
        CHECK (effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_reservations_command_schema_id_check
        CHECK (octet_length(command_schema_id) BETWEEN 1 AND 512 AND
               command_schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:[1-9][0-9]*:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_reservations_command_digest_check
        CHECK (command_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_reservations_epoch_check
        CHECK (octet_length(authority_epoch) = 32),
    CONSTRAINT nonce_reservations_chain_id_check
        CHECK (chain_id BETWEEN 1 AND 18446744073709551615),
    CONSTRAINT nonce_reservations_genesis_hash_check
        CHECK (octet_length(genesis_hash) = 32),
    CONSTRAINT nonce_reservations_sender_check
        CHECK (octet_length(sender) = 20),
    CONSTRAINT nonce_reservations_nonce_check
        CHECK (reserved_nonce BETWEEN 0 AND 18446744073709551615),
    CONSTRAINT nonce_reservations_domain_fkey
        FOREIGN KEY (authority_epoch, chain_id, genesis_hash, sender)
        REFERENCES mfm_evm_tx.nonce_domains
            (authority_epoch, chain_id, genesis_hash, sender)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);

CREATE TABLE mfm_evm_tx.prepared_transactions (
    effect_id        TEXT COLLATE "C" NOT NULL,
    transaction_hash BYTEA NOT NULL,
    raw_transaction  BYTEA NOT NULL,
    CONSTRAINT prepared_transactions_pkey PRIMARY KEY (effect_id),
    CONSTRAINT prepared_transactions_effect_id_check
        CHECK (effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT prepared_transactions_hash_check
        CHECK (octet_length(transaction_hash) = 32),
    CONSTRAINT prepared_transactions_raw_check
        CHECK (octet_length(raw_transaction) BETWEEN 1 AND 132096),
    CONSTRAINT prepared_transactions_reservation_fkey
        FOREIGN KEY (effect_id) REFERENCES mfm_evm_tx.nonce_reservations (effect_id)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);

CREATE TABLE mfm_evm_tx.transaction_settlements (
    effect_id        TEXT COLLATE "C" NOT NULL,
    settlement_bytes BYTEA NOT NULL,
    CONSTRAINT transaction_settlements_pkey PRIMARY KEY (effect_id),
    CONSTRAINT transaction_settlements_effect_id_check
        CHECK (effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT transaction_settlements_bytes_check
        CHECK (octet_length(settlement_bytes) BETWEEN 1 AND 65536),
    CONSTRAINT transaction_settlements_prepared_fkey
        FOREIGN KEY (effect_id) REFERENCES mfm_evm_tx.prepared_transactions (effect_id)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);

REVOKE ALL ON SCHEMA mfm_evm_tx FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_evm_tx.mfm_evm_tx_schema FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_evm_tx.nonce_domains FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_evm_tx.nonce_reservations FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_evm_tx.prepared_transactions FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE mfm_evm_tx.transaction_settlements FROM PUBLIC, mfm_runtime;
GRANT USAGE ON SCHEMA mfm_evm_tx TO mfm_runtime;
GRANT SELECT ON TABLE mfm_evm_tx.mfm_evm_tx_schema TO mfm_runtime;
GRANT SELECT, INSERT ON TABLE mfm_evm_tx.nonce_domains TO mfm_runtime;
GRANT SELECT, INSERT ON TABLE mfm_evm_tx.nonce_reservations TO mfm_runtime;
GRANT SELECT, INSERT ON TABLE mfm_evm_tx.prepared_transactions TO mfm_runtime;
GRANT SELECT, INSERT ON TABLE mfm_evm_tx.transaction_settlements TO mfm_runtime;
