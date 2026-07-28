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
    expected_next_sequence_for_test as expected_next_sequence,
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
    PreparedCommitPlan, RequiredRunState, ResourceLaneKey, Retention, RunJournalStore, RunState,
    SagaEngagementReason, SagaTerminal, SideEffectPhase, StoreError, StoreScopeId, StoreScopeStore,
    StreamSeq,
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

#[path = "fact_support.rs"]
mod fact_support;
use self::fact_support::{
    fact_artifact_ref, fact_artifact_ref_with_height, fact_attempt_completed, fact_attempt_started,
    fact_cell_produced, fact_descriptor_artifact_ref, fact_descriptor_bytes, fact_descriptor_hash,
    fact_key, fact_output_artifact_ref, fact_query_plan, fact_query_plan_with_limit, fact_recorded,
    fact_response_bytes, fact_response_bytes_with_height,
};

#[path = "side_effect_support.rs"]
mod side_effect_support;
use self::side_effect_support::{
    default_side_effect_contract, prepared_artifact_ref, resource_key, resource_lane_claim_intent,
    resource_lane_key, side_effect_ambiguous, side_effect_attempt_failed,
    side_effect_attempt_started, side_effect_claim, side_effect_failed, side_effect_intent,
    side_effect_ledger_key, side_effect_output_cell, side_effect_pair_id, side_effect_prepared,
    side_effect_started, side_effect_submission_observed, side_effect_submission_unknown,
    side_effect_submit_attempt_completed, side_effect_submit_boundary_output_skipped,
    side_effect_verify_attempt_started, submission_schema, submit_node_id, unknown_schema,
    verify_node_id,
};

#[path = "authority_support.rs"]
mod authority_support;
use self::authority_support::{
    fact_authority_spec, fact_run_id, run_id, run_id_with_saga_policy,
    run_identity_material_for_fact_run_id, run_identity_material_for_run_id, saga_authority_spec,
    store_scope_hex,
};

#[path = "authority_tests.rs"]
mod authority_tests;
#[path = "event_support.rs"]
mod event_support;
#[path = "execution_claim_tests.rs"]
mod execution_claim_tests;
use self::event_support::*;
#[path = "commit_support.rs"]
mod commit_support;
use self::commit_support::*;
#[path = "resource_support.rs"]
mod resource_support;
use self::resource_support::*;

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

pub(super) async fn test_store() -> (PostgresStore, String) {
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
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect schema-scoped postgres");
    crate::schema::migrate_pool(&pool)
        .await
        .expect("migrate schema");
    let authority = crate::schema::validate_pool(&pool)
        .await
        .expect("validate schema");
    let store = PostgresStore {
        pool,
        authority,
        committed_journal_load_test_barrier: None,
    };
    (store, schema)
}

pub(super) async fn drop_schema(store: &PostgresStore, schema: &str) {
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
    query_entries: usize,
    terms: usize,
) {
    assert_eq!(projection.fact_descriptors().count(), descriptors);
    assert_eq!(projection.fact_query_entries().count(), query_entries);
    assert_eq!(
        projection
            .fact_query_entries()
            .map(|(_, projection)| projection.terms().count())
            .sum::<usize>(),
        terms
    );
}

async fn assert_fact_projection_table_counts(
    store: &PostgresStore,
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
            "SELECT COUNT(*) FROM fact_query_projection WHERE source_run_id = $1"
        )
        .await,
        index_entries as i64
    );
    assert_eq!(
        fact_projection_table_count(
            &store.pool,
            run,
            "SELECT COUNT(*) FROM fact_query_terms WHERE source_run_id = $1"
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

async fn global_fact_descriptor_catalog_count(store: &PostgresStore) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fact_descriptor_catalog")
        .fetch_one(&store.pool)
        .await
        .expect("global fact descriptor catalog count")
}

fn assert_fact_query_receipt(
    _store: &PostgresStore,
    _plan: &mfm_facts::CanonicalFactQueryPlan,
    result: &mfm_facts::FactQueryResult,
    expected_cardinality: mfm_facts::QueryResultCardinality,
) {
    assert_eq!(result.receipt().result_cardinality(), expected_cardinality);
}

#[path = "artifacts.rs"]
mod artifact_tests;
#[path = "facts_retention.rs"]
mod facts_retention_tests;
#[path = "journal_manual_authorization.rs"]
mod journal_manual_authorization_tests;
#[path = "journal.rs"]
mod journal_tests;
#[path = "lifecycle.rs"]
mod lifecycle_tests;
#[path = "observation.rs"]
mod observation_tests;
#[path = "saga.rs"]
mod saga_tests;
#[path = "tenant_fact_frontier_postgres_prototype.rs"]
mod tenant_fact_frontier_postgres_prototype_tests;
