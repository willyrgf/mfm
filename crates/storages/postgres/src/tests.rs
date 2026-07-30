use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mfm_canonical::{
    sha256_digest_bytes, CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContractV2,
};
use mfm_ids::{
    AppendRequestId, ContentRef, DigestAlgorithm, EntryPointId, FieldPath, GenesisDigest,
    InvocationIdentity, RunId, RunSemanticStateDigest, SemanticTypeId, SpecHash, StableId,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::v1::{
    ArtifactIdPreimage, BatchPurpose, ConfiguredValueBinding, ConfiguredValueKey,
    JournalPredecessor, ObjectEvidencePreimage, ProducerBinding, RecordLogicalKey, RunAdmitted,
    RunAdmittedFields, RunJournalRecord, RunJournalRecordFields, TenantFactCoordinateFields,
    ValueRef,
};
use mfm_qualified_run_test_support::{PreparedQualifiedRun, QualifiedRunFixture};
use mfm_runtime::{
    AdmissionDisposition, AuthorizedAdmissionPlan, DriveOutcome, DriveWaitReason, Runtime,
    RuntimeError,
};
use mfm_spec::v1::RetainedValueContract;
use mfm_store::v1::test_support::{
    FactScanConformanceFixture, LegalAdmissionFixture, PreparedLegalAdmission,
};
use mfm_store::v1::{
    AdmissionMaterial, AppendOutcome, ExistingRunAppendMaterial, FactSelectionAuthorizationOutcome,
    NewlyAppended, ObjectGraphProposal, ProducedObjectRoot, ProducedOutputSlot,
    ProposedAdmissionInput, QualifiedRunStore, QualifiedSupportGraph, QualifiedSupportMember,
    RunAccessAuthorityIssuer, RunHistoryWriter, SettlementMaterial, StoreError, TransitionMaterial,
    TransitionTracePageRequest, VerifiedRunView,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row, Transaction};

use crate::qualification::{
    AuthoritativeWriterContext, AuthoritativeWriterFence, AuthoritativeWriterFenceFuture,
    TestAuthoritativeWriterFence,
};
use crate::schema::{
    migrate_pool, validate_authoritative_schema, validate_authoritative_schema_at,
};
use crate::store::TestCommitFailurePoint;
use crate::{open_authoritative, PostgresRunJournalBackend, PostgresStoreError};

static DATABASE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

type PostgresWriter = RunHistoryWriter<PostgresRunJournalBackend>;

fn runtime_admission_plan(
    fixture: &QualifiedRunFixture,
    prepared: PreparedQualifiedRun,
    replacement_input: Option<PlainCanonicalJsonBytes>,
) -> AuthorizedAdmissionPlan {
    let (_registry, authority, append_request_id, artifacts, input, configured, sources) =
        prepared.into_parts();
    let input = match replacement_input {
        Some(canonical) => {
            let contract = input.value_contract().clone();
            ProposedAdmissionInput::new(canonical, contract)
        }
        None => input,
    };
    AuthorizedAdmissionPlan::new(
        authority,
        append_request_id,
        fixture.entry_point_id().clone(),
        fixture.entry_point_operation_id().clone(),
        fixture.invocation_identity().clone(),
        artifacts,
        input,
        configured,
        sources,
    )
}

async fn admit_recorded_run(
    store: QualifiedRunStore<PostgresRunJournalBackend>,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &QualifiedRunFixture,
) -> RunId {
    let registry = fixture
        .qualify_on(&store, issuer)
        .await
        .expect("qualify recorded Runtime registry");
    let (writer, reader) = store.split();
    let prepared = fixture
        .prepare_on(&reader, issuer, Arc::clone(&registry))
        .await
        .expect("prepare recorded Runtime admission");
    let runtime = Runtime::new(writer, registry);
    let admitted = runtime
        .admit(runtime_admission_plan(fixture, prepared, None))
        .await
        .expect("admit recorded Runtime history");
    assert_eq!(admitted.disposition(), AdmissionDisposition::NewlyAdmitted);
    admitted.run_id().clone()
}

async fn drive_with_reopened_candidate(
    database: &TestDatabase,
    fixture: &QualifiedRunFixture,
    tenant_scope_id: &TenantScopeId,
    run_id: RunId,
) -> DriveOutcome {
    let pool = database.independent_store_pool().await;
    let (store, issuer) = open_authoritative(pool, TestAuthoritativeWriterFence)
        .await
        .expect("reopen authoritative Runtime backend");
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify reopened candidate registry");
    let (writer, _reader) = store.split();
    Runtime::new(writer, registry)
        .drive_once(issuer.authorize_drive(tenant_scope_id.clone(), run_id))
        .await
        .expect("classify reopened Runtime candidate")
}

#[tokio::test]
async fn authoritative_baseline_opens_with_stable_identity() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("baseline").await;

    let (first, first_issuer) =
        open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
            .await
            .expect("the exact migrated baseline must qualify");
    let store_scope_id = first.store_identity().store_scope_id().clone();
    let store_epoch = first.store_identity().store_epoch();
    let (first_writer, first_reader) = first.split();
    let cloned_reader = first_reader.clone();
    first_reader
        .check_ready()
        .await
        .expect("the qualified writer must remain ready");
    cloned_reader
        .check_ready()
        .await
        .expect("a cloned production reader must share the qualified backend");
    assert_eq!(
        first_reader.store_identity(),
        cloned_reader.store_identity()
    );
    drop(first_writer);
    drop(first_reader);
    drop(cloned_reader);
    drop(first_issuer);

    let (second, _second_issuer) =
        open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
            .await
            .expect("reopening the same retained writer must qualify");
    let (second_writer, second_reader) = second.split();
    second_reader
        .check_ready()
        .await
        .expect("the reopened writer must remain ready");
    assert_eq!(
        second_reader.store_identity().store_scope_id(),
        &store_scope_id
    );
    assert_eq!(second_reader.store_identity().store_epoch(), store_epoch);
    drop(second_writer);
    drop(second_reader);

    database.cleanup().await;
}

#[tokio::test]
async fn application_role_only_login_opens_and_is_ready() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let mut database = TestDatabase::create("application_role_only").await;
    let role = unique_identifier("mfm_application", "writer");
    sqlx::query(AssertSqlSafe(format!("CREATE ROLE {role} LOGIN")))
        .execute(&database.admin_pool)
        .await
        .expect("create exact application login");
    database.roles.push(role.clone());
    sqlx::query(AssertSqlSafe(format!(
        "GRANT mfm_store_application TO {role}"
    )))
    .execute(&database.admin_pool)
    .await
    .expect("grant only the application role");
    let migration_acl = sqlx::query(
        "SELECT \
             pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'SELECT' \
             ) AS can_select, \
             pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'INSERT' \
             ) OR pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'UPDATE' \
             ) OR pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'DELETE' \
             ) OR pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'TRUNCATE' \
             ) OR pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'REFERENCES' \
             ) OR pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'TRIGGER' \
             ) OR pg_catalog.has_table_privilege( \
                 'mfm_store_application', \
                 pg_catalog.format('%I._sqlx_migrations', current_schema()), \
                 'MAINTAIN' \
             ) AS has_broader_privilege",
    )
    .fetch_one(&database.pool)
    .await
    .expect("inspect the operational migration-ledger ACL");
    assert!(migration_acl
        .try_get::<bool, _>("can_select")
        .expect("migration-ledger select privilege"));
    assert!(!migration_acl
        .try_get::<bool, _>("has_broader_privilege")
        .expect("migration-ledger broader privilege"));
    let options = database
        .connect_options()
        .username(&role)
        .options([("search_path", database.schema.as_str())]);
    let role_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect exact application login");
    let (store, issuer) = open_authoritative(role_pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("application-role-only login must qualify");
    let (writer, reader) = store.split();
    reader
        .check_ready()
        .await
        .expect("application-role-only login must remain ready");

    drop(writer);
    drop(reader);
    drop(issuer);
    role_pool.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn qualified_operations_pin_the_validated_schema_on_every_transaction() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("schema_pin").await;
    let options = database
        .connect_options()
        .options([("search_path", database.schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect single-connection qualified pool");
    let raw_clone = pool.clone();
    let (store, _issuer) = open_authoritative(pool, TestAuthoritativeWriterFence)
        .await
        .expect("qualify exact schema");
    let (writer, reader) = store.split();
    let count_statement = format!(
        "SELECT \
             (SELECT count(*)::bigint FROM {schema}.store_identity) \
                 AS identity_rows, \
             (SELECT count(*)::bigint FROM {schema}.store_schema_metadata) \
                 AS metadata_rows, \
             (SELECT count(*)::bigint FROM {schema}.journal_commits) \
                 AS journal_rows",
        schema = database.schema
    );
    let before = sqlx::query(AssertSqlSafe(count_statement.clone()))
        .fetch_one(&raw_clone)
        .await
        .expect("snapshot readiness-owned rows");
    let before_counts = (
        before
            .try_get::<i64, _>("identity_rows")
            .expect("identity count"),
        before
            .try_get::<i64, _>("metadata_rows")
            .expect("metadata count"),
        before
            .try_get::<i64, _>("journal_rows")
            .expect("journal count"),
    );

    sqlx::query("SET search_path TO pg_catalog")
        .execute(&raw_clone)
        .await
        .expect("simulate caller session search-path drift");
    reader
        .check_ready()
        .await
        .expect("readiness must pin the qualified schema");
    let restored = sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
        .fetch_one(&raw_clone)
        .await
        .expect("read caller search path after readiness rollback");
    assert_eq!(restored, "pg_catalog");
    let after = sqlx::query(AssertSqlSafe(count_statement))
        .fetch_one(&raw_clone)
        .await
        .expect("snapshot rows after readiness");
    let after_counts = (
        after
            .try_get::<i64, _>("identity_rows")
            .expect("identity count"),
        after
            .try_get::<i64, _>("metadata_rows")
            .expect("metadata count"),
        after
            .try_get::<i64, _>("journal_rows")
            .expect("journal count"),
    );
    assert_eq!(after_counts, before_counts);

    drop(writer);
    drop(reader);
    raw_clone.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn authoritative_schema_validation_pins_the_probed_schema_over_session_drift() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("validation_schema_pin").await;
    let options = database
        .connect_options()
        .options([("search_path", database.schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect schema-validation pool");
    sqlx::query("SET search_path TO pg_catalog")
        .execute(&pool)
        .await
        .expect("drift the ambient validation schema");

    validate_authoritative_schema_at(&pool, &database.schema)
        .await
        .expect("validate only the explicitly probed schema");
    let ambient_schema = sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
        .fetch_one(&pool)
        .await
        .expect("read ambient schema after local validation pin");
    assert_eq!(ambient_schema, "pg_catalog");

    pool.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn qualification_requires_the_external_writer_fence() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("fence").await;

    let result = open_authoritative(database.pool.clone(), RejectFence).await;
    assert!(matches!(
        result,
        Err(PostgresStoreError::WriterFenceRejected)
    ));

    database.cleanup().await;
}

#[tokio::test]
async fn readiness_rejects_a_changed_retained_store_lineage() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("readiness_lineage").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify readiness lineage store");
    let (writer, reader) = store.split();
    let changed_scope = format!(
        "mfm.store_scope.v1:{}",
        &digest_hex(b"readiness-changed-store-lineage")[..32]
    );
    sqlx::query("ALTER TABLE store_identity DISABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("disable identity guard for readiness simulation");
    sqlx::query("UPDATE store_identity SET store_scope_id = $1")
        .bind(changed_scope)
        .execute(&database.pool)
        .await
        .expect("change retained readiness lineage");
    sqlx::query("ALTER TABLE store_identity ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore identity guard");

    assert!(matches!(
        reader.check_ready().await,
        Err(PostgresStoreError::WriterFenceRejected)
    ));

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn readiness_rejects_missing_schema_metadata_singleton() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("readiness_metadata").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify readiness metadata store");
    let (writer, reader) = store.split();
    sqlx::query("ALTER TABLE store_schema_metadata DISABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("disable metadata guard for readiness simulation");
    sqlx::query("DELETE FROM store_schema_metadata")
        .execute(&database.pool)
        .await
        .expect("remove retained schema metadata");
    sqlx::query("ALTER TABLE store_schema_metadata ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore metadata guard");

    assert!(matches!(
        reader.check_ready().await,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn readiness_rejects_a_changed_schema_contract_version() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("readiness_version").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify readiness version store");
    let (writer, reader) = store.split();
    sqlx::query(
        "ALTER TABLE store_schema_metadata \
         DISABLE TRIGGER ALL, \
         DROP CONSTRAINT store_schema_metadata_version_v1",
    )
    .execute(&database.pool)
    .await
    .expect("remove metadata guards for readiness simulation");
    sqlx::query(
        "UPDATE store_schema_metadata \
         SET schema_contract_version = 'mfm.recoverability-postgres.invalid'",
    )
    .execute(&database.pool)
    .await
    .expect("change retained schema contract version");
    sqlx::query("ALTER TABLE store_schema_metadata ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore metadata mutation guard");

    assert!(matches!(
        reader.check_ready().await,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn readiness_rechecks_application_role_assumption() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let mut database = TestDatabase::create("readiness_role").await;
    let role = unique_identifier("mfm_readiness", "membership");
    sqlx::query(AssertSqlSafe(format!("CREATE ROLE {role} LOGIN")))
        .execute(&database.admin_pool)
        .await
        .expect("create readiness membership role");
    database.roles.push(role.clone());
    sqlx::query(AssertSqlSafe(format!(
        "GRANT mfm_store_application TO {role}"
    )))
    .execute(&database.admin_pool)
    .await
    .expect("temporarily qualify application membership");
    let options = database
        .connect_options()
        .username(&role)
        .options([("search_path", database.schema.as_str())]);
    let role_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect readiness role");
    let (store, issuer) = open_authoritative(role_pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify readiness role before revocation");
    let (writer, reader) = store.split();

    sqlx::query(AssertSqlSafe(format!(
        "REVOKE SET OPTION FOR mfm_store_application FROM {role}"
    )))
    .execute(&database.admin_pool)
    .await
    .expect("revoke application role assumption after qualification");
    let role_proof = sqlx::query(
        "SELECT \
             pg_catalog.pg_has_role($1, 'mfm_store_application', 'MEMBER') AS is_member, \
             pg_catalog.pg_has_role($1, 'mfm_store_application', 'SET') AS can_set, \
             pg_catalog.has_table_privilege($1, 'journal_commits', 'SELECT') AS can_select, \
             pg_catalog.has_table_privilege($1, 'journal_commits', 'INSERT') AS can_insert",
    )
    .bind(&role)
    .fetch_one(&database.pool)
    .await
    .expect("inspect revoked application role assumption");
    assert!(role_proof
        .try_get::<bool, _>("is_member")
        .expect("retained application membership"));
    assert!(!role_proof
        .try_get::<bool, _>("can_set")
        .expect("revoked application SET option"));
    assert!(role_proof
        .try_get::<bool, _>("can_select")
        .expect("retained journal select"));
    assert!(role_proof
        .try_get::<bool, _>("can_insert")
        .expect("retained journal insert"));
    assert!(matches!(
        reader.check_ready().await,
        Err(PostgresStoreError::WriterRequired)
    ));

    drop(writer);
    drop(reader);
    drop(issuer);
    role_pool.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn readiness_rechecks_required_journal_privileges() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let mut database = TestDatabase::create("readiness_privilege").await;
    let role = unique_identifier("mfm_readiness", "privilege");
    sqlx::query(AssertSqlSafe(format!("CREATE ROLE {role} LOGIN")))
        .execute(&database.admin_pool)
        .await
        .expect("create readiness login role");
    database.roles.push(role.clone());
    sqlx::query(AssertSqlSafe(format!(
        "GRANT mfm_store_application TO {role}"
    )))
    .execute(&database.admin_pool)
    .await
    .expect("grant application authority");
    let options = database
        .connect_options()
        .username(&role)
        .options([("search_path", database.schema.as_str())]);
    let role_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect readiness privilege role");
    let (store, issuer) = open_authoritative(role_pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify readiness privilege role");
    let (writer, reader) = store.split();

    sqlx::query(AssertSqlSafe(format!(
        "REVOKE INSERT ON TABLE {schema}.journal_commits \
         FROM mfm_store_application",
        schema = database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("revoke required journal insertion");
    assert!(matches!(
        reader.check_ready().await,
        Err(PostgresStoreError::WriterRequired)
    ));
    sqlx::query(AssertSqlSafe(format!(
        "GRANT INSERT ON TABLE {schema}.journal_commits \
         TO mfm_store_application",
        schema = database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("restore required journal insertion");
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE SELECT ON TABLE {schema}.journal_commits \
         FROM mfm_store_application",
        schema = database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("revoke required journal selection");
    assert!(matches!(
        reader.check_ready().await,
        Err(PostgresStoreError::WriterRequired)
    ));

    drop(writer);
    drop(reader);
    drop(issuer);
    role_pool.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn writer_condition_proof_rejects_an_actual_read_only_transaction() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("readiness_read_only").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify read-only proof store");
    let mut transaction = database
        .pool
        .begin()
        .await
        .expect("begin read-only proof transaction");
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .expect("enter an actual PostgreSQL read-only transaction");
    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(&database.schema)
    .execute(&mut *transaction)
    .await
    .expect("pin the qualified schema");
    let row = sqlx::query(
        "SELECT pg_catalog.pg_is_in_recovery() AS in_recovery, \
                pg_catalog.current_setting('transaction_read_only') \
                    AS transaction_read_only, \
                pg_catalog.pg_has_role( \
                    current_user, 'mfm_store_application', 'SET' \
                ) AS application_role_settable, \
                pg_catalog.has_table_privilege( \
                    current_user, 'journal_commits', 'SELECT' \
                ) AS journal_select, \
                pg_catalog.has_table_privilege( \
                    current_user, 'journal_commits', 'INSERT' \
                ) AS journal_insert",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read actual PostgreSQL writer conditions");
    let in_recovery = row
        .try_get::<bool, _>("in_recovery")
        .expect("recovery state");
    let transaction_read_only = row
        .try_get::<String, _>("transaction_read_only")
        .expect("transaction mode");
    let application_role_settable = row
        .try_get::<bool, _>("application_role_settable")
        .expect("application role assumption");
    let journal_select = row
        .try_get::<bool, _>("journal_select")
        .expect("journal select privilege");
    let journal_insert = row
        .try_get::<bool, _>("journal_insert")
        .expect("journal insert privilege");
    assert!(!in_recovery);
    assert_eq!(transaction_read_only, "on");
    assert!(application_role_settable && journal_select && journal_insert);
    assert!(matches!(
        crate::store::verify_writer_conditions(
            in_recovery,
            &transaction_read_only,
            application_role_settable,
            journal_select,
            journal_insert,
        ),
        Err(PostgresStoreError::WriterRequired)
    ));
    transaction
        .rollback()
        .await
        .expect("roll back read-only proof");

    drop(store);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn readiness_reports_a_closed_pool_as_a_connection_failure() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("readiness_connection").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify readiness connection store");
    let (writer, reader) = store.split();
    database.pool.close().await;

    assert!(matches!(
        reader.check_ready().await,
        Err(PostgresStoreError::Connection)
    ));

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn admission_source_verification_mints_a_store_sealed_empty_set() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("empty_admission_sources").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify admission-source store");
    let (writer, reader) = store.split();
    let tenant = tenant_scope("empty-admission-sources")
        .parse::<TenantScopeId>()
        .expect("typed admission-source tenant");
    let authority = issuer.authorize_admit(
        tenant,
        EntryPointId::new("mfm.test/empty-admission-sources@1")
            .expect("admission-source entry point"),
        StableId::new("operation/empty-admission-sources")
            .expect("admission-source entry operation"),
        InvocationIdentity::new("00000000-0000-4000-8000-000000000040")
            .expect("admission-source invocation"),
    );

    let verified = reader
        .verify_no_admission_sources(&authority)
        .await
        .expect("mint an exact empty source set");
    assert!(verified.is_empty());

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn admission_source_verification_loads_the_complete_recursive_source_closure() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("recursive_admission_sources").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify recursive admission-source store");
    let source_a =
        LegalAdmissionFixture::for_store(store.store_identity().clone(), 50).expect("source A");
    let source_b = LegalAdmissionFixture::for_store_in_tenant(
        store.store_identity().clone(),
        source_a.tenant_scope_id().clone(),
        51,
    )
    .expect("source B")
    .with_effective_output_source();
    let consumer_c = LegalAdmissionFixture::for_store_in_tenant(
        store.store_identity().clone(),
        source_a.tenant_scope_id().clone(),
        52,
    )
    .expect("consumer C")
    .with_effective_output_source();
    for fixture in [&source_a, &source_b, &consumer_c] {
        provision_configured_value(
            &database.pool,
            fixture.configured_binding(),
            fixture.configured_bytes(),
        )
        .await;
    }
    let support_a = source_a
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify source A support");
    let support_b = source_b
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify source B support");
    let support_c = consumer_c
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify consumer C support");
    let (writer, reader) = store.split();

    let prepared_a = source_a
        .prepare_on(&writer, &reader, &issuer, &support_a)
        .await
        .expect("prepare source-free A");
    let view_a = append_and_close_legal_fixture(&writer, &issuer, &source_a, prepared_a).await;

    let proposed_a = source_b
        .proposed_effective_output_sources(&view_a)
        .expect("derive A effective-output source");
    let prepared_b = source_b
        .prepare_on_with_sources(&writer, &reader, &issuer, &support_b, proposed_a)
        .await
        .expect("prepare B with direct source A");
    let view_b = append_and_close_legal_fixture(&writer, &issuer, &source_b, prepared_b).await;
    let [requirement_a] = view_b.admission_source_requirements().cross_run_sources() else {
        panic!("B must retain exactly one direct source requirement");
    };
    assert_eq!(requirement_a.source_run_id(), view_a.run_id());

    let proposed_b = consumer_c
        .proposed_effective_output_sources(&view_b)
        .expect("derive B effective-output source");
    let prepared_c = consumer_c
        .prepare_on_with_sources(&writer, &reader, &issuer, &support_c, proposed_b)
        .await
        .expect("recursive verification must load B and then A");
    let (authority_c, append_c) = prepared_c.into_parts();
    let run_c = match writer
        .append_admission(&authority_c, append_c)
        .await
        .expect("append C after recursive verification")
    {
        AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) => {
            admitted.run_id().clone()
        }
        _ => panic!("C must be newly admitted"),
    };
    let drive_c = issuer.authorize_drive(consumer_c.tenant_scope_id().clone(), run_c);
    let view_c = writer
        .load_for_drive(&drive_c)
        .await
        .expect("load C")
        .verify_recorded_history()
        .expect("verify C");
    let [requirement_b] = view_c.admission_source_requirements().cross_run_sources() else {
        panic!("C must retain exactly one direct source requirement");
    };
    assert_eq!(requirement_b.source_run_id(), view_b.run_id());

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn qualified_run_fixture_admits_loads_qualifies_and_prepares_a_frame() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("qualified_admission").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify genuine-admission store");
    let fixture = QualifiedRunFixture::for_store(store.store_identity().clone(), 41)
        .expect("genuine qualified fixture");
    provision_configured_value(
        &database.pool,
        fixture.configured_binding(),
        fixture.configured_bytes(),
    )
    .await;
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify genuine fixture support");
    let (writer, reader) = store.split();

    let prepared = fixture
        .prepare_on(&reader, &issuer, registry)
        .await
        .expect("prepare genuine qualified admission");
    let (registry, authority, append_request_id, artifacts, input, configured, sources) =
        prepared.into_parts();
    let append = writer
        .prepare_admission(
            &authority,
            append_request_id,
            AdmissionMaterial::new(
                artifacts,
                input,
                &configured,
                registry.admitted_support(),
                &sources,
            ),
        )
        .expect("store-prepare genuine admission");
    let run_id = match writer
        .append_admission(&authority, append)
        .await
        .expect("append genuine qualified admission")
    {
        AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) => {
            admitted.run_id().clone()
        }
        _ => panic!("genuine qualified admission must be newly appended"),
    };

    let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id);
    let view = writer
        .load_for_drive(&drive)
        .await
        .expect("load genuine qualified admission")
        .verify_recorded_history()
        .expect("verify genuine qualified admission");
    registry
        .qualify_admitted_run(&view)
        .expect("the exact genuine registry must qualify its admitted run");
    let node = view
        .certified_spec()
        .nodes()
        .first()
        .expect("genuine fixture has one certified node");
    let frame = writer
        .prepare_frame(&drive, &view, node.node_id())
        .await
        .expect("prepare genuine callback frame");
    assert_eq!(frame.node_id(), node.node_id());
    assert_eq!(frame.config().bytes(), fixture.configured_bytes());
    assert_eq!(frame.input().bytes(), b"{}");
    assert!(frame.context().is_none());

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn fact_scan_continuation_attestation_and_replay_match_memory() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("fact_scan_continuation").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify fact-scan store");
    let namespace = LegalAdmissionFixture::for_store(store.store_identity().clone(), 70)
        .expect("fact-scan namespace");
    let fixture = FactScanConformanceFixture::new(
        store.store_identity().clone(),
        namespace.tenant_scope_id().clone(),
        71,
        72,
        73,
    )
    .expect("fact-scan fixture");
    for run in [
        fixture.producer(),
        fixture.late_producer(),
        fixture.consumer(),
    ] {
        provision_configured_value(
            &database.pool,
            run.configured_binding(),
            run.configured_bytes(),
        )
        .await;
    }

    fixture
        .verify_on(store, &issuer)
        .await
        .expect("PostgreSQL fact-scan conformance");

    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn fact_selection_authorization_acknowledgement_ambiguity_does_not_remint_authority() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("fact_authorization_acknowledgement_ambiguity").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify fact-authorization ambiguity store");
    let namespace = LegalAdmissionFixture::for_store(store.store_identity().clone(), 74)
        .expect("fact-authorization ambiguity namespace");
    let fixture = FactScanConformanceFixture::new(
        store.store_identity().clone(),
        namespace.tenant_scope_id().clone(),
        75,
        76,
        77,
    )
    .expect("fact-authorization ambiguity fixture");
    provision_configured_value(
        &database.pool,
        fixture.consumer().configured_binding(),
        fixture.consumer().configured_bytes(),
    )
    .await;
    let support = fixture
        .consumer()
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fact-selection consumer support");
    let (writer, reader) = store.split();

    let prepared = fixture
        .consumer()
        .prepare_on(&writer, &reader, &issuer, &support)
        .await
        .expect("prepare fact-selection consumer admission");
    let (admission_authority, admission) = prepared.into_parts();
    let run_id = match writer
        .append_admission(&admission_authority, admission)
        .await
        .expect("append fact-selection consumer admission")
    {
        AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) => {
            admitted.run_id().clone()
        }
        _ => panic!("fact-selection consumer admission must be newly appended"),
    };
    let drive =
        issuer.authorize_drive(fixture.consumer().tenant_scope_id().clone(), run_id.clone());
    let initial_view = writer
        .load_for_drive(&drive)
        .await
        .expect("load fact-selection consumer admission")
        .verify_recorded_history()
        .expect("verify fact-selection consumer admission");
    let initial_head = initial_view.journal_head().clone();
    let append_request_id = AppendRequestId::new("fact-authorization-acknowledgement-ambiguity")
        .expect("fact-selection authorization append request id");
    let (first_append, first_request) = fixture
        .prepare_authorization_on(&writer, &drive, &initial_view, append_request_id.clone())
        .await
        .expect("prepare first identical fact-selection authorization");
    let (retry_append, retry_request) = fixture
        .prepare_authorization_on(&writer, &drive, &initial_view, append_request_id.clone())
        .await
        .expect("prepare retry fact-selection authorization");
    assert_eq!(first_request, retry_request);
    assert_eq!(first_append.authorization(), retry_append.authorization());
    assert_eq!(
        first_append
            .authorization()
            .fields()
            .expect("decode prepared fact-selection authorization")
            .semantic_anchor
            .fields()
            .expect("decode prepared fact-selection semantic anchor")
            .journal_head,
        initial_head
    );

    writer
        .inject_commit_failure(
            run_id.clone(),
            BatchPurpose::ExternalAccessAuthorization,
            TestCommitFailurePoint::AfterCommitBeforeAcknowledgement,
        )
        .expect("arm post-commit fact-selection acknowledgement failure");
    match writer
        .append_fact_selection_authorization(&drive, first_append, first_request)
        .await
        .expect("return authority-free ambiguous fact-selection outcome")
    {
        FactSelectionAuthorizationOutcome::OutcomeUnknown => {}
        FactSelectionAuthorizationOutcome::NewlyAuthorized(_)
        | FactSelectionAuthorizationOutcome::AlreadyCommitted(_)
        | FactSelectionAuthorizationOutcome::Rejected(_) => {
            panic!("ambiguous acknowledgement must not return fact-scan authority");
        }
    }
    assert!(!writer
        .commit_failure_is_armed()
        .expect("post-commit failure selector must be consumed"));

    let reconciled = writer
        .load_for_drive(&drive)
        .await
        .expect("reload ambiguous fact-selection append")
        .verify_recorded_history()
        .expect("verify ambiguous fact-selection append");
    let [admission_commit, authorization_commit] = reconciled.journal().commits() else {
        panic!("reconciled journal must contain one admission and one authorization");
    };
    assert!(matches!(
        admission_commit
            .records()
            .first()
            .expect("admission commit has one record")
            .candidate()
            .fields()
            .expect("decode admission candidate")
            .payload
            .fields()
            .expect("decode admission payload"),
        RunJournalRecordFields::RunAdmitted(_)
    ));
    let authorization_fields = authorization_commit
        .envelope()
        .fields()
        .expect("decode reconciled authorization commit");
    assert_eq!(authorization_fields.core.run_sequence, 2);
    assert_eq!(
        authorization_fields.core.append_request_id,
        append_request_id
    );
    assert_eq!(
        authorization_fields.core.predecessor,
        JournalPredecessor::journal_head(&initial_head)
            .expect("derive exact authorization predecessor")
    );
    match authorization_fields
        .core
        .tenant_fact_coordinate
        .fields()
        .expect("decode reconciled fact-selection barrier")
    {
        TenantFactCoordinateFields::FactSelectionBarrier {
            tenant_scope_id,
            frontier_fact_order,
        } => {
            assert_eq!(tenant_scope_id, *fixture.consumer().tenant_scope_id());
            assert_eq!(frontier_fact_order, 0);
        }
        TenantFactCoordinateFields::None | TenantFactCoordinateFields::FactPublication { .. } => {
            panic!("fact-selection authorization must retain its exact barrier");
        }
    }
    let [authorization_record] = authorization_commit.records() else {
        panic!("authorization commit must contain exactly one record");
    };
    let recorded_authorization = match authorization_record
        .candidate()
        .fields()
        .expect("decode reconciled authorization candidate")
        .payload
        .fields()
        .expect("decode reconciled authorization payload")
    {
        RunJournalRecordFields::ExternalAccessAuthorized(authorization) => authorization,
        RunJournalRecordFields::RunAdmitted(_)
        | RunJournalRecordFields::StateTransitionCommitted(_)
        | RunJournalRecordFields::ExternalAccessObserved(_)
        | RunJournalRecordFields::RunClosed(_) => {
            panic!("reconciled record must be an external-access authorization");
        }
    };
    assert_eq!(&recorded_authorization, retry_append.authorization());
    let expected_authorization_head = authorization_commit
        .envelope()
        .journal_head()
        .expect("derive reconciled authorization head");
    let expected_authorization_record = authorization_record
        .record_ref(&run_id, authorization_fields.core.run_sequence)
        .expect("derive reconciled authorization record reference");
    let mut authorizations = reconciled.authorizations();
    let (authorization_ref, folded_authorization) = authorizations
        .next()
        .expect("verified fold must retain the authorization");
    assert!(authorizations.next().is_none());
    assert_eq!(folded_authorization, retry_append.authorization());
    assert_eq!(
        authorization_ref
            .record_ref()
            .expect("project folded authorization reference"),
        expected_authorization_record
    );
    assert_eq!(reconciled.unobserved_authorizations().count(), 1);

    let retry_committed = match writer
        .append_fact_selection_authorization(&drive, retry_append, retry_request)
        .await
        .expect("resolve exact fact-selection authorization retry")
    {
        FactSelectionAuthorizationOutcome::AlreadyCommitted(committed) => committed,
        FactSelectionAuthorizationOutcome::NewlyAuthorized(_)
        | FactSelectionAuthorizationOutcome::Rejected(_)
        | FactSelectionAuthorizationOutcome::OutcomeUnknown => {
            panic!("exact retry must reconcile without reminting fact-scan authority");
        }
    };
    assert_eq!(retry_committed.journal_head(), &expected_authorization_head);
    assert_eq!(
        retry_committed.record_refs(),
        std::slice::from_ref(&expected_authorization_record)
    );
    let retry_frontier = retry_committed
        .fact_frontier()
        .expect("exact retry must recover the committed barrier")
        .fields()
        .expect("decode exact retry fact frontier");
    assert_eq!(
        retry_frontier.tenant_scope_id,
        *fixture.consumer().tenant_scope_id()
    );
    assert_eq!(retry_frontier.fact_order, 0);

    let final_view = writer
        .load_for_drive(&drive)
        .await
        .expect("reload reconciled fact-selection authorization")
        .verify_recorded_history()
        .expect("verify reconciled fact-selection authorization");
    assert_eq!(final_view.journal().commits().len(), 2);
    assert_eq!(final_view.authorizations().count(), 1);

    let append_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) \
           FROM journal_commits \
          WHERE run_id = $1 AND append_request_id = $2",
    )
    .bind(run_id.as_str())
    .bind(append_request_id.as_str())
    .fetch_one(&database.pool)
    .await
    .expect("count exact fact-selection authorization append");
    assert_eq!(append_count, 1);
    let barrier_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) \
           FROM journal_commits \
          WHERE run_id = $1 \
            AND tenant_fact_coordinate_kind = 'fact_selection_barrier' \
            AND tenant_fact_order = 0",
    )
    .bind(run_id.as_str())
    .fetch_one(&database.pool)
    .await
    .expect("count exact fact-selection barrier");
    assert_eq!(barrier_count, 1);
    let retained_fact_head = sqlx::query_scalar::<_, String>(
        "SELECT current_fact_order::text \
           FROM tenant_fact_order_heads \
          WHERE tenant_scope_id = $1",
    )
    .bind(fixture.consumer().tenant_scope_id().as_str())
    .fetch_one(&database.pool)
    .await
    .expect("load retained fact head after selection barrier");
    assert_eq!(retained_fact_head, "0");

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn independently_opened_writers_reconcile_the_same_admission_under_cas() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("independent_writer_admission_cas").await;
    let first_pool = database.independent_store_pool().await;
    let second_pool = database.independent_store_pool().await;
    let mut first_connection = first_pool
        .acquire()
        .await
        .expect("acquire first independent connection");
    let mut second_connection = second_pool
        .acquire()
        .await
        .expect("acquire second independent connection");
    let first_backend_pid = sqlx::query_scalar::<_, i32>("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&mut *first_connection)
        .await
        .expect("inspect first independent connection");
    let second_backend_pid = sqlx::query_scalar::<_, i32>("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&mut *second_connection)
        .await
        .expect("inspect second independent connection");
    assert_ne!(
        first_backend_pid, second_backend_pid,
        "the CAS race must cross independently constructed PostgreSQL connections"
    );
    drop(first_connection);
    drop(second_connection);
    let (first, first_issuer) = open_authoritative(first_pool, TestAuthoritativeWriterFence)
        .await
        .expect("open first authoritative writer");
    let (second, second_issuer) = open_authoritative(second_pool, TestAuthoritativeWriterFence)
        .await
        .expect("open second authoritative writer");
    assert_eq!(first.store_identity(), second.store_identity());

    let fixture = LegalAdmissionFixture::for_store(first.store_identity().clone(), 78)
        .expect("independent-writer fixture");
    provision_configured_value(
        &database.pool,
        fixture.configured_binding(),
        fixture.configured_bytes(),
    )
    .await;
    let first_support = fixture
        .qualify_on(&first, &first_issuer)
        .await
        .expect("qualify support through first writer");
    let second_support = fixture
        .qualify_on(&second, &second_issuer)
        .await
        .expect("reconcile support through second writer");
    let (first_writer, first_reader) = first.split();
    let (second_writer, second_reader) = second.split();
    let first_prepared = fixture
        .prepare_on(&first_writer, &first_reader, &first_issuer, &first_support)
        .await
        .expect("prepare admission through first writer");
    let second_prepared = fixture
        .prepare_on(
            &second_writer,
            &second_reader,
            &second_issuer,
            &second_support,
        )
        .await
        .expect("prepare admission through second writer");
    assert_eq!(
        first_prepared.append().admission(),
        second_prepared.append().admission(),
        "independent qualification must author the same immutable root"
    );

    let (first_authority, first_append) = first_prepared.into_parts();
    let (second_authority, second_append) = second_prepared.into_parts();
    let start = Arc::new(tokio::sync::Barrier::new(3));
    let first_start = Arc::clone(&start);
    let first_task = tokio::spawn(async move {
        first_start.wait().await;
        first_writer
            .append_admission(&first_authority, first_append)
            .await
    });
    let second_start = Arc::clone(&start);
    let second_task = tokio::spawn(async move {
        second_start.wait().await;
        second_writer
            .append_admission(&second_authority, second_append)
            .await
    });
    start.wait().await;
    let first_outcome = first_task
        .await
        .expect("first writer task must not panic")
        .expect("first writer append");
    let second_outcome = second_task
        .await
        .expect("second writer task must not panic")
        .expect("second writer append");
    assert!(
        matches!(
            (&first_outcome, &second_outcome),
            (
                AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(_)),
                AppendOutcome::AlreadyCommitted(_)
            ) | (
                AppendOutcome::AlreadyCommitted(_),
                AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(_))
            )
        ),
        "exactly one independent writer must publish while the other reconciles"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*)::bigint FROM journal_commits WHERE run_sequence = 1"
        )
        .fetch_one(&database.pool)
        .await
        .expect("count retained admission roots"),
        1
    );

    drop(first_reader);
    drop(second_reader);
    drop(first_issuer);
    drop(second_issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn runtime_normalizes_changed_root_admission_retry_to_admission_conflict() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("runtime_changed_root_admission").await;
    let pool = database.independent_store_pool().await;
    let (store, issuer) = open_authoritative(pool, TestAuthoritativeWriterFence)
        .await
        .expect("open authoritative Runtime store");
    let fixture = QualifiedRunFixture::for_store(store.store_identity().clone(), 79)
        .expect("changed-root Runtime fixture");
    provision_configured_value(
        &database.pool,
        fixture.configured_binding(),
        fixture.configured_bytes(),
    )
    .await;
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify changed-root Runtime registry");
    let (writer, reader) = store.split();
    let first = fixture
        .prepare_on(&reader, &issuer, Arc::clone(&registry))
        .await
        .expect("prepare first Runtime admission");
    let changed = fixture
        .prepare_on(&reader, &issuer, Arc::clone(&registry))
        .await
        .expect("prepare changed-root Runtime retry");
    let runtime = Runtime::new(writer, registry);
    let admitted = runtime
        .admit(runtime_admission_plan(&fixture, first, None))
        .await
        .expect("commit first Runtime admission");
    let error = match runtime
        .admit(runtime_admission_plan(
            &fixture,
            changed,
            Some(
                PlainCanonicalJsonBytes::from_json_str(r#"{"changed":true}"#)
                    .expect("changed canonical admission input"),
            ),
        ))
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("changed immutable root must conflict"),
    };
    assert!(matches!(
        error,
        RuntimeError::Store(StoreError::AdmissionConflict)
    ));

    let replay =
        issuer.authorize_replay(fixture.tenant_scope_id().clone(), admitted.run_id().clone());
    assert_eq!(
        reader
            .load_for_replay(&replay)
            .await
            .expect("load retained admission after conflict")
            .commits()
            .len(),
        1,
        "the changed-root retry must not publish another root"
    );

    drop(runtime);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn reopened_runtime_classifies_unavailable_candidate_as_operational_block() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("runtime_reopen_unavailable").await;
    let pool = database.independent_store_pool().await;
    let (store, issuer) = open_authoritative(pool, TestAuthoritativeWriterFence)
        .await
        .expect("open recorded Runtime backend");
    let identity = store.store_identity().clone();
    let recorded = QualifiedRunFixture::for_store(identity.clone(), 80).expect("recorded fixture");
    let unavailable = QualifiedRunFixture::for_store_with_operation(
        identity,
        81,
        StableId::new("mfm.fixture/unavailable-operation").expect("unavailable operation"),
    )
    .expect("unavailable candidate fixture");
    for fixture in [&recorded, &unavailable] {
        provision_configured_value(
            &database.pool,
            fixture.configured_binding(),
            fixture.configured_bytes(),
        )
        .await;
    }
    let run_id = admit_recorded_run(store, &issuer, &recorded).await;
    drop(issuer);

    assert!(matches!(
        drive_with_reopened_candidate(&database, &unavailable, recorded.tenant_scope_id(), run_id,)
            .await,
        DriveOutcome::Waiting {
            reason: DriveWaitReason::OperationalBlock,
            ..
        }
    ));
    database.cleanup().await;
}

#[tokio::test]
async fn reopened_runtime_classifies_incompatible_candidate_as_integrity_block() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("runtime_reopen_incompatible").await;
    let pool = database.independent_store_pool().await;
    let (store, issuer) = open_authoritative(pool, TestAuthoritativeWriterFence)
        .await
        .expect("open recorded Runtime backend");
    let identity = store.store_identity().clone();
    let recorded = QualifiedRunFixture::for_store(identity.clone(), 82).expect("recorded fixture");
    let incompatible = QualifiedRunFixture::for_store(identity, 83)
        .expect("incompatible candidate fixture")
        .with_incompatible_planning_profile();
    for fixture in [&recorded, &incompatible] {
        provision_configured_value(
            &database.pool,
            fixture.configured_binding(),
            fixture.configured_bytes(),
        )
        .await;
    }
    let run_id = admit_recorded_run(store, &issuer, &recorded).await;
    drop(issuer);

    assert!(matches!(
        drive_with_reopened_candidate(
            &database,
            &incompatible,
            recorded.tenant_scope_id(),
            run_id,
        )
        .await,
        DriveOutcome::Waiting {
            reason: DriveWaitReason::IntegrityBlock,
            ..
        }
    ));
    database.cleanup().await;
}

#[tokio::test]
async fn reopened_runtime_classifies_same_identity_decode_failure_as_integrity_block() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("runtime_reopen_decode_failure").await;
    let pool = database.independent_store_pool().await;
    let (store, issuer) = open_authoritative(pool, TestAuthoritativeWriterFence)
        .await
        .expect("open recorded Runtime backend");
    let identity = store.store_identity().clone();
    let recorded = QualifiedRunFixture::for_store(identity.clone(), 84).expect("recorded fixture");
    let integrity_failing = QualifiedRunFixture::for_store(identity, 84)
        .expect("same-identity candidate fixture")
        .with_integrity_failing_state_callback();
    provision_configured_value(
        &database.pool,
        recorded.configured_binding(),
        recorded.configured_bytes(),
    )
    .await;
    let run_id = admit_recorded_run(store, &issuer, &recorded).await;
    drop(issuer);

    assert!(matches!(
        drive_with_reopened_candidate(
            &database,
            &integrity_failing,
            recorded.tenant_scope_id(),
            run_id,
        )
        .await,
        DriveOutcome::Waiting {
            reason: DriveWaitReason::IntegrityBlock,
            ..
        }
    ));
    database.cleanup().await;
}

#[tokio::test]
async fn admission_retry_and_successor_serialize_on_the_existing_run_without_deadlock() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("admission_retry_successor_race").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify genuine race store");
    let fixture = QualifiedRunFixture::for_store(store.store_identity().clone(), 42)
        .expect("genuine qualified fixture");
    provision_configured_value(
        &database.pool,
        fixture.configured_binding(),
        fixture.configured_bytes(),
    )
    .await;
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify genuine race support");
    let (writer, reader) = store.split();

    let prepared = fixture
        .prepare_on(&reader, &issuer, registry)
        .await
        .expect("prepare initial genuine admission");
    let (registry, admission_authority, append_request_id, artifacts, input, configured, sources) =
        prepared.into_parts();
    let admission = writer
        .prepare_admission(
            &admission_authority,
            append_request_id,
            AdmissionMaterial::new(
                artifacts,
                input,
                &configured,
                registry.admitted_support(),
                &sources,
            ),
        )
        .expect("prepare initial genuine admission append");
    let run_id = match writer
        .append_admission(&admission_authority, admission)
        .await
        .expect("append initial genuine admission")
    {
        AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) => {
            admitted.run_id().clone()
        }
        _ => panic!("initial genuine admission must be newly appended"),
    };

    let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = writer
        .load_for_drive(&drive)
        .await
        .expect("load initial genuine admission")
        .verify_recorded_history()
        .expect("verify initial genuine admission");
    registry
        .qualify_admitted_run(&view)
        .expect("the genuine registry must qualify the raced run");
    let node = view
        .certified_spec()
        .nodes()
        .first()
        .expect("fixture has one certified node");
    let node_id = node.node_id().clone();
    let certified_output = node
        .settlement_contract()
        .output_slots()
        .first()
        .expect("fixture has one certified output");
    let output_ordinal = certified_output.output_ordinal();
    let output_path = certified_output.field_path().clone();
    let output_contract = certified_output.value_contract().clone();
    let frame = writer
        .prepare_frame(&drive, &view, &node_id)
        .await
        .expect("prepare genuine pure frame");
    let output = PlainCanonicalJsonBytes::from_json_str(r#"{"value":43}"#)
        .expect("canonical genuine output");
    let output_schema_id = output_contract.schema_id().clone();
    let output_digest = RecoverabilityContractV2::embedded()
        .expect("embedded recoverability contract")
        .raw_content_digest(output.as_bytes());
    let successor_append_request_id =
        AppendRequestId::new("qualified-fixture-successor/2a").expect("successor request id");
    let successor = writer
        .prepare_append(
            &drive,
            &view,
            successor_append_request_id,
            ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                prepared_frame: Box::new(frame),
                settlement: SettlementMaterial::Succeeded {
                    output_roots: vec![ProducedOutputSlot::new(
                        output_ordinal,
                        output_path,
                        ProducedObjectRoot::new(output_contract, output),
                    )],
                    fact_roots: Vec::new(),
                },
                object_graph: ObjectGraphProposal::empty(),
            })),
        )
        .expect("prepare genuine successor");
    let retry = fixture
        .prepare_on(&reader, &issuer, Arc::clone(&registry))
        .await
        .expect("prepare immutable-root retry");
    let (
        retry_registry,
        retry_authority,
        retry_append_request_id,
        retry_artifacts,
        retry_input,
        retry_configured,
        retry_sources,
    ) = retry.into_parts();
    let retry_admission = writer
        .prepare_admission(
            &retry_authority,
            retry_append_request_id,
            AdmissionMaterial::new(
                retry_artifacts,
                retry_input,
                &retry_configured,
                retry_registry.admitted_support(),
                &retry_sources,
            ),
        )
        .expect("prepare immutable-root retry append");

    let run_lock_key =
        crate::journal_store::run_advisory_lock_key(&run_id).expect("derive exact run lock");
    let mut holder = database.pool.begin().await.expect("begin run-lock holder");
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
        .bind(run_lock_key)
        .execute(&mut *holder)
        .await
        .expect("hold exact run lock");
    let hook = writer
        .inject_before_admission_run_lock(fixture.append_request_id().clone())
        .expect("arm admission run-lock hook");

    let writer = Arc::new(writer);
    let retry_store = Arc::clone(&writer);
    let mut retry_task = tokio::spawn(async move {
        retry_store
            .append_admission(&retry_authority, retry_admission)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), hook.wait_until_reached())
        .await
        .expect("admission retry must reach the run-lock boundary");

    let successor_store = Arc::clone(&writer);
    let mut successor_task =
        tokio::spawn(async move { successor_store.append(&drive, successor).await });
    wait_for_advisory_waiters(&database.pool, run_lock_key, 1).await;
    hook.release();
    wait_for_advisory_waiters(&database.pool, run_lock_key, 2).await;
    assert!(
        !retry_task.is_finished() && !successor_task.is_finished(),
        "both appends must remain serialized behind the held run lock"
    );

    holder
        .rollback()
        .await
        .expect("release exact run-lock holder");
    let successor_outcome = tokio::time::timeout(Duration::from_secs(5), &mut successor_task)
        .await
        .expect("successor must make progress without deadlock")
        .expect("successor task must not panic")
        .expect("successor append must succeed");
    assert!(matches!(
        successor_outcome,
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
    ));
    let retry_outcome = tokio::time::timeout(Duration::from_secs(5), &mut retry_task)
        .await
        .expect("admission retry must make progress without deadlock")
        .expect("admission retry task must not panic")
        .expect("admission retry append must succeed");
    assert!(matches!(retry_outcome, AppendOutcome::AlreadyCommitted(_)));

    let final_drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let journal = writer
        .load_for_drive(&final_drive)
        .await
        .expect("load final raced journal");
    let admission_count = journal
        .commits()
        .iter()
        .flat_map(|commit| commit.records())
        .filter(|record| {
            matches!(
                record
                    .candidate()
                    .fields()
                    .expect("decode final candidate")
                    .payload
                    .fields()
                    .expect("decode final payload"),
                RunJournalRecordFields::RunAdmitted(_)
            )
        })
        .count();
    assert_eq!(admission_count, 1);
    assert_eq!(journal.commits().len(), 2);
    let final_view = journal
        .verify_recorded_history()
        .expect("verify final raced journal");
    let comparison = final_view
        .comparison_frames()
        .expect("reconstruct exact comparison frames");
    assert_eq!(comparison.frames().len(), 1);
    let final_head = final_view.journal_head().clone();
    drop(comparison);
    drop(final_view);

    let public_authority =
        issuer.authorize_read_public(fixture.tenant_scope_id().clone(), run_id.clone());
    let public = reader
        .read_public_run(&public_authority)
        .await
        .expect("read genuine public run")
        .into_validated();
    assert_eq!(public.schema_contract(), "mfm.public-run-view.v1");
    let public: serde_json::Value =
        serde_json::from_slice(public.as_bytes()).expect("decode genuine public run");
    assert_eq!(public["status"], "succeeded");
    assert_eq!(public["active"], serde_json::Value::Null);
    let [public_output] = public["public_outputs"]
        .as_array()
        .expect("public outputs array")
        .as_slice()
    else {
        panic!("the genuine run must project exactly one public output");
    };
    assert_eq!(public_output["name"], "result");
    assert_eq!(public_output["schema_id"], output_schema_id.as_str());
    assert_eq!(public_output["value_digest"], output_digest.as_str());
    assert_eq!(public_output["value"], serde_json::json!({"value": 43}));

    let trace_authority =
        issuer.authorize_inspect_trace(fixture.tenant_scope_id().clone(), run_id.clone());
    let trace_request =
        TransitionTracePageRequest::new(None, 0, 1).expect("bounded first trace page");
    let requirements = reader
        .discover_transition_trace_sources(&trace_authority, trace_request)
        .await
        .expect("discover genuine trace sources");
    assert!(requirements.source_run_ids().is_empty());
    let trace_page = reader
        .inspect_transition_trace(&trace_authority, requirements, &[])
        .await
        .expect("render genuine transition trace");
    assert_eq!(trace_page.run_id(), &run_id);
    assert_eq!(trace_page.at_journal_head(), &final_head);
    assert_eq!(trace_page.transitions().len(), 1);
    assert!(!trace_page.has_more());
    assert_eq!(trace_page.next_index(), None);
    let CanonicalValue::Object(trace) = trace_page.transitions()[0].canonical_value() else {
        panic!("verified trace must be an annex object");
    };
    let version = trace
        .entries()
        .find_map(|(key, value)| (key == "version").then_some(value));
    assert!(matches!(
        version,
        Some(CanonicalValue::String(value)) if value == "mfm.transition-trace.v1"
    ));

    drop(writer);
    drop(reader);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn generation_fence_rejects_stale_writer_rollback_and_identity_change() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let mut database = TestDatabase::create("generation_fence").await;
    let tenant = tenant_scope("generation-fence");
    let admission = CommitInput::admission(
        "generation-fence-run",
        &tenant,
        "operation/generation-fence",
        "00000000-0000-4000-8000-000000000010",
    );
    insert_complete_commit(&database.pool, &admission)
        .await
        .expect("insert fenced admission");
    let successor = CommitInput::successor(
        "generation-fence-successor",
        &admission,
        "none",
        None,
        false,
    );
    insert_complete_commit(&database.pool, &successor)
        .await
        .expect("insert retained fenced suffix");
    let head_admission = CommitInput::admission(
        "generation-fence-head-run",
        &tenant,
        "operation/generation-fence-head",
        "00000000-0000-4000-8000-000000000012",
    );
    insert_complete_commit(&database.pool, &head_admission)
        .await
        .expect("insert tenant-head admission");
    let head_publication = CommitInput::successor(
        "generation-fence-head-publication",
        &head_admission,
        "fact_publication",
        Some(1),
        true,
    );
    let mut head_transaction = database
        .pool
        .begin()
        .await
        .expect("begin tenant-head publication");
    let assigned = assign_tenant_coordinate(&mut head_transaction, &tenant, "fact_publication")
        .await
        .expect("assign fenced tenant head");
    assert_eq!(assigned, "1");
    insert_complete_commit_tx(&mut head_transaction, &head_publication)
        .await
        .expect("insert fenced tenant-head publication");
    head_transaction
        .commit()
        .await
        .expect("commit fenced tenant head");

    let (fence, expected_scope) = database
        .create_generation_fence(&successor, &tenant, 1, 1)
        .await;
    let (store, issuer) = open_authoritative(database.pool.clone(), fence.clone())
        .await
        .expect("the exact retained generation must qualify");
    assert_eq!(
        store.store_identity().store_scope_id().as_str(),
        expected_scope
    );
    drop(store);
    drop(issuer);

    let stale = open_authoritative(database.pool.clone(), fence.clone()).await;
    assert!(matches!(
        stale,
        Err(PostgresStoreError::WriterFenceRejected)
    ));

    sqlx::query("ALTER TABLE journal_records DISABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("disable record mutation guard for rollback simulation");
    sqlx::query("ALTER TABLE journal_commits DISABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("disable commit mutation guard for rollback simulation");
    sqlx::query("DELETE FROM journal_records WHERE run_id = $1 AND run_sequence = $2")
        .bind(&successor.run_id)
        .bind(successor.run_sequence)
        .execute(&database.pool)
        .await
        .expect("simulate record-suffix rollback");
    sqlx::query("DELETE FROM journal_commits WHERE run_id = $1 AND run_sequence = $2")
        .bind(&successor.run_id)
        .bind(successor.run_sequence)
        .execute(&database.pool)
        .await
        .expect("simulate commit-suffix rollback");
    sqlx::query("ALTER TABLE journal_records ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore record mutation guard");
    sqlx::query("ALTER TABLE journal_commits ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore commit mutation guard");

    let rollback =
        open_authoritative(database.pool.clone(), fence.with_expected_generation(2)).await;
    assert!(matches!(
        rollback,
        Err(PostgresStoreError::WriterFenceRejected)
    ));

    insert_complete_commit(&database.pool, &successor)
        .await
        .expect("restore suffix before identity rollback simulation");
    let changed_scope = format!(
        "mfm.store_scope.v1:{}",
        &digest_hex(b"changed-store-lineage")[..32]
    );
    sqlx::query("ALTER TABLE store_identity DISABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("disable identity guard for rollback simulation");
    sqlx::query("UPDATE store_identity SET store_scope_id = $1")
        .bind(changed_scope)
        .execute(&database.pool)
        .await
        .expect("simulate a different retained store identity");
    sqlx::query("ALTER TABLE store_identity ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore identity guard");

    let identity_change =
        open_authoritative(database.pool.clone(), fence.with_expected_generation(2)).await;
    assert!(matches!(
        identity_change,
        Err(PostgresStoreError::WriterFenceRejected)
    ));

    database.cleanup().await;
}

#[tokio::test]
async fn qualification_rejects_a_writable_role_without_application_authority() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let mut database = TestDatabase::create("unqualified").await;
    let role = database.create_unqualified_login_role().await;
    let options = database
        .connect_options()
        .username(&role)
        .options([("search_path", database.schema.as_str())]);
    let unqualified_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect the unqualified test role");

    let result = open_authoritative(unqualified_pool.clone(), TestAuthoritativeWriterFence).await;
    assert!(matches!(result, Err(PostgresStoreError::WriterRequired)));
    unqualified_pool.close().await;

    database.cleanup().await;
}

#[tokio::test]
async fn qualification_rejects_application_membership_without_role_assumption() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let mut database = TestDatabase::create("application_role_set_disabled").await;
    let role = unique_identifier("mfm_application", "set_disabled");
    sqlx::query(AssertSqlSafe(format!("CREATE ROLE {role} LOGIN")))
        .execute(&database.admin_pool)
        .await
        .expect("create SET-disabled application login");
    database.roles.push(role.clone());
    sqlx::query(AssertSqlSafe(format!(
        "GRANT mfm_store_application TO {role} WITH SET FALSE"
    )))
    .execute(&database.admin_pool)
    .await
    .expect("grant inheritable application membership without role assumption");

    let role_proof = sqlx::query(
        "SELECT \
             pg_catalog.pg_has_role($1, 'mfm_store_application', 'MEMBER') AS is_member, \
             pg_catalog.pg_has_role($1, 'mfm_store_application', 'SET') AS can_set, \
             pg_catalog.has_table_privilege($1, 'journal_commits', 'SELECT') AS can_select, \
             pg_catalog.has_table_privilege($1, 'journal_commits', 'INSERT') AS can_insert",
    )
    .bind(&role)
    .fetch_one(&database.pool)
    .await
    .expect("inspect SET-disabled application membership");
    assert!(role_proof
        .try_get::<bool, _>("is_member")
        .expect("application membership"));
    assert!(!role_proof
        .try_get::<bool, _>("can_set")
        .expect("application SET option"));
    assert!(role_proof
        .try_get::<bool, _>("can_select")
        .expect("inherited journal select"));
    assert!(role_proof
        .try_get::<bool, _>("can_insert")
        .expect("inherited journal insert"));

    let options = database
        .connect_options()
        .username(&role)
        .options([("search_path", database.schema.as_str())]);
    let role_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect SET-disabled application login");
    let result = open_authoritative(role_pool.clone(), TestAuthoritativeWriterFence).await;
    assert!(matches!(result, Err(PostgresStoreError::WriterRequired)));
    role_pool.close().await;

    database.cleanup().await;
}

#[tokio::test]
async fn schema_validation_rejects_retired_or_extra_authority_tables() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("oldschema").await;

    sqlx::query("CREATE TABLE store_commit_order (retired BIGINT PRIMARY KEY)")
        .execute(&database.pool)
        .await
        .expect("inject a retired schema table");
    let result = validate_authoritative_schema(&database.pool).await;
    assert!(matches!(
        result,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    database.cleanup().await;
}

#[tokio::test]
async fn schema_validation_rejects_a_changed_migration_checksum() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("checksum").await;

    sqlx::query("UPDATE _sqlx_migrations SET checksum = decode('00', 'hex')")
        .execute(&database.pool)
        .await
        .expect("mutate the test migration ledger");
    let result = validate_authoritative_schema(&database.pool).await;
    assert!(matches!(
        result,
        Err(PostgresStoreError::MigrationChecksumMismatch)
    ));

    database.cleanup().await;
}

#[tokio::test]
#[ignore = "the managed SQLx task probes its caller-selected schema"]
async fn verification_probe_accepts_the_current_authoritative_schema() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL is required for the schema probe");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect the caller-selected schema");
    validate_authoritative_schema(&pool)
        .await
        .expect("current schema must match the authoritative runtime model");
    pool.close().await;
}

#[tokio::test]
async fn schema_validation_rejects_a_changed_constraint_with_the_authoritative_name() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("constraint_definition").await;

    sqlx::query(
        "ALTER TABLE configured_values \
         DROP CONSTRAINT configured_values_binding_bounds_v1, \
         ADD CONSTRAINT configured_values_binding_bounds_v1 CHECK (TRUE)",
    )
    .execute(&database.pool)
    .await
    .expect("replace one constraint without changing its authoritative name");
    let result = validate_authoritative_schema(&database.pool).await;
    assert!(matches!(
        result,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    database.cleanup().await;
}

#[tokio::test]
async fn journal_load_rejects_admission_routing_that_disagrees_with_the_canonical_root() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("admission_route").await;
    let tenant = tenant_scope("admission-route");
    let admission = CommitInput::admission(
        "admission-route",
        &tenant,
        "operation/admission-route",
        "00000000-0000-4000-8000-000000000021",
    );
    insert_decodable_admission(&database.pool, &admission).await;

    sqlx::query("ALTER TABLE journal_commits DISABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("disable commit mutation guard for hostile routing simulation");
    sqlx::query(
        "UPDATE journal_commits \
            SET admission_entry_point_operation_id = 'operation/hostile-route', \
                admission_invocation_identity = '00000000-0000-4000-8000-000000000022' \
          WHERE run_id = $1 AND run_sequence = 1",
    )
    .bind(&admission.run_id)
    .execute(&database.pool)
    .await
    .expect("replace physical admission routing copies");
    sqlx::query("ALTER TABLE journal_commits ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore commit mutation guard");

    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("physical routing corruption must be detected by the exact run loader");
    let store_identity = store.store_identity().clone();
    let tenant_scope_id = admission
        .tenant_scope_id
        .parse::<TenantScopeId>()
        .expect("typed admission tenant");
    let run_id = admission
        .run_id
        .parse::<RunId>()
        .expect("typed admission run");
    let mut transaction = database
        .pool
        .begin()
        .await
        .expect("begin hostile admission load");
    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(&database.schema)
    .execute(&mut *transaction)
    .await
    .expect("pin hostile admission schema");
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .expect("use the application role for the hostile load");
    let error = crate::journal_store::verify_persisted_run_for_test(
        &mut transaction,
        &store_identity,
        &tenant_scope_id,
        &run_id,
    )
    .await
    .expect_err("physical admission routing must agree with canonical RunAdmitted");
    assert!(matches!(
        error,
        PostgresStoreError::Corruption("admission routing disagrees with canonical admission")
    ));
    transaction
        .rollback()
        .await
        .expect("release hostile load snapshot");

    drop(store);
    drop(issuer);
    database.cleanup().await;
}

#[tokio::test]
async fn schema_validation_rejects_an_unapproved_direct_grantee() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("direct_grantee").await;

    sqlx::query("GRANT INSERT ON journal_commits TO PUBLIC")
        .execute(&database.pool)
        .await
        .expect("inject an unapproved direct grant");
    let result = validate_authoritative_schema(&database.pool).await;
    assert!(matches!(
        result,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    database.cleanup().await;
}

#[tokio::test]
async fn schema_validation_rejects_application_mutation_of_the_migration_ledger() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("migration_ledger_acl").await;

    sqlx::query("GRANT INSERT ON _sqlx_migrations TO mfm_store_application")
        .execute(&database.pool)
        .await
        .expect("inject a migration-ledger mutation grant");
    let result = validate_authoritative_schema(&database.pool).await;
    assert!(matches!(
        result,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    database.cleanup().await;
}

#[tokio::test]
async fn application_role_cannot_mutate_sealed_authority() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("privileges").await;

    let mut transaction = database.pool.begin().await.expect("begin role test");
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .expect("assume the application role");
    let identity_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM store_identity")
        .fetch_one(&mut *transaction)
        .await
        .expect("application role may read retained identity");
    assert_eq!(identity_count, 1);
    transaction.rollback().await.expect("rollback role read");

    assert_permission_denied(
        &database.pool,
        "UPDATE journal_commits SET committed_at = committed_at",
    )
    .await;
    assert_permission_denied(
        &database.pool,
        "INSERT INTO tenant_fact_order_heads \
         (tenant_scope_id, current_fact_order) \
         VALUES ('mfm.tenant_scope.v1:00000000000000000000000000000000', 0)",
    )
    .await;
    assert_permission_denied(
        &database.pool,
        "INSERT INTO configured_values \
         (store_scope_id, tenant_scope_id, entry_point_id, target, artifact_id, \
          content_digest, evidence_hash, canonical_value_ref, canonical_binding) \
         VALUES ('mfm.store_scope.v1:00000000000000000000000000000000', \
                 'mfm.tenant_scope.v1:00000000000000000000000000000000', \
                 'mfm.test/config@1', 'settings', \
                 'artifact:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000', \
                 'content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000', \
                 'sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000', \
                 decode('7b7d', 'hex'), decode('7b7d', 'hex'))",
    )
    .await;

    let error = sqlx::query("UPDATE store_identity SET store_epoch = store_epoch")
        .execute(&database.pool)
        .await
        .expect_err("even a migration owner must hit the immutable identity trigger");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("P0001")
    );

    database.cleanup().await;
}

#[tokio::test]
async fn qualified_support_graph_is_atomic_exact_and_producer_complete() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("support_graph").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify support store");
    let scope = semantic_type("qualified-support");
    let authority = issuer.authorize_qualified_deployment(scope.clone());

    let admitted = store
        .admit_support_graph(
            &authority,
            support_graph(
                &scope,
                &[
                    ("support.first", r#"{"shared":true}"#),
                    ("support.second", r#"{"shared":true}"#),
                ],
            ),
        )
        .await
        .expect("atomically admit exact support graph");
    let first_path = FieldPath::new("support.first").expect("first support path");
    let second_path = FieldPath::new("support.second").expect("second support path");
    let first = admitted.member(&first_path).expect("first admitted member");
    let second = admitted
        .member(&second_path)
        .expect("second admitted member");
    let first_fields = first.value_ref().fields().expect("first support authority");
    let second_fields = second
        .value_ref()
        .fields()
        .expect("second support authority");
    assert_ne!(
        first.value_ref().as_bytes(),
        second.value_ref().as_bytes(),
        "producer binding is part of full support authority"
    );
    assert_eq!(first_fields.artifact_id, second_fields.artifact_id);
    assert_eq!(first_fields.content_digest, second_fields.content_digest);
    assert_eq!(first_fields.evidence_hash, second_fields.evidence_hash);

    let retried = store
        .admit_support_graph(
            &authority,
            support_graph(
                &scope,
                &[
                    ("support.first", r#"{"shared":true}"#),
                    ("support.second", r#"{"shared":true}"#),
                ],
            ),
        )
        .await
        .expect("exact support retry is idempotent");
    assert_eq!(retried.members().len(), admitted.members().len());
    for (path, retained) in admitted.members() {
        let retry = retried.member(path).expect("retried support member");
        assert_eq!(retry.value_ref(), retained.value_ref());
        assert_eq!(retry.bytes(), retained.bytes());
    }

    let conflict = match store
        .admit_support_graph(
            &authority,
            support_graph(
                &scope,
                &[
                    ("support.first", r#"{"shared":true}"#),
                    ("support.second", r#"{"shared":false}"#),
                ],
            ),
        )
        .await
    {
        Ok(_) => panic!("changed retained bytes must conflict"),
        Err(error) => error,
    };
    assert!(matches!(
        conflict,
        PostgresStoreError::Store(error)
            if matches!(error.as_ref(), StoreError::ObjectAuthorityConflict { .. })
    ));

    let incomplete = match store
        .admit_support_graph(
            &authority,
            support_graph(&scope, &[("support.first", r#"{"shared":true}"#)]),
        )
        .await
    {
        Ok(_) => panic!("a retained graph cannot be retried with a missing path"),
        Err(error) => error,
    };
    assert!(matches!(
        incomplete,
        PostgresStoreError::Store(error)
            if matches!(error.as_ref(), StoreError::InvalidObjectAuthority { .. })
    ));

    let other_scope = semantic_type("other-qualified-support");
    let wrong_authority = issuer.authorize_qualified_deployment(other_scope);
    let denied = match store
        .admit_support_graph(
            &wrong_authority,
            support_graph(
                &scope,
                &[
                    ("support.first", r#"{"shared":true}"#),
                    ("support.second", r#"{"shared":true}"#),
                ],
            ),
        )
        .await
    {
        Ok(_) => panic!("qualification scope must match exact authority"),
        Err(error) => error,
    };
    assert!(matches!(
        denied,
        PostgresStoreError::Store(error)
            if matches!(
                error.as_ref(),
                StoreError::AccessDenied {
                    purpose: "admit_support_graph"
                }
            )
    ));

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM artifact_blobs")
            .fetch_one(&database.pool)
            .await
            .expect("count support blobs"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM artifact_admissions")
            .fetch_one(&database.pool)
            .await
            .expect("count full support authorities"),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM qualified_support_members")
            .fetch_one(&database.pool)
            .await
            .expect("count support path bindings"),
        2
    );

    drop(store);
    database.cleanup().await;
}

#[tokio::test]
async fn configured_values_resolve_only_the_exact_immutable_admission_key() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("configured_value").await;
    let (store, issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify configured-value store");
    let tenant = tenant_scope("configured-value")
        .parse::<TenantScopeId>()
        .expect("configured-value tenant");
    let entry_point =
        EntryPointId::new("mfm.test/configured-value@1").expect("configured entry point");
    let target = StableId::new("settings").expect("configured target");
    let authority = issuer.authorize_admit(
        tenant.clone(),
        entry_point.clone(),
        StableId::new("operation/configured-value").expect("entry operation"),
        InvocationIdentity::new("00000000-0000-4000-8000-000000000030")
            .expect("configured invocation"),
    );
    let contract = retained_contract("configured-value");
    let bytes =
        PlainCanonicalJsonBytes::from_json_str(r#"{"enabled":true}"#).expect("configured bytes");
    let store_scope_id = store.store_identity().store_scope_id().clone();
    let key = ConfiguredValueKey::new(&store_scope_id, &tenant, &entry_point, &target)
        .expect("configured key");
    let producer =
        ProducerBinding::configured_value(&store_scope_id, &tenant, &entry_point, &target)
            .expect("configured producer");
    let value_ref = derive_value_ref(&contract, &producer, bytes.as_bytes());
    let binding = ConfiguredValueBinding::new(&key, &value_ref).expect("configured binding");
    provision_configured_value(&database.pool, &binding, bytes.as_bytes()).await;
    let (writer, reader) = store.split();

    let resolved = reader
        .resolve_configured_value(&authority, &entry_point, &target, &contract)
        .await
        .expect("resolve exact configured value");
    assert_eq!(resolved.binding(), &binding);
    assert_eq!(resolved.value_ref(), &value_ref);
    assert_eq!(resolved.bytes(), bytes.as_bytes());

    let missing_target = StableId::new("other-settings").expect("other target");
    let missing = match reader
        .resolve_configured_value(&authority, &entry_point, &missing_target, &contract)
        .await
    {
        Ok(_) => panic!("target-only fallback must not exist"),
        Err(error) => error,
    };
    assert!(matches!(
        missing,
        PostgresStoreError::Store(error)
            if matches!(error.as_ref(), StoreError::ObjectNotReachable)
    ));

    let mismatched_contract = retained_contract("different-configured-value");
    let mismatch = match reader
        .resolve_configured_value(&authority, &entry_point, &target, &mismatched_contract)
        .await
    {
        Ok(_) => panic!("the certified contract must match the immutable binding"),
        Err(error) => error,
    };
    assert!(matches!(
        mismatch,
        PostgresStoreError::Store(error)
            if matches!(error.as_ref(), StoreError::InvalidObjectAuthority { .. })
    ));

    let mutation = sqlx::query("UPDATE configured_values SET target = target")
        .execute(&database.pool)
        .await
        .expect_err("even the owner cannot mutate an immutable configured binding");
    assert_eq!(
        mutation
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("P0001")
    );

    let other_scope = StoreScopeId::new("mfm.store_scope.v1:00000000000000000000000000000000")
        .expect("other store scope");
    assert_ne!(&other_scope, &store_scope_id);
    let mut transaction = database
        .pool
        .begin()
        .await
        .expect("begin wrong-scope provisioning");
    let value = value_ref.fields().expect("configured value fields");
    sqlx::query(
        "INSERT INTO configured_values \
            (store_scope_id, tenant_scope_id, entry_point_id, target, artifact_id, \
             content_digest, evidence_hash, canonical_value_ref, canonical_binding) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(other_scope.as_str())
    .bind(tenant.as_str())
    .bind(entry_point.as_str())
    .bind(target.as_str())
    .bind(value.artifact_id.as_str())
    .bind(value.content_digest.as_str())
    .bind(value.evidence_hash.as_str())
    .bind(value_ref.as_bytes())
    .bind(binding.as_bytes())
    .execute(&mut *transaction)
    .await
    .expect("deferred identity foreign key permits staging");
    let wrong_scope = transaction
        .commit()
        .await
        .expect_err("configured values cannot target another store lineage");
    assert_eq!(
        wrong_scope
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23503")
    );

    drop(writer);
    drop(reader);
    database.cleanup().await;
}

#[tokio::test]
async fn commit_failure_selector_is_exact_non_consuming_and_one_shot() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("failure_selector").await;
    let (store, _issuer) = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
        .await
        .expect("qualify failure-selector store");
    let (writer, reader) = store.split();
    let selected = run_id("failure-selector-selected")
        .parse::<RunId>()
        .expect("selected run id");
    let unrelated = run_id("failure-selector-unrelated")
        .parse::<RunId>()
        .expect("unrelated run id");

    writer
        .inject_commit_failure(
            selected.clone(),
            BatchPurpose::PureSettlement,
            TestCommitFailurePoint::BeforeCommit,
        )
        .expect("arm exact commit failure");
    assert!(writer
        .commit_failure_is_armed()
        .expect("inspect armed selector"));
    assert!(writer
        .inject_commit_failure(
            selected.clone(),
            BatchPurpose::PureSettlement,
            TestCommitFailurePoint::AfterCommitBeforeAcknowledgement,
        )
        .is_err());
    assert_eq!(
        writer
            .take_commit_failure(&unrelated, BatchPurpose::PureSettlement)
            .expect("unrelated run does not consume"),
        None
    );
    assert_eq!(
        writer
            .take_commit_failure(&selected, BatchPurpose::ReadSettlement)
            .expect("unrelated purpose does not consume"),
        None
    );
    assert!(writer
        .commit_failure_is_armed()
        .expect("selector remains armed"));
    assert_eq!(
        writer
            .take_commit_failure(&selected, BatchPurpose::PureSettlement)
            .expect("matching append consumes once"),
        Some(TestCommitFailurePoint::BeforeCommit)
    );
    assert!(!writer
        .commit_failure_is_armed()
        .expect("selector is disarmed"));
    assert_eq!(
        writer
            .take_commit_failure(&selected, BatchPurpose::PureSettlement)
            .expect("consumed selector stays absent"),
        None
    );

    drop(writer);
    drop(reader);
    database.cleanup().await;
}

#[tokio::test]
async fn admission_logical_key_is_unique_across_run_ids() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("admission").await;
    let tenant = tenant_scope("admission");
    let invocation = "00000000-0000-4000-8000-000000000001";
    let first = CommitInput::admission("admission-first", &tenant, "operation/admit", invocation);
    insert_complete_commit(&database.pool, &first)
        .await
        .expect("insert first admission");

    let conflicting =
        CommitInput::admission("admission-second", &tenant, "operation/admit", invocation);
    let mut transaction = database.pool.begin().await.expect("begin conflict");
    let error = insert_commit_row(&mut transaction, &conflicting)
        .await
        .expect_err("the same tenant admission key must not admit a second run");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23505")
    );
    transaction.rollback().await.expect("rollback conflict");

    let commit_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM journal_commits")
        .fetch_one(&database.pool)
        .await
        .expect("count admissions");
    assert_eq!(commit_count, 1);

    database.cleanup().await;
}

#[tokio::test]
async fn transaction_advisory_locks_serialize_only_the_same_run_and_release_on_rollback() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("advisory").await;
    let first_run = run_id("advisory-first")
        .parse::<RunId>()
        .expect("valid first advisory run id");
    let second_run = run_id("advisory-second")
        .parse::<RunId>()
        .expect("valid second advisory run id");
    let first_key = crate::journal_store::run_advisory_lock_key(&first_run)
        .expect("derive stable first run lock");
    let second_key = crate::journal_store::run_advisory_lock_key(&second_run)
        .expect("derive stable second run lock");
    assert_ne!(first_key, second_key);

    let mut holder = database.pool.begin().await.expect("begin lock holder");
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
        .bind(first_key)
        .execute(&mut *holder)
        .await
        .expect("hold first run lock");

    tokio::time::timeout(Duration::from_secs(2), async {
        let mut independent = database
            .pool
            .begin()
            .await
            .expect("begin independent lock transaction");
        sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
            .bind(second_key)
            .execute(&mut *independent)
            .await
            .expect("different run lock must make progress");
        independent
            .rollback()
            .await
            .expect("release independent lock");
    })
    .await
    .expect("different lock key must not wait");

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let waiting_pool = database.pool.clone();
    let mut waiter = tokio::spawn(async move {
        let mut transaction = waiting_pool.begin().await.expect("begin same-key waiter");
        started_tx.send(()).expect("signal same-key waiter");
        sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
            .bind(first_key)
            .execute(&mut *transaction)
            .await
            .expect("acquire released same-run lock");
        transaction
            .rollback()
            .await
            .expect("rollback same-key waiter");
    });
    started_rx.await.expect("same-key waiter started");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiter)
            .await
            .is_err(),
        "the same run key must remain serialized while its transaction is live"
    );
    holder
        .rollback()
        .await
        .expect("rollback must release transaction-scoped lock");
    tokio::time::timeout(Duration::from_secs(2), &mut waiter)
        .await
        .expect("same-key waiter must progress after rollback")
        .expect("same-key waiter task succeeds");

    database.cleanup().await;
}

#[tokio::test]
async fn locked_head_cas_and_append_request_idempotency_observe_one_current_commit() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("cas").await;
    let tenant = tenant_scope("cas");
    let admission = CommitInput::admission(
        "cas-run",
        &tenant,
        "operation/cas",
        "00000000-0000-4000-8000-000000000011",
    );
    insert_complete_commit(&database.pool, &admission)
        .await
        .expect("insert CAS admission");
    let successor = CommitInput::successor("cas-successor", &admission, "none", None, false);
    let typed_run = admission.run_id.parse::<RunId>().expect("valid CAS run id");
    let lock_key =
        crate::journal_store::run_advisory_lock_key(&typed_run).expect("derive CAS run lock");

    let mut publisher = database.pool.begin().await.expect("begin CAS publisher");
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut *publisher)
        .await
        .expect("lock CAS run");
    let observed_head = sqlx::query_scalar::<_, String>(
        "SELECT commit_digest FROM journal_commits \
          WHERE run_id = $1 ORDER BY run_sequence DESC LIMIT 1",
    )
    .bind(&admission.run_id)
    .fetch_one(&mut *publisher)
    .await
    .expect("load locked predecessor");
    assert_eq!(observed_head, admission.commit_digest);
    insert_complete_commit_tx(&mut publisher, &successor)
        .await
        .expect("stage CAS successor");

    let waiting_pool = database.pool.clone();
    let waiting_run_id = admission.run_id.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut waiter = tokio::spawn(async move {
        let mut transaction = waiting_pool.begin().await.expect("begin CAS waiter");
        started_tx.send(()).expect("signal CAS waiter");
        sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *transaction)
            .await
            .expect("lock CAS waiter");
        let head = sqlx::query(
            "SELECT run_sequence::text AS run_sequence, commit_digest \
               FROM journal_commits \
              WHERE run_id = $1 ORDER BY run_sequence DESC LIMIT 1",
        )
        .bind(&waiting_run_id)
        .fetch_one(&mut *transaction)
        .await
        .expect("load post-serialization head");
        let sequence = head
            .try_get::<String, _>("run_sequence")
            .expect("decode current sequence");
        let digest = head
            .try_get::<String, _>("commit_digest")
            .expect("decode current digest");
        transaction.rollback().await.expect("rollback CAS waiter");
        (sequence, digest)
    });
    started_rx.await.expect("CAS waiter started");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiter)
            .await
            .is_err(),
        "the second CAS must wait for the first publication"
    );
    publisher.commit().await.expect("publish CAS successor");
    let (sequence, digest) = tokio::time::timeout(Duration::from_secs(2), &mut waiter)
        .await
        .expect("CAS waiter must progress")
        .expect("CAS waiter task succeeds");
    assert_eq!(sequence, "2");
    assert_eq!(digest, successor.commit_digest);
    assert_ne!(digest, admission.commit_digest);

    let retained = sqlx::query(
        "SELECT candidate_digest, predecessor_commit_digest \
           FROM journal_commits WHERE run_id = $1 AND append_request_id = $2",
    )
    .bind(&successor.run_id)
    .bind(&successor.append_request_id)
    .fetch_one(&database.pool)
    .await
    .expect("resolve exact idempotent append");
    assert_eq!(
        retained
            .try_get::<String, _>("candidate_digest")
            .expect("decode retained candidate"),
        successor.candidate_digest
    );
    assert_eq!(
        retained
            .try_get::<String, _>("predecessor_commit_digest")
            .expect("decode retained predecessor"),
        admission.commit_digest
    );

    let conflicting = CommitInput {
        run_sequence: 3,
        predecessor_run_sequence: Some(2),
        predecessor_commit_digest: successor.commit_digest.clone(),
        candidate_digest: semantic_digest(b"conflicting-retry-candidate"),
        commit_digest: semantic_digest(b"conflicting-retry-commit"),
        ..CommitInput::successor("cas-conflicting-retry", &successor, "none", None, false)
    };
    let conflicting = CommitInput {
        append_request_id: successor.append_request_id.clone(),
        ..conflicting
    };
    let mut transaction = database
        .pool
        .begin()
        .await
        .expect("begin conflicting retry");
    let error = insert_commit_row(&mut transaction, &conflicting)
        .await
        .expect_err("one append request id cannot name different immutable content");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23505")
    );
    transaction
        .rollback()
        .await
        .expect("rollback conflicting retry");

    database.cleanup().await;
}

#[tokio::test]
async fn deferred_batch_failure_rolls_back_commit_records_and_tenant_head() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("atomic").await;
    let tenant = tenant_scope("atomic");
    let admission = CommitInput::admission(
        "atomic-run",
        &tenant,
        "operation/atomic",
        "00000000-0000-4000-8000-000000000002",
    );
    insert_complete_commit(&database.pool, &admission)
        .await
        .expect("insert admission");

    let mut publication = CommitInput::successor(
        "atomic-publication",
        &admission,
        "fact_publication",
        Some(1),
        true,
    );
    let mut transaction = database.pool.begin().await.expect("begin publication");
    let assigned = assign_tenant_coordinate(&mut transaction, &tenant, "fact_publication")
        .await
        .expect("assign first tenant publication");
    assert_eq!(assigned, "1");
    publication.fact_order = Some(1);
    insert_complete_commit_tx(&mut transaction, &publication)
        .await
        .expect("insert publication batch");
    transaction.commit().await.expect("commit publication");

    let barrier = CommitInput::successor(
        "atomic-barrier",
        &publication,
        "fact_selection_barrier",
        Some(1),
        false,
    );
    let mut transaction = database.pool.begin().await.expect("begin barrier");
    let assigned = assign_tenant_coordinate(&mut transaction, &tenant, "fact_selection_barrier")
        .await
        .expect("read the stable barrier");
    assert_eq!(assigned, "1");
    insert_complete_commit_tx(&mut transaction, &barrier)
        .await
        .expect("insert barrier batch");
    transaction.commit().await.expect("commit barrier");

    let mut incomplete = CommitInput::successor(
        "atomic-incomplete",
        &barrier,
        "fact_publication",
        Some(2),
        true,
    );
    let mut transaction = database.pool.begin().await.expect("begin incomplete batch");
    let assigned = assign_tenant_coordinate(&mut transaction, &tenant, "fact_publication")
        .await
        .expect("tentatively assign second tenant publication");
    assert_eq!(assigned, "2");
    incomplete.fact_order = Some(2);
    insert_commit_row(&mut transaction, &incomplete)
        .await
        .expect("the incomplete commit row is accepted until deferred validation");
    transaction
        .commit()
        .await
        .expect_err("deferred record validation must reject the incomplete batch");

    let retained_head = sqlx::query_scalar::<_, String>(
        "SELECT current_fact_order::text \
           FROM tenant_fact_order_heads \
          WHERE tenant_scope_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&database.pool)
    .await
    .expect("load retained tenant head");
    assert_eq!(retained_head, "1");
    let commit_count =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM journal_commits WHERE run_id = $1")
            .bind(&admission.run_id)
            .fetch_one(&database.pool)
            .await
            .expect("count atomic run commits");
    assert_eq!(commit_count, 3);
    let record_count =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM journal_records WHERE run_id = $1")
            .bind(&admission.run_id)
            .fetch_one(&database.pool)
            .await
            .expect("count atomic run records");
    assert_eq!(record_count, 3);

    database.cleanup().await;
}

#[tokio::test]
async fn qualification_rejects_corrupt_tenant_fact_history() {
    let _serial = DATABASE_TEST_LOCK.lock().await;
    let database = TestDatabase::create("corrupthead").await;
    let tenant = tenant_scope("corrupthead");
    let admission = CommitInput::admission(
        "corrupt-run",
        &tenant,
        "operation/corrupt",
        "00000000-0000-4000-8000-000000000003",
    );
    insert_complete_commit(&database.pool, &admission)
        .await
        .expect("insert admission");
    let publication = CommitInput::successor(
        "corrupt-publication",
        &admission,
        "fact_publication",
        Some(1),
        true,
    );
    let mut transaction = database.pool.begin().await.expect("begin publication");
    assign_tenant_coordinate(&mut transaction, &tenant, "fact_publication")
        .await
        .expect("assign publication");
    insert_complete_commit_tx(&mut transaction, &publication)
        .await
        .expect("insert publication");
    transaction.commit().await.expect("commit publication");

    sqlx::query("ALTER TABLE tenant_fact_order_heads DISABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("disable test corruption guards");
    sqlx::query(
        "UPDATE tenant_fact_order_heads \
            SET current_fact_order = current_fact_order + 1 \
          WHERE tenant_scope_id = $1",
    )
    .bind(&tenant)
    .execute(&database.pool)
    .await
    .expect("inject retained-head corruption");
    sqlx::query("ALTER TABLE tenant_fact_order_heads ENABLE TRIGGER ALL")
        .execute(&database.pool)
        .await
        .expect("restore test corruption guards");

    let result = open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence).await;
    assert!(matches!(
        result,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    database.cleanup().await;
}

fn semantic_type(label: &str) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        label,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(label.as_bytes()),
    )
    .expect("test semantic type")
}

fn retained_contract(label: &str) -> RetainedValueContract {
    let recoverability =
        RecoverabilityContractV2::embedded().expect("embedded recoverability contract");
    let evidence = recoverability
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String(format!("{label}.evidence")),
        )
        .expect("test evidence contract");
    RetainedValueContract::new(
        recoverability
            .schema_id("mfm.primitive-canonical_value.v1")
            .expect("test retained schema")
            .clone(),
        semantic_type(label),
        StableId::new(label).expect("test retained role"),
        "application/json",
        recoverability
            .content_ref(&evidence)
            .expect("test evidence contract reference"),
    )
    .expect("test retained contract")
}

fn support_graph(scope: &SemanticTypeId, members: &[(&str, &str)]) -> QualifiedSupportGraph {
    QualifiedSupportGraph::new(
        scope.clone(),
        members.iter().map(|(field_path, json)| {
            QualifiedSupportMember::new(
                FieldPath::new(field_path).expect("test support field path"),
                PlainCanonicalJsonBytes::from_json_str(json).expect("test support bytes"),
                retained_contract("qualified-support-value"),
            )
        }),
    )
    .expect("test support graph")
}

fn derive_value_ref(
    contract: &RetainedValueContract,
    producer: &ProducerBinding,
    bytes: &[u8],
) -> ValueRef {
    let recoverability =
        RecoverabilityContractV2::embedded().expect("embedded recoverability contract");
    let content_digest = recoverability.raw_content_digest(bytes);
    let artifact_id = ArtifactIdPreimage::new(
        contract.schema_id(),
        &content_digest,
        contract.semantic_type_id(),
    )
    .expect("test artifact preimage")
    .artifact_id()
    .expect("test artifact id");
    let byte_length = u64::try_from(bytes.len()).expect("bounded test bytes");
    let evidence_hash = ObjectEvidencePreimage::new(
        &artifact_id,
        &content_digest,
        contract.schema_id(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
    )
    .expect("test object evidence preimage")
    .evidence_hash()
    .expect("test object evidence digest");
    ValueRef::new(
        &artifact_id,
        &content_digest,
        &evidence_hash,
        contract.schema_id(),
        contract.semantic_type_id(),
        contract.role(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
        producer,
    )
    .expect("test full value reference")
}

fn admission_reference() -> ContentRef {
    let recoverability =
        RecoverabilityContractV2::embedded().expect("embedded recoverability contract");
    let value = recoverability
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String("mfm.test/admission-reference".to_owned()),
        )
        .expect("test admission reference value");
    recoverability
        .content_ref(&value)
        .expect("test admission content reference")
}

fn decodable_admission_record(input: &CommitInput) -> (RunJournalRecord, RecordLogicalKey) {
    let run_id = input
        .run_id
        .parse::<RunId>()
        .expect("typed test admission run");
    let reference = admission_reference();
    let spec_hash = format!(
        "spec:sha256-jcs-v1:{}",
        digest_hex(b"decodable-test-admission-spec")
    )
    .parse::<SpecHash>()
    .expect("test admission spec hash");
    let genesis_digest = semantic_digest(b"decodable-test-admission-genesis")
        .parse::<GenesisDigest>()
        .expect("test admission genesis digest");
    let initial_run_state_digest = semantic_digest(b"decodable-test-admission-state")
        .parse::<RunSemanticStateDigest>()
        .expect("test admission state digest");
    let admission = RunAdmitted::new(&RunAdmittedFields {
        run_id: run_id.clone(),
        tenant_scope_id: input
            .tenant_scope_id
            .parse()
            .expect("typed test admission tenant"),
        invocation_identity: input
            .admission_invocation_identity
            .as_deref()
            .expect("test admission invocation")
            .parse()
            .expect("typed test admission invocation"),
        entry_point_operation_id: StableId::new(
            input
                .admission_entry_point_operation_id
                .as_deref()
                .expect("test admission operation"),
        )
        .expect("typed test admission operation"),
        operation_contract_ref: reference.clone(),
        executable_identity_ref: reference.clone(),
        spec_hash,
        certified_spec_ref: reference.clone(),
        certificate_ref: reference.clone(),
        state_implementation_manifest_ref: reference.clone(),
        capability_binding_manifest_ref: reference.clone(),
        config_manifest_ref: reference.clone(),
        seed_manifest_ref: reference.clone(),
        context_manifest_ref: reference.clone(),
        cross_run_source_manifest_ref: reference,
        initial_bindings: Vec::new(),
        genesis_digest,
        initial_run_state_digest,
    })
    .expect("construct decodable test admission");
    (
        RunJournalRecord::run_admitted(&admission).expect("construct decodable admission record"),
        RecordLogicalKey::run_admission(&run_id).expect("construct admission logical key"),
    )
}

async fn insert_decodable_admission(pool: &PgPool, input: &CommitInput) {
    let (payload, logical_key) = decodable_admission_record(input);
    let spec_hash = match payload.fields().expect("decode test admission record") {
        RunJournalRecordFields::RunAdmitted(admission) => {
            admission.fields().expect("decode test admission").spec_hash
        }
        _ => panic!("test record is not an admission"),
    };
    let seed = format!("{}:{}", input.run_id, input.run_sequence);
    let mut transaction = pool.begin().await.expect("begin decodable admission");
    insert_commit_row(&mut transaction, input)
        .await
        .expect("insert decodable admission commit");
    sqlx::query(
        "INSERT INTO journal_records ( \
             run_id, run_sequence, tenant_scope_id, fact_order, ordinal, record_id, \
             record_schema_id, spec_hash, logical_key, record_hash, canonical_payload, \
             emits_facts \
         ) VALUES ($1, $2, $3, NULL, 0, $4, $5, $6, $7, $8, $9, FALSE)",
    )
    .bind(&input.run_id)
    .bind(input.run_sequence)
    .bind(&input.tenant_scope_id)
    .bind(format!(
        "record:sha256-jcs-v1:{}",
        digest_hex(seed.as_bytes())
    ))
    .bind(payload.schema_id().as_str())
    .bind(spec_hash.as_str())
    .bind(logical_key.as_bytes())
    .bind(semantic_digest(format!("{seed}:record").as_bytes()))
    .bind(payload.as_bytes())
    .execute(&mut *transaction)
    .await
    .expect("insert decodable admission record");
    transaction
        .commit()
        .await
        .expect("commit decodable admission");
}

async fn provision_configured_value(pool: &PgPool, binding: &ConfiguredValueBinding, bytes: &[u8]) {
    let binding_fields = binding.fields().expect("configured binding fields");
    let key = binding_fields.key.fields().expect("configured key fields");
    let value = binding_fields
        .value_ref
        .fields()
        .expect("configured value fields");
    let mut transaction = pool.begin().await.expect("begin owner provisioning");
    sqlx::query(
        "INSERT INTO artifact_blobs (content_digest, byte_length, bytes) \
         VALUES ($1, $2::numeric, $3)",
    )
    .bind(value.content_digest.as_str())
    .bind(value.byte_length.to_string())
    .bind(bytes)
    .execute(&mut *transaction)
    .await
    .expect("provision configured blob");
    sqlx::query(
        "INSERT INTO artifact_admissions \
            (artifact_id, evidence_hash, content_digest, schema_id, semantic_type_id, role, \
             byte_length, media_type, evidence_contract_schema_id, \
             evidence_contract_content_digest, canonical_value_ref) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, $9, $10, $11)",
    )
    .bind(value.artifact_id.as_str())
    .bind(value.evidence_hash.as_str())
    .bind(value.content_digest.as_str())
    .bind(value.schema_id.as_str())
    .bind(value.semantic_type_id.as_str())
    .bind(value.role.as_str())
    .bind(value.byte_length.to_string())
    .bind(&value.media_type)
    .bind(value.evidence_contract_ref.schema_id().as_str())
    .bind(value.evidence_contract_ref.content_digest().as_str())
    .bind(binding_fields.value_ref.as_bytes())
    .execute(&mut *transaction)
    .await
    .expect("provision configured object authority");
    sqlx::query(
        "INSERT INTO configured_values \
            (store_scope_id, tenant_scope_id, entry_point_id, target, artifact_id, \
             content_digest, evidence_hash, canonical_value_ref, canonical_binding) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(key.store_scope_id.as_str())
    .bind(key.tenant_scope_id.as_str())
    .bind(key.entry_point_id.as_str())
    .bind(key.target.as_str())
    .bind(value.artifact_id.as_str())
    .bind(value.content_digest.as_str())
    .bind(value.evidence_hash.as_str())
    .bind(binding_fields.value_ref.as_bytes())
    .bind(binding.as_bytes())
    .execute(&mut *transaction)
    .await
    .expect("provision configured binding");
    transaction
        .commit()
        .await
        .expect("commit configured-value provisioning");
}

struct RejectFence;

impl AuthoritativeWriterFence for RejectFence {
    type Error = ();

    fn verify<'a>(
        &'a self,
        _writer_pool: &'a PgPool,
        _context: &'a AuthoritativeWriterContext,
    ) -> AuthoritativeWriterFenceFuture<'a, Self::Error> {
        Box::pin(async { Err(()) })
    }
}

#[derive(Clone)]
struct GenerationFence {
    control_schema: String,
    expected_generation: i64,
}

impl GenerationFence {
    fn with_expected_generation(&self, expected_generation: i64) -> Self {
        Self {
            control_schema: self.control_schema.clone(),
            expected_generation,
        }
    }
}

impl AuthoritativeWriterFence for GenerationFence {
    type Error = ();

    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a AuthoritativeWriterContext,
    ) -> AuthoritativeWriterFenceFuture<'a, Self::Error> {
        Box::pin(async move {
            let mut transaction = writer_pool.begin().await.map_err(|_| ())?;
            let statement = format!(
                "UPDATE {control}.writer_generation \
                    SET generation = generation + 1 \
                  WHERE singleton \
                    AND generation = $1 \
                    AND database_name = $2 \
                    AND schema_name = $3 \
                    AND database_oid = $4 \
                    AND store_scope_id = $5 \
                    AND store_epoch = $6::numeric \
                    AND EXISTS ( \
                        SELECT 1 FROM journal_commits \
                         WHERE run_id = required_run_id \
                           AND run_sequence = required_run_sequence \
                           AND commit_digest = required_commit_digest \
                    ) \
                    AND EXISTS ( \
                        SELECT 1 FROM tenant_fact_order_heads \
                         WHERE tenant_scope_id = required_tenant_scope_id \
                           AND current_fact_order >= required_tenant_fact_order \
                    ) \
                RETURNING generation",
                control = self.control_schema
            );
            let updated = sqlx::query(AssertSqlSafe(statement))
                .bind(self.expected_generation)
                .bind(context.database_name())
                .bind(context.schema_name())
                .bind(i64::from(context.database_oid()))
                .bind(context.store_scope_id().as_str())
                .bind(context.store_epoch().get().to_string())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| ())?;
            if updated.is_none() {
                transaction.rollback().await.map_err(|_| ())?;
                return Err(());
            }
            transaction.commit().await.map_err(|_| ())
        })
    }
}

async fn append_and_close_legal_fixture(
    writer: &PostgresWriter,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &LegalAdmissionFixture,
    prepared: PreparedLegalAdmission,
) -> VerifiedRunView {
    let (authority, admission) = prepared.into_parts();
    let run_id = match writer
        .append_admission(&authority, admission)
        .await
        .expect("append legal fixture admission")
    {
        AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) => {
            admitted.run_id().clone()
        }
        _ => panic!("legal fixture admission must be newly appended"),
    };
    let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = writer
        .load_for_drive(&drive)
        .await
        .expect("load open legal fixture run")
        .verify_recorded_history()
        .expect("verify open legal fixture run");
    let node = view
        .certified_spec()
        .nodes()
        .first()
        .expect("fixture has one certified node");
    let node_id = node.node_id().clone();
    let certified_output = node
        .settlement_contract()
        .output_slots()
        .first()
        .expect("fixture has one certified output");
    let output_ordinal = certified_output.output_ordinal();
    let output_path = certified_output.field_path().clone();
    let output_contract = certified_output.value_contract().clone();
    let frame = writer
        .prepare_frame(&drive, &view, &node_id)
        .await
        .expect("prepare legal fixture frame");
    let output = PlainCanonicalJsonBytes::from_json_str(r#"{"result":"settled"}"#)
        .expect("canonical legal fixture output");
    let successor = writer
        .prepare_append(
            &drive,
            &view,
            fixture.successor_append_request_id().clone(),
            ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                prepared_frame: Box::new(frame),
                settlement: SettlementMaterial::Succeeded {
                    output_roots: vec![ProducedOutputSlot::new(
                        output_ordinal,
                        output_path,
                        ProducedObjectRoot::new(output_contract, output),
                    )],
                    fact_roots: Vec::new(),
                },
                object_graph: ObjectGraphProposal::empty(),
            })),
        )
        .expect("prepare legal fixture settlement");
    assert!(matches!(
        writer
            .append(&drive, successor)
            .await
            .expect("append legal fixture settlement"),
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
    ));

    let final_drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id);
    writer
        .load_for_drive(&final_drive)
        .await
        .expect("load closed legal fixture run")
        .verify_recorded_history()
        .expect("verify closed legal fixture run")
}

async fn wait_for_advisory_waiters(pool: &PgPool, lock_key: i64, expected: i64) {
    let bytes = lock_key.to_be_bytes();
    let class_id = i64::from(u32::from_be_bytes(
        bytes[..4].try_into().expect("advisory key high half"),
    ));
    let object_id = i64::from(u32::from_be_bytes(
        bytes[4..].try_into().expect("advisory key low half"),
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let waiters = sqlx::query_scalar::<_, i64>(
                "SELECT count(*)::bigint \
                   FROM pg_catalog.pg_locks \
                  WHERE locktype = 'advisory' \
                    AND database = ( \
                        SELECT oid FROM pg_catalog.pg_database \
                         WHERE datname = pg_catalog.current_database() \
                    ) \
                    AND classid::bigint = $1 \
                    AND objid::bigint = $2 \
                    AND objsubid = 1 \
                    AND NOT granted",
            )
            .bind(class_id)
            .bind(object_id)
            .fetch_one(pool)
            .await
            .expect("inspect exact advisory-lock waiters");
            if waiters == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("exact advisory lock did not reach {expected} ungranted waiter(s)"));
}

struct TestDatabase {
    admin_pool: PgPool,
    pool: PgPool,
    schema: String,
    auxiliary_schemas: Vec<String>,
    roles: Vec<String>,
    database_url: String,
}

impl TestDatabase {
    async fn create(label: &str) -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
        let admin_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("connect PostgreSQL test administrator");
        let schema = unique_identifier("mfm_store", label);
        let create_schema = format!("CREATE SCHEMA {schema}");
        sqlx::query(AssertSqlSafe(create_schema))
            .execute(&admin_pool)
            .await
            .expect("create isolated store schema");
        let options = database_url
            .parse::<PgConnectOptions>()
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .expect("connect isolated store schema");
        migrate_pool(&pool)
            .await
            .expect("apply destructive PostgreSQL baseline");
        Self {
            admin_pool,
            pool,
            schema,
            auxiliary_schemas: Vec::new(),
            roles: Vec::new(),
            database_url,
        }
    }

    fn connect_options(&self) -> PgConnectOptions {
        self.database_url
            .parse()
            .expect("parse PostgreSQL test URL")
    }

    async fn independent_store_pool(&self) -> PgPool {
        let options = self
            .database_url
            .parse::<PgConnectOptions>()
            .expect("parse PostgreSQL test URL")
            .options([("search_path", self.schema.as_str())]);
        PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("connect independent isolated store pool")
    }

    async fn create_unqualified_login_role(&mut self) -> String {
        let role = unique_identifier("mfm_unqualified", "writer");
        let create_role = format!("CREATE ROLE {role} LOGIN");
        sqlx::query(AssertSqlSafe(create_role))
            .execute(&self.admin_pool)
            .await
            .expect("create unqualified writer role");
        let grant_schema = format!(
            "GRANT USAGE ON SCHEMA {schema} TO {role}",
            schema = self.schema
        );
        sqlx::query(AssertSqlSafe(grant_schema))
            .execute(&self.admin_pool)
            .await
            .expect("grant schema use to unqualified writer");
        let grant_tables = format!(
            "GRANT SELECT, INSERT ON TABLE \
                         {schema}.journal_commits, {schema}.journal_records \
                     TO {role}",
            schema = self.schema
        );
        sqlx::query(AssertSqlSafe(grant_tables))
            .execute(&self.admin_pool)
            .await
            .expect("grant only physical journal privileges");
        self.roles.push(role.clone());
        role
    }

    async fn create_generation_fence(
        &mut self,
        required: &CommitInput,
        required_tenant_scope_id: &str,
        required_tenant_fact_order: i64,
        generation: i64,
    ) -> (GenerationFence, String) {
        let control_schema = unique_identifier("mfm_fence", "control");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {control_schema}")))
            .execute(&self.admin_pool)
            .await
            .expect("create external writer-fence schema");
        sqlx::query(AssertSqlSafe(format!(
            "CREATE TABLE {control_schema}.writer_generation ( \
                 singleton BOOLEAN PRIMARY KEY CHECK (singleton), \
                 generation BIGINT NOT NULL, \
                 database_name TEXT NOT NULL, \
                 schema_name TEXT NOT NULL, \
                 database_oid BIGINT NOT NULL, \
                 store_scope_id TEXT NOT NULL, \
                 store_epoch NUMERIC(20, 0) NOT NULL, \
                 required_run_id TEXT NOT NULL, \
                 required_run_sequence NUMERIC(20, 0) NOT NULL, \
                 required_commit_digest TEXT NOT NULL, \
                 required_tenant_scope_id TEXT NOT NULL, \
                 required_tenant_fact_order NUMERIC(20, 0) NOT NULL \
             )"
        )))
        .execute(&self.admin_pool)
        .await
        .expect("create external writer-fence control");
        let insert = format!(
            "INSERT INTO {control_schema}.writer_generation ( \
                 singleton, generation, database_name, schema_name, database_oid, \
                 store_scope_id, store_epoch, required_run_id, required_run_sequence, \
                 required_commit_digest, required_tenant_scope_id, \
                 required_tenant_fact_order \
             ) \
             SELECT TRUE, $1, current_database(), current_schema(), database.oid::bigint, \
                    identity.store_scope_id, identity.store_epoch, $2, $3::numeric, $4, \
                    $5, $6::numeric \
               FROM store_identity AS identity \
               JOIN pg_catalog.pg_database AS database \
                 ON database.datname = current_database()"
        );
        sqlx::query(AssertSqlSafe(insert))
            .bind(generation)
            .bind(&required.run_id)
            .bind(required.run_sequence.to_string())
            .bind(&required.commit_digest)
            .bind(required_tenant_scope_id)
            .bind(required_tenant_fact_order.to_string())
            .execute(&self.pool)
            .await
            .expect("seed external writer-fence generation");
        let expected_scope =
            sqlx::query_scalar::<_, String>("SELECT store_scope_id FROM store_identity")
                .fetch_one(&self.pool)
                .await
                .expect("load expected fenced store identity");
        self.auxiliary_schemas.push(control_schema.clone());
        (
            GenerationFence {
                control_schema,
                expected_generation: generation,
            },
            expected_scope,
        )
    }

    async fn cleanup(self) {
        self.pool.close().await;
        let drop_schema = format!("DROP SCHEMA IF EXISTS {} CASCADE", self.schema);
        sqlx::query(AssertSqlSafe(drop_schema))
            .execute(&self.admin_pool)
            .await
            .expect("drop isolated store schema");
        for schema in self.auxiliary_schemas {
            sqlx::query(AssertSqlSafe(format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE"
            )))
            .execute(&self.admin_pool)
            .await
            .expect("drop external test-control schema");
        }
        for role in self.roles {
            let drop_role = format!("DROP ROLE IF EXISTS {role}");
            sqlx::query(AssertSqlSafe(drop_role))
                .execute(&self.admin_pool)
                .await
                .expect("drop isolated login role");
        }
        self.admin_pool.close().await;
    }
}

async fn assert_permission_denied(pool: &PgPool, statement: &'static str) {
    let mut transaction = pool.begin().await.expect("begin privilege test");
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .expect("assume the application role");
    let error = sqlx::query(statement)
        .execute(&mut *transaction)
        .await
        .expect_err("the application role must not have this privilege");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("42501")
    );
    transaction
        .rollback()
        .await
        .expect("rollback denied mutation");
}

async fn assign_tenant_coordinate(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_scope_id: &str,
    coordinate_kind: &str,
) -> std::result::Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT mfm_assign_tenant_fact_coordinate($1, $2)::text")
        .bind(tenant_scope_id)
        .bind(coordinate_kind)
        .fetch_one(&mut **transaction)
        .await
}

async fn insert_complete_commit(
    pool: &PgPool,
    input: &CommitInput,
) -> std::result::Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    insert_complete_commit_tx(&mut transaction, input).await?;
    transaction.commit().await
}

async fn insert_complete_commit_tx(
    transaction: &mut Transaction<'_, Postgres>,
    input: &CommitInput,
) -> std::result::Result<(), sqlx::Error> {
    insert_commit_row(transaction, input).await?;
    insert_record_row(transaction, input).await
}

async fn insert_commit_row(
    transaction: &mut Transaction<'_, Postgres>,
    input: &CommitInput,
) -> std::result::Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO journal_commits ( \
             run_id, run_sequence, append_request_id, candidate_digest, \
             predecessor_kind, predecessor_run_sequence, predecessor_commit_digest, \
             commit_digest, tenant_scope_id, admission_entry_point_operation_id, \
             admission_invocation_identity, tenant_fact_coordinate_kind, tenant_fact_order, \
             record_count, committed_at \
         ) VALUES ( \
             $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 1, $14 \
         )",
    )
    .bind(&input.run_id)
    .bind(input.run_sequence)
    .bind(&input.append_request_id)
    .bind(&input.candidate_digest)
    .bind(&input.predecessor_kind)
    .bind(input.predecessor_run_sequence)
    .bind(&input.predecessor_commit_digest)
    .bind(&input.commit_digest)
    .bind(&input.tenant_scope_id)
    .bind(input.admission_entry_point_operation_id.as_deref())
    .bind(input.admission_invocation_identity.as_deref())
    .bind(&input.coordinate_kind)
    .bind(input.fact_order)
    .bind(input.run_sequence)
    .execute(&mut **transaction)
    .await
    .map(drop)
}

async fn insert_record_row(
    transaction: &mut Transaction<'_, Postgres>,
    input: &CommitInput,
) -> std::result::Result<(), sqlx::Error> {
    let seed = format!("{}:{}", input.run_id, input.run_sequence);
    let record_id = format!("record:sha256-jcs-v1:{}", digest_hex(seed.as_bytes()));
    let schema_id = format!(
        "schema:mfm.test.record:1:sha256-jcs-v1:{}",
        digest_hex(b"mfm.test.record.v1")
    );
    let spec_hash = format!("spec:sha256-jcs-v1:{}", digest_hex(b"mfm.test.spec.v1"));
    let record_hash = semantic_digest(format!("{seed}:record").as_bytes());
    let logical_key = format!(
        r#"{{"run":"{}","sequence":{}}}"#,
        input.run_id, input.run_sequence
    );

    sqlx::query(
        "INSERT INTO journal_records ( \
             run_id, run_sequence, tenant_scope_id, fact_order, ordinal, record_id, \
             record_schema_id, spec_hash, logical_key, record_hash, canonical_payload, \
             emits_facts \
         ) VALUES ($1, $2, $3, $4, 0, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&input.run_id)
    .bind(input.run_sequence)
    .bind(&input.tenant_scope_id)
    .bind(input.fact_order.filter(|_| input.emits_facts))
    .bind(record_id)
    .bind(schema_id)
    .bind(spec_hash)
    .bind(logical_key.as_bytes())
    .bind(record_hash)
    .bind(b"{}" as &[u8])
    .bind(input.emits_facts)
    .execute(&mut **transaction)
    .await
    .map(drop)
}

struct CommitInput {
    run_id: String,
    run_sequence: i64,
    append_request_id: String,
    candidate_digest: String,
    predecessor_kind: String,
    predecessor_run_sequence: Option<i64>,
    predecessor_commit_digest: String,
    commit_digest: String,
    tenant_scope_id: String,
    admission_entry_point_operation_id: Option<String>,
    admission_invocation_identity: Option<String>,
    coordinate_kind: String,
    fact_order: Option<i64>,
    emits_facts: bool,
}

impl CommitInput {
    fn admission(
        seed: &str,
        tenant_scope_id: &str,
        entry_point_operation_id: &str,
        invocation_identity: &str,
    ) -> Self {
        Self {
            run_id: run_id(seed),
            run_sequence: 1,
            append_request_id: format!("append/{seed}/1"),
            candidate_digest: semantic_digest(format!("{seed}:candidate").as_bytes()),
            predecessor_kind: "genesis".to_owned(),
            predecessor_run_sequence: None,
            predecessor_commit_digest: semantic_digest(b"genesis"),
            commit_digest: semantic_digest(format!("{seed}:commit").as_bytes()),
            tenant_scope_id: tenant_scope_id.to_owned(),
            admission_entry_point_operation_id: Some(entry_point_operation_id.to_owned()),
            admission_invocation_identity: Some(invocation_identity.to_owned()),
            coordinate_kind: "none".to_owned(),
            fact_order: None,
            emits_facts: false,
        }
    }

    fn successor(
        seed: &str,
        predecessor: &Self,
        coordinate_kind: &str,
        fact_order: Option<i64>,
        emits_facts: bool,
    ) -> Self {
        let run_sequence = predecessor.run_sequence + 1;
        Self {
            run_id: predecessor.run_id.clone(),
            run_sequence,
            append_request_id: format!("append/{seed}/{run_sequence}"),
            candidate_digest: semantic_digest(format!("{seed}:candidate").as_bytes()),
            predecessor_kind: "journal_head".to_owned(),
            predecessor_run_sequence: Some(predecessor.run_sequence),
            predecessor_commit_digest: predecessor.commit_digest.clone(),
            commit_digest: semantic_digest(format!("{seed}:commit").as_bytes()),
            tenant_scope_id: predecessor.tenant_scope_id.clone(),
            admission_entry_point_operation_id: None,
            admission_invocation_identity: None,
            coordinate_kind: coordinate_kind.to_owned(),
            fact_order,
            emits_facts,
        }
    }
}

fn run_id(seed: &str) -> String {
    format!("run:sha256-jcs-v1:{}", digest_hex(seed.as_bytes()))
}

fn tenant_scope(seed: &str) -> String {
    format!("mfm.tenant_scope.v1:{}", &digest_hex(seed.as_bytes())[..32])
}

fn semantic_digest(seed: &[u8]) -> String {
    format!("sha256-jcs-v1:{}", digest_hex(seed))
}

fn digest_hex(seed: &[u8]) -> String {
    sha256_digest_bytes(seed).to_string()
}

fn unique_identifier(prefix: &str, label: &str) -> String {
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    format!(
        "{prefix}_{label}_{}_{}_{}",
        std::process::id(),
        timestamp,
        counter
    )
}
