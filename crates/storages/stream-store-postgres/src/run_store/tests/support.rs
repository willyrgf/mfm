use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DigestAlgorithm, EffectKind, EffectVersion, LoweringVersion, NodeId, RunId,
    SchemaId, SideEffectPairId, SpecHash, SpecVersion, StateVersion,
};
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualAuthorizationSignatureBytes,
    ManualResolutionAuthorizationProof, ManualResolutionAuthorizationSignature,
    ManualResolutionBlockReason, ManualResolutionEvidenceRef, ManualResolutionPrefixAuthority,
    ManualResolutionProofAuthority, VerifiedManualResolutionForPrefix,
};
use mfm_spec::v1::{
    self as spec, CanonicalizerIdentity, ManualResolutionEvidenceSpec, ResourceNamespace,
    SagaPolicySpec, ValueLineageRef,
};
use mfm_store::v1::test_support::{
    artifact_bytes_for_digest_for_test, artifact_content_digest_for_test as content_digest,
    confirmation_terminal_policies_for_projection_for_test as confirmation_terminal_policies_for_projection,
    fact_descriptor_projection_fixture_for_test, fixed_attempt_id_for_test as attempt_id,
    fixed_cell_id_for_test as cell_id, fixed_descriptor_id_for_test as descriptor_id,
    fixed_digest_bytes_for_test as digest_bytes, fixed_node_id_for_test as node_id,
    fixed_schema_id_for_test as schema_id, fixed_scope_id_for_test as scope_id,
    fixed_semantic_type_id_for_test as semantic_id, fixed_spec_hash_for_test as spec_hash,
    fixed_state_kind_for_test as state_kind, media_type_for_test as media_type,
    prepared_commit_plan_for_test as test_prepared_commit_plan,
    run_artifact_ref_from_store_artifact_for_test as run_artifact_ref,
    run_identity_material_for_test, FactDescriptorProjectionFixtureForTest,
};
use mfm_store::v1::{
    AdmissionLease, AdmissionToken, AdmissionWaiter, ArtifactEvidenceRef, AttemptStatus,
    CellTerminalProjection, CertifiedRunStoreAuthority, CommitArtifactEvidenceSet, CommitKey,
    CommitOutcome, CommitPreconditions, ExecutionClaimStatus, ExecutionClaimStore,
    ExistingArtifactAdmission, ManualResolution, NowaitSkipAdmissionResult, PreparedCommit,
    PreparedCommitPlan, RequiredRunState, ResourceLaneKey, Retention, RunState,
    SagaEngagementReason, SagaTerminal, SagaTerminalProof, SideEffectPhase, StoreError,
    StoreScopeId, StoreScopeStore, StreamSeq,
};
use sqlx::postgres::PgConnectOptions;
use sqlx::AssertSqlSafe;

use super::*;

#[path = "manual_resolution_support.rs"]
mod manual_resolution_support;
use self::manual_resolution_support::{
    manual_resolution_artifacts, manual_resolution_prepared_artifact_bytes,
    manual_resolution_recorded, manual_saga_policy, saga_preconditions,
    verified_manual_resolution_for_seq,
};

#[path = "authority_support.rs"]
mod authority_support;
use self::authority_support::{
    fact_authority_spec, fact_run_id, run_id, run_id_with_saga_policy,
    run_identity_material_for_fact_run_id, run_identity_material_for_run_id, saga_authority_spec,
    store_scope_hex,
};

static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_schema() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("run_store_{}_{}_{}", std::process::id(), nanos, counter)
}

fn artifact_id(byte: u8) -> ArtifactId {
    let digest = content_digest(byte);
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
}

fn test_prepared_artifact_bytes(
    evidence: &ArtifactEvidenceRef,
) -> mfm_store::v1::Result<PreparedArtifactBytes> {
    let bytes = artifact_bytes_for_digest_for_test(&evidence.digest).ok_or_else(|| {
        StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "bytes",
        }
    })?;
    PreparedArtifactBytes::new(bytes, evidence.clone())
}

async fn test_store() -> (PostgresRunStore, String) {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let admin_pool = PgPool::connect(&database_url)
        .await
        .expect("connect postgres");

    let schema = unique_schema();
    // The schema name is generated from process/time/counter digits and never comes from user
    // input; dynamic DDL is required because PostgreSQL does not parameterize identifiers.
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .expect("create schema");
    let options = PgConnectOptions::from_str(&database_url)
        .expect("postgres URL")
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect schema-scoped postgres");
    crate::schema::migrate_pool(&pool)
        .await
        .expect("migrate schema");
    let authority = crate::schema::validate_pool(&pool)
        .await
        .expect("validate schema");
    let store = PostgresRunStore { pool, authority };
    (store, schema)
}

async fn drop_schema(store: &PostgresRunStore, schema: &str) {
    store.pool.close().await;
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let admin_pool = PgPool::connect(&database_url)
        .await
        .expect("connect postgres");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema} CASCADE"
    )))
    .execute(&admin_pool)
    .await
    .expect("drop schema");
}

fn assert_fact_projection_counts(
    projection: &ProjectionSnapshot,
    descriptors: usize,
    records: usize,
    index_entries: usize,
    terms: usize,
) {
    assert_eq!(projection.fact_descriptors().count(), descriptors);
    assert_eq!(projection.fact_records().count(), records);
    assert_eq!(projection.fact_index_entries().count(), index_entries);
    assert_eq!(projection.fact_term_entries().count(), terms);
}

async fn assert_fact_projection_table_counts(
    store: &PostgresRunStore,
    run: &RunId,
    descriptors: usize,
    index_entries: usize,
    terms: usize,
) {
    assert_eq!(
        fact_projection_table_count(
            &store.pool,
            run,
            "SELECT COUNT(*) FROM run_fact_descriptor_admissions WHERE run_id = $1"
        )
        .await,
        descriptors as i64
    );
    assert_eq!(
        fact_projection_table_count(
            &store.pool,
            run,
            "SELECT COUNT(*) FROM fact_index WHERE source_run_id = $1"
        )
        .await,
        index_entries as i64
    );
    assert_eq!(
        fact_projection_table_count(
            &store.pool,
            run,
            "SELECT COUNT(*) FROM fact_index_terms WHERE source_run_id = $1"
        )
        .await,
        terms as i64
    );
}

async fn fact_projection_table_count(pool: &PgPool, run: &RunId, sql: &'static str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .bind(run.as_str())
        .fetch_one(pool)
        .await
        .expect("fact projection table count")
}

async fn global_fact_descriptor_catalog_count(store: &PostgresRunStore) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fact_descriptor_index")
        .fetch_one(&store.pool)
        .await
        .expect("global fact descriptor catalog count")
}

fn assert_fact_query_receipt(
    _store: &PostgresRunStore,
    _plan: &mfm_facts::CanonicalFactQueryPlan,
    result: &mfm_facts::FactQueryResult,
    expected_cardinality: mfm_facts::QueryResultCardinality,
) {
    assert_eq!(result.receipt().result_cardinality(), expected_cardinality);
}

async fn mutate_store_metadata_unchecked(pool: &PgPool, statements: &[&'static str]) {
    sqlx::query("ALTER TABLE store_metadata DISABLE TRIGGER store_metadata_no_update")
        .execute(pool)
        .await
        .expect("disable metadata mutation guard");
    for statement in statements.iter().copied() {
        sqlx::query(statement)
            .execute(pool)
            .await
            .expect("mutate store metadata fixture");
    }
    sqlx::query("ALTER TABLE store_metadata ENABLE TRIGGER store_metadata_no_update")
        .execute(pool)
        .await
        .expect("reenable metadata mutation guard");
}

#[tokio::test]
async fn schema_validation_accepts_store_commit_order_authority() {
    let (store, schema) = test_store().await;
    let mut tx = store.pool.begin().await.expect("begin transaction");
    let commit_order: i64 = sqlx::query_scalar(
        "INSERT INTO commits \
         (commit_id, run_id, seq, commit_key, commit_purpose, prepared_commit_plan_fingerprint, \
          commit_batch_hash, store_commit_order, event_count) \
         VALUES \
         ('schema-trigger-commit', 'schema-trigger-run', 1, 'schema-trigger-key', \
          'schema-trigger-purpose', $1, $2, 1, 1) \
         RETURNING store_commit_order",
    )
    .bind(content_digest(252).as_str())
    .bind(content_digest(253).as_str())
    .fetch_one(&mut *tx)
    .await
    .expect("insert commit with explicit store commit order");
    assert_eq!(commit_order, 1);
    tx.rollback().await.expect("rollback manual authority rows");
    crate::schema::validate_pool(&store.pool)
        .await
        .expect("schema validation still passes");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn store_authority_rejects_schema_drift_cases() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        MissingMigrationRecord,
        MigrationChecksumMismatch,
        StaleSchemaObject,
        MissingFactProjectionTable,
        RetiredFactProjectionObject,
        InvalidStoreMetadata,
        InvalidStoreScopeBinding,
        MissingMutationGuardTrigger,
    }

    for (case, expected) in [
        (
            Case::MissingMigrationRecord,
            PostgresStoreAuthorityError::Migrations,
        ),
        (
            Case::MigrationChecksumMismatch,
            PostgresStoreAuthorityError::Migrations,
        ),
        (
            Case::StaleSchemaObject,
            PostgresStoreAuthorityError::Catalog,
        ),
        (
            Case::MissingFactProjectionTable,
            PostgresStoreAuthorityError::Catalog,
        ),
        (
            Case::RetiredFactProjectionObject,
            PostgresStoreAuthorityError::Catalog,
        ),
        (
            Case::InvalidStoreMetadata,
            PostgresStoreAuthorityError::Metadata,
        ),
        (
            Case::InvalidStoreScopeBinding,
            PostgresStoreAuthorityError::StoreScope,
        ),
        (
            Case::MissingMutationGuardTrigger,
            PostgresStoreAuthorityError::Catalog,
        ),
    ] {
        let (store, schema) = test_store().await;

        match case {
            Case::MissingMigrationRecord => {
                sqlx::query("DELETE FROM _sqlx_migrations")
                    .execute(&store.pool)
                    .await
                    .expect("delete migration ledger");
            }
            Case::MigrationChecksumMismatch => {
                sqlx::query(
                    "UPDATE _sqlx_migrations SET checksum = decode(repeat('00', 32), 'hex')",
                )
                .execute(&store.pool)
                .await
                .expect("mutate migration checksum");
            }
            Case::StaleSchemaObject => {
                sqlx::query("CREATE TABLE typed_run_heads (id TEXT PRIMARY KEY)")
                    .execute(&store.pool)
                    .await
                    .expect("create stale retired table");
            }
            Case::MissingFactProjectionTable => {
                sqlx::query("DROP TABLE fact_index_terms")
                    .execute(&store.pool)
                    .await
                    .expect("drop fact term table");
            }
            Case::RetiredFactProjectionObject => {
                sqlx::query("CREATE TABLE typed_fact_projection (id TEXT PRIMARY KEY)")
                    .execute(&store.pool)
                    .await
                    .expect("create retired fact projection table");
            }
            Case::InvalidStoreMetadata => {
                mutate_store_metadata_unchecked(
                    &store.pool,
                    &["UPDATE store_metadata SET schema_contract_version = 'mfm.postgres.run_store.v0'"],
                )
                .await;
            }
            Case::InvalidStoreScopeBinding => {
                mutate_store_metadata_unchecked(
                    &store.pool,
                    &[
                        "ALTER TABLE store_metadata ALTER COLUMN store_scope_id DROP NOT NULL",
                        "UPDATE store_metadata SET store_scope_id = NULL",
                    ],
                )
                .await;
            }
            Case::MissingMutationGuardTrigger => {
                sqlx::query("DROP TRIGGER run_events_no_update ON run_events")
                    .execute(&store.pool)
                    .await
                    .expect("drop run events mutation guard");
            }
        }

        let error = crate::schema::validate_pool(&store.pool)
            .await
            .expect_err("schema drift should fail authority validation");
        assert_authority_error(error, expected);

        drop_schema(&store, &schema).await;
    }
}

#[tokio::test]
async fn store_scope_survives_reconnects_and_rejects_mutation() {
    let (store, schema) = test_store().await;

    let store_scope = store.load_store_scope_id().await.expect("load store scope");
    assert!(store_scope.as_str().starts_with(StoreScopeId::PREFIX));
    assert_eq!(store.store_authority().store_scope_id(), &store_scope);

    let restarted = PostgresRunStore {
        pool: store.pool.clone(),
        authority: store.store_authority().clone(),
    };
    let restarted_store_scope = restarted
        .load_store_scope_id()
        .await
        .expect("load restarted store scope");
    assert_eq!(store_scope, restarted_store_scope);

    sqlx::query(
        "UPDATE store_metadata \
         SET store_scope_id = 'mfm.store_scope.v1:ffffffffffffffffffffffffffffffff' \
         WHERE singleton",
    )
    .execute(&store.pool)
    .await
    .expect_err("store scope mutation is rejected");
    crate::schema::validate_pool(&store.pool)
        .await
        .expect("metadata remains valid");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn execution_claim_acquire_busy_and_release_are_token_matched() {
    let (store, schema) = test_store().await;
    let run = run_id(6);
    let holder = admission_token("mfm.test.execution_claim.holder");
    let other = admission_token("mfm.test.execution_claim.other");

    mfm_store::v1::test_support::assert_execution_claim_token_lifecycle_for_test(
        &store, &run, holder, other,
    )
    .await
    .expect("execution claim lifecycle");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn execution_claim_expiry_requires_explicit_reap() {
    let (store, schema) = test_store().await;
    let run = run_id(7);
    let scope = execution_claim_scope(7);
    let holder = admission_token("mfm.test.execution_claim.expired_holder");
    let other = admission_token("mfm.test.execution_claim.expired_other");

    let lease = acquire_execution_claim_lease(&store, &scope, &run, holder.clone()).await;
    expire_execution_claim_row(&store, &run).await;
    assert!(matches!(
        store
            .execution_claim_status(&scope)
            .await
            .expect("expired execution claim status"),
        ExecutionClaimStatus::Expired(status) if status.token == lease.token
    ));

    let busy = store
        .acquire_execution_claim(&scope, &run, other.clone())
        .await
        .expect("expired holder still blocks acquire");
    let NowaitSkipAdmissionResult::Busy(busy) = busy else {
        panic!("expired holder should remain busy before explicit reap");
    };
    assert_eq!(busy.holder.expect("expired holder").token, lease.token);

    let expired = store
        .expired_execution_claims()
        .await
        .expect("expired execution claims");
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].run_id, run);
    assert_eq!(expired[0].lease.token, holder);

    assert!(!store
        .reap_expired_execution_claim(&scope, &run, &other)
        .await
        .expect("wrong-token reap"));
    assert!(matches!(
        store
            .acquire_execution_claim(&scope, &run, other.clone())
            .await
            .expect("busy after wrong-token reap"),
        NowaitSkipAdmissionResult::Busy(_)
    ));

    assert!(store
        .reap_expired_execution_claim(&scope, &run, &holder)
        .await
        .expect("matching-token reap"));
    assert!(matches!(
        store
            .acquire_execution_claim(&scope, &run, other)
            .await
            .expect("acquire after explicit reap"),
        NowaitSkipAdmissionResult::Admitted(_)
    ));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn execution_claim_stale_token_cannot_reap_newer_holder() {
    let (store, schema) = test_store().await;
    let run = run_id(8);
    let scope = execution_claim_scope(8);
    let first = admission_token("mfm.test.execution_claim.first");
    let second = admission_token("mfm.test.execution_claim.second");
    let third = admission_token("mfm.test.execution_claim.third");

    acquire_execution_claim_lease(&store, &scope, &run, first.clone()).await;
    expire_execution_claim_row(&store, &run).await;
    assert!(store
        .reap_expired_execution_claim(&scope, &run, &first)
        .await
        .expect("first reap"));

    let second_lease = acquire_execution_claim_lease(&store, &scope, &run, second.clone()).await;
    expire_execution_claim_row(&store, &run).await;

    assert!(!store
        .reap_expired_execution_claim(&scope, &run, &first)
        .await
        .expect("stale-token reap"));
    let busy = store
        .acquire_execution_claim(&scope, &run, third)
        .await
        .expect("newer holder still blocks after stale reap");
    let NowaitSkipAdmissionResult::Busy(busy) = busy else {
        panic!("newer holder should remain busy");
    };
    assert_eq!(busy.holder.expect("newer holder").token, second_lease.token);

    assert!(store
        .reap_expired_execution_claim(&scope, &run, &second)
        .await
        .expect("newer holder reap"));

    drop_schema(&store, &schema).await;
}

async fn acquire_execution_claim_lease(
    store: &PostgresRunStore,
    scope: &mfm_store::v1::ExecutionClaimScope,
    run_id: &RunId,
    token: AdmissionToken,
) -> AdmissionLease {
    let admitted = store
        .acquire_execution_claim(scope, run_id, token)
        .await
        .expect("acquire execution claim");
    let NowaitSkipAdmissionResult::Admitted(lease) = admitted else {
        panic!("execution claim should be admitted");
    };
    lease
}

fn prepared_artifact_bytes_from_bytes(bytes: Vec<u8>, role: ArtifactRole) -> PreparedArtifactBytes {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let mut evidence = store_artifact_ref(artifact_id, digest, role);
    evidence.byte_len = bytes.len() as u64;
    PreparedArtifactBytes::new(bytes, evidence).expect("prepared artifact bytes")
}

fn admission_token(value: &str) -> AdmissionToken {
    AdmissionToken::new(value).expect("admission token")
}

fn execution_claim_scope(byte: u8) -> mfm_store::v1::ExecutionClaimScope {
    let identity = run_identity_material_for_test(spec_hash(byte), &store_scope_hex(byte));
    mfm_store::v1::ExecutionClaimScope::from_run_identity_material(&identity)
}

async fn expire_execution_claim_row(store: &PostgresRunStore, run_id: &RunId) {
    sqlx::query(
        "UPDATE admission_lane \
         SET lease_expires_at = statement_timestamp() - make_interval(secs => 1), \
             updated_at = statement_timestamp() \
         WHERE class = 'execution_claim' AND execution_run_id = $1",
    )
    .bind(run_id.as_str())
    .execute(&store.pool)
    .await
    .expect("expire execution claim row");
}

fn run_admitted_with_saga_policy(
    run_id: RunId,
    saga_policy: &SagaPolicySpec,
) -> KernelEventPayload {
    let authority_spec = saga_authority_spec(saga_policy.clone());
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("saga authority spec hash");
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    let identity_material = run_identity_material_for_run_id(&run_id, saga_policy);
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: entry_point_launch_evidence(),
        spec_hash: certified_spec_hash,
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        saga_policy_digest: saga_policy
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(9),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        seed_cells: Vec::new(),
    }))
}

fn run_admitted_with_fact_descriptor(run_id: RunId) -> KernelEventPayload {
    let authority_spec = fact_authority_spec();
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("fact authority spec hash");
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    let fact_descriptor_artifact = fact_descriptor_artifact_ref();
    let identity_material = run_identity_material_for_fact_run_id(&run_id);
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: entry_point_launch_evidence(),
        spec_hash: certified_spec_hash,
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: vec![run_artifact_ref(&fact_descriptor_artifact)],
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        saga_policy_digest: SagaPolicySpec::NoSideEffects
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(9),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        seed_cells: Vec::new(),
    }))
}

fn entry_point_launch_evidence() -> events::EntryPointLaunchEvidence {
    events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new("mfm.test:portfolio_snapshot:1")
            .expect("entry-point op id"),
        entry_point_registry_digest: content_digest(30),
    }
}

fn state_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        attempt_no: 1,
        state_kind: state_kind(12),
        state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
    })
}

fn state_attempt_completed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        output_cell_id: cell_id(21),
    })
}

fn state_attempt_interrupted() -> KernelEventPayload {
    KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
    })
}

fn cell_produced(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    let evidence = store_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        ArtifactRole::StateOutput,
    );
    KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        cell_id: cell_id(21),
        scope_id: scope_id(22),
        attempt_id: attempt_id(23),
        semantic_type_id: semantic_id("position", 24),
        schema_id: schema_id("mfm.test.position", 25),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(26),
        },
        context: spec::CellContextSpec::no_context(),
        artifact_id,
        content_digest: digest,
        evidence_hash: evidence.evidence_hash().expect("cell evidence hash"),
        producer_state_kind: None,
        producer_state_version: None,
    })
}

fn fact_key() -> mfm_facts::FactKey {
    fact_subject_evidence().fact_key().clone()
}

fn fact_descriptor() -> mfm_facts::FactDescriptor {
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new("mfm.test.fact").expect("fact kind"),
        mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
        schema_id("mfm.test.fact_subject", 37),
        schema_id("mfm.test.fact_response", 36),
        vec![
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("subject.chain").expect("field id"),
                mfm_facts::FactFieldValueType::String,
                mfm_facts::FactFieldExtraction::Subject(
                    mfm_facts::CanonicalValuePath::new("chain").expect("path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .required(),
            )
            .expect("subject field"),
            mfm_facts::FactFieldDescriptor::new(
                mfm_facts::FactFieldId::new("result.height").expect("field id"),
                mfm_facts::FactFieldValueType::UnsignedInteger,
                mfm_facts::FactFieldExtraction::Response(
                    mfm_facts::CanonicalValuePath::new("height").expect("path"),
                ),
                mfm_facts::FactFieldPolicy::new(
                    vec![
                        mfm_facts::FactQueryOperator::Equal,
                        mfm_facts::FactQueryOperator::GreaterThanOrEqual,
                    ],
                    mfm_facts::FactFieldExposure::Returnable,
                )
                .sortable()
                .required(),
            )
            .expect("response field"),
        ],
        vec![mfm_facts::FactOrderingPolicy::new(
            mfm_facts::FactOrderingName::new("result.height.desc").expect("ordering"),
            vec![mfm_facts::FactOrderingTerm::new(
                mfm_facts::FactFieldId::new("result.height").expect("field id"),
                mfm_facts::SortDirection::Descending,
                mfm_facts::NullOrdering::Last,
                false,
            )],
        )
        .expect("height ordering")],
    )
    .expect("fact descriptor")
}

fn fact_descriptor_hash() -> ContentDigest {
    fact_descriptor_fixture().descriptor_hash
}

fn fact_descriptor_bytes() -> Vec<u8> {
    fact_descriptor_fixture().descriptor_bytes
}

fn fact_descriptor_artifact_ref() -> ArtifactEvidenceRef {
    fact_descriptor_fixture().descriptor_evidence
}

fn fact_descriptor_fixture() -> FactDescriptorProjectionFixtureForTest {
    fact_descriptor_projection_fixture_for_test(fact_descriptor()).expect("descriptor fixture")
}

fn fact_subject_evidence() -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.chain").expect("field"),
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::string("postgres_test_chain"),
    )
    .expect("subject value")])
    .expect("subject material");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(&fact_descriptor()).expect("subject namespace hash");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

fn fact_response_bytes() -> Vec<u8> {
    fact_response_bytes_with_height(12_345)
}

fn fact_response_bytes_with_height(height: u64) -> Vec<u8> {
    PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"height":{height}}}"#))
        .expect("response bytes")
        .to_vec()
}

fn fact_artifact_ref() -> ArtifactEvidenceRef {
    fact_artifact_ref_for_response_bytes(&fact_response_bytes())
}

fn fact_artifact_ref_with_height(height: u64) -> ArtifactEvidenceRef {
    fact_artifact_ref_for_response_bytes(&fact_response_bytes_with_height(height))
}

fn fact_artifact_ref_for_response_bytes(bytes: &[u8]) -> ArtifactEvidenceRef {
    let digest = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .expect("canonical response bytes")
        .content_digest();
    ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.fact_response", 36)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(30)),
        producer_seed_id: None,
        artifact_role: ArtifactRole::FactResponse,
    }
}

fn fact_query_plan() -> mfm_facts::CanonicalFactQueryPlan {
    fact_query_plan_with_limit(Some(1))
}

fn fact_query_plan_with_limit(limit: Option<u64>) -> mfm_facts::CanonicalFactQueryPlan {
    let input = mfm_facts::FactQueryInput::new(
        mfm_facts::StoreScopeRef::new("default").expect("store scope"),
        mfm_facts::FactQueryScope::new(
            mfm_facts::FactAudience::Platform,
            mfm_facts::FactVisibilityScope::Default,
        ),
        mfm_facts::ScopeDecisionEvidence::new(content_digest(180)),
        vec![
            mfm_facts::FactQueryPredicate::new(
                mfm_facts::FactFieldId::new("subject.chain").expect("field"),
                mfm_facts::FactQueryOperator::Equal,
                mfm_facts::FactCanonicalScalar::string("postgres_test_chain"),
            ),
            mfm_facts::FactQueryPredicate::new(
                mfm_facts::FactFieldId::new("result.height").expect("field"),
                mfm_facts::FactQueryOperator::GreaterThanOrEqual,
                mfm_facts::FactCanonicalScalar::UnsignedInteger(12_000),
            ),
        ],
        vec![
            mfm_facts::FactFieldId::new("subject.chain").expect("field"),
            mfm_facts::FactFieldId::new("result.height").expect("field"),
        ],
        mfm_facts::FactOrderingName::new("result.height.desc").expect("ordering"),
        limit,
    )
    .expect("fact query input");
    mfm_facts::compile_fact_query_plan(&fact_descriptor(), input).expect("fact query plan")
}

fn fact_claim_with_visibility(
    response: &ArtifactEvidenceRef,
    visibility: mfm_facts::FactVisibility,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility,
        fact_kind: mfm_facts::FactKind::new("mfm.test.fact").expect("fact kind"),
        fact_descriptor_hash: fact_descriptor_hash(),
        subject: fact_subject_evidence(),
        observed_at: Some("2026-01-02T03:04:05Z".to_owned()),
        request: Some(mfm_facts::FactRequestEvidence::new(
            schema_id("mfm.test.fact_request", 34),
            content_digest(35),
        )),
        response: mfm_facts::FactResponseEvidence::new(
            schema_id("mfm.test.fact_response", 36),
            response.digest.clone(),
            response.artifact_id.clone(),
            response.evidence_hash().expect("response evidence hash"),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.test",
                "fact",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(32),
            )
            .expect("capability kind"),
            CapabilityVersion::new("mfm.test.fact.v1").expect("capability version"),
            AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(33),
            )
            .expect("adapter kind"),
            AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        ),
    })
    .expect("fact claim")
}

fn fact_recorded_with_visibility(
    response: &ArtifactEvidenceRef,
    visibility: mfm_facts::FactVisibility,
) -> KernelEventPayload {
    KernelEventPayload::FactRecorded(events::FactRecorded {
        spec_hash: spec_hash(1),
        node_id: node_id(30),
        attempt_id: attempt_id(31),
        claim: fact_claim_with_visibility(response, visibility),
    })
}

fn fact_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(30),
        attempt_id: attempt_id(31),
        attempt_no: 1,
        state_kind: state_kind(30),
        state_version: StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
    })
}

fn run_completed(run_id: RunId, outcome: events::RunCompletionOutcome) -> KernelEventPayload {
    KernelEventPayload::RunCompleted(events::RunCompleted {
        run_id,
        spec_hash: spec_hash(1),
        outcome,
    })
}

fn side_effect_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key")
}

fn side_effect_pair_id() -> SideEffectPairId {
    side_effect_pair_id_for_contract(&default_side_effect_contract())
}

fn side_effect_pair_id_for_contract(contract: &spec::SideEffectContractSpec) -> SideEffectPairId {
    spec::side_effect_pair_id(&submit_node_id(), &side_effect_output_cell(), contract)
        .expect("certified side-effect pair id")
}

fn submit_node_id() -> NodeId {
    node_id(70)
}

fn submit_attempt_id() -> AttemptId {
    attempt_id(72)
}

fn verify_node_id() -> NodeId {
    node_id(170)
}

fn verify_attempt_id() -> AttemptId {
    attempt_id(172)
}

fn side_effect_output_cell() -> CellId {
    cell_id(78)
}

fn default_side_effect_contract() -> spec::SideEffectContractSpec {
    spec::SideEffectContractSpec {
        contract_digest: content_digest(77),
        resource_claim: spec::ResourceClaimSpec::ManualOnly,
        verification: spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    }
}

fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

fn side_effect_attempt_started() -> KernelEventPayload {
    side_effect_attempt_started_for(submit_node_id(), submit_attempt_id())
}

fn side_effect_verify_attempt_started() -> KernelEventPayload {
    side_effect_attempt_started_for(verify_node_id(), verify_attempt_id())
}

fn side_effect_attempt_started_for(node_id: NodeId, attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        attempt_no: 1,
        state_kind: state_kind(70),
        state_version: StateVersion::new("mfm.test.side_effect_state.v1").expect("state version"),
    })
}

fn side_effect_intent(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        schema_id("mfm.test.side_effect_intent", 70),
        ArtifactRole::SideEffectIntent,
    );
    KernelEventPayload::SideEffectIntentPersisted(events::side_effect::IntentPersisted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        scope_id: scope_id(71),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.test.side_effect_intent", 70),
        intent_hash: digest,
        intent_artifact_id: artifact_id,
        intent_artifact_evidence_hash: evidence.evidence_hash().expect("intent evidence hash"),
        idempotency_input_schema_id: schema_id("mfm.test.idempotency_input", 73),
        idempotency_input_hash: content_digest(74),
        idempotency_key: events::IdempotencyKeyRef::new("idem-key-1").expect("idempotency key"),
        capability_kind: CapabilityKind::new(
            "mfm.test",
            "side_effect",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(75),
        )
        .expect("capability kind"),
        capability_version: CapabilityVersion::new("mfm.test.side_effect.v1")
            .expect("capability version"),
        adapter_kind: AdapterKind::new(
            "mfm.test",
            "adapter",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(76),
        )
        .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
    })
}

fn side_effect_claim() -> KernelEventPayload {
    KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        invocation_epoch: 1,
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
    })
}

fn side_effect_prepared() -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationPrepared(events::side_effect::InvocationPrepared {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
        resource_key: None,
        prepared_artifact_id: None,
        prepared_hash: None,
        prepared_artifact_evidence_hash: None,
    })
}

fn resource_namespace() -> ResourceNamespace {
    ResourceNamespace::new("mfm.test.account_nonce").expect("resource namespace")
}

fn resource_key(value: &str, schema_byte: u8) -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace: resource_namespace(),
        key_schema_id: schema_id("mfm.test.resource_key", schema_byte),
        key: events::ResourceKey::new(value).expect("resource key"),
    }
}

fn resource_lane_key(value: &str) -> ResourceLaneKey {
    ResourceLaneKey::from_evidence(&resource_key(value, 201))
}

fn resource_lane_claim_intent(resource_key: events::ResourceKeyEvidence) -> KernelEventPayload {
    KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        resource_key,
        requirement_digest: content_digest(210),
        resolved_by_capability_impl: events::RunnerFactoryId::new("mfm.test.postgres.runner")
            .expect("runner factory"),
    })
}

fn side_effect_started() -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationStarted(events::side_effect::InvocationStarted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
    })
}

fn unknown_schema() -> SchemaId {
    schema_id("mfm.test.submission_unknown", 83)
}

fn submission_schema() -> SchemaId {
    schema_id("mfm.test.submission", 77)
}

fn side_effect_submission_unknown(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    KernelEventPayload::SideEffectSubmissionUnknown(events::side_effect::SubmissionUnknown {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        evidence_schema_id: unknown_schema(),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
        evidence_artifact_evidence_hash: evidence.evidence_hash().expect("unknown evidence hash"),
    })
}

fn side_effect_submission_observed(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    KernelEventPayload::SideEffectSubmissionObserved(events::side_effect::SubmissionObserved {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        submission_schema_id: submission_schema(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
        submission_artifact_evidence_hash: evidence
            .evidence_hash()
            .expect("submission evidence hash"),
    })
}

fn side_effect_ambiguous(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        schema_id("mfm.test.ambiguity", 84),
        ArtifactRole::AmbiguityEvidence,
    );
    KernelEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
        spec_hash: spec_hash(1),
        node_id: verify_node_id(),
        attempt_id: verify_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        ambiguity_code: events::AmbiguityCode::new("ambiguous").expect("ambiguity code"),
        evidence_schema_id: schema_id("mfm.test.ambiguity", 84),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
        evidence_artifact_evidence_hash: evidence.evidence_hash().expect("ambiguity evidence hash"),
    })
}

fn side_effect_attempt_failed() -> KernelEventPayload {
    side_effect_attempt_failed_for(verify_node_id(), verify_attempt_id())
}

fn side_effect_submit_boundary_output_skipped() -> KernelEventPayload {
    KernelEventPayload::CellSkipped(events::CellSkipped {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        cell_id: side_effect_output_cell(),
        scope_id: scope_id(71),
        attempt_id: submit_attempt_id(),
        semantic_type_id: semantic_id("side_effect_output", 98),
        schema_id: schema_id("mfm.test.side_effect_output", 97),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(99),
        },
        context: spec::CellContextSpec::no_context(),
        skip_reason: events::SkipReason {
            code: events::ErrorCode::new("side_effect_submission_boundary")
                .expect("skip reason code"),
            safe_message: "side-effect submit boundary recorded; verification is delegated to the paired verify node".to_owned(),
        },
    })
}

fn side_effect_submit_attempt_completed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        output_cell_id: side_effect_output_cell(),
    })
}

fn side_effect_attempt_failed_for(node_id: NodeId, attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        retryable: false,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("side_effect_ambiguous").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable: false,
            safe_message: "side-effect outcome is ambiguous".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

fn side_effect_failed() -> KernelEventPayload {
    side_effect_failed_for(verify_node_id(), verify_attempt_id())
}

fn side_effect_failed_for(node_id: NodeId, attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::SideEffectFailed(events::side_effect::Failed {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
        retryable: false,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable: false,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

fn store_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
) -> ArtifactEvidenceRef {
    let (schema_id, semantic_type_id, producer_node_id) = match role {
        ArtifactRole::StateOutput => (
            Some(schema_id("mfm.test.position", 25)),
            Some(semantic_id("position", 24)),
            Some(node_id(20)),
        ),
        ArtifactRole::FactResponse => (
            Some(schema_id("mfm.test.fact_response", 36)),
            None,
            Some(node_id(30)),
        ),
        _ => (None, None, None),
    };
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id,
        semantic_type_id,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn retention_refs_appended(
    run_id: RunId,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
) -> KernelEventPayload {
    retention_refs_appended_with_reason(
        run_id,
        artifact_id,
        digest,
        role,
        events::RetentionReason::RuntimeEvidence,
    )
}

fn retention_refs_appended_with_reason(
    run_id: RunId,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
    reason: events::RetentionReason,
) -> KernelEventPayload {
    let evidence_hash = store_artifact_ref(artifact_id.clone(), digest.clone(), role)
        .evidence_hash()
        .expect("retention evidence hash");
    KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
        run_id,
        spec_hash: spec_hash(1),
        refs: vec![events::RetentionRef {
            artifact_id,
            role,
            evidence_hash,
            content_digest: digest,
        }],
        reason,
    })
}

fn retention_refs_appended_for_evidence(
    run_id: RunId,
    evidence: &ArtifactEvidenceRef,
    reason: events::RetentionReason,
) -> KernelEventPayload {
    KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
        run_id,
        spec_hash: spec_hash(1),
        refs: vec![events::RetentionRef {
            artifact_id: evidence.artifact_id.clone(),
            role: evidence.artifact_role,
            evidence_hash: evidence.evidence_hash().expect("retention evidence hash"),
            content_digest: evidence.digest.clone(),
        }],
        reason,
    })
}

fn side_effect_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    role: ArtifactRole,
) -> ArtifactEvidenceRef {
    let producer_node_id = match role {
        ArtifactRole::Receipt | ArtifactRole::Confirmation | ArtifactRole::AmbiguityEvidence => {
            verify_node_id()
        }
        _ => submit_node_id(),
    };
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id),
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn spec_artifact_ref() -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(2),
        digest: content_digest(2),
        byte_len: 128,
        media_type: media_type("application/vnd.mfm.typed-execution-spec+json;version=1"),
        schema_id: Some(spec::typed_execution_spec_schema_id().expect("typed spec schema")),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedExecutionSpec,
    }
}

fn certificate_artifact_ref() -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(4),
        digest: content_digest(4),
        byte_len: 128,
        media_type: media_type("application/vnd.mfm.typed-spec-certificate+json;version=1"),
        schema_id: Some(
            mfm_certify::typed_spec_certificate_schema_id().expect("typed certificate schema"),
        ),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedSpecCertificate,
    }
}

fn request(
    run_id: RunId,
    seq: u64,
    key: &str,
    payloads: Vec<KernelEventPayload>,
) -> mfm_store::v1::CommitRequest {
    mfm_store::v1::CommitRequest::from_payloads(
        run_id,
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(key).expect("commit key"),
        payloads,
        Vec::new(),
        CommitPreconditions::default(),
    )
    .expect("typed commit request")
}

fn certified_request(
    run_id: RunId,
    seq: u64,
    key: &str,
    payloads: Vec<KernelEventPayload>,
) -> mfm_store::v1::CommitRequest {
    certified_request_with_saga_policy(run_id, seq, key, payloads, SagaPolicySpec::NoSideEffects)
}

fn certified_request_with_saga_policy(
    run_id: RunId,
    seq: u64,
    key: &str,
    mut payloads: Vec<KernelEventPayload>,
    saga_policy: SagaPolicySpec,
) -> mfm_store::v1::CommitRequest {
    let preconditions = certify_payloads_for_policy(&run_id, saga_policy, &mut payloads);
    mfm_store::v1::CommitRequest::from_payloads(
        run_id,
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(key).expect("commit key"),
        payloads,
        Vec::new(),
        preconditions,
    )
    .expect("typed certified commit request")
}

fn certified_fact_request(
    run_id: RunId,
    seq: u64,
    key: &str,
    mut payloads: Vec<KernelEventPayload>,
) -> mfm_store::v1::CommitRequest {
    let preconditions = certify_payloads_for_fact_spec(&run_id, &mut payloads);
    mfm_store::v1::CommitRequest::from_payloads(
        run_id,
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(key).expect("commit key"),
        payloads,
        Vec::new(),
        preconditions,
    )
    .expect("typed certified fact commit request")
}

fn certify_payloads_for_policy(
    run_id: &RunId,
    saga_policy: SagaPolicySpec,
    payloads: &mut [KernelEventPayload],
) -> CommitPreconditions {
    let authority_spec = saga_authority_spec(saga_policy);
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("saga authority spec hash");
    for payload in payloads {
        set_payload_spec_hash(payload, &certified_spec_hash);
    }
    CommitPreconditions {
        certified_run_authority: Some(
            CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                .expect("certified run authority"),
        ),
        ..CommitPreconditions::default()
    }
}

fn certify_payloads_for_fact_spec(
    run_id: &RunId,
    payloads: &mut [KernelEventPayload],
) -> CommitPreconditions {
    let authority_spec = fact_authority_spec();
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("fact authority spec hash");
    for payload in payloads {
        set_payload_spec_hash(payload, &certified_spec_hash);
    }
    CommitPreconditions {
        certified_run_authority: Some(
            CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                .expect("certified run authority"),
        ),
        ..CommitPreconditions::default()
    }
}

fn set_payload_spec_hash(payload: &mut KernelEventPayload, spec_hash: &SpecHash) {
    match payload {
        KernelEventPayload::RunAdmitted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::StateAttemptStarted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::FactRecorded(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ArtifactReferenced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::CellProduced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::CellSkipped(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectClaimed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::ResourceLaneClaimed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneClaimIntent(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectInvocationStarted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectReceiptObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectAmbiguous(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectFailed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneReleased(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneReleaseIntent(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::PublicOutputProduced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::StateAttemptCompleted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::StateAttemptFailed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::RunCompleted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::RetentionRefsAppended(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::RetentionManifestProjected(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
    }
}

fn run_start_request(run_id: RunId, key: &str) -> mfm_store::v1::CommitRequest {
    run_start_request_with_saga_policy(run_id, key, &SagaPolicySpec::NoSideEffects)
}

fn run_start_request_with_saga_policy(
    run_id: RunId,
    key: &str,
    saga_policy: &SagaPolicySpec,
) -> mfm_store::v1::CommitRequest {
    let authority_spec = saga_authority_spec(saga_policy.clone());
    mfm_store::v1::CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(key).expect("commit key"),
        vec![run_admitted_with_saga_policy(run_id.clone(), saga_policy)],
        vec![spec_artifact_ref(), certificate_artifact_ref()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                    .expect("certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    )
    .expect("typed run start request")
}

fn fact_run_start_request(run_id: RunId, key: &str) -> mfm_store::v1::CommitRequest {
    let authority_spec = fact_authority_spec();
    mfm_store::v1::CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(key).expect("commit key"),
        vec![run_admitted_with_fact_descriptor(run_id.clone())],
        vec![
            spec_artifact_ref(),
            certificate_artifact_ref(),
            fact_descriptor_artifact_ref(),
        ],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                    .expect("certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    )
    .expect("typed fact run start request")
}

async fn append_prepared(
    store: &PostgresRunStore,
    mut request: mfm_store::v1::CommitRequest,
    artifacts: Vec<ArtifactEvidenceRef>,
) -> Result<CommitOutcome> {
    if request.required_artifacts().is_empty() && !artifacts.is_empty() {
        request = request.with_required_artifacts(artifacts.clone());
    }
    let plan = test_prepared_commit_plan(request, artifacts)?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle(plan)?)
        .await
}

fn retention_artifact_bundle(
    run_id: RunId,
    seq: u64,
    commit_key: &str,
    artifact: PreparedArtifactBytes,
    preconditions: CommitPreconditions,
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    let evidence = artifact.evidence().clone();
    let request = mfm_store::v1::CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(commit_key).expect("commit key"),
        vec![retention_refs_appended_for_evidence(
            run_id,
            &evidence,
            events::RetentionReason::RuntimeEvidence,
        )],
        vec![evidence.clone()],
        preconditions,
    )?;
    let artifact_set =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), vec![evidence])?;
    let plan: PreparedCommitPlan = PreparedCommit::<Retention>::new(request, artifact_set)?.into();
    PreparedCommitBundle::new(plan, vec![artifact], Vec::new())
}

fn test_prepared_commit_bundle(
    plan: PreparedCommitPlan,
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    let artifact_bytes = plan
        .admitted_artifacts()
        .iter()
        .map(test_prepared_artifact_bytes)
        .collect::<mfm_store::v1::Result<Vec<_>>>()?;
    PreparedCommitBundle::new(plan, artifact_bytes, Vec::new())
}

fn test_prepared_commit_bundle_with_artifact_bytes(
    plan: PreparedCommitPlan,
    exact_artifacts: Vec<PreparedArtifactBytes>,
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    let mut artifact_bytes = Vec::new();
    for evidence in plan.admitted_artifacts() {
        if let Some(exact) = exact_artifacts
            .iter()
            .find(|artifact| artifact.evidence() == evidence)
        {
            artifact_bytes.push(exact.clone());
        } else {
            artifact_bytes.push(test_prepared_artifact_bytes(evidence)?);
        }
    }
    PreparedCommitBundle::new(plan, artifact_bytes, Vec::new())
}

fn test_prepared_commit_bundle_with_existing_artifact(
    plan: PreparedCommitPlan,
    evidence: &ArtifactEvidenceRef,
) -> mfm_store::v1::Result<PreparedCommitBundle> {
    let evidence_hash = evidence.evidence_hash()?;
    PreparedCommitBundle::new(
        plan,
        Vec::new(),
        vec![ExistingArtifactAdmission::new(
            evidence.artifact_id.clone(),
            evidence_hash,
        )],
    )
}

async fn append_fact_run_start(store: &PostgresRunStore, run_id: RunId) -> Result<CommitOutcome> {
    let descriptor_ref = fact_descriptor_artifact_ref();
    let descriptor_bytes =
        PreparedArtifactBytes::new(fact_descriptor_bytes(), descriptor_ref.clone())?;
    let request = fact_run_start_request(run_id, "run-start");
    let plan = test_prepared_commit_plan(
        request,
        vec![
            spec_artifact_ref(),
            certificate_artifact_ref(),
            descriptor_ref,
        ],
    )?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle_with_artifact_bytes(
            plan,
            vec![descriptor_bytes],
        )?)
        .await
}

async fn append_fact_attempt_start(
    store: &PostgresRunStore,
    run: RunId,
    commit_key: &str,
) -> Result<CommitOutcome> {
    append_prepared(
        store,
        certified_fact_request(run, 2, commit_key, vec![fact_attempt_started()]),
        Vec::new(),
    )
    .await
}

fn fact_commit_request(
    run_id: RunId,
    seq: u64,
    key: &str,
    response: &ArtifactEvidenceRef,
) -> mfm_store::v1::CommitRequest {
    fact_commit_request_with_visibility(
        run_id,
        seq,
        key,
        response,
        mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
    )
}

fn fact_commit_request_with_visibility(
    run_id: RunId,
    seq: u64,
    key: &str,
    response: &ArtifactEvidenceRef,
    visibility: mfm_facts::FactVisibility,
) -> mfm_store::v1::CommitRequest {
    let request = certified_fact_request(
        run_id,
        seq,
        key,
        vec![fact_recorded_with_visibility(response, visibility)],
    );
    let mut preconditions = request.preconditions().clone();
    preconditions.required_run_state = RequiredRunState::Started;
    request
        .with_preconditions(preconditions)
        .with_required_artifacts(vec![response.clone()])
}

async fn append_fact_commit(
    store: &PostgresRunStore,
    request: mfm_store::v1::CommitRequest,
    response: &ArtifactEvidenceRef,
) -> Result<CommitOutcome> {
    append_fact_commit_with_response_bytes(store, request, response, fact_response_bytes()).await
}

async fn append_fact_commit_with_response_bytes(
    store: &PostgresRunStore,
    request: mfm_store::v1::CommitRequest,
    response: &ArtifactEvidenceRef,
    response_bytes: Vec<u8>,
) -> Result<CommitOutcome> {
    let response_bytes = PreparedArtifactBytes::new(response_bytes, response.clone())?;
    let plan = test_prepared_commit_plan(request, vec![response.clone()])?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle_with_artifact_bytes(
            plan,
            vec![response_bytes],
        )?)
        .await
}

async fn assert_empty_fact_query(
    store: &PostgresRunStore,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) {
    let query_result = store
        .execute_fact_query(plan)
        .await
        .expect("empty fact query execution");
    assert!(query_result.rows().is_empty());
    assert_fact_query_receipt(
        store,
        plan,
        &query_result,
        mfm_facts::QueryResultCardinality::Exact(0),
    );
}

async fn assert_single_public_fact_query(
    store: &PostgresRunStore,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> mfm_facts::FactQueryResult {
    let query_result = store
        .execute_fact_query(plan)
        .await
        .expect("fact query execution");
    assert_eq!(query_result.rows().len(), 1);
    let row = &query_result.rows()[0];
    assert_eq!(row.fact_ref().fact_key(), &fact_key());
    assert_eq!(row.returned_fields().len(), 2);
    assert_eq!(
        row.returned_fields()[0].field_id().as_str(),
        "subject.chain"
    );
    assert_eq!(
        row.returned_fields()[0].value(),
        &mfm_facts::FactCanonicalScalar::string("postgres_test_chain")
    );
    assert_eq!(
        row.returned_fields()[1].field_id().as_str(),
        "result.height"
    );
    assert_eq!(
        row.returned_fields()[1].value(),
        &mfm_facts::FactCanonicalScalar::UnsignedInteger(12_345)
    );
    query_result
}

async fn append_fact_commit_with_missing_existing_artifact(
    store: &PostgresRunStore,
    request: mfm_store::v1::CommitRequest,
    response: &ArtifactEvidenceRef,
) -> Result<CommitOutcome> {
    let plan = test_prepared_commit_plan(request, vec![response.clone()])?;
    store
        .append_prepared_commit_bundle(test_prepared_commit_bundle_with_existing_artifact(
            plan, response,
        )?)
        .await
}

async fn append_run_start(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> Result<CommitOutcome> {
    append_prepared(
        store,
        run_start_request(run_id.clone(), commit_key),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
}

async fn append_resource_lane_attempt_start(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> Result<CommitOutcome> {
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            2,
            commit_key,
            vec![side_effect_attempt_started()],
        ),
        Vec::new(),
    )
    .await
}

async fn append_resource_lane_prepare(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
    lane_value: &str,
    artifact_byte: u8,
) -> Result<CommitOutcome> {
    let intent_artifact = artifact_id(artifact_byte);
    let intent_digest = content_digest(artifact_byte);
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            3,
            commit_key,
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                resource_lane_claim_intent(resource_key(lane_value, 201)),
            ],
        ),
        vec![side_effect_artifact_ref(
            intent_artifact,
            intent_digest,
            schema_id("mfm.test.side_effect_intent", 70),
            ArtifactRole::SideEffectIntent,
        )],
    )
    .await
}

async fn append_resource_lane_release(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
    lane_value: &str,
) -> Result<CommitOutcome> {
    let projection = store
        .status_projection_snapshot(run_id)
        .await
        .expect("resource lane projection");
    let lane_key = resource_lane_key(lane_value);
    let lane = projection
        .resource_lane(&lane_key)
        .expect("active resource lane");
    let next_seq = store
        .expected_next_seq(run_id)
        .await
        .expect("release next seq")
        .as_u64();
    let verify_start_key = format!("{commit_key}-verify-attempt-start");
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            next_seq,
            verify_start_key.as_str(),
            vec![side_effect_verify_attempt_started()],
        ),
        Vec::new(),
    )
    .await?;
    let next_seq = store
        .expected_next_seq(run_id)
        .await
        .expect("release next seq after verify start")
        .as_u64();
    append_prepared(
        store,
        certified_request(
            run_id.clone(),
            next_seq,
            commit_key,
            vec![
                KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
                    spec_hash: spec_hash(1),
                    ledger_key: side_effect_ledger_key(),
                    ledger_purpose: lane.ledger_purpose.clone(),
                    pair_id: lane.holder.pair_id.clone(),
                    pair_role: events::SideEffectPairRole::Verify,
                    invocation_epoch: lane.invocation_epoch,
                    claim_id: lane.claim_id.clone(),
                    release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
                    release_reason: events::ResourceLaneReleaseReason::new("mfm.test.release")
                        .expect("release reason"),
                }),
                side_effect_failed(),
                side_effect_attempt_failed(),
            ],
        ),
        Vec::new(),
    )
    .await
}

fn assert_resource_lane_blocked(
    outcome: CommitOutcome,
    expected_lane_key: &ResourceLaneKey,
    expected_holder_run: &RunId,
) {
    let CommitOutcome::AdmissionBlocked(block) = outcome else {
        panic!("expected typed resource lane block, got {outcome:?}");
    };
    assert_eq!(&block.resource_lane_key, expected_lane_key);
    let holder = block.holder.as_ref().expect("blocked holder");
    assert_eq!(&holder.run_id, expected_holder_run);
    assert_eq!(holder.pair_id, side_effect_pair_id());
}

fn assert_wait_fifo_admission_blocked(
    outcome: CommitOutcome,
    expected_lane_key: &ResourceLaneKey,
    expected_holder_run: Option<&RunId>,
) -> AdmissionWaiter {
    let CommitOutcome::AdmissionBlocked(block) = outcome else {
        panic!("expected typed resource lane block, got {outcome:?}");
    };
    assert_eq!(&block.resource_lane_key, expected_lane_key);
    match (block.holder.as_ref(), expected_holder_run) {
        (Some(holder), Some(expected_holder_run)) => {
            assert_eq!(&holder.run_id, expected_holder_run);
            assert_eq!(holder.pair_id, side_effect_pair_id());
        }
        (None, None) => {}
        (actual, expected) => {
            panic!("unexpected block holder {actual:?}, expected {expected:?}")
        }
    }
    block.waiter.expect("wait-fifo admission waiter block")
}

fn assert_corruption(error: PostgresStoreError, expected: &str) {
    let PostgresStoreError::Corruption(message) = error else {
        panic!("expected corruption error, got {error:?}");
    };
    assert!(
        message.contains(expected),
        "expected corruption containing {expected:?}, got {message:?}"
    );
}

fn assert_authority_error(error: PostgresStoreError, expected: PostgresStoreAuthorityError) {
    let PostgresStoreError::Authority(actual) = error else {
        panic!("expected store authority error, got {error:?}");
    };
    assert_eq!(actual, expected);
}

fn assert_invalid_cursor(error: PostgresStoreError, expected: &str) {
    let PostgresStoreError::Store(StoreError::InvalidCursor { message }) = error else {
        panic!("expected invalid cursor error, got {error:?}");
    };
    assert!(
        message.contains(expected),
        "expected invalid cursor containing {expected:?}, got {message:?}"
    );
}

fn assert_cursor_expired(error: PostgresStoreError) {
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::CursorExpired)
    ));
}

async fn disable_observation_cursor_mutation_guard(pool: &PgPool) {
    sqlx::query(
        "ALTER TABLE run_observation_cursors DISABLE TRIGGER \
         run_observation_cursors_no_update",
    )
    .execute(pool)
    .await
    .expect("disable cursor mutation guard");
}

async fn observation_row_cursor_for_commit(
    store: &PostgresRunStore,
    run: &RunId,
    seq: u64,
) -> String {
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let row = sqlx::query("SELECT store_commit_order FROM commits WHERE run_id = $1 AND seq = $2")
        .bind(run.as_str())
        .bind(i64::try_from(seq).expect("seq fits i64"))
        .fetch_one(&store.pool)
        .await
        .expect("load commit cursor position");
    encode_observation_cursor(
        &store.pool,
        &metadata,
        &CursorPosition {
            store_commit_order: u64::try_from(
                row.try_get::<i64, _>("store_commit_order")
                    .expect("store commit order"),
            )
            .expect("positive store commit order"),
        },
    )
    .await
    .expect("encode row cursor")
}

async fn append_retention_commit(
    store: &PostgresRunStore,
    run: &RunId,
    seq: u64,
    commit_key: &str,
    artifact_byte: u8,
) -> Result<CommitOutcome> {
    let artifact = artifact_id(artifact_byte);
    let digest = content_digest(artifact_byte);
    let evidence = store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(commit_key).expect("commit key"),
        vec![retention_refs_appended(
            run.clone(),
            artifact,
            digest,
            ArtifactRole::StateOutput,
        )],
        vec![evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("retention request");
    append_prepared(store, request, vec![evidence]).await
}

#[path = "artifacts.rs"]
mod artifact_tests;
#[path = "facts_retention.rs"]
mod facts_retention_tests;
#[path = "lifecycle.rs"]
mod lifecycle_tests;
#[path = "observation.rs"]
mod observation_tests;
#[path = "saga.rs"]
mod saga_tests;
