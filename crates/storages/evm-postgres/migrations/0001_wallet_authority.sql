-- Sole current PostgreSQL authority for EVM wallet-domain activation and nonce progression.
--
-- This pre-release baseline deliberately has no compatibility relation to any
-- executor-owned wallet schema. An installation with different bytes must be
-- reset or promoted through the qualified deployment boundary.

DO $mfm_evm_wallet_roles$
DECLARE
    role_name text;
BEGIN
    IF current_user IN (
        'mfm_evm_wallet_activation_public',
        'mfm_evm_wallet_nonce_application',
        'mfm_store_application'
    ) THEN
        RAISE EXCEPTION 'the migration must run through the migration-owner boundary';
    END IF;

    FOREACH role_name IN ARRAY ARRAY[
        'mfm_evm_wallet_owner',
        'mfm_evm_wallet_activation_admin',
        'mfm_evm_wallet_activation_public',
        'mfm_evm_wallet_nonce_application',
        'mfm_store_application',
        'mfm_evm_wallet_test'
    ]
    LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = role_name
        ) THEN
            EXECUTE format(
                'CREATE ROLE %I NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS',
                role_name
            );
        ELSE
            EXECUTE format(
                'ALTER ROLE %I NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS',
                role_name
            );
        END IF;
        EXECUTE format(
            'GRANT %I TO %I WITH ADMIN FALSE, INHERIT TRUE, SET TRUE',
            role_name,
            current_user
        );
    END LOOP;
END
$mfm_evm_wallet_roles$;

DO $mfm_evm_wallet_schema_acl$
DECLARE
    schema_name text := current_schema();
BEGIN
    EXECUTE format('REVOKE ALL ON SCHEMA %I FROM PUBLIC', schema_name);
    EXECUTE format('GRANT USAGE, CREATE ON SCHEMA %I TO mfm_evm_wallet_owner', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_evm_wallet_activation_admin', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_evm_wallet_activation_public', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_evm_wallet_nonce_application', schema_name);
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO mfm_evm_wallet_test', schema_name);
END
$mfm_evm_wallet_schema_acl$;

CREATE TABLE wallet_store_schema_metadata (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    schema_contract_version TEXT NOT NULL,
    CONSTRAINT wallet_store_schema_metadata_singleton_v1 CHECK (singleton),
    CONSTRAINT wallet_store_schema_metadata_version_v1 CHECK (
        schema_contract_version = 'mfm.evm.wallet-authority-postgres.v1'
    )
);

INSERT INTO wallet_store_schema_metadata (singleton, schema_contract_version)
VALUES (TRUE, 'mfm.evm.wallet-authority-postgres.v1');

CREATE TABLE wallet_store_incarnations (
    wallet_nonce_store_lineage_id TEXT NOT NULL,
    writer_epoch NUMERIC(20, 0) NOT NULL,
    incarnation_ref TEXT NOT NULL UNIQUE,
    predecessor_incarnation_ref TEXT,
    incarnation_json TEXT NOT NULL,
    promotion_successor_ref TEXT UNIQUE,
    promotion_attestation_json TEXT,
    CONSTRAINT wallet_store_incarnations_primary_v1 PRIMARY KEY (
        wallet_nonce_store_lineage_id,
        writer_epoch
    ),
    CONSTRAINT wallet_store_incarnations_lineage_v1 CHECK (
        length(wallet_nonce_store_lineage_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT wallet_store_incarnations_epoch_v1 CHECK (
        writer_epoch >= 1
        AND writer_epoch <= 18446744073709551615::numeric
        AND trunc(writer_epoch) = writer_epoch
    ),
    CONSTRAINT wallet_store_incarnations_ref_v1 CHECK (
        length(incarnation_ref) BETWEEN 1 AND 1024
    ),
    CONSTRAINT wallet_store_incarnations_predecessor_v1 CHECK (
        (writer_epoch = 1 AND predecessor_incarnation_ref IS NULL)
        OR (writer_epoch > 1 AND predecessor_incarnation_ref IS NOT NULL)
    ),
    CONSTRAINT wallet_store_incarnations_promotion_pair_v1 CHECK (
        (promotion_successor_ref IS NULL) = (promotion_attestation_json IS NULL)
    ),
    CONSTRAINT wallet_store_incarnations_json_v1 CHECK (
        length(incarnation_json) BETWEEN 2 AND 1048576
        AND (
            promotion_attestation_json IS NULL
            OR length(promotion_attestation_json) BETWEEN 2 AND 1048576
        )
    )
);

CREATE TABLE wallet_store_lineage_heads (
    wallet_nonce_store_lineage_id TEXT PRIMARY KEY,
    current_writer_epoch NUMERIC(20, 0) NOT NULL,
    current_incarnation_ref TEXT NOT NULL UNIQUE,
    CONSTRAINT wallet_store_lineage_heads_lineage_v1 CHECK (
        length(wallet_nonce_store_lineage_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT wallet_store_lineage_heads_epoch_v1 CHECK (
        current_writer_epoch >= 1
        AND current_writer_epoch <= 18446744073709551615::numeric
        AND trunc(current_writer_epoch) = current_writer_epoch
    ),
    CONSTRAINT wallet_store_lineage_heads_incarnation_fk_v1 FOREIGN KEY (
        wallet_nonce_store_lineage_id,
        current_writer_epoch
    ) REFERENCES wallet_store_incarnations (
        wallet_nonce_store_lineage_id,
        writer_epoch
    )
);

CREATE TABLE wallet_domain_activations (
    wallet_nonce_domain_id TEXT PRIMARY KEY,
    activation_record_ref TEXT NOT NULL UNIQUE,
    wallet_nonce_store_lineage_id TEXT NOT NULL,
    observed_store_incarnation_ref TEXT NOT NULL,
    activation_record_json TEXT NOT NULL,
    registry_issuance_ref TEXT NOT NULL UNIQUE,
    activation_attestation_json TEXT NOT NULL,
    CONSTRAINT wallet_domain_activations_domain_v1 CHECK (
        length(wallet_nonce_domain_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT wallet_domain_activations_refs_v1 CHECK (
        length(activation_record_ref) BETWEEN 1 AND 1024
        AND length(observed_store_incarnation_ref) BETWEEN 1 AND 1024
        AND length(registry_issuance_ref) BETWEEN 1 AND 1024
    ),
    CONSTRAINT wallet_domain_activations_json_v1 CHECK (
        length(activation_record_json) BETWEEN 2 AND 4194304
        AND length(activation_attestation_json) BETWEEN 2 AND 4194304
    ),
    CONSTRAINT wallet_domain_activations_lineage_fk_v1 FOREIGN KEY (
        wallet_nonce_store_lineage_id
    ) REFERENCES wallet_store_lineage_heads (wallet_nonce_store_lineage_id)
);

CREATE TABLE wallet_nonce_domains (
    wallet_nonce_domain_id TEXT PRIMARY KEY,
    activation_record_ref TEXT NOT NULL UNIQUE,
    wallet_nonce_store_lineage_id TEXT NOT NULL,
    activation_record_json TEXT NOT NULL,
    activation_attestation_json TEXT NOT NULL,
    local_high_water_nonce NUMERIC(20, 0),
    retained_reservation_count NUMERIC(20, 0) NOT NULL DEFAULT 0,
    retained_reservation_chain_head_ref TEXT,
    active_reservation_key TEXT,
    current_resource_frontier_ref TEXT,
    current_incarnation_ref TEXT,
    CONSTRAINT wallet_nonce_domains_domain_v1 CHECK (
        length(wallet_nonce_domain_id) BETWEEN 1 AND 512
    ),
    CONSTRAINT wallet_nonce_domains_activation_ref_v1 CHECK (
        length(activation_record_ref) BETWEEN 1 AND 1024
    ),
    CONSTRAINT wallet_nonce_domains_json_v1 CHECK (
        length(activation_record_json) BETWEEN 2 AND 4194304
        AND length(activation_attestation_json) BETWEEN 2 AND 4194304
    ),
    -- EIP-2681: nonce 2^64-1 is invalid; high water is at most 2^64-2.
    CONSTRAINT wallet_nonce_domains_high_water_v1 CHECK (
        local_high_water_nonce IS NULL
        OR (
            local_high_water_nonce >= 0
            AND local_high_water_nonce <= 18446744073709551614::numeric
            AND trunc(local_high_water_nonce) = local_high_water_nonce
        )
    ),
    CONSTRAINT wallet_nonce_domains_retained_count_v1 CHECK (
        retained_reservation_count >= 0
        AND retained_reservation_count <= 18446744073709551614::numeric
        AND trunc(retained_reservation_count) = retained_reservation_count
        AND (
            (retained_reservation_count = 0 AND retained_reservation_chain_head_ref IS NULL)
            OR (retained_reservation_count > 0
                AND retained_reservation_chain_head_ref IS NOT NULL
                AND length(retained_reservation_chain_head_ref) BETWEEN 1 AND 512)
        )
    ),
    CONSTRAINT wallet_nonce_domains_projection_v1 CHECK (
        (active_reservation_key IS NULL OR length(active_reservation_key) BETWEEN 1 AND 512)
        AND (current_resource_frontier_ref IS NULL OR length(current_resource_frontier_ref) BETWEEN 1 AND 1024)
        AND (current_incarnation_ref IS NULL OR length(current_incarnation_ref) BETWEEN 1 AND 1024)
    )
);

CREATE TABLE wallet_nonce_reservations (
    semantic_reservation_key TEXT PRIMARY KEY,
    wallet_nonce_domain_id TEXT NOT NULL,
    submission_intent_id TEXT NOT NULL,
    nonce NUMERIC(20, 0) NOT NULL,
    transaction_intent_digest TEXT NOT NULL,
    candidate_family_ref TEXT NOT NULL,
    submission_semantics_digest TEXT NOT NULL,
    request_json TEXT NOT NULL,
    transaction_intent_json TEXT NOT NULL,
    candidate_family_json TEXT NOT NULL,
    reservation_json TEXT NOT NULL,
    state_input_json TEXT NOT NULL,
    CONSTRAINT wallet_nonce_reservations_intent_v1 UNIQUE (
        wallet_nonce_domain_id,
        submission_intent_id
    ),
    CONSTRAINT wallet_nonce_reservations_nonce_v1 UNIQUE (
        wallet_nonce_domain_id,
        nonce
    ),
    CONSTRAINT wallet_nonce_reservations_domain_fk_v1 FOREIGN KEY (
        wallet_nonce_domain_id
    ) REFERENCES wallet_nonce_domains (wallet_nonce_domain_id),
    -- EIP-2681: transaction nonce 2^64-1 is permanently invalid.
    CONSTRAINT wallet_nonce_reservations_quantity_v1 CHECK (
        nonce >= 0
        AND nonce <= 18446744073709551614::numeric
        AND trunc(nonce) = nonce
    ),
    CONSTRAINT wallet_nonce_reservations_identity_v1 CHECK (
        length(semantic_reservation_key) BETWEEN 1 AND 512
        AND length(submission_intent_id) BETWEEN 1 AND 512
        AND length(transaction_intent_digest) BETWEEN 1 AND 512
        AND length(candidate_family_ref) BETWEEN 1 AND 512
        AND length(submission_semantics_digest) BETWEEN 1 AND 512
    ),
    CONSTRAINT wallet_nonce_reservations_json_v1 CHECK (
        length(request_json) BETWEEN 2 AND 8388608
        AND length(transaction_intent_json) BETWEEN 2 AND 4194304
        AND length(candidate_family_json) BETWEEN 2 AND 4194304
        AND length(reservation_json) BETWEEN 2 AND 4194304
        AND length(state_input_json) BETWEEN 2 AND 4194304
    )
);

CREATE TABLE wallet_nonce_candidates (
    semantic_candidate_operation_key TEXT PRIMARY KEY,
    semantic_reservation_key TEXT NOT NULL,
    candidate_ordinal INTEGER NOT NULL,
    request_json TEXT NOT NULL,
    active_candidate_json TEXT NOT NULL,
    state_input_json TEXT NOT NULL,
    CONSTRAINT wallet_nonce_candidates_ordinal_v1 UNIQUE (
        semantic_reservation_key,
        candidate_ordinal
    ),
    CONSTRAINT wallet_nonce_candidates_reservation_fk_v1 FOREIGN KEY (
        semantic_reservation_key
    ) REFERENCES wallet_nonce_reservations (semantic_reservation_key),
    CONSTRAINT wallet_nonce_candidates_ordinal_bound_v1 CHECK (
        candidate_ordinal >= 0 AND candidate_ordinal < 32
    ),
    CONSTRAINT wallet_nonce_candidates_identity_v1 CHECK (
        length(semantic_candidate_operation_key) BETWEEN 1 AND 512
        AND length(semantic_reservation_key) BETWEEN 1 AND 512
    ),
    CONSTRAINT wallet_nonce_candidates_json_v1 CHECK (
        length(request_json) BETWEEN 2 AND 4194304
        AND length(active_candidate_json) BETWEEN 2 AND 4194304
        AND length(state_input_json) BETWEEN 2 AND 4194304
    )
);

CREATE TABLE wallet_nonce_completions (
    semantic_completion_key TEXT PRIMARY KEY,
    semantic_reservation_key TEXT NOT NULL UNIQUE,
    request_json TEXT NOT NULL,
    canonical_terminal_outcome_json TEXT NOT NULL,
    completion_json TEXT NOT NULL,
    state_input_json TEXT NOT NULL,
    CONSTRAINT wallet_nonce_completions_reservation_fk_v1 FOREIGN KEY (
        semantic_reservation_key
    ) REFERENCES wallet_nonce_reservations (semantic_reservation_key),
    CONSTRAINT wallet_nonce_completions_identity_v1 CHECK (
        length(semantic_completion_key) BETWEEN 1 AND 512
        AND length(semantic_reservation_key) BETWEEN 1 AND 512
    ),
    CONSTRAINT wallet_nonce_completions_json_v1 CHECK (
        length(request_json) BETWEEN 2 AND 8388608
        AND length(canonical_terminal_outcome_json) BETWEEN 2 AND 4194304
        AND length(completion_json) BETWEEN 2 AND 4194304
        AND length(state_input_json) BETWEEN 2 AND 4194304
    )
);

CREATE FUNCTION reject_immutable_wallet_row_change()
RETURNS trigger
LANGUAGE plpgsql
AS $mfm_wallet_immutable$
BEGIN
    RAISE EXCEPTION 'immutable EVM wallet authority row';
END
$mfm_wallet_immutable$;

CREATE FUNCTION enforce_wallet_lineage_head_cas()
RETURNS trigger
LANGUAGE plpgsql
AS $mfm_wallet_head_cas$
BEGIN
    IF NEW.wallet_nonce_store_lineage_id <> OLD.wallet_nonce_store_lineage_id
       OR NEW.current_writer_epoch <> OLD.current_writer_epoch + 1
       OR NEW.current_incarnation_ref = OLD.current_incarnation_ref THEN
        RAISE EXCEPTION 'invalid wallet lineage-head transition';
    END IF;
    RETURN NEW;
END
$mfm_wallet_head_cas$;

CREATE FUNCTION enforce_wallet_nonce_domain_update()
RETURNS trigger
LANGUAGE plpgsql
AS $mfm_wallet_domain_update$
BEGIN
    IF NEW.wallet_nonce_domain_id <> OLD.wallet_nonce_domain_id
       OR NEW.activation_record_ref <> OLD.activation_record_ref
       OR NEW.wallet_nonce_store_lineage_id <> OLD.wallet_nonce_store_lineage_id
       OR NEW.activation_record_json <> OLD.activation_record_json
       OR NEW.activation_attestation_json <> OLD.activation_attestation_json
       OR NEW.local_high_water_nonce IS NULL
       OR (OLD.local_high_water_nonce IS NOT NULL
           AND NEW.local_high_water_nonce < OLD.local_high_water_nonce)
       OR (NEW.local_high_water_nonce = OLD.local_high_water_nonce
           AND NEW.retained_reservation_count = OLD.retained_reservation_count
           AND NEW.retained_reservation_chain_head_ref
               IS NOT DISTINCT FROM OLD.retained_reservation_chain_head_ref
           AND NEW.active_reservation_key IS NOT DISTINCT FROM OLD.active_reservation_key
           AND NEW.current_resource_frontier_ref IS NOT DISTINCT FROM OLD.current_resource_frontier_ref
           AND NEW.current_incarnation_ref IS NOT DISTINCT FROM OLD.current_incarnation_ref) THEN
        RAISE EXCEPTION 'invalid wallet nonce-domain transition';
    END IF;
    RETURN NEW;
END
$mfm_wallet_domain_update$;

CREATE TRIGGER wallet_store_incarnations_immutable_v1
BEFORE UPDATE OR DELETE ON wallet_store_incarnations
FOR EACH ROW EXECUTE FUNCTION reject_immutable_wallet_row_change();

CREATE TRIGGER wallet_domain_activations_immutable_v1
BEFORE UPDATE OR DELETE ON wallet_domain_activations
FOR EACH ROW EXECUTE FUNCTION reject_immutable_wallet_row_change();

CREATE TRIGGER wallet_nonce_reservations_immutable_v1
BEFORE UPDATE OR DELETE ON wallet_nonce_reservations
FOR EACH ROW EXECUTE FUNCTION reject_immutable_wallet_row_change();

CREATE TRIGGER wallet_nonce_candidates_immutable_v1
BEFORE UPDATE OR DELETE ON wallet_nonce_candidates
FOR EACH ROW EXECUTE FUNCTION reject_immutable_wallet_row_change();

CREATE TRIGGER wallet_nonce_completions_immutable_v1
BEFORE UPDATE OR DELETE ON wallet_nonce_completions
FOR EACH ROW EXECUTE FUNCTION reject_immutable_wallet_row_change();

CREATE TRIGGER wallet_store_lineage_heads_cas_v1
BEFORE UPDATE ON wallet_store_lineage_heads
FOR EACH ROW EXECUTE FUNCTION enforce_wallet_lineage_head_cas();

CREATE TRIGGER wallet_store_lineage_heads_no_delete_v1
BEFORE DELETE ON wallet_store_lineage_heads
FOR EACH ROW EXECUTE FUNCTION reject_immutable_wallet_row_change();

CREATE TRIGGER wallet_nonce_domains_update_v1
BEFORE UPDATE ON wallet_nonce_domains
FOR EACH ROW EXECUTE FUNCTION enforce_wallet_nonce_domain_update();

CREATE TRIGGER wallet_nonce_domains_no_delete_v1
BEFORE DELETE ON wallet_nonce_domains
FOR EACH ROW EXECUTE FUNCTION reject_immutable_wallet_row_change();

ALTER TABLE wallet_store_schema_metadata OWNER TO mfm_evm_wallet_owner;
ALTER TABLE wallet_store_incarnations OWNER TO mfm_evm_wallet_owner;
ALTER TABLE wallet_store_lineage_heads OWNER TO mfm_evm_wallet_owner;
ALTER TABLE wallet_domain_activations OWNER TO mfm_evm_wallet_owner;
ALTER TABLE wallet_nonce_domains OWNER TO mfm_evm_wallet_owner;
ALTER TABLE wallet_nonce_reservations OWNER TO mfm_evm_wallet_owner;
ALTER TABLE wallet_nonce_candidates OWNER TO mfm_evm_wallet_owner;
ALTER TABLE wallet_nonce_completions OWNER TO mfm_evm_wallet_owner;
ALTER FUNCTION reject_immutable_wallet_row_change() OWNER TO mfm_evm_wallet_owner;
ALTER FUNCTION enforce_wallet_lineage_head_cas() OWNER TO mfm_evm_wallet_owner;
ALTER FUNCTION enforce_wallet_nonce_domain_update() OWNER TO mfm_evm_wallet_owner;

REVOKE ALL ON TABLE wallet_store_schema_metadata FROM PUBLIC;
REVOKE ALL ON TABLE wallet_store_incarnations FROM PUBLIC;
REVOKE ALL ON TABLE wallet_store_lineage_heads FROM PUBLIC;
REVOKE ALL ON TABLE wallet_domain_activations FROM PUBLIC;
REVOKE ALL ON TABLE wallet_nonce_domains FROM PUBLIC;
REVOKE ALL ON TABLE wallet_nonce_reservations FROM PUBLIC;
REVOKE ALL ON TABLE wallet_nonce_candidates FROM PUBLIC;
REVOKE ALL ON TABLE wallet_nonce_completions FROM PUBLIC;
REVOKE ALL ON FUNCTION reject_immutable_wallet_row_change() FROM PUBLIC;
REVOKE ALL ON FUNCTION enforce_wallet_lineage_head_cas() FROM PUBLIC;
REVOKE ALL ON FUNCTION enforce_wallet_nonce_domain_update() FROM PUBLIC;
REVOKE ALL ON TABLE _sqlx_migrations FROM PUBLIC;

GRANT SELECT ON TABLE _sqlx_migrations TO mfm_evm_wallet_activation_admin;
GRANT SELECT ON TABLE _sqlx_migrations TO mfm_evm_wallet_activation_public;
GRANT SELECT ON TABLE _sqlx_migrations TO mfm_evm_wallet_nonce_application;
GRANT SELECT ON TABLE _sqlx_migrations TO mfm_evm_wallet_test;

DO $mfm_evm_wallet_exact_grants$
DECLARE
    schema_name text := current_schema();
BEGIN
    EXECUTE format(
        'GRANT SELECT ON TABLE %I.wallet_store_schema_metadata TO mfm_evm_wallet_activation_admin',
        schema_name
    );
    EXECUTE format(
        'GRANT SELECT, INSERT ON TABLE %I.wallet_store_incarnations TO mfm_evm_wallet_activation_admin',
        schema_name
    );
    EXECUTE format(
        'GRANT SELECT, INSERT, UPDATE ON TABLE %I.wallet_store_lineage_heads TO mfm_evm_wallet_activation_admin',
        schema_name
    );
    EXECUTE format(
        'GRANT SELECT, INSERT ON TABLE %I.wallet_domain_activations TO mfm_evm_wallet_activation_admin',
        schema_name
    );

    EXECUTE format(
        'GRANT SELECT ON TABLE %I.wallet_store_schema_metadata, %I.wallet_store_incarnations, %I.wallet_store_lineage_heads, %I.wallet_domain_activations TO mfm_evm_wallet_activation_public',
        schema_name,
        schema_name,
        schema_name,
        schema_name
    );

    EXECUTE format(
        'GRANT SELECT ON TABLE %I.wallet_store_schema_metadata TO mfm_evm_wallet_nonce_application',
        schema_name
    );
    EXECUTE format(
        'GRANT SELECT, INSERT, UPDATE ON TABLE %I.wallet_nonce_domains TO mfm_evm_wallet_nonce_application',
        schema_name
    );
    EXECUTE format(
        'GRANT SELECT, INSERT ON TABLE %I.wallet_nonce_reservations, %I.wallet_nonce_candidates, %I.wallet_nonce_completions TO mfm_evm_wallet_nonce_application',
        schema_name,
        schema_name,
        schema_name
    );

    EXECUTE format(
        'GRANT SELECT ON TABLE %I.wallet_store_schema_metadata, %I.wallet_store_incarnations, %I.wallet_store_lineage_heads, %I.wallet_domain_activations, %I.wallet_nonce_domains, %I.wallet_nonce_reservations, %I.wallet_nonce_candidates, %I.wallet_nonce_completions TO mfm_evm_wallet_test',
        schema_name,
        schema_name,
        schema_name,
        schema_name,
        schema_name,
        schema_name,
        schema_name,
        schema_name
    );
END
$mfm_evm_wallet_exact_grants$;
