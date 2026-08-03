use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_certify::structured::{
    AccessTargetSelection, ProgramRegistryBuilder, ReadPhysicalBindingKind, RuntimeProcessRegistry,
};
use mfm_facts::{
    CanonicalFactPredicate, FactOrdering, FactProposal, FactSelectionLimit, FactSelectionQuery,
    FactSelectionReadFailure, FactSelectionReadFailureCode, FactSelectionReadResponse,
    FactSelectionRequest, FactSelectionScanBounds, FactSet, FactTieBreak, ProposedFactValue,
};
use mfm_ids::{
    AppendRequestId, ContentRef, DigestAlgorithm, InvocationIdentity, RunId, SchemaId, StableId,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, CommittedBatch, HistoryObject, ObservationOutcome,
    PriorRunFactSelectionResponse, PriorRunFactSourceManifest, PriorRunFactSourceRule, RunRecord,
    StateOutcomeRef, TenantFactCoordinate, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_program::structured::{
    state_contract, CommittedObservation, Direct, Never, OperationBuilder,
    PriorRunFactSelectionCapability, Pure, Read, ReviewedSafeFailureCase, SafeFailureNotApplicable,
    SafeFailureSuccessOnly, State, StateFrame, StateSettlement, StructuredStateCallbacks,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::structured::{split_qualified_registry, DriveOutcome, Runtime};
use mfm_spec::structured::{
    ProposedStateOutcome, SecretFreeExecutableIdentity, SecretFreeImplementationDescriptor,
    SecretFreeQualificationArtifact, StructuredComponentKind, StructuredExpansionProfile,
    StructuredFactDescriptor,
};
use mfm_storage_postgres::{
    open_configuration_maintenance, open_structured_authoritative,
    open_structured_authoritative_with_configuration, AuthoritativeWriterContext,
    AuthoritativeWriterFence, AuthoritativeWriterFenceFuture, PostgresStoreError,
    PostgresStructuredHistoryBackend, TestAuthoritativeWriterFence,
};
use mfm_store::structured::{
    AccessAuthorizationProposal, BackendAppendOutcome, ConfigurationAppendRequest,
    ConfigurationRevision, ConfigurationStreamKey, PhysicalBindingAuthorization,
    PhysicalBindingSupersession, ProposedCanonicalValue, PublicPhysicalBindingVerifier,
    StateTransitionProposal, StructuredAdmissionMaterial, StructuredAdmissionRequest,
    StructuredFrontier, StructuredHistoryBackend, StructuredMemoryBackend,
    StructuredRunHistoryReader, StructuredRunHistoryWriter, StructuredRunStore,
    StructuredStoreError, StructuredStoreIdentity,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, ConnectOptions, PgPool, Row};

static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
const FRESH_PROCESS_SCHEMA_ENV: &str = "MFM_STRUCTURED_HISTORY_TEST_SCHEMA";
const FRESH_PROCESS_MODE_ENV: &str = "MFM_STRUCTURED_HISTORY_TEST_MODE";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.postgres.fixture",
    name = "value",
    version = "1",
    schema = "mfm.postgres.fixture.value"
)]
struct Value {
    value: u64,
}

struct CopyState;

impl State for CopyState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.postgres.fixture/copy-state")
    }
}

struct FactCopyState;

impl State for FactCopyState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.postgres.fixture/fact-copy-state")
    }

    fn fact_slots() -> mfm_program::Result<Vec<mfm_spec::CertifiedFactSlot>> {
        let contract = mfm_spec::structured::structured_value_contract::<Value>()?;
        let descriptor = fact_descriptor()?;
        Ok(vec![mfm_spec::CertifiedFactSlot::new(
            0,
            1,
            1,
            descriptor.descriptor_ref,
            contract.clone(),
            contract,
        )?])
    }
}

struct FactReadState;

impl State for FactReadState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = FactSelectionRequest;
    type Returned = FactSelectionReadResponse;
    type SafeFailure = FactSelectionReadFailure;
    type Execution = Read<PriorRunFactSelectionCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.postgres.fixture/fact-read-state")
    }
}

struct NoPhysicalBindings;

impl PublicPhysicalBindingVerifier for NoPhysicalBindings {
    fn verify_authorization(
        &self,
        _context: &PhysicalBindingAuthorization<'_>,
        _certificate: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Err(StructuredStoreError::Certification)
    }

    fn verify_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Err(StructuredStoreError::Certification)
    }
}

#[tokio::test]
async fn configured_value_history_is_durable_append_only_and_application_read_only() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured").expect("operation id");
    let (program_verifier, _) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let pool = database.combined_pool().await;
    let pool_control = pool.clone();
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        physical_verifier,
    )
    .await
    .expect("qualify structured stores");
    let (writer, reader) = configuration.split();
    let stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32))).expect("tenant"),
        operation_id,
        StableId::new("mfm.postgres.fixture/configured-target").expect("target"),
    );
    let contract = admission_object(
        ADMISSION_CONFIGURATION_OBJECT_TYPE,
        "mfm.postgres.fixture.configured-contract",
        31,
    )
    .content_ref;
    let first = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-first").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("value"),
        ))
        .await
        .expect("append first configuration revision");
    let second = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-second").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
        ))
        .await
        .expect("append second configuration revision");
    assert_eq!(
        reader
            .resolve(&stream, &contract)
            .await
            .expect("resolve current configuration")
            .revision(),
        &second
    );

    let audit_pool = database.independent_pool().await;
    let privilege_row = sqlx::query(
        "SELECT pg_catalog.has_table_privilege( \
             'mfm_store_application', \
             pg_catalog.format('%I.configuration_revisions', pg_catalog.current_schema()), \
             'INSERT' \
         ) AS application_insert, \
         pg_catalog.has_table_privilege( \
             'mfm_store_configuration_maintenance', \
             pg_catalog.format('%I.configuration_revisions', pg_catalog.current_schema()), \
             'INSERT' \
         ) AS maintenance_insert, \
         pg_catalog.has_table_privilege( \
             'mfm_store_application', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'SELECT' \
         ) AS application_head_select, \
         (pg_catalog.has_table_privilege( \
             'mfm_store_application', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'INSERT' \
         ) OR pg_catalog.has_table_privilege( \
             'mfm_store_application', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'UPDATE' \
         )) AS application_head_write, \
         (pg_catalog.has_table_privilege( \
             'mfm_store_configuration_maintenance', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'SELECT' \
         ) AND pg_catalog.has_table_privilege( \
             'mfm_store_configuration_maintenance', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'INSERT' \
         ) AND pg_catalog.has_table_privilege( \
             'mfm_store_configuration_maintenance', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'UPDATE' \
         )) AS maintenance_head_access, \
         (pg_catalog.has_table_privilege( \
             'mfm_store_configuration_maintenance', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'DELETE' \
         ) OR pg_catalog.has_table_privilege( \
             'mfm_store_configuration_maintenance', \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'TRUNCATE' \
         )) AS maintenance_head_destructive",
    )
    .fetch_one(&audit_pool)
    .await
    .expect("inspect configured-value privileges");
    assert!(!privilege_row
        .try_get::<bool, _>("application_insert")
        .expect("application privilege"));
    assert!(privilege_row
        .try_get::<bool, _>("maintenance_insert")
        .expect("maintenance privilege"));
    assert!(privilege_row
        .try_get::<bool, _>("application_head_select")
        .expect("application head select"));
    assert!(!privilege_row
        .try_get::<bool, _>("application_head_write")
        .expect("application head write"));
    assert!(privilege_row
        .try_get::<bool, _>("maintenance_head_access")
        .expect("maintenance head access"));
    assert!(!privilege_row
        .try_get::<bool, _>("maintenance_head_destructive")
        .expect("maintenance destructive head access"));

    audit_pool.close().await;
    drop(reader);
    drop(writer);
    drop(run_history);
    pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn maintenance_only_login_can_preflight_and_append_configuration() {
    let database = TestDatabase::create().await;
    let pool = database.maintenance_pool().await;
    let pool_control = pool.clone();
    let writer = open_configuration_maintenance(pool, TestAuthoritativeWriterFence)
        .await
        .expect("qualify exact maintenance-only login");
    let stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "8".repeat(32))).expect("tenant"),
        stable("mfm.postgres.fixture/maintenance-only").expect("operation id"),
        StableId::new("mfm.postgres.fixture/maintenance-only-target").expect("target"),
    );
    let contract = admission_object(
        ADMISSION_CONFIGURATION_OBJECT_TYPE,
        "mfm.postgres.fixture.maintenance-only-contract",
        38,
    )
    .content_ref;
    let revision = writer
        .append(ConfigurationAppendRequest::new(
            stream,
            None,
            AppendRequestId::new("postgres-maintenance-only-append").expect("append id"),
            contract,
            ProposedCanonicalValue::from_json(r#"{"maintenance":true}"#).expect("value"),
        ))
        .await
        .expect("maintenance-only preflight and append");
    assert_eq!(revision.sequence(), 1);

    drop(writer);
    pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn configured_value_history_linearizes_same_stream_append_races() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured-race").expect("operation id");
    let (program_verifier, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let pool = database.combined_pool().await;
    let pool_control = pool.clone();
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&program_verifier),
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("qualify structured stores");
    let (writer, reader) = configuration.split();
    let stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "3".repeat(32))).expect("tenant"),
        operation_id.clone(),
        StableId::new("mfm.postgres.fixture/configured-race-target").expect("target"),
    );
    let contract = admission_object(
        ADMISSION_CONFIGURATION_OBJECT_TYPE,
        "mfm.postgres.fixture.configured-race-contract",
        32,
    )
    .content_ref;
    let baseline = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-race-baseline").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("value"),
        ))
        .await
        .expect("append baseline configuration revision");

    let (left, right) = tokio::join!(
        writer.append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(baseline.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-race-left").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"winner":"left"}"#).expect("value"),
        )),
        writer.append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(baseline.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-race-right").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"winner":"right"}"#).expect("value"),
        )),
    );
    let winner = match (left, right) {
        (Ok(winner), Err(StructuredStoreError::StaleHead))
        | (Err(StructuredStoreError::StaleHead), Ok(winner)) => winner,
        (left, right) => panic!("different-id race did not linearize once: {left:?}, {right:?}"),
    };
    assert_eq!(
        reader
            .resolve(&stream, &contract)
            .await
            .expect("resolve different-id winner")
            .revision(),
        &winner
    );

    let same_id = "postgres-configured-race-existing-same";
    let (first_replay, second_replay) = tokio::join!(
        writer.append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(winner.revision_ref().clone()),
            AppendRequestId::new(same_id).expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":3}"#).expect("value"),
        )),
        writer.append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(winner.revision_ref().clone()),
            AppendRequestId::new(same_id).expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":3}"#).expect("value"),
        )),
    );
    let first_replay = first_replay.expect("first byte-identical append result");
    let second_replay = second_replay.expect("second byte-identical append result");
    assert_eq!(first_replay, second_replay);

    let conflicting_id = "postgres-configured-race-conflicting-same-id";
    let (left, right) = tokio::join!(
        writer.append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first_replay.revision_ref().clone()),
            AppendRequestId::new(conflicting_id).expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"conflict":"left"}"#).expect("value"),
        )),
        writer.append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first_replay.revision_ref().clone()),
            AppendRequestId::new(conflicting_id).expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"conflict":"right"}"#).expect("value"),
        )),
    );
    let conflict_winner = match (left, right) {
        (Ok(winner), Err(StructuredStoreError::AppendConflict))
        | (Err(StructuredStoreError::AppendConflict), Ok(winner)) => winner,
        (left, right) => {
            panic!("same-id conflict did not preserve one result: {left:?}, {right:?}")
        }
    };
    assert_eq!(
        reader
            .resolve(&stream, &contract)
            .await
            .expect("resolve same-id conflict winner")
            .revision(),
        &conflict_winner
    );

    let audit_pool = database.independent_pool().await;
    for append_request_id in [same_id, conflicting_id] {
        let count = sqlx::query(
            "SELECT pg_catalog.count(*)::bigint AS row_count \
             FROM configuration_revisions WHERE append_request_id = $1",
        )
        .bind(append_request_id)
        .fetch_one(&audit_pool)
        .await
        .expect("count append identity rows")
        .try_get::<i64, _>("row_count")
        .expect("append identity count");
        assert_eq!(count, 1, "append identity must occupy one durable row");
    }

    audit_pool.close().await;
    drop(reader);
    drop(writer);
    drop(run_history);
    pool_control.close().await;

    let reopened_pool = database.combined_pool().await;
    let reopened_pool_control = reopened_pool.clone();
    let (reopened_history, reopened_configuration) =
        open_structured_authoritative_with_configuration(
            reopened_pool,
            TestAuthoritativeWriterFence,
            program_verifier,
            physical_verifier,
        )
        .await
        .expect("reopen structured stores");
    let (reopened_writer, reopened_reader) = reopened_configuration.split();
    let reopened_winner = reopened_reader
        .resolve(&stream, &contract)
        .await
        .expect("resolve winning revision after reopen");
    assert_eq!(reopened_winner.revision(), &conflict_winner);

    let configuration = HistoryObject::new(
        stable(ADMISSION_CONFIGURATION_OBJECT_TYPE).expect("configuration object type"),
        SchemaId::new(
            "mfm.structured-configuration-revision",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(
                b"mfm.structured-schema.v1:mfm.structured-configuration-revision:1",
            ),
        )
        .expect("configuration revision schema"),
        canonical_json(reopened_winner.revision())
            .expect("canonical winning configuration revision")
            .as_str(),
    )
    .expect("winning configuration admission object");
    let configuration_ref = configuration.content_ref.clone();
    let material = StructuredAdmissionMaterial::new(
        configuration,
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "mfm.postgres.fixture.configured-race-context",
            33,
        ),
        PriorRunFactSourceManifest::new(Vec::new())
            .and_then(|manifest| manifest.to_history_object())
            .expect("configured-race source manifest"),
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "mfm.postgres.fixture.configured-race-routing",
            34,
        ),
        Vec::new(),
    )
    .expect("configured-race admission material");
    let (history_writer, history_reader) = reopened_history.split();
    let admitted_run_id = run_id(96);
    history_writer
        .admit_run(StructuredAdmissionRequest::new(
            admitted_run_id.clone(),
            stream.tenant_scope_id().clone(),
            InvocationIdentity::new("00000000-0000-4000-8000-000000000096")
                .expect("configured-race invocation"),
            operation_id,
            document,
            material,
            vec![ProposedCanonicalValue::from_value(&Value { value: 96 })
                .expect("configured-race initial value")],
            AppendRequestId::new("postgres-configured-race-admission")
                .expect("configured-race admission append id"),
        ))
        .await
        .expect("admit the reopened race winner");

    let successor = reopened_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(conflict_winner.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-race-post-admission")
                .expect("post-admission append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":"after-admission"}"#)
                .expect("post-admission value"),
        ))
        .await
        .expect("append configuration after admission");
    assert_eq!(
        reopened_reader
            .resolve(&stream, &contract)
            .await
            .expect("resolve post-admission successor")
            .revision(),
        &successor
    );

    let admitted = history_reader
        .load_verified(&admitted_run_id)
        .await
        .expect("reload admitted race winner");
    assert_eq!(
        admitted
            .admission()
            .admission_material_refs
            .configuration_ref,
        configuration_ref
    );
    let retained_configuration = admitted
        .object(&configuration_ref)
        .expect("admitted configuration object");
    let retained_revision: ConfigurationRevision =
        serde_json::from_str(&retained_configuration.canonical_json)
            .expect("decode admitted configuration revision");
    assert_eq!(retained_revision, conflict_winner);

    drop(history_reader);
    drop(history_writer);
    drop(reopened_reader);
    drop(reopened_writer);
    reopened_pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn configured_value_head_update_is_atomic_and_target_isolated() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured-head").expect("operation id");
    let (program_verifier, _) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let pool = database.combined_pool().await;
    let pool_control = pool.clone();
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        physical_verifier,
    )
    .await
    .expect("qualify structured stores");
    let (writer, reader) = configuration.split();
    let tenant =
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "7".repeat(32))).expect("tenant");
    let stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        tenant.clone(),
        operation_id.clone(),
        StableId::new("mfm.postgres.fixture/configured-head-a").expect("target"),
    );
    let other_stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        tenant,
        operation_id,
        StableId::new("mfm.postgres.fixture/configured-head-b").expect("target"),
    );
    let contract = admission_object(
        ADMISSION_CONFIGURATION_OBJECT_TYPE,
        "mfm.postgres.fixture.configured-head-contract",
        33,
    )
    .content_ref;
    let first = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-head-first").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("value"),
        ))
        .await
        .expect("first revision");

    let audit_pool = database.independent_pool().await;
    sqlx::query(
        "ALTER TABLE configuration_heads \
         ADD CONSTRAINT configuration_heads_injected_failure \
         CHECK (revision_sequence = 1)",
    )
    .execute(&audit_pool)
    .await
    .expect("install injected head failure");
    let failed = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-head-second").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
        ))
        .await;
    assert_eq!(failed, Err(StructuredStoreError::BackendUnavailable));
    let row = sqlx::query(
        "SELECT (SELECT pg_catalog.count(*)::bigint FROM configuration_revisions \
                  WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
                    AND entry_point_operation_id = $3 AND target_id = $4) AS revision_count, \
                (SELECT revision_sequence::text FROM configuration_heads \
                  WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
                    AND entry_point_operation_id = $3 AND target_id = $4) AS head_sequence",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .fetch_one(&audit_pool)
    .await
    .expect("inspect atomic rollback");
    assert_eq!(row.try_get::<i64, _>("revision_count").ok(), Some(1));
    assert_eq!(
        row.try_get::<String, _>("head_sequence").ok().as_deref(),
        Some("1")
    );
    assert_eq!(
        reader
            .resolve(&stream, &contract)
            .await
            .expect("first revision remains current")
            .revision(),
        &first
    );
    sqlx::query(
        "ALTER TABLE configuration_heads \
         DROP CONSTRAINT configuration_heads_injected_failure",
    )
    .execute(&audit_pool)
    .await
    .expect("remove injected head failure");

    let second = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-head-second").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
        ))
        .await
        .expect("retry atomically commits revision and head");

    sqlx::query(
        "UPDATE configuration_revisions SET canonical_revision_json = '{}' \
          WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
            AND entry_point_operation_id = $3 AND target_id = $4 \
            AND revision_sequence = 2",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .execute(&audit_pool)
    .await
    .expect("mutate current revision");
    assert_eq!(
        reader.resolve(&stream, &contract).await,
        Err(StructuredStoreError::InvalidHistory)
    );
    sqlx::query(
        "UPDATE configuration_revisions SET canonical_revision_json = $5 \
          WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
            AND entry_point_operation_id = $3 AND target_id = $4 \
            AND revision_sequence = 2",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .bind(
        canonical_json(&second)
            .expect("canonical second revision")
            .as_str(),
    )
    .execute(&audit_pool)
    .await
    .expect("restore current revision");

    update_configuration_head(&audit_pool, &stream, &first).await;
    assert_eq!(
        reader.resolve(&stream, &contract).await,
        Err(StructuredStoreError::InvalidHistory)
    );
    update_configuration_head(&audit_pool, &stream, &second).await;

    sqlx::query(
        "DELETE FROM configuration_heads \
          WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
            AND entry_point_operation_id = $3 AND target_id = $4",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .execute(&audit_pool)
    .await
    .expect("delete configuration head as owner");
    assert_eq!(
        reader.resolve(&stream, &contract).await,
        Err(StructuredStoreError::InvalidHistory)
    );
    insert_configuration_head(&audit_pool, &stream, &second).await;

    assert!(sqlx::query(
        "DELETE FROM configuration_revisions \
          WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
            AND entry_point_operation_id = $3 AND target_id = $4 \
            AND revision_sequence = 2",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .execute(&audit_pool)
    .await
    .is_err());
    assert!(sqlx::query("TRUNCATE TABLE configuration_revisions")
        .execute(&audit_pool)
        .await
        .is_err());

    let other = writer
        .append(ConfigurationAppendRequest::new(
            other_stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-head-other").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"target":"other"}"#).expect("value"),
        ))
        .await
        .expect("other target genesis");
    assert_eq!(
        reader
            .resolve(&stream, &contract)
            .await
            .expect("selected target")
            .revision(),
        &second
    );
    assert_eq!(
        reader
            .resolve(&other_stream, &contract)
            .await
            .expect("other target")
            .revision(),
        &other
    );

    audit_pool.close().await;
    drop(reader);
    drop(writer);
    drop(run_history);
    pool_control.close().await;
    database.cleanup().await;
}

async fn update_configuration_head(
    pool: &PgPool,
    stream: &ConfigurationStreamKey,
    revision: &ConfigurationRevision,
) {
    sqlx::query(
        "UPDATE configuration_heads \
            SET revision_sequence = $5::numeric, revision_schema_id = $6, revision_digest = $7 \
          WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
            AND entry_point_operation_id = $3 AND target_id = $4",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .bind(revision.sequence().to_string())
    .bind(revision.revision_ref().schema_id().as_str())
    .bind(revision.revision_ref().content_digest().as_str())
    .execute(pool)
    .await
    .expect("update configuration head as owner");
}

async fn insert_configuration_head(
    pool: &PgPool,
    stream: &ConfigurationStreamKey,
    revision: &ConfigurationRevision,
) {
    sqlx::query(
        "INSERT INTO configuration_heads ( \
            store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
            revision_sequence, revision_schema_id, revision_digest \
         ) VALUES ($1, $2, $3, $4, $5::numeric, $6, $7)",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .bind(revision.sequence().to_string())
    .bind(revision.revision_ref().schema_id().as_str())
    .bind(revision.revision_ref().content_digest().as_str())
    .execute(pool)
    .await
    .expect("insert configuration head as owner");
}

struct PinnedConfigurationHeadFence {
    stream: ConfigurationStreamKey,
    minimum_sequence: u64,
    minimum_revision_ref: ContentRef,
}

impl AuthoritativeWriterFence for PinnedConfigurationHeadFence {
    type Error = ();

    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a AuthoritativeWriterContext,
    ) -> AuthoritativeWriterFenceFuture<'a, Self::Error> {
        Box::pin(async move {
            let mut transaction = writer_pool.begin().await.map_err(|_| ())?;
            sqlx::query(
                "SELECT pg_catalog.set_config( \
                    'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
                 )",
            )
            .bind(context.schema_name())
            .execute(&mut *transaction)
            .await
            .map_err(|_| ())?;
            sqlx::query("SET LOCAL ROLE mfm_store_qualification")
                .execute(&mut *transaction)
                .await
                .map_err(|_| ())?;
            let row = sqlx::query(
                "SELECT revision_sequence::text AS revision_sequence, \
                        revision_schema_id, revision_digest \
                   FROM configuration_heads \
                  WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
                    AND entry_point_operation_id = $3 AND target_id = $4",
            )
            .bind(self.stream.store_scope_id().as_str())
            .bind(self.stream.tenant_scope_id().as_str())
            .bind(self.stream.entry_point_operation_id().as_str())
            .bind(self.stream.target_id().as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| ())?;
            transaction.rollback().await.map_err(|_| ())?;
            let row = row.ok_or(())?;
            if row
                .try_get::<String, _>("revision_sequence")
                .ok()
                .as_deref()
                == Some(self.minimum_sequence.to_string().as_str())
                && row
                    .try_get::<String, _>("revision_schema_id")
                    .ok()
                    .as_deref()
                    == Some(self.minimum_revision_ref.schema_id().as_str())
                && row.try_get::<String, _>("revision_digest").ok().as_deref()
                    == Some(self.minimum_revision_ref.content_digest().as_str())
            {
                Ok(())
            } else {
                Err(())
            }
        })
    }
}

#[tokio::test]
async fn external_writer_fence_rejects_a_coordinated_configuration_rollback() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured-rollback").expect("operation id");
    let (program_verifier, _) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let pool = database.combined_pool().await;
    let pool_control = pool.clone();
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&program_verifier),
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("qualify structured stores");
    let (writer, reader) = configuration.split();
    let stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "8".repeat(32))).expect("tenant"),
        operation_id,
        StableId::new("mfm.postgres.fixture/configured-rollback-target").expect("target"),
    );
    let contract = admission_object(
        ADMISSION_CONFIGURATION_OBJECT_TYPE,
        "mfm.postgres.fixture.configured-rollback-contract",
        34,
    )
    .content_ref;
    let first = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-rollback-first").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("value"),
        ))
        .await
        .expect("first revision");
    let second = writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-rollback-second").expect("append id"),
            contract,
            ProposedCanonicalValue::from_json(r#"{"revision":2}"#).expect("value"),
        ))
        .await
        .expect("second revision");
    let external_checkpoint = PinnedConfigurationHeadFence {
        stream: stream.clone(),
        minimum_sequence: second.sequence(),
        minimum_revision_ref: second.revision_ref().clone(),
    };
    drop(reader);
    drop(writer);
    drop(run_history);
    pool_control.close().await;

    let audit_pool = database.independent_pool().await;
    update_configuration_head(&audit_pool, &stream, &first).await;
    sqlx::query(
        "DELETE FROM configuration_revisions \
          WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
            AND entry_point_operation_id = $3 AND target_id = $4 \
            AND revision_sequence = 2",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .execute(&audit_pool)
    .await
    .expect("coordinated owner rollback");
    audit_pool.close().await;

    let local_pool = database.combined_pool().await;
    let local_pool_control = local_pool.clone();
    let local = open_structured_authoritative_with_configuration(
        local_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&program_verifier),
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("locally consistent rollback remains locally valid");
    drop(local);
    local_pool_control.close().await;

    let fenced_pool = database.combined_pool().await;
    let fenced_pool_control = fenced_pool.clone();
    let rejected = open_structured_authoritative_with_configuration(
        fenced_pool,
        external_checkpoint,
        program_verifier,
        physical_verifier,
    )
    .await;
    assert!(matches!(
        rejected,
        Err(PostgresStoreError::WriterFenceRejected)
    ));
    fenced_pool_control.close().await;
    database.cleanup().await;
}

async fn qualification_attempt(
    pool: PgPool,
) -> mfm_storage_postgres::Result<StructuredRunStore<PostgresStructuredHistoryBackend>> {
    let (program_verifier, _) =
        qualified_program(stable("mfm.postgres.fixture/qualification").expect("operation id"));
    open_structured_authoritative(
        pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        Arc::new(NoPhysicalBindings),
    )
    .await
}

async fn assert_schema_reopen_rejected(database: &TestDatabase) {
    let pool = database.application_pool().await;
    let pool_control = pool.clone();
    assert!(matches!(
        qualification_attempt(pool).await,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));
    pool_control.close().await;
}

async fn assert_session_reopen_rejected(pool: PgPool) {
    let pool_control = pool.clone();
    assert!(matches!(
        qualification_attempt(pool).await,
        Err(PostgresStoreError::WriterRequired)
    ));
    pool_control.close().await;
}

#[tokio::test]
async fn qualification_rejects_replaced_check_definition_even_when_the_name_is_retained() {
    let database = TestDatabase::create().await;
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {}.run_history_heads \
         DROP CONSTRAINT run_history_heads_run_id_v1, \
         ADD CONSTRAINT run_history_heads_run_id_v1 CHECK (TRUE)",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("replace named check with a permissive definition");

    assert_schema_reopen_rejected(&database).await;
    database.cleanup().await;
}

#[tokio::test]
async fn qualification_rejects_public_and_hostile_schema_or_table_grants() {
    let database = TestDatabase::create().await;
    sqlx::query(AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA {} TO PUBLIC",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("grant hostile public schema access");
    assert_schema_reopen_rejected(&database).await;
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE USAGE ON SCHEMA {} FROM PUBLIC",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("restore closed schema ACL");

    let hostile_role = format!("{}_hostile", database.application_login.role_name);
    sqlx::query(AssertSqlSafe(format!(
        "CREATE ROLE {hostile_role} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
         NOINHERIT NOREPLICATION NOBYPASSRLS"
    )))
    .execute(&database.admin_pool)
    .await
    .expect("create hostile grantee");
    sqlx::query(AssertSqlSafe(format!(
        "GRANT SELECT ON TABLE {}.run_history_batches TO {hostile_role}",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("grant hostile table access");
    assert_schema_reopen_rejected(&database).await;
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE SELECT ON TABLE {}.run_history_batches FROM {hostile_role}",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("restore closed table ACL");
    sqlx::query(AssertSqlSafe(format!("DROP ROLE {hostile_role}")))
        .execute(&database.admin_pool)
        .await
        .expect("drop hostile grantee");

    let pool = database.application_pool().await;
    let pool_control = pool.clone();
    drop(
        qualification_attempt(pool)
            .await
            .expect("restored exact ACLs qualify"),
    );
    pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn qualification_rejects_extra_membership_inheritance_and_admin_session_substitution() {
    let database = TestDatabase::create().await;

    let maintenance_pool = database.maintenance_pool().await;
    let maintenance_control = maintenance_pool.clone();
    drop(
        open_configuration_maintenance(maintenance_pool, TestAuthoritativeWriterFence)
            .await
            .expect("exact maintenance login qualifies"),
    );
    maintenance_control.close().await;

    sqlx::query(AssertSqlSafe(format!(
        "GRANT mfm_store_configuration_maintenance TO {} \
         WITH INHERIT FALSE, SET TRUE",
        database.application_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("inject extra incoming membership");
    assert_session_reopen_rejected(database.application_pool().await).await;
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE mfm_store_configuration_maintenance FROM {}",
        database.application_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("remove extra incoming membership");

    sqlx::query(AssertSqlSafe(format!(
        "GRANT mfm_store_application TO {} WITH ADMIN OPTION",
        database.application_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("inject membership administration authority");
    assert_session_reopen_rejected(database.application_pool().await).await;
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE ADMIN OPTION FOR mfm_store_application FROM {}",
        database.application_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("remove membership administration authority");

    sqlx::query(AssertSqlSafe(format!(
        "ALTER ROLE {} INHERIT",
        database.application_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("inject inherited session authority");
    assert_session_reopen_rejected(database.application_pool().await).await;
    sqlx::query(AssertSqlSafe(format!(
        "ALTER ROLE {} NOINHERIT",
        database.application_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("restore non-inheriting session role");

    let substituted_options = database
        .database_url
        .parse::<PgConnectOptions>()
        .expect("parse DATABASE_URL")
        .options([
            ("search_path", database.schema.as_str()),
            ("role", database.application_login.role_name.as_str()),
        ]);
    let substituted_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(substituted_options)
        .await
        .expect("connect privileged session with a substituted current role");
    assert_session_reopen_rejected(substituted_pool).await;

    let pool = database.application_pool().await;
    let pool_control = pool.clone();
    drop(
        qualification_attempt(pool)
            .await
            .expect("restored exact application login qualifies"),
    );
    pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn structured_history_fresh_process_worker() {
    let Some(mode) = std::env::var_os(FRESH_PROCESS_MODE_ENV) else {
        return;
    };
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
    let schema = std::env::var(FRESH_PROCESS_SCHEMA_ENV).expect("worker schema is required");
    let pool = isolated_pool(&database_url, &schema).await;
    let pool_control = pool.clone();
    let operation_id = stable("mfm.postgres.fixture/reopen").expect("operation id");
    let (program_verifier, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let store = open_structured_authoritative(
        pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        physical_verifier,
    )
    .await
    .expect("qualify fresh-process structured store");
    let (writer, reader) = store.split();
    let run_id = run_id(1);

    match mode.to_str().expect("worker mode is UTF-8") {
        "admit" => {
            let replay_document = document.clone();
            let replay_operation_id = operation_id.clone();
            let first_attempt = writer
                .admit_run(admission(
                    run_id.clone(),
                    operation_id,
                    document,
                    "postgres-admit",
                ))
                .await
                .expect("persist admission in first process");
            assert!(matches!(
                first_attempt.outcome(),
                mfm_store::structured::BackendAppendOutcome::NewlyCommitted(_)
            ));
            let replay = writer
                .admit_run(admission(
                    run_id.clone(),
                    replay_operation_id,
                    replay_document,
                    "postgres-admit",
                ))
                .await
                .expect("replay exact admission in first process");
            assert!(matches!(
                replay.outcome(),
                mfm_store::structured::BackendAppendOutcome::ExistingSame(_)
            ));
            assert_eq!(first_attempt.committed(), replay.committed());
            assert!(matches!(
                reader
                    .load_verified(&run_id)
                    .await
                    .expect("first-process refold")
                    .frontier(),
                StructuredFrontier::Actions(_)
            ));
        }
        "continue" => {
            let verified = reader
                .load_verified(&run_id)
                .await
                .expect("second-process refold");
            assert!(matches!(
                verified.frontier(),
                StructuredFrontier::Actions(_)
            ));
            let transition = writer
                .commit_state_transition(
                    verified,
                    &StateTransitionProposal::success(
                        AppendRequestId::new("postgres-transition").expect("transition append id"),
                        ProposedCanonicalValue::from_value(&Value { value: 8 })
                            .expect("transition value"),
                        mfm_facts::FactSet::empty(),
                    ),
                )
                .await
                .expect("continue in second process");
            let batch = transition.committed().expect("known transition commit");
            assert_eq!(batch.records.len(), 2);
            assert!(matches!(
                &batch.records[0].record,
                RunRecord::StateTransitionCommitted(_)
            ));
            assert!(matches!(&batch.records[1].record, RunRecord::RunClosed(_)));
            assert!(matches!(
                reader
                    .load_verified(&run_id)
                    .await
                    .expect("second-process closed refold")
                    .frontier(),
                StructuredFrontier::Complete
            ));
        }
        _ => panic!("unknown structured-history worker mode"),
    }

    drop(reader);
    drop(writer);
    pool_control.close().await;
}

#[tokio::test]
async fn fresh_process_refolds_and_continues_the_same_structured_run() {
    let database = TestDatabase::create().await;
    run_fresh_process_worker(&database, "admit").await;
    run_fresh_process_worker(&database, "continue").await;

    let operation_id = stable("mfm.postgres.fixture/reopen").expect("operation id");
    let (program_verifier, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let run_id = run_id(1);
    let audit_pool = database.independent_pool().await;
    let batches = load_normalized_batches(&audit_pool, &run_id).await;
    audit_pool.close().await;
    let [postgres_admission_batch, postgres_transition_batch] = batches.as_slice() else {
        panic!("fresh processes must commit exactly admission and transition batches");
    };
    assert_eq!(postgres_admission_batch.records.len(), 1);
    assert!(matches!(
        &postgres_admission_batch.records[0].record,
        RunRecord::RunAdmitted(_)
    ));
    assert_eq!(postgres_transition_batch.records.len(), 2);
    assert!(matches!(
        &postgres_transition_batch.records[0].record,
        RunRecord::StateTransitionCommitted(_)
    ));
    assert!(matches!(
        &postgres_transition_batch.records[1].record,
        RunRecord::RunClosed(_)
    ));

    let (memory_program_verifier, memory_document) = qualified_program(operation_id.clone());
    assert_eq!(memory_document, document);
    let memory = StructuredRunStore::new(
        StructuredMemoryBackend::new(StructuredStoreIdentity {
            store_scope_id: postgres_admission_batch.store_scope_id.clone(),
            store_epoch: postgres_admission_batch.store_epoch,
        }),
        memory_program_verifier,
        Arc::clone(&physical_verifier),
    );
    let (memory_writer, memory_reader) = memory.split();
    let memory_admission = memory_writer
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            memory_document,
            "postgres-admit",
        ))
        .await
        .expect("persist identical admission through memory");
    assert_eq!(memory_admission.committed(), Some(postgres_admission_batch));
    let memory_verified = memory_reader
        .load_verified(&run_id)
        .await
        .expect("refold memory admission");
    let memory_transition = memory_writer
        .commit_state_transition(
            memory_verified,
            &StateTransitionProposal::success(
                AppendRequestId::new("postgres-transition").expect("transition append id"),
                ProposedCanonicalValue::from_value(&Value { value: 8 }).expect("transition value"),
                mfm_facts::FactSet::empty(),
            ),
        )
        .await
        .expect("commit identical transition through memory");
    assert_eq!(
        memory_transition.committed(),
        Some(postgres_transition_batch)
    );

    let verification_pool = database.application_pool().await;
    let verification_pool_control = verification_pool.clone();
    let verification = open_structured_authoritative(
        verification_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&program_verifier),
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("qualify verification structured store");
    let (verification_writer, verification_reader) = verification.split();
    assert_eq!(
        verification_writer
            .resolve_append(
                &run_id,
                &postgres_admission_batch.append_request_id,
                &postgres_admission_batch.candidate_digest,
            )
            .await
            .expect("resolve first-process admission")
            .as_ref(),
        Some(postgres_admission_batch)
    );
    assert_eq!(
        verification_writer
            .resolve_append(
                &run_id,
                &postgres_transition_batch.append_request_id,
                &postgres_transition_batch.candidate_digest,
            )
            .await
            .expect("resolve second-process transition")
            .as_ref(),
        Some(postgres_transition_batch)
    );
    assert!(matches!(
        verification_reader
            .load_verified(&run_id)
            .await
            .expect("verification-process closed refold")
            .frontier(),
        StructuredFrontier::Complete
    ));
    assert!(matches!(
        mfm_replay::structured::verify_recorded_history(&verification_reader, &run_id)
            .await
            .expect("callback-free replay after fresh-process continuation")
            .frontier(),
        StructuredFrontier::Complete
    ));
    assert!(matches!(
        memory_reader
            .load_verified(&run_id)
            .await
            .expect("memory parity closed refold")
            .frontier(),
        StructuredFrontier::Complete
    ));
    drop(verification_reader);
    drop(verification_writer);
    verification_pool_control.close().await;

    let unavailable_pool = database.application_pool().await;
    let unavailable_control = unavailable_pool.clone();
    let unavailable = open_structured_authoritative(
        unavailable_pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        physical_verifier,
    )
    .await
    .expect("qualify store before outage");
    let (_unavailable_writer, unavailable_reader) = unavailable.split();
    unavailable_control.close().await;
    assert_eq!(
        unavailable_reader
            .load_verified(&run_id)
            .await
            .expect_err("closed PostgreSQL pool must fail without fallback"),
        StructuredStoreError::BackendUnavailable
    );

    database.cleanup().await;
}

#[tokio::test]
async fn tenant_fact_publications_are_dense_atomic_and_exactly_routed() {
    let database = TestDatabase::create().await;
    let operation_id =
        stable("mfm.postgres.fixture/tenant-fact-publication").expect("operation id");
    let (program_verifier, document) = qualified_fact_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let store_pool = database.application_pool().await;
    let store_pool_control = store_pool.clone();
    let store = open_structured_authoritative(
        store_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&program_verifier),
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("qualify fact publication store");
    let (writer, reader) = store.split();
    let first_run = run_id(40);
    let second_run = run_id(41);
    writer
        .admit_run(admission(
            first_run.clone(),
            operation_id.clone(),
            document.clone(),
            "first-fact-publication-admit",
        ))
        .await
        .expect("admit first fact producer");
    writer
        .admit_run(admission(
            second_run.clone(),
            operation_id.clone(),
            document.clone(),
            "second-fact-publication-admit",
        ))
        .await
        .expect("admit second fact producer");
    let first_verified = reader
        .load_verified(&first_run)
        .await
        .expect("verify first fact producer");
    let second_verified = reader
        .load_verified(&second_run)
        .await
        .expect("verify second fact producer");
    let first_proposal = StateTransitionProposal::success(
        AppendRequestId::new("first-fact-publication").expect("first append id"),
        ProposedCanonicalValue::from_value(&Value { value: 8 }).expect("first output"),
        fact_set(Value { value: 7 }, Value { value: 8 }),
    );
    let second_proposal = StateTransitionProposal::success(
        AppendRequestId::new("second-fact-publication").expect("second append id"),
        ProposedCanonicalValue::from_value(&Value { value: 9 }).expect("second output"),
        fact_set(Value { value: 7 }, Value { value: 9 }),
    );

    let (first_attempt, second_attempt) = tokio::join!(
        writer.commit_state_transition(first_verified, &first_proposal),
        writer.commit_state_transition(second_verified, &second_proposal),
    );
    for attempt in [&first_attempt, &second_attempt] {
        match attempt {
            Ok(attempt)
                if matches!(
                    attempt.outcome(),
                    BackendAppendOutcome::NewlyCommitted(_)
                        | BackendAppendOutcome::StaleHead
                        | BackendAppendOutcome::AcknowledgementUnknown
                ) => {}
            Err(StructuredStoreError::BackendUnavailable) => {}
            other => panic!("unexpected racing fact publication outcome: {other:?}"),
        }
    }

    let first_verified = reader
        .load_verified(&first_run)
        .await
        .expect("reload first fact producer");
    if matches!(first_verified.frontier(), StructuredFrontier::Actions(_)) {
        let retry = writer
            .commit_state_transition(first_verified, &first_proposal)
            .await
            .expect("retry first fact publication");
        assert!(matches!(
            retry.outcome(),
            BackendAppendOutcome::NewlyCommitted(_)
        ));
    }
    let second_verified = reader
        .load_verified(&second_run)
        .await
        .expect("reload second fact producer");
    if matches!(second_verified.frontier(), StructuredFrontier::Actions(_)) {
        let retry = writer
            .commit_state_transition(second_verified, &second_proposal)
            .await
            .expect("retry second fact publication");
        assert!(matches!(
            retry.outcome(),
            BackendAppendOutcome::NewlyCommitted(_)
        ));
    }

    let audit_pool = database.independent_pool().await;
    let first_batches = load_normalized_batches(&audit_pool, &first_run).await;
    let second_batches = load_normalized_batches(&audit_pool, &second_run).await;
    let [_, first_publication] = first_batches.as_slice() else {
        panic!("first fact producer must contain admission and transition");
    };
    let [_, second_publication] = second_batches.as_slice() else {
        panic!("second fact producer must contain admission and transition");
    };
    let mut publications = vec![first_publication.clone(), second_publication.clone()];
    publications.sort_by_key(|batch| match &batch.tenant_fact_coordinate {
        TenantFactCoordinate::FactPublication { frontier } => frontier.fact_order,
        _ => panic!("fact transition must carry a publication coordinate"),
    });
    assert_eq!(publications.len(), 2);
    for (index, batch) in publications.iter().enumerate() {
        let TenantFactCoordinate::FactPublication { frontier } = &batch.tenant_fact_coordinate
        else {
            panic!("fact transition must publish");
        };
        assert_eq!(
            frontier.fact_order,
            u64::try_from(index + 1).expect("bounded fact publication order")
        );
        assert_eq!(batch.records[0].record_ref.ordinal, 0);
    }

    let rows = sqlx::query(
        "SELECT fact_order::text AS fact_order, run_id, \
                run_sequence::text AS run_sequence, transition_ordinal, \
                transition_record_hash \
           FROM tenant_fact_publications ORDER BY fact_order",
    )
    .fetch_all(&audit_pool)
    .await
    .expect("load tenant fact routes");
    assert_eq!(rows.len(), publications.len());
    for (row, batch) in rows.iter().zip(&publications) {
        let transition_ref = &batch.records[0].record_ref;
        let TenantFactCoordinate::FactPublication { frontier } = &batch.tenant_fact_coordinate
        else {
            panic!("fact transition must publish");
        };
        assert_eq!(
            row.try_get::<String, _>("fact_order").expect("fact order"),
            frontier.fact_order.to_string()
        );
        assert_eq!(
            row.try_get::<String, _>("run_id").expect("route run id"),
            transition_ref.run_id.as_str()
        );
        assert_eq!(
            row.try_get::<String, _>("run_sequence")
                .expect("route sequence"),
            transition_ref.run_sequence.to_string()
        );
        assert_eq!(
            row.try_get::<i32, _>("transition_ordinal")
                .expect("route ordinal"),
            0
        );
        assert_eq!(
            row.try_get::<String, _>("transition_record_hash")
                .expect("route record hash"),
            transition_ref.record_hash.as_str()
        );
    }
    let head = sqlx::query_scalar::<_, String>(
        "SELECT fact_order::text FROM tenant_fact_heads WHERE tenant_scope_id = $1",
    )
    .bind(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
    .fetch_one(&audit_pool)
    .await
    .expect("load tenant fact head");
    assert_eq!(head, "2");

    sqlx::query("DELETE FROM tenant_fact_heads WHERE tenant_scope_id = $1")
        .bind(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
        .execute(&audit_pool)
        .await
        .expect("inject missing tenant fact head");
    let corrupted_run = run_id(42);
    writer
        .admit_run(admission(
            corrupted_run.clone(),
            operation_id,
            document,
            "corrupted-head-producer-admit",
        ))
        .await
        .expect("admit producer after tenant head corruption");
    assert!(matches!(
        writer
            .commit_state_transition(
                reader
                    .load_verified(&corrupted_run)
                    .await
                    .expect("verify producer after tenant head corruption"),
                &StateTransitionProposal::success(
                    AppendRequestId::new("corrupted-head-publication")
                        .expect("corrupted head append id"),
                    ProposedCanonicalValue::from_value(&Value { value: 8 })
                        .expect("corrupted head output"),
                    fact_set(Value { value: 7 }, Value { value: 8 }),
                ),
            )
            .await,
        Err(StructuredStoreError::InvalidHistory)
    ));
    let corrupted_pool = database.application_pool().await;
    assert!(matches!(
        open_structured_authoritative(
            corrupted_pool,
            TestAuthoritativeWriterFence,
            program_verifier,
            physical_verifier,
        )
        .await,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
    ));

    audit_pool.close().await;
    drop(reader);
    drop(writer);
    store_pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn prior_run_fact_scan_survives_reopen_and_matches_memory_bytes() {
    let database = TestDatabase::create().await;
    let postgres_fixture = qualified_fact_scan_fixture();
    let store_pool = database.application_pool().await;
    let store_pool_control = store_pool.clone();
    let postgres_store = open_structured_authoritative(
        store_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&postgres_fixture.program_verifier),
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("qualify fact scanner store");
    let (postgres_writer, postgres_reader) = postgres_store.split();
    let postgres_identity = postgres_reader.store_identity().clone();
    let postgres_response =
        drive_qualified_fact_scan(postgres_writer, &postgres_reader, postgres_fixture).await;

    let reopened_fixture = qualified_fact_scan_fixture();
    let reopened_pool = database.application_pool().await;
    let reopened_pool_control = reopened_pool.clone();
    let reopened_store = open_structured_authoritative(
        reopened_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&reopened_fixture.program_verifier),
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("reopen fact scanner store through an independent pool");
    let (reopened_writer, reopened_reader) = reopened_store.split();
    let reopened_response = retained_fact_response(&reopened_reader, &run_id(81)).await;
    assert_eq!(reopened_response, postgres_response);

    let memory_fixture = qualified_fact_scan_fixture();
    let memory_store = StructuredRunStore::new(
        StructuredMemoryBackend::new(postgres_identity),
        Arc::clone(&memory_fixture.program_verifier),
        Arc::new(NoPhysicalBindings),
    );
    let (memory_writer, memory_reader) = memory_store.split();
    let memory_response =
        drive_qualified_fact_scan(memory_writer, &memory_reader, memory_fixture).await;
    assert_eq!(memory_response, postgres_response);

    drop(memory_reader);
    drop(reopened_reader);
    drop(reopened_writer);
    drop(postgres_reader);
    reopened_pool_control.close().await;
    store_pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn prior_run_fact_scan_accepts_empty_frontier_and_excludes_ineligible_source() {
    let database = TestDatabase::create().await;
    let allowed = qualified_fact_scan_fixture();
    let allowed_pool = database.application_pool().await;
    let allowed_pool_control = allowed_pool.clone();
    let allowed_store = open_structured_authoritative(
        allowed_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&allowed.program_verifier),
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("qualify empty-frontier scanner store");
    let (allowed_writer, allowed_reader) = allowed_store.split();
    let allowed_runtime = Runtime::new(allowed_writer, allowed.processes);
    let empty_run = run_id(84);
    allowed_runtime
        .admit_run(fact_scan_admission_for_tenant(
            empty_run.clone(),
            allowed.consumer_operation.clone(),
            allowed.consumer_document.clone(),
            admission_material_with_source(84, allowed.source_object.clone()),
            Value { value: 7 },
            "empty-frontier-fact-scan-admit",
            84,
            '3',
        ))
        .await
        .expect("admit empty-frontier consumer");
    assert_eq!(
        allowed_runtime
            .drive_once(&empty_run)
            .await
            .expect("drive empty-frontier fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        allowed_runtime
            .drive_once(&empty_run)
            .await
            .expect("settle empty-frontier fact scan"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    let empty_response: PriorRunFactSelectionResponse =
        serde_json::from_str(&retained_fact_response(&allowed_reader, &empty_run).await)
            .expect("decode empty-frontier response");
    assert_eq!(empty_response.attestation.frontier.fact_order, 0);
    assert_eq!(empty_response.query_results.len(), 1);
    assert!(empty_response.query_results[0].selected.is_empty());

    let producer_run = run_id(85);
    allowed_runtime
        .admit_run(fact_scan_admission(
            producer_run.clone(),
            allowed.producer_operation,
            allowed.producer_document,
            admission_material(85),
            Value { value: 7 },
            "excluded-source-producer-admit",
            85,
        ))
        .await
        .expect("admit excluded-source producer");
    assert_eq!(
        allowed_runtime
            .drive_once(&producer_run)
            .await
            .expect("drive excluded-source producer"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    drop(allowed_runtime);
    drop(allowed_reader);
    allowed_pool_control.close().await;

    let excluded = qualified_fact_scan_fixture_excluding_producer();
    let excluded_pool = database.application_pool().await;
    let excluded_pool_control = excluded_pool.clone();
    let excluded_store = open_structured_authoritative(
        excluded_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&excluded.program_verifier),
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("reopen scanner with an excluding manifest");
    let (excluded_writer, excluded_reader) = excluded_store.split();
    let excluded_runtime = Runtime::new(excluded_writer, excluded.processes);
    let excluded_run = run_id(86);
    excluded_runtime
        .admit_run(fact_scan_admission(
            excluded_run.clone(),
            excluded.consumer_operation,
            excluded.consumer_document,
            admission_material_with_source(86, excluded.source_object),
            Value { value: 7 },
            "excluded-source-consumer-admit",
            86,
        ))
        .await
        .expect("admit excluded-source consumer");
    assert_eq!(
        excluded_runtime
            .drive_once(&excluded_run)
            .await
            .expect("drive excluded-source fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        excluded_runtime
            .drive_once(&excluded_run)
            .await
            .expect("settle excluded-source fact scan"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    let excluded_response: PriorRunFactSelectionResponse =
        serde_json::from_str(&retained_fact_response(&excluded_reader, &excluded_run).await)
            .expect("decode excluded-source response");
    assert_eq!(excluded_response.attestation.frontier.fact_order, 1);
    assert_eq!(excluded_response.query_results.len(), 1);
    assert!(excluded_response.query_results[0].selected.is_empty());

    drop(excluded_runtime);
    drop(excluded_reader);
    excluded_pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn fact_publication_and_selection_barrier_have_one_tenant_linearization() {
    let database = TestDatabase::create().await;
    let fixture = qualified_fact_scan_fixture();
    let store_pool = database.application_pool().await;
    let store_pool_control = store_pool.clone();
    let store = open_structured_authoritative(
        store_pool,
        TestAuthoritativeWriterFence,
        Arc::clone(&fixture.program_verifier),
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("qualify publication-barrier race store");
    let (writer, reader) = store.split();
    let producer_run = run_id(82);
    let consumer_run = run_id(83);
    writer
        .admit_run(fact_scan_admission(
            producer_run.clone(),
            fixture.producer_operation.clone(),
            fixture.producer_document.clone(),
            admission_material(82),
            Value { value: 7 },
            "publication-barrier-producer-admit",
            82,
        ))
        .await
        .expect("admit racing producer");
    writer
        .admit_run(fact_scan_admission(
            consumer_run.clone(),
            fixture.consumer_operation.clone(),
            fixture.consumer_document.clone(),
            admission_material_with_source(83, fixture.source_object.clone()),
            Value { value: 7 },
            "publication-barrier-consumer-admit",
            83,
        ))
        .await
        .expect("admit racing consumer");

    let producer = reader
        .load_verified(&producer_run)
        .await
        .expect("verify racing producer");
    let consumer = reader
        .load_verified(&consumer_run)
        .await
        .expect("verify racing consumer");
    let StructuredFrontier::Actions(actions) = consumer.frontier() else {
        panic!("racing consumer must be actionable");
    };
    let [action] = actions.as_slice() else {
        panic!("racing consumer must expose one fact read");
    };
    let action = action.clone();
    let target = AccessTargetSelection {
        run_id: consumer.run_id(),
        occurrence_id: &action.occurrence_id,
        state_input_ref: &action.input,
        store_scope_id: &consumer.admission().store_scope_id,
        store_epoch: consumer.admission().store_epoch,
        tenant_scope_id: &consumer.admission().tenant_scope_id,
        admitted_prior_run_source_manifest_ref: &consumer
            .admission()
            .admission_material_refs
            .prior_run_source_manifest_ref,
        admitted_routing_policy_ref: &consumer
            .admission()
            .admission_material_refs
            .routing_policy_ref,
        stable_resource_lineage_contract_ref: None,
        minimum_lineage_head_ref: None,
    };
    let capability_identity = fixture
        .processes
        .component_identity(
            StructuredComponentKind::Capability,
            action
                .capability_contract_ref
                .as_ref()
                .expect("fact scanner capability"),
        )
        .expect("qualified fact scanner capability");
    let binding = fixture
        .processes
        .prepare_access::<ReadPhysicalBindingKind>(
            &capability_identity,
            target,
            mfm_spec::CanonicalJsonValue::new(
                serde_json::to_value(&fixture.request).expect("fact request JSON"),
            )
            .expect("canonical fact request"),
        )
        .await
        .expect("prepare fact scanner binding")
        .expect("qualified fact scanner binding");
    let authorization = AccessAuthorizationProposal::new(
        AppendRequestId::new("publication-barrier-authorization").expect("authorization append"),
        action.input,
        ProposedCanonicalValue::from_value(&fixture.request).expect("fact request proposal"),
        binding.public_certificate().clone(),
    );
    let publication = StateTransitionProposal::success(
        AppendRequestId::new("publication-barrier-transition").expect("transition append"),
        ProposedCanonicalValue::from_value(&Value { value: 8 }).expect("producer output"),
        fact_set(Value { value: 7 }, Value { value: 8 }),
    );

    let (publication_attempt, barrier_attempt) = tokio::join!(
        writer.commit_state_transition(producer, &publication),
        writer.authorize_access(consumer, &authorization),
    );
    assert_tenant_race_attempt(&publication_attempt);
    assert_tenant_race_attempt(&barrier_attempt);

    let producer = reader
        .load_verified(&producer_run)
        .await
        .expect("reload racing producer");
    if matches!(producer.frontier(), StructuredFrontier::Actions(_)) {
        let retry = writer
            .commit_state_transition(producer, &publication)
            .await
            .expect("retry racing publication");
        assert!(matches!(
            retry.outcome(),
            BackendAppendOutcome::NewlyCommitted(_)
                | BackendAppendOutcome::ExistingSame(_)
                | BackendAppendOutcome::AcknowledgementUnknown
        ));
    }
    let consumer = reader
        .load_verified(&consumer_run)
        .await
        .expect("reload racing consumer");
    if matches!(consumer.frontier(), StructuredFrontier::Actions(_)) {
        let retry = writer
            .authorize_access(consumer, &authorization)
            .await
            .expect("retry racing selection barrier");
        assert!(matches!(
            retry.outcome(),
            BackendAppendOutcome::NewlyCommitted(_)
                | BackendAppendOutcome::ExistingSame(_)
                | BackendAppendOutcome::AcknowledgementUnknown
        ));
    }

    let audit_pool = database.independent_pool().await;
    let producer_batches = load_normalized_batches(&audit_pool, &producer_run).await;
    let consumer_batches = load_normalized_batches(&audit_pool, &consumer_run).await;
    let [_, publication_batch] = producer_batches.as_slice() else {
        panic!("racing producer must commit one atomic publication");
    };
    let [_, barrier_batch] = consumer_batches.as_slice() else {
        panic!("racing consumer must commit one atomic authorization barrier");
    };
    let TenantFactCoordinate::FactPublication {
        frontier: publication_frontier,
    } = &publication_batch.tenant_fact_coordinate
    else {
        panic!("producer transition must publish its fact");
    };
    let TenantFactCoordinate::FactSelectionBarrier {
        frontier: barrier_frontier,
    } = &barrier_batch.tenant_fact_coordinate
    else {
        panic!("consumer authorization must capture a selection barrier");
    };
    assert_eq!(publication_frontier.fact_order, 1);
    assert!(matches!(barrier_frontier.fact_order, 0 | 1));
    let visible_publications = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM tenant_fact_publications \
          WHERE tenant_scope_id = $1 AND fact_order <= $2::numeric",
    )
    .bind(barrier_frontier.tenant_scope_id.as_str())
    .bind(barrier_frontier.fact_order.to_string())
    .fetch_one(&audit_pool)
    .await
    .expect("count publications visible through the barrier");
    assert_eq!(
        visible_publications,
        i64::try_from(barrier_frontier.fact_order).expect("barrier order fits i64")
    );

    drop(binding);
    drop(reader);
    drop(writer);
    audit_pool.close().await;
    store_pool_control.close().await;
    database.cleanup().await;
}

fn assert_tenant_race_attempt(
    attempt: &std::result::Result<
        mfm_store::structured::StructuredAppendAttempt,
        StructuredStoreError,
    >,
) {
    match attempt {
        Ok(attempt)
            if matches!(
                attempt.outcome(),
                BackendAppendOutcome::NewlyCommitted(_)
                    | BackendAppendOutcome::ExistingSame(_)
                    | BackendAppendOutcome::StaleHead
                    | BackendAppendOutcome::AcknowledgementUnknown
            ) => {}
        Err(StructuredStoreError::BackendUnavailable) => {}
        other => panic!("unexpected publication-barrier race outcome: {other:?}"),
    }
}

#[tokio::test]
async fn object_row_failure_rolls_back_batch_objects_and_head() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/object-row-rollback").expect("operation id");
    let (program_verifier, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let store_pool = database.application_pool().await;
    let store_pool_control = store_pool.clone();
    let store = open_structured_authoritative(
        store_pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        physical_verifier,
    )
    .await
    .expect("qualify store before injecting object failure");
    let (writer, reader) = store.split();
    let mutation_pool = database.independent_pool().await;
    sqlx::query(
        "CREATE FUNCTION reject_structured_object_insert() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected object-row failure'; END $$",
    )
    .execute(&mutation_pool)
    .await
    .expect("create object-row failure function");
    sqlx::query(
        "CREATE TRIGGER reject_structured_object_insert \
         BEFORE INSERT ON run_history_batch_objects \
         FOR EACH ROW EXECUTE FUNCTION reject_structured_object_insert()",
    )
    .execute(&mutation_pool)
    .await
    .expect("create object-row failure trigger");

    let run_id = run_id(31);
    assert_eq!(
        writer
            .admit_run(admission(
                run_id.clone(),
                operation_id,
                document,
                "object-row-rollback",
            ))
            .await
            .expect_err("injected child-row failure must reject the append"),
        StructuredStoreError::BackendUnavailable
    );
    let retained_rows = sqlx::query_scalar::<_, i64>(
        "SELECT (SELECT count(*) FROM run_history_batches) \
              + (SELECT count(*) FROM run_history_batch_objects) \
              + (SELECT count(*) FROM run_history_heads)",
    )
    .fetch_one(&mutation_pool)
    .await
    .expect("count rows after injected failure");
    assert_eq!(retained_rows, 0);
    assert_eq!(
        reader
            .load_verified(&run_id)
            .await
            .expect_err("rolled-back append must leave no run"),
        StructuredStoreError::RunNotFound
    );

    drop(reader);
    drop(writer);
    mutation_pool.close().await;
    store_pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn malformed_object_rows_fail_closed_after_qualification() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/malformed-objects").expect("operation id");
    let (program_verifier, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let store_pool = database.application_pool().await;
    let store_pool_control = store_pool.clone();
    let store = open_structured_authoritative(
        store_pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        physical_verifier,
    )
    .await
    .expect("qualify store before hostile object mutations");
    let (writer, reader) = store.split();
    let run_id = run_id(32);
    writer
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            "malformed-object-rows",
        ))
        .await
        .expect("admit corruption fixture");
    let mutation_pool = database.independent_pool().await;
    let snapshots = sqlx::query(
        "SELECT object_ordinal, object_type, content_schema_id, content_digest, canonical_json \
           FROM run_history_batch_objects \
          WHERE run_id = $1 AND run_sequence = 1 \
          ORDER BY object_ordinal LIMIT 2",
    )
    .bind(run_id.as_str())
    .fetch_all(&mutation_pool)
    .await
    .expect("load object snapshots")
    .iter()
    .map(ObjectRowSnapshot::from_row)
    .collect::<Vec<_>>();
    let [first, second] = snapshots.as_slice() else {
        panic!("admission fixture must retain at least two objects");
    };
    let object_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM run_history_batch_objects WHERE run_id = $1 AND run_sequence = 1",
    )
    .bind(run_id.as_str())
    .fetch_one(&mutation_pool)
    .await
    .expect("count admitted objects");

    sqlx::query(
        "DELETE FROM run_history_batch_objects \
          WHERE run_id = $1 AND run_sequence = 1 AND object_ordinal = $2",
    )
    .bind(run_id.as_str())
    .bind(first.ordinal)
    .execute(&mutation_pool)
    .await
    .expect("remove one object row");
    assert!(matches!(
        reader.load_verified(&run_id).await,
        Err(StructuredStoreError::InvalidHistory)
    ));
    insert_object_snapshot(&mutation_pool, &run_id, first).await;

    sqlx::query(
        "INSERT INTO run_history_batch_objects ( \
            run_id, run_sequence, object_ordinal, object_type, content_schema_id, \
            content_digest, canonical_json \
         ) SELECT run_id, run_sequence, $2, object_type, content_schema_id, \
                  content_digest, canonical_json \
             FROM run_history_batch_objects \
            WHERE run_id = $1 AND run_sequence = 1 AND object_ordinal = 0",
    )
    .bind(run_id.as_str())
    .bind(i32::try_from(object_count).expect("bounded object count"))
    .execute(&mutation_pool)
    .await
    .expect("append one extra object row");
    assert!(matches!(
        reader.load_verified(&run_id).await,
        Err(StructuredStoreError::InvalidHistory)
    ));
    sqlx::query(
        "DELETE FROM run_history_batch_objects \
          WHERE run_id = $1 AND run_sequence = 1 AND object_ordinal = $2",
    )
    .bind(run_id.as_str())
    .bind(i32::try_from(object_count).expect("bounded object count"))
    .execute(&mutation_pool)
    .await
    .expect("remove extra object row");

    update_object_snapshot(&mutation_pool, &run_id, first.ordinal, second).await;
    update_object_snapshot(&mutation_pool, &run_id, second.ordinal, first).await;
    assert!(matches!(
        reader.load_verified(&run_id).await,
        Err(StructuredStoreError::InvalidHistory)
    ));
    update_object_snapshot(&mutation_pool, &run_id, first.ordinal, first).await;
    update_object_snapshot(&mutation_pool, &run_id, second.ordinal, second).await;

    sqlx::query(
        "UPDATE run_history_batch_objects SET content_digest = $3 \
          WHERE run_id = $1 AND run_sequence = 1 AND object_ordinal = $2",
    )
    .bind(run_id.as_str())
    .bind(first.ordinal)
    .bind(second.content_digest.as_str())
    .execute(&mutation_pool)
    .await
    .expect("mismatch object content reference");
    assert!(matches!(
        reader.load_verified(&run_id).await,
        Err(StructuredStoreError::InvalidHistory)
    ));
    update_object_snapshot(&mutation_pool, &run_id, first.ordinal, first).await;

    sqlx::query(
        "ALTER TABLE run_history_batch_objects \
         DROP CONSTRAINT run_history_batch_objects_json_v1",
    )
    .execute(&mutation_pool)
    .await
    .expect("remove object frame bound for hostile mutation");
    sqlx::query(
        "UPDATE run_history_batch_objects \
            SET canonical_json = '\"' || repeat('a', 16777216) || '\"' \
          WHERE run_id = $1 AND run_sequence = 1 AND object_ordinal = $2",
    )
    .bind(run_id.as_str())
    .bind(first.ordinal)
    .execute(&mutation_pool)
    .await
    .expect("persist oversized hostile object row");
    assert!(matches!(
        reader.load_verified(&run_id).await,
        Err(StructuredStoreError::InvalidHistory)
    ));
    update_object_snapshot(&mutation_pool, &run_id, first.ordinal, first).await;
    sqlx::query(
        "ALTER TABLE run_history_batch_objects \
         ADD CONSTRAINT run_history_batch_objects_json_v1 CHECK ( \
             octet_length(canonical_json) BETWEEN 1 AND 16777216 \
         )",
    )
    .execute(&mutation_pool)
    .await
    .expect("restore object frame bound");
    reader
        .load_verified(&run_id)
        .await
        .expect("restored object rows must refold");

    drop(reader);
    drop(writer);
    mutation_pool.close().await;
    store_pool_control.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn numeric_batch_order_refolds_across_the_tenth_append() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/numeric-batch-order").expect("operation id");
    let (program_verifier, document) = qualified_program_with_state_count(operation_id.clone(), 10);
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let store_pool = database.application_pool().await;
    let store_pool_control = store_pool.clone();
    let store = open_structured_authoritative(
        store_pool,
        TestAuthoritativeWriterFence,
        program_verifier,
        physical_verifier,
    )
    .await
    .expect("qualify numeric-order store");
    let (writer, reader) = store.split();
    let run_id = run_id(33);
    writer
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            "numeric-order-admit",
        ))
        .await
        .expect("admit numeric-order fixture");
    for transition in 0..10 {
        let verified = reader
            .load_verified(&run_id)
            .await
            .expect("refold numeric-order prefix");
        writer
            .commit_state_transition(
                verified,
                &StateTransitionProposal::success(
                    AppendRequestId::new(format!("numeric-order-transition-{transition}"))
                        .expect("transition append id"),
                    ProposedCanonicalValue::from_value(&Value {
                        value: u64::try_from(transition).expect("transition index") + 8,
                    })
                    .expect("transition value"),
                    mfm_facts::FactSet::empty(),
                ),
            )
            .await
            .expect("commit sequential transition");
    }
    assert!(matches!(
        reader
            .load_verified(&run_id)
            .await
            .expect("refold eleven numeric batches")
            .frontier(),
        StructuredFrontier::Complete
    ));
    let audit_pool = database.independent_pool().await;
    let batches = load_normalized_batches(&audit_pool, &run_id).await;
    assert_eq!(
        batches
            .iter()
            .map(|batch| batch.head.run_sequence)
            .collect::<Vec<_>>(),
        (1..=11).collect::<Vec<_>>()
    );
    audit_pool.close().await;

    drop(reader);
    drop(writer);
    store_pool_control.close().await;
    database.cleanup().await;
}

async fn run_fresh_process_worker(database: &TestDatabase, mode: &str) {
    let output = tokio::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("structured_history_fresh_process_worker")
        .arg("--nocapture")
        .env("DATABASE_URL", database.application_database_url())
        .env(FRESH_PROCESS_SCHEMA_ENV, &database.schema)
        .env(FRESH_PROCESS_MODE_ENV, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .expect("run fresh-process structured-history worker");
    assert!(
        output.status.success(),
        "fresh-process worker {mode} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn qualified_program(
    operation_id: StableId,
) -> (
    Arc<dyn mfm_store::structured::StructuredProgramVerifier>,
    mfm_spec::structured::CertifiedProgramDocument,
) {
    qualified_program_with_state_count(operation_id, 1)
}

fn qualified_program_with_state_count(
    operation_id: StableId,
    state_count: usize,
) -> (
    Arc<dyn mfm_store::structured::StructuredProgramVerifier>,
    mfm_spec::structured::CertifiedProgramDocument,
) {
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    let state_contract_ref = state_contract::<CopyState>()
        .expect("state contract")
        .state_contract_ref;
    let descriptor = implementation_descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_contract_ref,
        "copy-state",
    );
    assembly
        .register_state::<CopyState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame: StateFrame<'_, Value>| {
                    ProposedStateOutcome::Success(frame.input().clone())
                }),
            },
        )
        .expect("state registration");
    let authored = sequential_state_program(operation_id.clone(), state_count);
    assembly
        .register_entry_point(
            operation_id.clone(),
            authored.clone(),
            StructuredExpansionProfile {
                policies: Vec::new(),
                max_occurrences: u32::try_from(state_count + 4).expect("bounded occurrences"),
                max_declarations: u32::try_from(state_count + 4).expect("bounded declarations"),
                max_lanes: 4,
                max_fan_out_depth: 2,
                max_branch_depth: 4,
            },
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(authored)
        .expect("certified program")
        .into_document();
    let (verifier, processes) = split_qualified_registry(registry);
    drop(processes);
    (verifier, document)
}

fn qualified_fact_program(
    operation_id: StableId,
) -> (
    Arc<dyn mfm_store::structured::StructuredProgramVerifier>,
    mfm_spec::structured::CertifiedProgramDocument,
) {
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    let fact_descriptor = fact_descriptor().expect("fact descriptor");
    assembly
        .register_fact_descriptor(fact_descriptor)
        .expect("fact descriptor registration");
    let state_contract_ref = state_contract::<FactCopyState>()
        .expect("fact state contract")
        .state_contract_ref;
    let descriptor = implementation_descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_contract_ref,
        "fact-copy-state",
    );
    assembly
        .register_state::<FactCopyState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame: StateFrame<'_, Value>| {
                    let subject = frame.input().clone();
                    let response = Value {
                        value: subject.value + 1,
                    };
                    ProposedStateOutcome::success_with_facts(
                        response.clone(),
                        fact_set(subject, response),
                    )
                }),
            },
        )
        .expect("fact state registration");
    let authored = fact_state_program(operation_id.clone());
    assembly
        .register_entry_point(
            operation_id.clone(),
            authored.clone(),
            StructuredExpansionProfile {
                policies: Vec::new(),
                max_occurrences: 8,
                max_declarations: 8,
                max_lanes: 4,
                max_fan_out_depth: 2,
                max_branch_depth: 4,
            },
        )
        .expect("fact entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified fact registry");
    let document = registry
        .certifier(&operation_id)
        .expect("fact certifier")
        .certify(authored)
        .expect("certified fact program")
        .into_document();
    let (verifier, processes) = split_qualified_registry(registry);
    drop(processes);
    (verifier, document)
}

struct QualifiedFactScanFixture {
    producer_operation: StableId,
    consumer_operation: StableId,
    producer_document: mfm_spec::structured::CertifiedProgramDocument,
    consumer_document: mfm_spec::structured::CertifiedProgramDocument,
    program_verifier: Arc<dyn mfm_store::structured::StructuredProgramVerifier>,
    processes: RuntimeProcessRegistry,
    source_object: HistoryObject,
    request: FactSelectionRequest,
}

fn qualified_fact_scan_fixture() -> QualifiedFactScanFixture {
    qualified_fact_scan_fixture_with_source_operation(None)
}

fn qualified_fact_scan_fixture_excluding_producer() -> QualifiedFactScanFixture {
    qualified_fact_scan_fixture_with_source_operation(Some(
        stable("mfm.postgres.fixture/excluded-fact-scan-producer")
            .expect("excluded source operation"),
    ))
}

fn qualified_fact_scan_fixture_with_source_operation(
    source_operation: Option<StableId>,
) -> QualifiedFactScanFixture {
    let producer_operation =
        stable("mfm.postgres.fixture/fact-scan-producer").expect("producer operation");
    let consumer_operation =
        stable("mfm.postgres.fixture/fact-scan-consumer").expect("consumer operation");
    let fact_descriptor = fact_descriptor().expect("fact descriptor");
    let source_manifest = PriorRunFactSourceManifest::new(vec![PriorRunFactSourceRule::new(
        source_operation.unwrap_or_else(|| producer_operation.clone()),
        Vec::new(),
        vec![fact_descriptor.descriptor_ref.clone()],
    )
    .expect("producer source rule")])
    .expect("source manifest");
    let source_object = source_manifest
        .to_history_object()
        .expect("source manifest object");
    let request = FactSelectionRequest::new(
        source_object.content_ref.clone(),
        FactSelectionScanBounds::new(1, 64, 65_536, 16, 65_536).expect("fact scan bounds"),
        vec![FactSelectionQuery::new(
            fact_descriptor.descriptor_ref.clone(),
            CanonicalFactPredicate::from_canonical_json(br#"{"value":7}"#).expect("fact predicate"),
            None,
            FactOrdering::Ascending,
            FactSelectionLimit::new(4).expect("fact selection limit"),
            FactTieBreak::FactIdentityAscending,
        )
        .expect("fact query")],
    )
    .expect("fact request");

    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    assembly
        .register_fact_descriptor(fact_descriptor)
        .expect("fact descriptor registration");
    let producer_state_ref = state_contract::<FactCopyState>()
        .expect("producer state contract")
        .state_contract_ref;
    let producer_descriptor = implementation_descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        producer_state_ref,
        "fact-scan-producer-state",
    );
    assembly
        .register_state::<FactCopyState>(
            producer_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame: StateFrame<'_, Value>| {
                    let subject = frame.input().clone();
                    let response = Value {
                        value: subject.value + 1,
                    };
                    ProposedStateOutcome::success_with_facts(
                        response.clone(),
                        fact_set(subject, response),
                    )
                }),
            },
        )
        .expect("producer state registration");

    let authored_request = request.clone();
    let consumer_state_ref = state_contract::<FactReadState>()
        .expect("consumer state contract")
        .state_contract_ref;
    let consumer_descriptor = implementation_descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        consumer_state_ref,
        "fact-scan-consumer-state",
    );
    assembly
        .register_state::<FactReadState>(
            consumer_descriptor,
            StructuredStateCallbacks::Read {
                request: Arc::new(move |_| authored_request.clone()),
                settle: Arc::new(|_frame, observation| {
                    let output = match observation.observation() {
                        CommittedObservation::Returned(returned) => {
                            let response: PriorRunFactSelectionResponse =
                                serde_json::from_str(returned.canonical_response_json())
                                    .expect("typed fact response");
                            response.query_results[0]
                                .selected
                                .first()
                                .map(|selected| {
                                    serde_json::from_str::<Value>(&selected.response_canonical_json)
                                        .expect("selected fact response")
                                })
                                .unwrap_or(Value { value: 0 })
                        }
                        CommittedObservation::SafeFailure(_) => Value { value: 0 },
                    };
                    StateSettlement::Proposed(ProposedStateOutcome::Success(output))
                }),
                reviewed_safe_failures: [
                    FactSelectionReadFailureCode::StoreUnavailable,
                    FactSelectionReadFailureCode::PublicationBoundExceeded,
                    FactSelectionReadFailureCode::FactBoundExceeded,
                    FactSelectionReadFailureCode::RetainedSourceBoundExceeded,
                    FactSelectionReadFailureCode::SelectedResultBoundExceeded,
                    FactSelectionReadFailureCode::ResponseBoundExceeded,
                ]
                .into_iter()
                .enumerate()
                .map(|(ordinal, code)| {
                    ReviewedSafeFailureCase::new(
                        Value {
                            value: u64::try_from(ordinal).expect("bounded failure ordinal"),
                        },
                        FactSelectionReadFailure::new(code),
                        ProposedStateOutcome::Success(Value { value: 0 }),
                    )
                })
                .collect(),
            },
        )
        .expect("consumer state registration");

    let producer_program = fact_state_program(producer_operation.clone());
    let consumer_program = fact_read_program(consumer_operation.clone());
    assembly
        .register_entry_point(
            producer_operation.clone(),
            producer_program.clone(),
            fact_scan_profile(),
        )
        .expect("producer entry point");
    assembly
        .register_entry_point(
            consumer_operation.clone(),
            consumer_program.clone(),
            fact_scan_profile(),
        )
        .expect("consumer entry point");
    let expected_entry_points = [producer_operation.clone(), consumer_operation.clone()];
    let registry = assembly
        .build(&expected_entry_points)
        .expect("qualified fact scan registry");
    let producer_document = registry
        .certifier(&producer_operation)
        .expect("producer certifier")
        .certify(producer_program)
        .expect("producer certification")
        .into_document();
    let consumer_document = registry
        .certifier(&consumer_operation)
        .expect("consumer certifier")
        .certify(consumer_program)
        .expect("consumer certification")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    QualifiedFactScanFixture {
        producer_operation,
        consumer_operation,
        producer_document,
        consumer_document,
        program_verifier,
        processes,
        source_object,
        request,
    }
}

fn fact_scan_profile() -> StructuredExpansionProfile {
    StructuredExpansionProfile {
        policies: Vec::new(),
        max_occurrences: 8,
        max_declarations: 8,
        max_lanes: 4,
        max_fan_out_depth: 2,
        max_branch_depth: 4,
    }
}

async fn drive_qualified_fact_scan<B: StructuredHistoryBackend>(
    writer: StructuredRunHistoryWriter<B>,
    reader: &StructuredRunHistoryReader<B>,
    fixture: QualifiedFactScanFixture,
) -> String {
    let QualifiedFactScanFixture {
        producer_operation,
        consumer_operation,
        producer_document,
        consumer_document,
        program_verifier: _,
        processes,
        source_object,
        request,
    } = fixture;
    assert_eq!(
        request
            .scan_bounds()
            .expect("fixture scan bounds")
            .maximum_publications(),
        1
    );
    let runtime = Runtime::new(writer, processes);
    let producer_run = run_id(80);
    let consumer_run = run_id(81);

    runtime
        .admit_run(fact_scan_admission(
            producer_run.clone(),
            producer_operation,
            producer_document,
            admission_material(80),
            Value { value: 7 },
            "fact-scan-producer-admit",
            80,
        ))
        .await
        .expect("admit fact scan producer");
    assert_eq!(
        runtime
            .drive_once(&producer_run)
            .await
            .expect("drive fact scan producer"),
        DriveOutcome::TransitionCommitted { closed: true }
    );

    runtime
        .admit_run(fact_scan_admission(
            consumer_run.clone(),
            consumer_operation,
            consumer_document,
            admission_material_with_source(81, source_object),
            Value { value: 7 },
            "fact-scan-consumer-admit",
            81,
        ))
        .await
        .expect("admit fact scan consumer");
    assert_eq!(
        runtime
            .drive_once(&consumer_run)
            .await
            .expect("authorize, invoke, and observe fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        runtime
            .drive_once(&consumer_run)
            .await
            .expect("settle fact scan response"),
        DriveOutcome::TransitionCommitted { closed: true }
    );

    let response = retained_fact_response(reader, &consumer_run).await;
    let response_value: PriorRunFactSelectionResponse =
        serde_json::from_str(&response).expect("decode retained fact selection response");
    assert_eq!(
        response_value.request_digest,
        request.request_digest().expect("fixture request digest")
    );
    assert_eq!(response_value.attestation.frontier.fact_order, 1);
    let [query] = response_value.query_results.as_slice() else {
        panic!("fact scanner must return its one authored query");
    };
    let [selected] = query.selected.as_slice() else {
        panic!("fact scanner must select its one matching producer fact");
    };
    assert_eq!(selected.producer_transition_ref.run_id, producer_run);
    assert_eq!(selected.subject_canonical_json, r#"{"value":7}"#);
    assert_eq!(selected.response_canonical_json, r#"{"value":8}"#);

    let producer = reader
        .load_verified(&producer_run)
        .await
        .expect("publicly recompute producer prefix");
    assert_eq!(
        selected.subject_canonical_json,
        producer
            .object(&selected.subject.value_ref)
            .expect("retained selected subject")
            .canonical_json
    );
    assert_eq!(
        selected.response_canonical_json,
        producer
            .object(&selected.response.value_ref)
            .expect("retained selected response")
            .canonical_json
    );
    assert_eq!(
        selected.claim_canonical_json,
        producer
            .object(&selected.claim_ref)
            .expect("retained selected claim closure")
            .canonical_json
    );

    let consumer = reader
        .load_verified(&consumer_run)
        .await
        .expect("publicly recompute consumer history");
    let output = consumer
        .records()
        .iter()
        .find_map(|assigned| match &assigned.record {
            RunRecord::StateTransitionCommitted(transition) => match &transition.outcome {
                StateOutcomeRef::Success(value) => Some(&value.value.value_ref),
                StateOutcomeRef::Failure(_) => None,
            },
            _ => None,
        })
        .and_then(|value_ref| consumer.object(value_ref))
        .expect("consumer transition output")
        .decode::<Value>()
        .expect("typed consumer transition output");
    assert_eq!(output, Value { value: 8 });
    response
}

async fn retained_fact_response<B: StructuredHistoryBackend>(
    reader: &StructuredRunHistoryReader<B>,
    run_id: &RunId,
) -> String {
    let verified = reader
        .load_verified(run_id)
        .await
        .expect("publicly recompute retained fact response");
    let returned = verified
        .records()
        .iter()
        .find_map(|assigned| match &assigned.record {
            RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                ObservationOutcome::Returned { value } => verified.object(&value.value_ref),
                _ => None,
            },
            _ => None,
        })
        .expect("retained fact response object")
        .decode::<FactSelectionReadResponse>()
        .expect("typed retained fact response");
    returned.canonical_response_json().to_owned()
}

fn sequential_state_program(
    operation_id: StableId,
    state_count: usize,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    assert!(state_count > 0);
    let mut builder =
        OperationBuilder::<Value, Never>::new(operation_id, stable("root").expect("root id"))
            .expect("builder");
    let mut output = builder
        .input::<Value>(stable("input").expect("input id"))
        .expect("input root");
    for ordinal in 0..state_count {
        output = builder
            .root()
            .state::<CopyState>(
                stable(&format!("copy-{ordinal}")).expect("copy label"),
                &output,
            )
            .expect("copy state")
            .infallible()
            .expect("infallible state");
    }
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("program")
}

fn fact_state_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Value, Never>::new(operation_id, stable("root").expect("root id"))
            .expect("builder");
    let input = builder
        .input::<Value>(stable("input").expect("input id"))
        .expect("input root");
    let output = builder
        .root()
        .state::<FactCopyState>(stable("fact-copy").expect("fact state label"), &input)
        .expect("fact state")
        .infallible()
        .expect("infallible fact state");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("fact program")
}

fn fact_read_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Value, Never>::new(operation_id, stable("root").expect("root id"))
            .expect("builder");
    let input = builder
        .input::<Value>(stable("input").expect("input id"))
        .expect("input root");
    let output = builder
        .root()
        .state::<FactReadState>(stable("fact-read").expect("fact read label"), &input)
        .expect("fact read state")
        .infallible()
        .expect("infallible fact read state");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("fact read program")
}

fn fact_descriptor() -> mfm_program::Result<StructuredFactDescriptor> {
    let contract_ref = mfm_spec::structured::structured_value_contract_ref::<Value>()?;
    StructuredFactDescriptor::new(
        stable("mfm.postgres.fixture/value-fact")?,
        contract_ref.clone(),
        contract_ref,
    )
    .map_err(Into::into)
}

fn fact_set(subject: Value, response: Value) -> FactSet {
    let descriptor = fact_descriptor().expect("fact descriptor");
    FactSet::one(
        FactProposal::new(
            0,
            descriptor.descriptor_ref,
            proposed_fact_value(subject),
            proposed_fact_value(response),
        )
        .expect("fact proposal"),
    )
}

fn proposed_fact_value(value: Value) -> ProposedFactValue {
    let contract =
        mfm_spec::structured::structured_value_contract::<Value>().expect("fact value contract");
    let json = serde_json::to_string(&value).expect("fact value JSON");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical fact value");
    ProposedFactValue::new(
        contract.schema_id().clone(),
        contract.semantic_type_id().clone(),
        contract.role().clone(),
        contract.media_type(),
        contract.evidence_contract_ref().clone(),
        canonical,
    )
    .expect("proposed fact value")
}

fn implementation_descriptor(
    assembly: &mut ProgramRegistryBuilder,
    component_kind: StructuredComponentKind,
    semantic_contract_ref: mfm_ids::ContentRef,
    suffix: &str,
) -> SecretFreeImplementationDescriptor {
    let executable_identity_ref = assembly
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable("mfm.postgres.fixture/test-executable").expect("executable"),
        })
        .expect("executable identity");
    let qualification_artifact_ref = assembly
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable("mfm.postgres.fixture/test-qualification")
                .expect("qualification"),
        })
        .expect("qualification artifact");
    SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id: stable(&format!("mfm.postgres.fixture/{suffix}-implementation"))
            .expect("implementation id"),
        executable_identity_ref,
        qualification_artifact_ref,
    }
}

fn admission(
    run_id: RunId,
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    append_id: &str,
) -> StructuredAdmissionRequest {
    StructuredAdmissionRequest::new(
        run_id,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32))).expect("tenant"),
        InvocationIdentity::new("00000000-0000-4000-8000-000000000001").expect("invocation"),
        operation_id,
        document,
        admission_material(9),
        vec![ProposedCanonicalValue::from_value(&Value { value: 7 }).expect("initial value")],
        AppendRequestId::new(append_id).expect("append id"),
    )
}

fn fact_scan_admission(
    run_id: RunId,
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    material: StructuredAdmissionMaterial,
    input: Value,
    append_id: &str,
    invocation_discriminator: u8,
) -> StructuredAdmissionRequest {
    fact_scan_admission_for_tenant(
        run_id,
        operation_id,
        document,
        material,
        input,
        append_id,
        invocation_discriminator,
        '2',
    )
}

#[allow(clippy::too_many_arguments)]
fn fact_scan_admission_for_tenant(
    run_id: RunId,
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    material: StructuredAdmissionMaterial,
    input: Value,
    append_id: &str,
    invocation_discriminator: u8,
    tenant_discriminator: char,
) -> StructuredAdmissionRequest {
    StructuredAdmissionRequest::new(
        run_id,
        TenantScopeId::new(format!(
            "{}{}",
            TenantScopeId::PREFIX,
            tenant_discriminator.to_string().repeat(32)
        ))
        .expect("tenant"),
        InvocationIdentity::new(format!(
            "00000000-0000-4000-8000-{invocation_discriminator:012}"
        ))
        .expect("fact scan invocation"),
        operation_id,
        document,
        material,
        vec![ProposedCanonicalValue::from_value(&input).expect("fact scan input")],
        AppendRequestId::new(append_id).expect("fact scan append id"),
    )
}

fn admission_material_with_source(
    discriminator: u8,
    source_object: HistoryObject,
) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "mfm.postgres.fixture.configuration",
            discriminator,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "mfm.postgres.fixture.context",
            discriminator,
        ),
        source_object,
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "mfm.postgres.fixture.routing",
            discriminator,
        ),
        Vec::new(),
    )
    .expect("fact scan admission material")
}

fn admission_material(discriminator: u8) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "mfm.postgres.fixture.configuration",
            discriminator,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "mfm.postgres.fixture.context",
            discriminator,
        ),
        mfm_journal::structured::PriorRunFactSourceManifest::new(Vec::new())
            .and_then(|manifest| manifest.to_history_object())
            .expect("source manifest"),
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "mfm.postgres.fixture.routing",
            discriminator,
        ),
        Vec::new(),
    )
    .expect("admission material")
}

fn admission_object(object_type: &str, schema: &str, discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        stable(object_type).expect("object type"),
        SchemaId::new(
            schema,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, schema.as_bytes()[0]]),
        )
        .expect("schema"),
        "{\"entries\":[]}",
    )
    .expect("admission object")
}

fn run_id(discriminator: u8) -> RunId {
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[discriminator, 5]),
    )
}

fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

struct TestDatabase {
    admin_pool: PgPool,
    database_url: String,
    schema: String,
    application_login: TestLogin,
    combined_login: TestLogin,
    maintenance_login: TestLogin,
}

struct TestLogin {
    role_name: String,
    password: String,
}

struct ObjectRowSnapshot {
    ordinal: i32,
    object_type: String,
    content_schema_id: String,
    content_digest: String,
    canonical_json: String,
}

impl ObjectRowSnapshot {
    fn from_row(row: &sqlx::postgres::PgRow) -> Self {
        Self {
            ordinal: row.try_get("object_ordinal").expect("object ordinal"),
            object_type: row.try_get("object_type").expect("object type"),
            content_schema_id: row.try_get("content_schema_id").expect("content schema id"),
            content_digest: row.try_get("content_digest").expect("content digest"),
            canonical_json: row.try_get("canonical_json").expect("canonical object"),
        }
    }
}

impl TestDatabase {
    async fn create() -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
        let admin_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("connect PostgreSQL administrator");
        let schema = unique_schema();
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("create isolated schema");
        let migration_pool = isolated_pool(&database_url, &schema).await;
        MIGRATOR
            .run(&migration_pool)
            .await
            .expect("migrate structured history schema");
        migration_pool.close().await;
        let application_login = TestLogin::create(
            &admin_pool,
            &schema,
            "application",
            &["mfm_store_qualification", "mfm_store_application"],
        )
        .await;
        let combined_login = TestLogin::create(
            &admin_pool,
            &schema,
            "combined",
            &[
                "mfm_store_qualification",
                "mfm_store_application",
                "mfm_store_configuration_maintenance",
            ],
        )
        .await;
        let maintenance_login = TestLogin::create(
            &admin_pool,
            &schema,
            "maintenance",
            &[
                "mfm_store_qualification",
                "mfm_store_configuration_maintenance",
            ],
        )
        .await;
        Self {
            admin_pool,
            database_url,
            schema,
            application_login,
            combined_login,
            maintenance_login,
        }
    }

    async fn independent_pool(&self) -> PgPool {
        isolated_pool(&self.database_url, &self.schema).await
    }

    async fn application_pool(&self) -> PgPool {
        self.application_login
            .isolated_pool(&self.database_url, &self.schema)
            .await
    }

    async fn combined_pool(&self) -> PgPool {
        self.combined_login
            .isolated_pool(&self.database_url, &self.schema)
            .await
    }

    async fn maintenance_pool(&self) -> PgPool {
        self.maintenance_login
            .isolated_pool(&self.database_url, &self.schema)
            .await
    }

    fn application_database_url(&self) -> String {
        self.application_login.database_url(&self.database_url)
    }

    async fn store_scope_id(&self) -> StoreScopeId {
        let pool = self.independent_pool().await;
        let value = sqlx::query_scalar::<_, String>(
            "SELECT store_scope_id FROM store_identity WHERE singleton",
        )
        .fetch_one(&pool)
        .await
        .expect("load store scope");
        pool.close().await;
        StoreScopeId::new(value).expect("stored scope")
    }

    async fn cleanup(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&self.admin_pool)
        .await
        .expect("drop isolated schema");
        self.application_login.drop(&self.admin_pool).await;
        self.combined_login.drop(&self.admin_pool).await;
        self.maintenance_login.drop(&self.admin_pool).await;
        self.admin_pool.close().await;
    }
}

impl TestLogin {
    async fn create(
        admin_pool: &PgPool,
        schema: &str,
        purpose: &str,
        memberships: &[&str],
    ) -> Self {
        let discriminator = schema
            .strip_prefix("mfm_structured_")
            .expect("test schema prefix")
            .replace('_', "");
        let role_name = format!("mfm_test_{}_{discriminator}", &purpose[..1]);
        let password = format!("MfmTest{discriminator}{}", purpose.len());
        sqlx::query(AssertSqlSafe(format!(
            "CREATE ROLE {role_name} LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE \
             NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD '{password}'"
        )))
        .execute(admin_pool)
        .await
        .expect("create restricted PostgreSQL test login");
        sqlx::query(AssertSqlSafe(format!(
            "GRANT {} TO {role_name} WITH INHERIT FALSE, SET TRUE",
            memberships.join(", ")
        )))
        .execute(admin_pool)
        .await
        .expect("grant exact PostgreSQL test memberships");
        Self {
            role_name,
            password,
        }
    }

    async fn isolated_pool(&self, database_url: &str, schema: &str) -> PgPool {
        let options = self
            .connect_options(database_url)
            .options([("search_path", schema)]);
        PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("connect restricted PostgreSQL test login")
    }

    fn database_url(&self, database_url: &str) -> String {
        self.connect_options(database_url)
            .to_url_lossy()
            .to_string()
    }

    fn connect_options(&self, database_url: &str) -> PgConnectOptions {
        database_url
            .parse::<PgConnectOptions>()
            .expect("parse DATABASE_URL")
            .username(&self.role_name)
            .password(&self.password)
    }

    async fn drop(self, admin_pool: &PgPool) {
        sqlx::query(AssertSqlSafe(format!("DROP ROLE {}", self.role_name)))
            .execute(admin_pool)
            .await
            .expect("drop restricted PostgreSQL test login");
    }
}

async fn insert_object_snapshot(pool: &PgPool, run_id: &RunId, snapshot: &ObjectRowSnapshot) {
    sqlx::query(
        "INSERT INTO run_history_batch_objects ( \
            run_id, run_sequence, object_ordinal, object_type, content_schema_id, \
            content_digest, canonical_json \
         ) VALUES ($1, 1, $2, $3, $4, $5, $6)",
    )
    .bind(run_id.as_str())
    .bind(snapshot.ordinal)
    .bind(snapshot.object_type.as_str())
    .bind(snapshot.content_schema_id.as_str())
    .bind(snapshot.content_digest.as_str())
    .bind(snapshot.canonical_json.as_str())
    .execute(pool)
    .await
    .expect("restore object snapshot");
}

async fn update_object_snapshot(
    pool: &PgPool,
    run_id: &RunId,
    ordinal: i32,
    snapshot: &ObjectRowSnapshot,
) {
    sqlx::query(
        "UPDATE run_history_batch_objects \
            SET object_type = $3, content_schema_id = $4, content_digest = $5, \
                canonical_json = $6 \
          WHERE run_id = $1 AND run_sequence = 1 AND object_ordinal = $2",
    )
    .bind(run_id.as_str())
    .bind(ordinal)
    .bind(snapshot.object_type.as_str())
    .bind(snapshot.content_schema_id.as_str())
    .bind(snapshot.content_digest.as_str())
    .bind(snapshot.canonical_json.as_str())
    .execute(pool)
    .await
    .expect("restore object payload");
}

async fn isolated_pool(database_url: &str, schema: &str) -> PgPool {
    let options = database_url
        .parse::<PgConnectOptions>()
        .expect("parse DATABASE_URL")
        .options([("search_path", schema)]);
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect isolated schema pool")
}

async fn load_normalized_batches(pool: &PgPool, run_id: &RunId) -> Vec<CommittedBatch> {
    let rows = sqlx::query(
        "SELECT run_sequence::text AS run_sequence, batch_envelope_json \
           FROM run_history_batches WHERE run_id = $1 \
          ORDER BY run_history_batches.run_sequence",
    )
    .bind(run_id.as_str())
    .fetch_all(pool)
    .await
    .expect("load normalized batch envelopes");
    let mut batches = Vec::with_capacity(rows.len());
    for row in rows {
        let run_sequence = row
            .try_get::<String, _>("run_sequence")
            .expect("batch sequence");
        let mut envelope = serde_json::from_str::<serde_json::Value>(
            &row.try_get::<String, _>("batch_envelope_json")
                .expect("batch envelope"),
        )
        .expect("decode batch envelope");
        let object_rows = sqlx::query(
            "SELECT object_type, content_schema_id, content_digest, canonical_json \
               FROM run_history_batch_objects \
              WHERE run_id = $1 AND run_sequence = $2::numeric ORDER BY object_ordinal",
        )
        .bind(run_id.as_str())
        .bind(run_sequence)
        .fetch_all(pool)
        .await
        .expect("load normalized batch objects");
        let objects = object_rows
            .iter()
            .map(|object| {
                serde_json::json!({
                    "object_type": object
                        .try_get::<String, _>("object_type")
                        .expect("object type"),
                    "content_ref": {
                        "schema_id": object
                            .try_get::<String, _>("content_schema_id")
                            .expect("content schema"),
                        "content_digest": object
                            .try_get::<String, _>("content_digest")
                            .expect("content digest"),
                    },
                    "canonical_json": object
                        .try_get::<String, _>("canonical_json")
                        .expect("canonical object"),
                })
            })
            .collect::<Vec<_>>();
        {
            let envelope_fields = envelope.as_object_mut().expect("batch envelope object");
            envelope_fields
                .remove("object_count")
                .expect("batch envelope object count");
            envelope_fields.insert("objects".to_owned(), serde_json::Value::Array(objects));
        }
        batches.push(serde_json::from_value(envelope).expect("reconstruct committed batch"));
    }
    batches
}

fn unique_schema() -> String {
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    format!(
        "mfm_structured_{}_{}_{}",
        std::process::id(),
        timestamp,
        counter
    )
}
