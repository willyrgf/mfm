use sqlx::PgConnection;

use crate::GateError;

const OWNER_SCHEMA_PRIVILEGES: &[&str] = &["CREATE", "USAGE"];
const OWNER_TABLE_PRIVILEGES: &[&str] = &[
    "DELETE",
    "INSERT",
    "MAINTAIN",
    "REFERENCES",
    "SELECT",
    "TRIGGER",
    "TRUNCATE",
    "UPDATE",
];

pub(crate) const RUN_SURFACE: PgSurfaceSpec = PgSurfaceSpec {
    schema: "public",
    closed: false,
    relations: &[
        RelationSpec {
            name: "mfm_run_frames",
            columns: &[
                column(1, "run_id", "text", -1, true, Some("C")),
                column(2, "run_sequence", "int8", -1, true, None),
                column(3, "frame_bytes", "bytea", -1, true, None),
                column(4, "head_digest", "text", -1, true, Some("C")),
            ],
            constraints: &[
                constraint("mfm_run_frames_bytes_check", "c", "CHECK (((octet_length(frame_bytes) >= 1) AND (octet_length(frame_bytes) <= 25231360)))"),
                constraint("mfm_run_frames_digest_check", "c", "CHECK ((head_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'::text))"),
                constraint("mfm_run_frames_pkey", "p", "PRIMARY KEY (run_id, run_sequence)"),
                constraint("mfm_run_frames_run_id_check", "c", "CHECK ((run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
                constraint("mfm_run_frames_sequence_check", "c", "CHECK (((run_sequence >= 1) AND (run_sequence <= 65536)))"),
            ],
            indexes: &[index("mfm_run_frames_pkey", "CREATE UNIQUE INDEX mfm_run_frames_pkey ON public.mfm_run_frames USING btree (run_id, run_sequence)")],
            runtime_privileges: &["INSERT", "SELECT"],
        },
        RelationSpec {
            name: "mfm_run_heads",
            columns: &[
                column(1, "run_id", "text", -1, true, Some("C")),
                column(2, "head_sequence", "int8", -1, true, None),
                column(3, "total_bytes", "int8", -1, true, None),
            ],
            constraints: &[
                constraint("mfm_run_heads_frame_fkey", "f", "FOREIGN KEY (run_id, head_sequence) REFERENCES mfm_run_frames(run_id, run_sequence)"),
                constraint("mfm_run_heads_pkey", "p", "PRIMARY KEY (run_id)"),
                constraint("mfm_run_heads_run_id_check", "c", "CHECK ((run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
                constraint("mfm_run_heads_sequence_check", "c", "CHECK (((head_sequence >= 1) AND (head_sequence <= 65536)))"),
                constraint("mfm_run_heads_total_bytes_check", "c", "CHECK (((total_bytes >= 1) AND (total_bytes <= 536870912)))"),
            ],
            indexes: &[index("mfm_run_heads_pkey", "CREATE UNIQUE INDEX mfm_run_heads_pkey ON public.mfm_run_heads USING btree (run_id)")],
            runtime_privileges: &["INSERT", "SELECT", "UPDATE"],
        },
        RelationSpec {
            name: "mfm_store_schema",
            columns: &[column(1, "schema_contract", "text", -1, true, Some("C"))],
            constraints: &[
                constraint("mfm_store_schema_contract_check", "c", "CHECK ((schema_contract = 'mfm.run-history-postgres.v1'::text))"),
                constraint("mfm_store_schema_pkey", "p", "PRIMARY KEY (schema_contract)"),
            ],
            indexes: &[index("mfm_store_schema_pkey", "CREATE UNIQUE INDEX mfm_store_schema_pkey ON public.mfm_store_schema USING btree (schema_contract)")],
            runtime_privileges: &["SELECT"],
        },
    ],
};

pub(crate) const CONFIG_SURFACE: PgSurfaceSpec = PgSurfaceSpec {
    schema: "mfm_config",
    closed: true,
    relations: &[
        RelationSpec {
            name: "config_revisions",
            columns: &[
                column(1, "config_name", "text", -1, true, Some("C")),
                column(2, "config_digest", "text", -1, true, Some("C")),
                column(3, "canonical", "bytea", -1, true, None),
            ],
            constraints: &[
                constraint("mfm_config_revisions_bytes_check", "c", "CHECK (((octet_length(canonical) >= 1) AND (octet_length(canonical) <= 262144)))"),
                constraint("mfm_config_revisions_digest_check", "c", "CHECK ((config_digest ~ '^content:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
                constraint("mfm_config_revisions_name_grammar_check", "c", "CHECK (((config_name ~ '^[a-z0-9][a-z0-9-]*$'::text) AND (\"right\"(config_name, 1) <> '-'::text)))"),
                constraint("mfm_config_revisions_name_length_check", "c", "CHECK (((octet_length(config_name) >= 1) AND (octet_length(config_name) <= 64)))"),
                constraint("mfm_config_revisions_pkey", "p", "PRIMARY KEY (config_name, config_digest)"),
            ],
            indexes: &[index("mfm_config_revisions_pkey", "CREATE UNIQUE INDEX mfm_config_revisions_pkey ON mfm_config.config_revisions USING btree (config_name, config_digest)")],
            runtime_privileges: &["DELETE", "INSERT", "SELECT"],
        },
        RelationSpec {
            name: "mfm_config_schema",
            columns: &[column(1, "schema_contract", "text", -1, true, Some("C"))],
            constraints: &[
                constraint("mfm_config_schema_contract_check", "c", "CHECK ((schema_contract = 'mfm.config-postgres.v2'::text))"),
                constraint("mfm_config_schema_pkey", "p", "PRIMARY KEY (schema_contract)"),
            ],
            indexes: &[index("mfm_config_schema_pkey", "CREATE UNIQUE INDEX mfm_config_schema_pkey ON mfm_config.mfm_config_schema USING btree (schema_contract)")],
            runtime_privileges: &["SELECT"],
        },
    ],
};

pub(crate) const EVM_TX_SURFACE: PgSurfaceSpec = PgSurfaceSpec {
    schema: "mfm_evm_tx",
    closed: true,
    relations: &[
        RelationSpec {
            name: "mfm_evm_tx_schema",
            columns: &[
                column(1, "schema_contract", "text", -1, true, Some("C")),
                column(2, "authority_epoch", "bytea", -1, true, None),
            ],
            constraints: &[
                constraint("mfm_evm_tx_schema_contract_check", "c", "CHECK ((schema_contract = 'mfm.evm-transaction-postgres.v2'::text))"),
                constraint("mfm_evm_tx_schema_epoch_check", "c", "CHECK ((octet_length(authority_epoch) = 32))"),
                constraint("mfm_evm_tx_schema_epoch_key", "u", "UNIQUE (authority_epoch)"),
                constraint("mfm_evm_tx_schema_pkey", "p", "PRIMARY KEY (schema_contract)"),
            ],
            indexes: &[
                index("mfm_evm_tx_schema_epoch_key", "CREATE UNIQUE INDEX mfm_evm_tx_schema_epoch_key ON mfm_evm_tx.mfm_evm_tx_schema USING btree (authority_epoch)"),
                index("mfm_evm_tx_schema_pkey", "CREATE UNIQUE INDEX mfm_evm_tx_schema_pkey ON mfm_evm_tx.mfm_evm_tx_schema USING btree (schema_contract)"),
            ],
            runtime_privileges: &["SELECT"],
        },
        RelationSpec {
            name: "nonce_reservations",
            columns: &[
                column(1, "effect_id", "text", -1, true, Some("C")),
                column(2, "command_schema_id", "text", -1, true, Some("C")),
                column(3, "command_content_digest", "text", -1, true, Some("C")),
                column(4, "authority_epoch", "bytea", -1, true, None),
                column(5, "chain_id", "numeric", 1_310_724, true, None),
                column(6, "genesis_hash", "bytea", -1, true, None),
                column(7, "sender", "bytea", -1, true, None),
                column(8, "reserved_nonce", "numeric", 1_310_724, true, None),
            ],
            constraints: &[
                constraint("nonce_reservations_chain_id_check", "c", "CHECK (((chain_id >= (1)::numeric) AND (chain_id <= '18446744073709551615'::numeric)))"),
                constraint("nonce_reservations_command_digest_check", "c", "CHECK ((command_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'::text))"),
                constraint("nonce_reservations_command_schema_id_check", "c", "CHECK ((((octet_length(command_schema_id) >= 1) AND (octet_length(command_schema_id) <= 512)) AND (command_schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:[1-9][0-9]*:sha256-jcs-v1:[0-9a-f]{64}$'::text)))"),
                constraint("nonce_reservations_domain_nonce_key", "u", "UNIQUE (authority_epoch, chain_id, genesis_hash, sender, reserved_nonce)"),
                constraint("nonce_reservations_effect_id_check", "c", "CHECK ((effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
                constraint("nonce_reservations_epoch_check", "c", "CHECK ((octet_length(authority_epoch) = 32))"),
                constraint("nonce_reservations_epoch_fkey", "f", "FOREIGN KEY (authority_epoch) REFERENCES mfm_evm_tx.mfm_evm_tx_schema(authority_epoch)"),
                constraint("nonce_reservations_genesis_hash_check", "c", "CHECK ((octet_length(genesis_hash) = 32))"),
                constraint("nonce_reservations_nonce_check", "c", "CHECK (((reserved_nonce >= (0)::numeric) AND (reserved_nonce <= '18446744073709551614'::numeric)))"),
                constraint("nonce_reservations_pkey", "p", "PRIMARY KEY (effect_id)"),
                constraint("nonce_reservations_sender_check", "c", "CHECK ((octet_length(sender) = 20))"),
            ],
            indexes: &[
                index("nonce_reservations_domain_nonce_key", "CREATE UNIQUE INDEX nonce_reservations_domain_nonce_key ON mfm_evm_tx.nonce_reservations USING btree (authority_epoch, chain_id, genesis_hash, sender, reserved_nonce)"),
                index("nonce_reservations_pkey", "CREATE UNIQUE INDEX nonce_reservations_pkey ON mfm_evm_tx.nonce_reservations USING btree (effect_id)"),
            ],
            runtime_privileges: &["INSERT", "SELECT"],
        },
        RelationSpec {
            name: "prepared_transactions",
            columns: &[
                column(1, "effect_id", "text", -1, true, Some("C")),
                column(2, "transaction_hash", "bytea", -1, true, None),
                column(3, "raw_transaction", "bytea", -1, true, None),
            ],
            constraints: &[
                constraint("prepared_transactions_effect_id_check", "c", "CHECK ((effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'::text))"),
                constraint("prepared_transactions_hash_check", "c", "CHECK ((octet_length(transaction_hash) = 32))"),
                constraint("prepared_transactions_pkey", "p", "PRIMARY KEY (effect_id)"),
                constraint("prepared_transactions_raw_check", "c", "CHECK (((octet_length(raw_transaction) >= 1) AND (octet_length(raw_transaction) <= 132096)))"),
                constraint("prepared_transactions_reservation_fkey", "f", "FOREIGN KEY (effect_id) REFERENCES mfm_evm_tx.nonce_reservations(effect_id)"),
            ],
            indexes: &[index("prepared_transactions_pkey", "CREATE UNIQUE INDEX prepared_transactions_pkey ON mfm_evm_tx.prepared_transactions USING btree (effect_id)")],
            runtime_privileges: &["INSERT", "SELECT"],
        },

    ],
};

pub(crate) struct PgSurfaceSpec {
    schema: &'static str,
    closed: bool,
    relations: &'static [RelationSpec],
}

impl PgSurfaceSpec {
    pub(crate) const fn schema(&self) -> &'static str {
        self.schema
    }

    fn relation_names(&self) -> Vec<String> {
        self.relations
            .iter()
            .map(|relation| relation.name.to_owned())
            .collect()
    }
}

struct RelationSpec {
    name: &'static str,
    columns: &'static [ColumnSpec],
    constraints: &'static [ConstraintSpec],
    indexes: &'static [IndexSpec],
    runtime_privileges: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct ColumnSpec {
    ordinal: i16,
    name: &'static str,
    type_name: &'static str,
    type_modifier: i32,
    not_null: bool,
    collation: Option<&'static str>,
}

const fn column(
    ordinal: i16,
    name: &'static str,
    type_name: &'static str,
    type_modifier: i32,
    not_null: bool,
    collation: Option<&'static str>,
) -> ColumnSpec {
    ColumnSpec {
        ordinal,
        name,
        type_name,
        type_modifier,
        not_null,
        collation,
    }
}

#[derive(Clone, Copy)]
struct ConstraintSpec {
    name: &'static str,
    kind: &'static str,
    definition: &'static str,
}

const fn constraint(
    name: &'static str,
    kind: &'static str,
    definition: &'static str,
) -> ConstraintSpec {
    ConstraintSpec {
        name,
        kind,
        definition,
    }
}

#[derive(Clone, Copy)]
struct IndexSpec {
    name: &'static str,
    definition: &'static str,
}

const fn index(name: &'static str, definition: &'static str) -> IndexSpec {
    IndexSpec { name, definition }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RelationFact {
    name: String,
    kind: String,
    persistence: String,
    owner: String,
    access_method: Option<String>,
    options: String,
    tablespace: String,
    row_security: bool,
    force_row_security: bool,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ColumnFact {
    relation: String,
    ordinal: i16,
    name: String,
    type_name: Option<String>,
    type_modifier: i32,
    not_null: bool,
    collation: Option<String>,
    default_expression: Option<String>,
    generated: String,
    identity: String,
    dropped: bool,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ConstraintFact {
    relation: String,
    name: String,
    kind: String,
    definition: String,
    validated: bool,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct IndexFact {
    relation: String,
    name: String,
    owner: String,
    definition: String,
    unique: bool,
    nulls_not_distinct: bool,
    predicate: Option<String>,
    expressions: Option<String>,
    access_method: String,
    options: String,
    tablespace: String,
    valid: bool,
    ready: bool,
    live: bool,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AclGrant {
    relation: Option<String>,
    column: Option<String>,
    grantor: String,
    grantee: String,
    privilege: String,
    grant_option: bool,
}

#[derive(PartialEq, Eq)]
struct CatalogSnapshot {
    database_owner: String,
    schema_owner: String,
    relations: Vec<RelationFact>,
    columns: Vec<ColumnFact>,
    constraints: Vec<ConstraintFact>,
    indexes: Vec<IndexFact>,
    schema_acl: Vec<AclGrant>,
    relation_acl: Vec<AclGrant>,
    column_acl: Vec<AclGrant>,
    policies: Vec<String>,
    rules: Vec<String>,
    triggers: Vec<String>,
}

pub(crate) async fn verify_surface(
    connection: &mut PgConnection,
    spec: &PgSurfaceSpec,
    expected_owner: Option<&str>,
) -> Result<(), GateError> {
    let mut actual = load_catalog_snapshot(connection, spec).await?;
    let owner = expected_owner.unwrap_or(&actual.schema_owner);
    if owner == "mfm_runtime" || actual.database_owner != owner || actual.schema_owner != owner {
        return Err(GateError::Incompatible);
    }
    let expected = expected_snapshot(spec, owner);
    actual.sort();
    (actual == expected)
        .then_some(())
        .ok_or(GateError::Incompatible)
}

pub(crate) async fn surface_exists(
    connection: &mut PgConnection,
    spec: &PgSurfaceSpec,
) -> Result<bool, GateError> {
    let exists: bool = if spec.closed {
        sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = \
             $1) AS \"value!\"",
            spec.schema,
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| GateError::Unavailable)?
    } else {
        let names = spec.relation_names();
        sqlx::query_scalar!(
            "SELECT EXISTS( SELECT 1 FROM pg_catalog.pg_class c JOIN \
             pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname \
             = $1 AND c.relname = ANY($2::text[])) AS \"value!\"",
            spec.schema,
            &names,
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| GateError::Unavailable)?
    };
    Ok(exists)
}

impl CatalogSnapshot {
    fn sort(&mut self) {
        self.relations.sort();
        self.columns.sort();
        self.constraints.sort();
        self.indexes.sort();
        self.schema_acl.sort();
        self.relation_acl.sort();
        self.column_acl.sort();
        self.policies.sort();
        self.rules.sort();
        self.triggers.sort();
    }
}

fn expected_snapshot(spec: &PgSurfaceSpec, owner: &str) -> CatalogSnapshot {
    let mut snapshot = CatalogSnapshot {
        database_owner: owner.to_owned(),
        schema_owner: owner.to_owned(),
        relations: Vec::new(),
        columns: Vec::new(),
        constraints: Vec::new(),
        indexes: Vec::new(),
        schema_acl: Vec::new(),
        relation_acl: Vec::new(),
        column_acl: Vec::new(),
        policies: Vec::new(),
        rules: Vec::new(),
        triggers: Vec::new(),
    };
    for privilege in OWNER_SCHEMA_PRIVILEGES {
        snapshot
            .schema_acl
            .push(acl(None, None, owner, owner, privilege));
    }
    snapshot
        .schema_acl
        .push(acl(None, None, owner, "mfm_runtime", "USAGE"));

    for relation in spec.relations {
        snapshot.relations.push(RelationFact {
            name: relation.name.to_owned(),
            kind: "r".to_owned(),
            persistence: "p".to_owned(),
            owner: owner.to_owned(),
            access_method: Some("heap".to_owned()),
            options: String::new(),
            tablespace: "pg_default".to_owned(),
            row_security: false,
            force_row_security: false,
        });
        for column in relation.columns {
            snapshot.columns.push(ColumnFact {
                relation: relation.name.to_owned(),
                ordinal: column.ordinal,
                name: column.name.to_owned(),
                type_name: Some(column.type_name.to_owned()),
                type_modifier: column.type_modifier,
                not_null: column.not_null,
                collation: column.collation.map(str::to_owned),
                default_expression: None,
                generated: String::new(),
                identity: String::new(),
                dropped: false,
            });
            if column.not_null {
                snapshot.constraints.push(ConstraintFact {
                    relation: relation.name.to_owned(),
                    name: format!("{}_{}_not_null", relation.name, column.name),
                    kind: "n".to_owned(),
                    definition: format!("NOT NULL {}", column.name),
                    validated: true,
                });
            }
        }
        for constraint in relation.constraints {
            snapshot.constraints.push(ConstraintFact {
                relation: relation.name.to_owned(),
                name: constraint.name.to_owned(),
                kind: constraint.kind.to_owned(),
                definition: constraint.definition.to_owned(),
                validated: true,
            });
        }
        for index in relation.indexes {
            snapshot.indexes.push(IndexFact {
                relation: relation.name.to_owned(),
                name: index.name.to_owned(),
                owner: owner.to_owned(),
                definition: index.definition.to_owned(),
                unique: true,
                nulls_not_distinct: false,
                predicate: None,
                expressions: None,
                access_method: "btree".to_owned(),
                options: String::new(),
                tablespace: "pg_default".to_owned(),
                valid: true,
                ready: true,
                live: true,
            });
        }
        for privilege in OWNER_TABLE_PRIVILEGES {
            snapshot
                .relation_acl
                .push(acl(Some(relation.name), None, owner, owner, privilege));
        }
        for privilege in relation.runtime_privileges {
            snapshot.relation_acl.push(acl(
                Some(relation.name),
                None,
                owner,
                "mfm_runtime",
                privilege,
            ));
        }
    }
    snapshot.sort();
    snapshot
}

fn acl(
    relation: Option<&str>,
    column: Option<&str>,
    grantor: &str,
    grantee: &str,
    privilege: &str,
) -> AclGrant {
    AclGrant {
        relation: relation.map(str::to_owned),
        column: column.map(str::to_owned),
        grantor: grantor.to_owned(),
        grantee: grantee.to_owned(),
        privilege: privilege.to_owned(),
        grant_option: false,
    }
}

async fn load_catalog_snapshot(
    connection: &mut PgConnection,
    spec: &PgSurfaceSpec,
) -> Result<CatalogSnapshot, GateError> {
    let owners: Option<(String, String)> = sqlx::query!(
        "SELECT pg_get_userbyid(d.datdba) AS \"database_owner!\", \
         pg_get_userbyid(n.nspowner) AS \"schema_owner!\" FROM \
         pg_catalog.pg_namespace n JOIN pg_catalog.pg_database d ON d.datname = \
         current_database() WHERE n.nspname = $1",
        spec.schema,
    )
    .fetch_optional(&mut *connection)
    .await
    .map(|rows| rows.map(|row| (row.database_owner, row.schema_owner)))
    .map_err(|_| GateError::Unavailable)?;
    let (database_owner, schema_owner) = owners.ok_or(GateError::Incompatible)?;
    let relation_names = spec.relation_names();

    let relations = sqlx::query_as!(
        RelationFact,
        "SELECT c.relname::text AS \"name!\", c.relkind::text AS \"kind!\", \
         c.relpersistence::text AS \"persistence!\", pg_get_userbyid(c.relowner) \
         AS \"owner!\", am.amname::text AS \"access_method?\", COALESCE((SELECT \
         string_agg(option, ',' ORDER BY option) FROM unnest(c.reloptions) \
         option), '') AS \"options!\", COALESCE(ts.spcname, dbts.spcname) AS \
         \"tablespace!\", c.relrowsecurity AS \"row_security!\", \
         c.relforcerowsecurity AS \"force_row_security!\" FROM \
         pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = \
         c.relnamespace LEFT JOIN pg_catalog.pg_am am ON am.oid = c.relam LEFT \
         JOIN pg_catalog.pg_tablespace ts ON ts.oid = c.reltablespace JOIN \
         pg_catalog.pg_database d ON d.datname = current_database() JOIN \
         pg_catalog.pg_tablespace dbts ON dbts.oid = d.dattablespace WHERE \
         n.nspname = $1 AND c.relkind <> 'i' AND ($2 OR c.relname = \
         ANY($3::text[]))",
        spec.schema,
        spec.closed,
        &relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;

    let columns = sqlx::query_as!(
        ColumnFact,
        "SELECT c.relname::text AS \"relation!\", a.attnum AS \"ordinal!\", \
         a.attname::text AS \"name!\", t.typname::text AS \"type_name?\", \
         a.atttypmod AS \"type_modifier!\", a.attnotnull AS \"not_null!\", \
         coll.collname::text AS \"collation?\", pg_get_expr(d.adbin, d.adrelid, \
         false) AS \"default_expression?\", a.attgenerated::text AS \
         \"generated!\", a.attidentity::text AS \"identity!\", a.attisdropped AS \
         \"dropped!\" FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n \
         ON n.oid = c.relnamespace JOIN pg_catalog.pg_attribute a ON a.attrelid = \
         c.oid LEFT JOIN pg_catalog.pg_type t ON t.oid = a.atttypid LEFT JOIN \
         pg_catalog.pg_collation coll ON coll.oid = a.attcollation LEFT JOIN \
         pg_catalog.pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum \
         WHERE n.nspname = $1 AND c.relname = ANY($2::text[]) AND a.attnum > 0",
        spec.schema,
        &relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;

    let constraints = sqlx::query_as!(
        ConstraintFact,
        "SELECT c.relname::text AS \"relation!\", con.conname AS \"name!\", \
         con.contype::text AS \"kind!\", pg_get_constraintdef(con.oid, false) AS \
         \"definition!\", con.convalidated AS \"validated!\" FROM \
         pg_catalog.pg_constraint con JOIN pg_catalog.pg_class c ON c.oid = \
         con.conrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relname = ANY($2::text[])",
        spec.schema,
        &relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;

    let indexes = sqlx::query_as!(
        IndexFact,
        "SELECT table_class.relname::text AS \"relation!\", \
         index_class.relname::text AS \"name!\", \
         pg_get_userbyid(index_class.relowner) AS \"owner!\", \
         pg_get_indexdef(index_class.oid) AS \"definition!\", idx.indisunique AS \
         \"unique!\", idx.indnullsnotdistinct AS \"nulls_not_distinct!\", \
         pg_get_expr(idx.indpred, idx.indrelid, false) AS \"predicate?\", \
         pg_get_expr(idx.indexprs, idx.indrelid, false) AS \"expressions?\", \
         am.amname::text AS \"access_method!\", COALESCE((SELECT \
         string_agg(option, ',' ORDER BY option) FROM \
         unnest(index_class.reloptions) option), '') AS \"options!\", \
         COALESCE(ts.spcname, dbts.spcname) AS \"tablespace!\", idx.indisvalid AS \
         \"valid!\", idx.indisready AS \"ready!\", idx.indislive AS \"live!\" \
         FROM pg_catalog.pg_index idx JOIN pg_catalog.pg_class table_class ON \
         table_class.oid = idx.indrelid JOIN pg_catalog.pg_class index_class ON \
         index_class.oid = idx.indexrelid JOIN pg_catalog.pg_namespace n ON n.oid \
         = table_class.relnamespace JOIN pg_catalog.pg_am am ON am.oid = \
         index_class.relam LEFT JOIN pg_catalog.pg_tablespace ts ON ts.oid = \
         index_class.reltablespace JOIN pg_catalog.pg_database d ON d.datname = \
         current_database() JOIN pg_catalog.pg_tablespace dbts ON dbts.oid = \
         d.dattablespace WHERE n.nspname = $1 AND table_class.relname = \
         ANY($2::text[])",
        spec.schema,
        &relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;

    let schema_acl = load_schema_acl(connection, spec.schema).await?;
    let relation_acl = load_relation_acl(connection, spec.schema, &relation_names).await?;
    let column_acl = load_column_acl(connection, spec.schema, &relation_names).await?;
    let policies = sqlx::query_scalar!(
        "SELECT c.relname || '.' || p.polname AS \"value!\" FROM \
         pg_catalog.pg_policy p JOIN pg_catalog.pg_class c ON c.oid = p.polrelid \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname \
         = $1 AND c.relname = ANY($2::text[])",
        spec.schema,
        &relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    let rules = sqlx::query_scalar!(
        "SELECT c.relname || '.' || r.rulename AS \"value!\" FROM \
         pg_catalog.pg_rewrite r JOIN pg_catalog.pg_class c ON c.oid = r.ev_class \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname \
         = $1 AND c.relname = ANY($2::text[])",
        spec.schema,
        &relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    let triggers = sqlx::query_scalar!(
        "SELECT c.relname || '.' || t.tgname AS \"value!\" FROM \
         pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname \
         = $1 AND c.relname = ANY($2::text[]) AND NOT t.tgisinternal",
        spec.schema,
        &relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;

    Ok(CatalogSnapshot {
        database_owner,
        schema_owner,
        relations,
        columns,
        constraints,
        indexes,
        schema_acl,
        relation_acl,
        column_acl,
        policies,
        rules,
        triggers,
    })
}

async fn load_schema_acl(
    connection: &mut PgConnection,
    schema: &str,
) -> Result<Vec<AclGrant>, GateError> {
    sqlx::query_as!(
        AclGrant,
        "SELECT NULL::text AS \"relation?\", NULL::text AS \"column?\", \
         pg_get_userbyid(acl.grantor) AS \"grantor!\", CASE WHEN acl.grantee = 0 \
         THEN 'PUBLIC' ELSE pg_get_userbyid(acl.grantee) END AS \"grantee!\", \
         acl.privilege_type AS \"privilege!\", acl.is_grantable AS \
         \"grant_option!\" FROM pg_catalog.pg_namespace n CROSS JOIN LATERAL \
         aclexplode( COALESCE(n.nspacl, acldefault('n'::\"char\", n.nspowner))) \
         acl WHERE n.nspname = $1",
        schema,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)
}

async fn load_relation_acl(
    connection: &mut PgConnection,
    schema: &str,
    relation_names: &[String],
) -> Result<Vec<AclGrant>, GateError> {
    sqlx::query_as!(
        AclGrant,
        "SELECT c.relname::text AS \"relation?\", NULL::text AS \"column?\", \
         pg_get_userbyid(acl.grantor) AS \"grantor!\", CASE WHEN acl.grantee = 0 \
         THEN 'PUBLIC' ELSE pg_get_userbyid(acl.grantee) END AS \"grantee!\", \
         acl.privilege_type AS \"privilege!\", acl.is_grantable AS \
         \"grant_option!\" FROM pg_catalog.pg_class c JOIN \
         pg_catalog.pg_namespace n ON n.oid = c.relnamespace CROSS JOIN LATERAL \
         aclexplode( COALESCE(c.relacl, acldefault('r'::\"char\", c.relowner))) \
         acl WHERE n.nspname = $1 AND c.relname = ANY($2::text[]) AND c.relkind = \
         'r'",
        schema,
        relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)
}

async fn load_column_acl(
    connection: &mut PgConnection,
    schema: &str,
    relation_names: &[String],
) -> Result<Vec<AclGrant>, GateError> {
    sqlx::query_as!(
        AclGrant,
        "SELECT c.relname::text AS \"relation?\", a.attname::text AS \
         \"column?\", pg_get_userbyid(acl.grantor) AS \"grantor!\", CASE WHEN \
         acl.grantee = 0 THEN 'PUBLIC' ELSE pg_get_userbyid(acl.grantee) END AS \
         \"grantee!\", acl.privilege_type AS \"privilege!\", acl.is_grantable AS \
         \"grant_option!\" FROM pg_catalog.pg_attribute a JOIN \
         pg_catalog.pg_class c ON c.oid = a.attrelid JOIN pg_catalog.pg_namespace \
         n ON n.oid = c.relnamespace CROSS JOIN LATERAL aclexplode( \
         COALESCE(a.attacl, acldefault('c'::\"char\", c.relowner))) acl WHERE \
         n.nspname = $1 AND c.relname = ANY($2::text[]) AND a.attnum > 0",
        schema,
        relation_names,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)
}
