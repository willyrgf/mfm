use std::collections::BTreeSet;

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{StoreEpoch, StoreScopeId};
use sqlx::{AssertSqlSafe, PgConnection, PgPool, Row};

use crate::error::{PostgresStoreError, Result};
use crate::roles::{TargetKey, TargetRoleKind, TargetRoleNames};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub(crate) const SCHEMA_CONTRACT_VERSION: &str = "mfm.structured-run-history-postgres.v5";

// These SHA-256 values bind canonical, schema-name-independent catalog rows. They are
// regenerated only with the destructive baseline and deliberately fail closed across
// PostgreSQL catalog-rendering changes. Placeholders are filled after the first online
// catalog probe against the v4 baseline.
const RELATION_MANIFEST_SHA256: &str =
    "cf3b8c51d969611e9f6ca7ca9585ac58aab35358cd3bbd46418b29a05adcbb76";
const CONSTRAINT_MANIFEST_SHA256: &str =
    "023fca709cdf093c44b51f79b74a372172e85df7edf86381c530f883c50bba5c";
const INDEX_MANIFEST_SHA256: &str =
    "8ae2874ce7db6d9186a54776764748579716aeec882a739a6189281aacb4ee81";
const EXECUTABLE_MANIFEST_SHA256: &str =
    "665fd6cb23c59ee9116c63c9b80f42cd85fc32d6fd994a30a3c11a538f58b920";
const ACL_MANIFEST_SHA256: &str =
    "69446804600c06508b741bdce8f681b1182c8174088b7072352ab90e90be2de6";

/// Administrative schema management for the sole destructive structured-history baseline.
pub struct PostgresSchema;

impl PostgresSchema {
    /// Applies the compiled baseline through a migration-owner connection.
    pub async fn migrate(database_url: &str) -> Result<()> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|_| PostgresStoreError::Connection)?;
        migrate_pool(&pool).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedStoreIdentity {
    pub(crate) store_scope_id: StoreScopeId,
    pub(crate) store_epoch: StoreEpoch,
}

pub(crate) async fn migrate_pool(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|_| PostgresStoreError::Database("apply structured history baseline"))
}

#[cfg(all(test, feature = "parity-tests"))]
pub(crate) async fn validate_authoritative_schema(pool: &PgPool) -> Result<ValidatedStoreIdentity> {
    validate_authoritative_schema_inner(pool, None).await
}

pub(crate) async fn validate_authoritative_schema_at(
    pool: &PgPool,
    expected_schema: &str,
) -> Result<ValidatedStoreIdentity> {
    validate_authoritative_schema_inner(pool, Some(expected_schema)).await
}

async fn validate_authoritative_schema_inner(
    pool: &PgPool,
    expected_schema: Option<&str>,
) -> Result<ValidatedStoreIdentity> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let connection = &mut *transaction;
    if let Some(expected_schema) = expected_schema {
        pin_schema(connection, expected_schema).await?;
    }
    assume_qualification_role(connection, expected_schema).await?;
    if let Some(expected_schema) = expected_schema {
        let actual = sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
        if actual != expected_schema {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
    }
    validate_migration_ledger(connection).await?;
    validate_catalog_shape(connection).await?;
    let roles = validate_managed_roles(connection).await?;
    let identity = validate_identity(connection).await?;
    validate_target_authority(connection, &roles).await?;
    validate_prefix_integrity(connection, &identity).await?;
    transaction
        .commit()
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    Ok(identity)
}

async fn assume_qualification_role(
    connection: &mut PgConnection,
    expected_schema: Option<&str>,
) -> Result<()> {
    let schema = if let Some(schema) = expected_schema {
        schema.to_owned()
    } else {
        sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
    };
    if schema == "pg_catalog" || schema.is_empty() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    let qualification = TargetKey::from_schema(&schema).role_name(TargetRoleKind::Qualification);
    // Role names are derived from the schema's closed target key and quoted.
    let set_role = format!("SET LOCAL ROLE {}", quote_ident(&qualification));
    sqlx::query(AssertSqlSafe(set_role))
        .execute(&mut *connection)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    Ok(())
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

async fn pin_schema(connection: &mut PgConnection, expected_schema: &str) -> Result<()> {
    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(expected_schema)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    Ok(())
}

async fn validate_migration_ledger(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "SELECT version, description, success, checksum, \
                installed_on <= pg_catalog.clock_timestamp() AS installed, \
                execution_time >= 0 AS execution_recorded \
           FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if rows.len() != MIGRATOR.iter().count() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    for migration in MIGRATOR.iter() {
        let row = rows
            .iter()
            .find(|row| row.try_get::<i64, _>("version").ok() == Some(migration.version))
            .ok_or(PostgresStoreError::SchemaAuthorityMismatch)?;
        if !row
            .try_get::<bool, _>("success")
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
            || !row
                .try_get::<bool, _>("installed")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
            || !row
                .try_get::<bool, _>("execution_recorded")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
            || row
                .try_get::<String, _>("description")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
                != migration.description
        {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
        let checksum = row
            .try_get::<Vec<u8>, _>("checksum")
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
        if checksum.as_slice() != migration.checksum.as_ref() {
            return Err(PostgresStoreError::MigrationChecksumMismatch);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogManifestHashes {
    relation: String,
    constraint: String,
    index: String,
    executable: String,
    acl: String,
}

impl CatalogManifestHashes {
    fn is_authoritative(&self) -> bool {
        self.relation == RELATION_MANIFEST_SHA256
            && self.constraint == CONSTRAINT_MANIFEST_SHA256
            && self.index == INDEX_MANIFEST_SHA256
            && self.executable == EXECUTABLE_MANIFEST_SHA256
            && self.acl == ACL_MANIFEST_SHA256
    }
}

async fn validate_catalog_shape(connection: &mut PgConnection) -> Result<()> {
    if !catalog_manifest_hashes(connection)
        .await?
        .is_authoritative()
    {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn catalog_manifest_hashes(connection: &mut PgConnection) -> Result<CatalogManifestHashes> {
    Ok(CatalogManifestHashes {
        relation: manifest_hash(connection, RELATION_MANIFEST_SQL).await?,
        constraint: manifest_hash(connection, CONSTRAINT_MANIFEST_SQL).await?,
        index: manifest_hash(connection, INDEX_MANIFEST_SQL).await?,
        executable: manifest_hash(connection, EXECUTABLE_MANIFEST_SQL).await?,
        acl: manifest_hash(connection, ACL_MANIFEST_SQL).await?,
    })
}

async fn manifest_hash(connection: &mut PgConnection, query: &'static str) -> Result<String> {
    let rows = sqlx::query_scalar::<_, String>(query)
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let mut canonical = Vec::new();
    for row in rows {
        canonical.extend_from_slice(row.as_bytes());
        canonical.push(b'\n');
    }
    Ok(sha256_digest_bytes(&canonical).to_string())
}

const RELATION_MANIFEST_SQL: &str = r#"
WITH target_namespace AS (
    SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = pg_catalog.current_schema()
), manifest AS (
    SELECT
        0 AS row_kind,
        relation.relname AS object_name,
        0::smallint AS object_position,
        pg_catalog.jsonb_build_array(
            'relation',
            relation.relname,
            relation.relkind::text,
            relation.relpersistence::text,
            CASE
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_qlf$' THEN '<target-qualification>'
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rrd$' THEN '<target-run-reader>'
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rwr$' THEN '<target-run-writer>'
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_crd$' THEN '<target-configuration-reader>'
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_cwr$' THEN '<target-configuration-writer>'
                ELSE owner_role.rolname
            END,
            COALESCE(access_method.amname, ''),
            relation.relchecks,
            relation.relhasindex,
            relation.relhasrules,
            relation.relhastriggers,
            relation.relrowsecurity,
            relation.relforcerowsecurity,
            relation.relreplident::text,
            relation.relispartition
        ) AS manifest_row
    FROM pg_catalog.pg_class AS relation
    JOIN target_namespace ON target_namespace.oid = relation.relnamespace
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = relation.relowner
    LEFT JOIN pg_catalog.pg_am AS access_method ON access_method.oid = relation.relam

    UNION ALL

    SELECT
        1,
        relation.relname,
        attribute.attnum,
        pg_catalog.jsonb_build_array(
            'column',
            relation.relname,
            attribute.attnum,
            attribute.attname,
            pg_catalog.format_type(attribute.atttypid, attribute.atttypmod),
            attribute.attnotnull,
            attribute.attidentity::text,
            attribute.attgenerated::text,
            attribute.attstorage::text,
            attribute.attcompression::text,
            CASE
                WHEN attribute.attcollation = 0 THEN NULL
                ELSE collation_namespace.nspname || '.' || collation_row.collname
            END,
            pg_catalog.pg_get_expr(attribute_default.adbin, attribute_default.adrelid, FALSE)
        )
    FROM pg_catalog.pg_class AS relation
    JOIN target_namespace ON target_namespace.oid = relation.relnamespace
    JOIN pg_catalog.pg_attribute AS attribute
      ON attribute.attrelid = relation.oid
     AND attribute.attnum > 0
     AND NOT attribute.attisdropped
    LEFT JOIN pg_catalog.pg_attrdef AS attribute_default
      ON attribute_default.adrelid = relation.oid
     AND attribute_default.adnum = attribute.attnum
    LEFT JOIN pg_catalog.pg_collation AS collation_row
      ON collation_row.oid = attribute.attcollation
    LEFT JOIN pg_catalog.pg_namespace AS collation_namespace
      ON collation_namespace.oid = collation_row.collnamespace
    WHERE relation.relkind IN ('r', 'p')
)
SELECT manifest_row::text
FROM manifest
ORDER BY row_kind, object_name, object_position
"#;

const CONSTRAINT_MANIFEST_SQL: &str = r#"
WITH target_namespace AS (
    SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = pg_catalog.current_schema()
)
SELECT pg_catalog.jsonb_build_array(
           relation.relname,
           constraint_row.conname,
           constraint_row.contype::text,
           constraint_row.convalidated,
           constraint_row.condeferrable,
           constraint_row.condeferred,
           constraint_row.connoinherit,
           constraint_row.conislocal,
           constraint_row.coninhcount,
           referenced_relation.relname,
           (
               SELECT pg_catalog.count(*)::bigint
               FROM pg_catalog.pg_trigger AS internal_trigger
               WHERE internal_trigger.tgconstraint = constraint_row.oid
                 AND internal_trigger.tgisinternal
           ),
           pg_catalog.replace(
               pg_catalog.pg_get_constraintdef(constraint_row.oid, FALSE),
               pg_catalog.format('%I.', pg_catalog.current_schema()),
               '<schema>.'
           )
       )::text
FROM pg_catalog.pg_constraint AS constraint_row
JOIN target_namespace ON target_namespace.oid = constraint_row.connamespace
LEFT JOIN pg_catalog.pg_class AS relation ON relation.oid = constraint_row.conrelid
LEFT JOIN pg_catalog.pg_class AS referenced_relation
  ON referenced_relation.oid = constraint_row.confrelid
ORDER BY relation.relname, constraint_row.conname
"#;

const INDEX_MANIFEST_SQL: &str = r#"
WITH target_namespace AS (
    SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = pg_catalog.current_schema()
)
SELECT pg_catalog.jsonb_build_array(
           table_relation.relname,
           index_relation.relname,
           access_method.amname,
           index_row.indisunique,
           index_row.indnullsnotdistinct,
           index_row.indisprimary,
           index_row.indisexclusion,
           index_row.indimmediate,
           index_row.indisclustered,
           index_row.indisvalid,
           index_row.indcheckxmin,
           index_row.indisready,
           index_row.indislive,
           index_row.indisreplident,
           index_row.indnkeyatts,
           index_row.indnatts,
           pg_catalog.pg_get_expr(index_row.indexprs, index_row.indrelid, FALSE),
           pg_catalog.pg_get_expr(index_row.indpred, index_row.indrelid, FALSE),
           pg_catalog.replace(
               pg_catalog.pg_get_indexdef(index_relation.oid, 0, FALSE),
               pg_catalog.format('%I.', pg_catalog.current_schema()),
               '<schema>.'
           )
       )::text
FROM pg_catalog.pg_index AS index_row
JOIN pg_catalog.pg_class AS table_relation ON table_relation.oid = index_row.indrelid
JOIN target_namespace ON target_namespace.oid = table_relation.relnamespace
JOIN pg_catalog.pg_class AS index_relation ON index_relation.oid = index_row.indexrelid
JOIN pg_catalog.pg_am AS access_method ON access_method.oid = index_relation.relam
ORDER BY table_relation.relname, index_relation.relname
"#;

const EXECUTABLE_MANIFEST_SQL: &str = r#"
WITH target_namespace AS (
    SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = pg_catalog.current_schema()
), manifest AS (
    SELECT
        'function'::text AS object_kind,
        procedure_row.proname AS object_name,
        pg_catalog.jsonb_build_array(
            'function',
            procedure_row.proname,
            pg_catalog.pg_get_function_identity_arguments(procedure_row.oid),
            pg_catalog.pg_get_function_result(procedure_row.oid),
            language.lanname,
            CASE
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                ELSE owner_role.rolname
            END,
            procedure_row.prokind::text,
            procedure_row.provolatile::text,
            procedure_row.proparallel::text,
            procedure_row.prosecdef,
            procedure_row.proleakproof,
            procedure_row.proisstrict,
            procedure_row.proconfig,
            pg_catalog.replace(
                pg_catalog.pg_get_functiondef(procedure_row.oid),
                pg_catalog.format('%I.', pg_catalog.current_schema()),
                '<schema>.'
            )
        ) AS manifest_row
    FROM pg_catalog.pg_proc AS procedure_row
    JOIN target_namespace ON target_namespace.oid = procedure_row.pronamespace
    JOIN pg_catalog.pg_language AS language ON language.oid = procedure_row.prolang
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = procedure_row.proowner

    UNION ALL

    SELECT
        'trigger',
        relation.relname || ':' || COALESCE(constraint_row.conname, trigger_row.tgname),
        pg_catalog.jsonb_build_array(
            'trigger',
            relation.relname,
            CASE WHEN trigger_row.tgisinternal THEN '<generated>' ELSE trigger_row.tgname END,
            constraint_row.conname,
            trigger_row.tgisinternal,
            trigger_row.tgenabled::text,
            trigger_row.tgtype,
            trigger_row.tgdeferrable,
            trigger_row.tginitdeferred,
            trigger_row.tgparentid = 0,
            trigger_row.tgattr::text,
            pg_catalog.encode(trigger_row.tgargs, 'hex'),
            trigger_row.tgoldtable,
            trigger_row.tgnewtable,
            trigger_function_namespace.nspname,
            trigger_function.proname,
            trigger_relation.relname,
            pg_catalog.regexp_replace(
                pg_catalog.replace(
                    pg_catalog.pg_get_triggerdef(trigger_row.oid, FALSE),
                    pg_catalog.format('%I.', pg_catalog.current_schema()),
                    '<schema>.'
                ),
                '"RI_ConstraintTrigger_[ac]_[0-9]+"',
                '"RI_ConstraintTrigger_<generated>"',
                'g'
            )
        )
    FROM pg_catalog.pg_trigger AS trigger_row
    JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger_row.tgrelid
    JOIN target_namespace ON target_namespace.oid = relation.relnamespace
    JOIN pg_catalog.pg_proc AS trigger_function ON trigger_function.oid = trigger_row.tgfoid
    JOIN pg_catalog.pg_namespace AS trigger_function_namespace
      ON trigger_function_namespace.oid = trigger_function.pronamespace
    LEFT JOIN pg_catalog.pg_constraint AS constraint_row
      ON constraint_row.oid = trigger_row.tgconstraint
    LEFT JOIN pg_catalog.pg_class AS trigger_relation
      ON trigger_relation.oid = trigger_row.tgconstrrelid

    UNION ALL

    SELECT
        'policy',
        policy.polname,
        pg_catalog.jsonb_build_array(
            'policy',
            relation.relname,
            policy.polname,
            policy.polcmd::text,
            policy.polpermissive,
            (
                SELECT pg_catalog.array_agg(
                    CASE WHEN role_oid = 0 THEN 'PUBLIC' ELSE role_row.rolname END
                    ORDER BY CASE WHEN role_oid = 0 THEN 'PUBLIC' ELSE role_row.rolname END
                )
                FROM pg_catalog.unnest(policy.polroles) AS role_oid
                LEFT JOIN pg_catalog.pg_roles AS role_row ON role_row.oid = role_oid
            ),
            pg_catalog.pg_get_expr(policy.polqual, policy.polrelid, FALSE),
            pg_catalog.pg_get_expr(policy.polwithcheck, policy.polrelid, FALSE)
        )
    FROM pg_catalog.pg_policy AS policy
    JOIN pg_catalog.pg_class AS relation ON relation.oid = policy.polrelid
    JOIN target_namespace ON target_namespace.oid = relation.relnamespace

    UNION ALL

    SELECT
        'rule',
        rewrite_rule.rulename,
        pg_catalog.jsonb_build_array(
            'rule',
            relation.relname,
            rewrite_rule.rulename,
            rewrite_rule.ev_type::text,
            rewrite_rule.is_instead,
            pg_catalog.replace(
                pg_catalog.pg_get_ruledef(rewrite_rule.oid, FALSE),
                pg_catalog.format('%I.', pg_catalog.current_schema()),
                '<schema>.'
            )
        )
    FROM pg_catalog.pg_rewrite AS rewrite_rule
    JOIN pg_catalog.pg_class AS relation ON relation.oid = rewrite_rule.ev_class
    JOIN target_namespace ON target_namespace.oid = relation.relnamespace
)
SELECT manifest_row::text
FROM manifest
ORDER BY object_kind, object_name, manifest_row::text
"#;

const ACL_MANIFEST_SQL: &str = r#"
WITH target_namespace AS (
    SELECT oid, nspname, nspowner, nspacl
    FROM pg_catalog.pg_namespace
    WHERE nspname = pg_catalog.current_schema()
), manifest AS (
    SELECT
        'schema-owner'::text AS object_kind,
        target_namespace.nspname AS object_name,
        ''::text AS subobject_name,
        pg_catalog.jsonb_build_array(
            'schema-owner',
            '<schema>',
            CASE
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                ELSE owner_role.rolname
            END
        ) AS manifest_row
    FROM target_namespace
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = target_namespace.nspowner

    UNION ALL

    SELECT
        'schema-acl',
        target_namespace.nspname,
        '',
        pg_catalog.jsonb_build_array(
            'schema-acl',
            '<schema>',
            CASE
                WHEN acl.grantee = 0 THEN 'PUBLIC'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_qlf$' THEN '<target-qualification>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rrd$' THEN '<target-run-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rwr$' THEN '<target-run-writer>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_crd$' THEN '<target-configuration-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_cwr$' THEN '<target-configuration-writer>'
                ELSE grantee_role.rolname
            END,
            CASE
                WHEN grantor_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                ELSE grantor_role.rolname
            END,
            acl.privilege_type,
            acl.is_grantable
        )
    FROM target_namespace
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(
            target_namespace.nspacl,
            pg_catalog.acldefault('n', target_namespace.nspowner)
        )
    ) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee_role ON grantee_role.oid = acl.grantee
    JOIN pg_catalog.pg_roles AS grantor_role ON grantor_role.oid = acl.grantor

    UNION ALL

    SELECT
        'relation-acl',
        relation.relname,
        '',
        pg_catalog.jsonb_build_array(
            'relation-acl',
            relation.relname,
            CASE
                WHEN acl.grantee = 0 THEN 'PUBLIC'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_qlf$' THEN '<target-qualification>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rrd$' THEN '<target-run-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rwr$' THEN '<target-run-writer>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_crd$' THEN '<target-configuration-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_cwr$' THEN '<target-configuration-writer>'
                ELSE grantee_role.rolname
            END,
            CASE
                WHEN grantor_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                ELSE grantor_role.rolname
            END,
            acl.privilege_type,
            acl.is_grantable
        )
    FROM target_namespace
    JOIN pg_catalog.pg_class AS relation ON relation.relnamespace = target_namespace.oid
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(relation.relacl, pg_catalog.acldefault('r', relation.relowner))
    ) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee_role ON grantee_role.oid = acl.grantee
    JOIN pg_catalog.pg_roles AS grantor_role ON grantor_role.oid = acl.grantor
    WHERE relation.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')

    UNION ALL

    SELECT
        'column-acl',
        relation.relname,
        attribute.attname,
        pg_catalog.jsonb_build_array(
            'column-acl',
            relation.relname,
            attribute.attname,
            CASE
                WHEN acl.grantee = 0 THEN 'PUBLIC'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_qlf$' THEN '<target-qualification>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rrd$' THEN '<target-run-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rwr$' THEN '<target-run-writer>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_crd$' THEN '<target-configuration-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_cwr$' THEN '<target-configuration-writer>'
                ELSE grantee_role.rolname
            END,
            CASE
                WHEN grantor_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                ELSE grantor_role.rolname
            END,
            acl.privilege_type,
            acl.is_grantable
        )
    FROM target_namespace
    JOIN pg_catalog.pg_class AS relation ON relation.relnamespace = target_namespace.oid
    JOIN pg_catalog.pg_attribute AS attribute
      ON attribute.attrelid = relation.oid
     AND attribute.attnum > 0
     AND NOT attribute.attisdropped
    CROSS JOIN LATERAL pg_catalog.aclexplode(attribute.attacl) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee_role ON grantee_role.oid = acl.grantee
    JOIN pg_catalog.pg_roles AS grantor_role ON grantor_role.oid = acl.grantor
    WHERE attribute.attacl IS NOT NULL

    UNION ALL

    SELECT
        'default-acl',
        CASE
            WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
            ELSE owner_role.rolname
        END,
        default_acl.defaclobjtype::text,
        pg_catalog.jsonb_build_array(
            'default-acl',
            CASE
                WHEN owner_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                ELSE owner_role.rolname
            END,
            default_acl.defaclobjtype::text,
            CASE
                WHEN acl.grantee = 0 THEN 'PUBLIC'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_qlf$' THEN '<target-qualification>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rrd$' THEN '<target-run-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_rwr$' THEN '<target-run-writer>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_crd$' THEN '<target-configuration-reader>'
                WHEN grantee_role.rolname ~ '^mfm_t_[0-9a-f]{16}_cwr$' THEN '<target-configuration-writer>'
                ELSE grantee_role.rolname
            END,
            CASE
                WHEN grantor_role.rolname ~ '^mfm_t_[0-9a-f]{16}_own$' THEN '<target-owner>'
                ELSE grantor_role.rolname
            END,
            acl.privilege_type,
            acl.is_grantable
        )
    FROM target_namespace
    JOIN pg_catalog.pg_default_acl AS default_acl
      ON default_acl.defaclnamespace = target_namespace.oid
    JOIN pg_catalog.pg_roles AS owner_role ON owner_role.oid = default_acl.defaclrole
    CROSS JOIN LATERAL pg_catalog.aclexplode(default_acl.defaclacl) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee_role ON grantee_role.oid = acl.grantee
    JOIN pg_catalog.pg_roles AS grantor_role ON grantor_role.oid = acl.grantor
)
SELECT manifest_row::text
FROM manifest
ORDER BY object_kind, object_name, subobject_name, manifest_row::text
"#;

async fn validate_managed_roles(connection: &mut PgConnection) -> Result<TargetRoleNames> {
    let schema = sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let roles = TargetRoleNames::from_target_key(TargetKey::from_schema(&schema));
    let expected_names = roles.managed_names();
    let expected = expected_names
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let rows = sqlx::query(
        "SELECT rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolcanlogin, \
                rolreplication, rolbypassrls, rolconnlimit, rolvaliduntil IS NULL AS no_expiry, \
                rolconfig IS NULL AS no_config \
           FROM pg_catalog.pg_roles WHERE rolname = ANY($1) ORDER BY rolname",
    )
    .bind(&expected_names[..])
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if rows.len() != expected.len() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    for role in &rows {
        if role.try_get::<bool, _>("rolsuper").unwrap_or(true)
            || !role.try_get::<bool, _>("rolinherit").unwrap_or(false)
            || role.try_get::<bool, _>("rolcreaterole").unwrap_or(true)
            || role.try_get::<bool, _>("rolcreatedb").unwrap_or(true)
            || role.try_get::<bool, _>("rolcanlogin").unwrap_or(true)
            || role.try_get::<bool, _>("rolreplication").unwrap_or(true)
            || role.try_get::<bool, _>("rolbypassrls").unwrap_or(true)
            || role.try_get::<i32, _>("rolconnlimit").ok() != Some(-1)
            || !role.try_get::<bool, _>("no_expiry").unwrap_or(false)
            || !role.try_get::<bool, _>("no_config").unwrap_or(false)
        {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
    }
    let actual = rows
        .iter()
        .map(|role| string(role, "rolname"))
        .collect::<Result<BTreeSet<_>>>()?;
    if actual != expected {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    let inherited_roles = sqlx::query_scalar::<_, String>(
        "SELECT parent_role.rolname \
           FROM pg_catalog.pg_auth_members AS membership \
           JOIN pg_catalog.pg_roles AS member_role ON member_role.oid = membership.member \
           JOIN pg_catalog.pg_roles AS parent_role ON parent_role.oid = membership.roleid \
          WHERE member_role.rolname = ANY($1) \
          ORDER BY member_role.rolname, parent_role.rolname",
    )
    .bind(&expected_names[..])
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if !inherited_roles.is_empty() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(roles)
}

async fn validate_target_authority(
    connection: &mut PgConnection,
    roles: &TargetRoleNames,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT target_key, fence_generation::text AS fence_generation, \
                release_epoch::text AS release_epoch, \
                owner_role, qualification_role, run_reader_role, run_writer_role, \
                configuration_reader_role, configuration_writer_role, \
                (SELECT count(*)::bigint FROM target_authority) AS authority_count \
           FROM target_authority WHERE singleton",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
    .ok_or(PostgresStoreError::SchemaAuthorityMismatch)?;
    if row.try_get::<i64, _>("authority_count").ok() != Some(1)
        || row.try_get::<String, _>("target_key").ok().as_deref() != Some(roles.target_key.as_str())
        || row.try_get::<String, _>("owner_role").ok().as_deref() != Some(roles.owner.as_str())
        || row
            .try_get::<String, _>("qualification_role")
            .ok()
            .as_deref()
            != Some(roles.qualification.as_str())
        || row.try_get::<String, _>("run_reader_role").ok().as_deref()
            != Some(roles.run_reader.as_str())
        || row.try_get::<String, _>("run_writer_role").ok().as_deref()
            != Some(roles.run_writer.as_str())
        || row
            .try_get::<String, _>("configuration_reader_role")
            .ok()
            .as_deref()
            != Some(roles.configuration_reader.as_str())
        || row
            .try_get::<String, _>("configuration_writer_role")
            .ok()
            .as_deref()
            != Some(roles.configuration_writer.as_str())
    {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    let fence_generation = row
        .try_get::<String, _>("fence_generation")
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let release_epoch = row
        .try_get::<String, _>("release_epoch")
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if fence_generation.parse::<u64>().ok().filter(|v| *v >= 1).is_none()
        || release_epoch.parse::<u64>().ok().filter(|v| *v >= 1).is_none()
        || fence_generation.parse::<u64>().ok().map(|v| v.to_string())
            != Some(fence_generation.clone())
        || release_epoch.parse::<u64>().ok().map(|v| v.to_string()) != Some(release_epoch)
    {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn validate_identity(connection: &mut PgConnection) -> Result<ValidatedStoreIdentity> {
    let row = sqlx::query(
        "SELECT identity.store_scope_id, identity.store_epoch::text AS store_epoch, \
                metadata.schema_contract_version, \
                (SELECT count(*)::bigint FROM store_identity) AS identity_count, \
                (SELECT count(*)::bigint FROM store_schema_metadata) AS metadata_count \
           FROM store_identity AS identity CROSS JOIN store_schema_metadata AS metadata \
          WHERE identity.singleton AND metadata.singleton",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
    .ok_or(PostgresStoreError::SchemaAuthorityMismatch)?;
    if row.try_get::<i64, _>("identity_count").ok() != Some(1)
        || row.try_get::<i64, _>("metadata_count").ok() != Some(1)
        || string(&row, "schema_contract_version")? != SCHEMA_CONTRACT_VERSION
    {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(ValidatedStoreIdentity {
        store_scope_id: StoreScopeId::new(string(&row, "store_scope_id")?)
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?,
        store_epoch: StoreEpoch::parse(string(&row, "store_epoch")?)
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?,
    })
}

async fn validate_prefix_integrity(
    connection: &mut PgConnection,
    identity: &ValidatedStoreIdentity,
) -> Result<()> {
    let invalid = sqlx::query_scalar::<_, i64>(
        "WITH ordered AS ( \
             SELECT batch.*, \
                    lag(run_sequence) OVER (PARTITION BY run_id ORDER BY run_sequence) AS prior_sequence, \
                    lag(head_commit_digest) OVER (PARTITION BY run_id ORDER BY run_sequence) AS prior_digest \
               FROM run_history_batches AS batch \
         ), invalid_batches AS ( \
             SELECT 1 FROM ordered \
              WHERE (run_sequence = 1 AND (predecessor_sequence IS NOT NULL OR predecessor_commit_digest IS NOT NULL)) \
                 OR (run_sequence > 1 AND (predecessor_sequence <> prior_sequence OR predecessor_commit_digest <> prior_digest)) \
                 OR prior_sequence IS DISTINCT FROM CASE WHEN run_sequence = 1 THEN NULL ELSE run_sequence - 1 END \
         ), invalid_heads AS ( \
             SELECT 1 FROM run_history_heads AS head \
              LEFT JOIN LATERAL ( \
                    SELECT run_sequence, head_commit_digest FROM run_history_batches \
                     WHERE run_id = head.run_id ORDER BY run_sequence DESC LIMIT 1 \
              ) AS last_batch ON TRUE \
              WHERE head.store_scope_id <> $1 OR head.store_epoch::text <> $2 \
                 OR last_batch.run_sequence IS NULL \
                 OR head.head_sequence <> last_batch.run_sequence \
                 OR head.head_commit_digest <> last_batch.head_commit_digest \
         ), orphan_batches AS ( \
             SELECT 1 FROM run_history_batches AS batch \
              LEFT JOIN run_history_heads AS head USING (run_id) WHERE head.run_id IS NULL \
         ), object_counts AS ( \
             SELECT batch.run_id, batch.run_sequence, \
                    batch.batch_envelope_json::jsonb ->> 'object_count' AS declared_count, \
                    count(object.object_ordinal) AS actual_count, \
                    min(object.object_ordinal) AS first_ordinal, \
                    max(object.object_ordinal) AS last_ordinal \
               FROM run_history_batches AS batch \
               LEFT JOIN run_history_batch_objects AS object \
                 USING (run_id, run_sequence) \
              GROUP BY batch.run_id, batch.run_sequence, batch.batch_envelope_json \
         ), invalid_objects AS ( \
             SELECT 1 FROM object_counts \
              WHERE declared_count IS NULL \
                 OR declared_count !~ '^(0|[1-9][0-9]{0,5})$' \
                 OR CASE \
                        WHEN declared_count ~ '^(0|[1-9][0-9]{0,5})$' \
                        THEN declared_count::bigint > 65536 \
                          OR declared_count::bigint <> actual_count \
                        ELSE TRUE \
                    END \
                 OR (actual_count > 0 AND (first_ordinal <> 0 OR last_ordinal::bigint <> actual_count - 1)) \
         ), ordered_fact_publications AS ( \
             SELECT publication.*, \
                    row_number() OVER ( \
                        PARTITION BY store_scope_id, store_epoch, tenant_scope_id \
                        ORDER BY fact_order \
                    ) AS dense_order \
               FROM tenant_fact_publications AS publication \
         ), invalid_fact_publications AS ( \
             SELECT 1 FROM ordered_fact_publications AS publication \
               JOIN run_history_batches AS batch \
                 ON batch.run_id = publication.run_id \
                AND batch.run_sequence = publication.run_sequence \
              WHERE publication.store_scope_id <> $1 \
                 OR publication.store_epoch::text <> $2 \
                 OR publication.fact_order <> publication.dense_order \
                 OR batch.batch_envelope_json::jsonb #>> '{tenant_fact_coordinate,kind}' \
                        <> 'fact_publication' \
                 OR batch.batch_envelope_json::jsonb \
                        #>> '{tenant_fact_coordinate,frontier,store_scope_id}' <> $1 \
                 OR batch.batch_envelope_json::jsonb \
                        #>> '{tenant_fact_coordinate,frontier,store_epoch}' <> $2 \
                 OR batch.batch_envelope_json::jsonb \
                        #>> '{tenant_fact_coordinate,frontier,tenant_scope_id}' \
                        <> publication.tenant_scope_id \
                 OR batch.batch_envelope_json::jsonb \
                        #>> '{tenant_fact_coordinate,frontier,fact_order}' \
                        <> publication.fact_order::text \
                 OR batch.batch_envelope_json::jsonb #>> '{records,0,record_ref,run_id}' \
                        <> publication.run_id \
                 OR batch.batch_envelope_json::jsonb \
                        #>> '{records,0,record_ref,run_sequence}' \
                        <> publication.run_sequence::text \
                 OR batch.batch_envelope_json::jsonb #>> '{records,0,record_ref,ordinal}' \
                        <> publication.transition_ordinal::text \
                 OR batch.batch_envelope_json::jsonb \
                        #>> '{records,0,record_ref,record_hash}' \
                        <> publication.transition_record_hash \
         ), invalid_fact_heads AS ( \
             SELECT 1 FROM tenant_fact_heads AS head \
              WHERE head.store_scope_id <> $1 OR head.store_epoch::text <> $2 \
                 OR head.publication_count <> head.fact_order \
                 OR (head.fact_order = 0 AND (head.minimum_order IS NOT NULL OR head.maximum_order IS NOT NULL)) \
                 OR (head.fact_order > 0 AND (head.minimum_order <> 1 OR head.maximum_order <> head.fact_order)) \
                 OR ( \
                        SELECT count(*)::numeric FROM tenant_fact_publications AS publication \
                         WHERE publication.store_scope_id = head.store_scope_id \
                           AND publication.store_epoch = head.store_epoch \
                           AND publication.tenant_scope_id = head.tenant_scope_id \
                    ) <> head.publication_count \
                 OR ( \
                        SELECT min(publication.fact_order) FROM tenant_fact_publications AS publication \
                         WHERE publication.store_scope_id = head.store_scope_id \
                           AND publication.store_epoch = head.store_epoch \
                           AND publication.tenant_scope_id = head.tenant_scope_id \
                    ) IS DISTINCT FROM head.minimum_order \
                 OR ( \
                        SELECT max(publication.fact_order) FROM tenant_fact_publications AS publication \
                         WHERE publication.store_scope_id = head.store_scope_id \
                           AND publication.store_epoch = head.store_epoch \
                           AND publication.tenant_scope_id = head.tenant_scope_id \
                    ) IS DISTINCT FROM head.maximum_order \
             UNION ALL \
             SELECT 1 FROM tenant_fact_publications AS publication \
              LEFT JOIN tenant_fact_heads AS head \
                ON head.store_scope_id = publication.store_scope_id \
               AND head.store_epoch = publication.store_epoch \
               AND head.tenant_scope_id = publication.tenant_scope_id \
              WHERE head.tenant_scope_id IS NULL \
         ), ordered_configuration AS ( \
             SELECT revision.*, \
                    lag(revision_sequence) OVER configuration_stream AS prior_sequence, \
                    lag(revision_schema_id) OVER configuration_stream AS prior_schema_id, \
                    lag(revision_digest) OVER configuration_stream AS prior_digest, \
                    count(*) OVER configuration_stream_all AS stream_count, \
                    count(*) OVER configuration_contract AS contract_count \
               FROM configuration_revisions AS revision \
             WINDOW configuration_stream AS ( \
                        PARTITION BY store_scope_id, tenant_scope_id, \
                                     entry_point_operation_id, target_id \
                        ORDER BY revision_sequence \
                    ), \
                    configuration_stream_all AS ( \
                        PARTITION BY store_scope_id, tenant_scope_id, \
                                     entry_point_operation_id, target_id \
                    ), \
                    configuration_contract AS ( \
                        PARTITION BY store_scope_id, tenant_scope_id, \
                                     entry_point_operation_id, target_id, \
                                     value_contract_schema_id, value_contract_digest \
                    ) \
         ), configuration_revision_heads AS ( \
             SELECT DISTINCT ON ( \
                        store_scope_id, tenant_scope_id, entry_point_operation_id, target_id \
                    ) \
                    store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
                    revision_sequence, revision_schema_id, revision_digest, \
                    count(*) OVER configuration_stream_all AS stream_count, \
                    min(revision_sequence) OVER configuration_stream_all AS minimum_sequence \
               FROM configuration_revisions \
             WINDOW configuration_stream_all AS ( \
                        PARTITION BY store_scope_id, tenant_scope_id, \
                                     entry_point_operation_id, target_id \
                    ) \
              ORDER BY store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
                       revision_sequence DESC \
         ), invalid_configuration_heads AS ( \
             SELECT 1 FROM configuration_heads AS head \
              FULL OUTER JOIN configuration_revision_heads AS revisions \
                ON revisions.store_scope_id = head.store_scope_id \
               AND revisions.tenant_scope_id = head.tenant_scope_id \
               AND revisions.entry_point_operation_id = head.entry_point_operation_id \
               AND revisions.target_id = head.target_id \
              WHERE head.store_scope_id IS NULL OR revisions.store_scope_id IS NULL \
                 OR head.store_scope_id <> $1 \
                 OR revisions.minimum_sequence <> 1 \
                 OR revisions.stream_count <> revisions.revision_sequence \
                 OR head.revision_sequence <> revisions.revision_sequence \
                 OR head.revision_schema_id <> revisions.revision_schema_id \
                 OR head.revision_digest <> revisions.revision_digest \
         ), invalid_configuration AS ( \
             SELECT 1 FROM ordered_configuration \
              WHERE store_scope_id <> $1 \
                 OR prior_sequence IS DISTINCT FROM \
                    CASE WHEN revision_sequence = 1 THEN NULL ELSE revision_sequence - 1 END \
                 OR (revision_sequence = 1 \
                     AND (predecessor_schema_id IS NOT NULL OR predecessor_digest IS NOT NULL)) \
                 OR (revision_sequence > 1 \
                     AND (predecessor_schema_id <> prior_schema_id \
                          OR predecessor_digest <> prior_digest)) \
                 OR stream_count <> contract_count \
         ) \
         SELECT (SELECT count(*) FROM invalid_batches) \
              + (SELECT count(*) FROM invalid_heads) \
              + (SELECT count(*) FROM orphan_batches) \
              + (SELECT count(*) FROM invalid_objects) \
              + (SELECT count(*) FROM invalid_fact_publications) \
              + (SELECT count(*) FROM invalid_fact_heads) \
              + (SELECT count(*) FROM invalid_configuration_heads) \
              + (SELECT count(*) FROM invalid_configuration)",
    )
    .bind(identity.store_scope_id.as_str())
    .bind(identity.store_epoch.get().to_string())
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if invalid != 0 {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

fn string(row: &sqlx::postgres::PgRow, column: &str) -> Result<String> {
    row.try_get(column)
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)
}
