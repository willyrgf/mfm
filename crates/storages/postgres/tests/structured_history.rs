use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mfm_canonical::limits::MAX_CONFIGURATION_REVISION_BYTES;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_certify::structured::ProgramRegistryBuilder;
use mfm_facts::{
    CanonicalFactPredicate, FactOrdering, FactProposal, FactSelectionLimit, FactSelectionQuery,
    FactSelectionReadFailure, FactSelectionReadResponse, FactSelectionRequest,
    FactSelectionScanBounds, FactSet, FactTieBreak, ProposedFactValue,
};
use mfm_ids::{
    AppendRequestId, ContentDigest, DigestAlgorithm, InvocationIdentity, RunId, SchemaId, StableId,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, CommittedBatch, HistoryObject, ObservationOutcome,
    PriorRunFactSelectionResponse, PriorRunFactSourceManifest, PriorRunFactSourceRule, RunRecord,
    SemanticHead, StateOutcomeRef, TenantFactCoordinate, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_program::structured::{
    state_contract, Direct, Never, OperationBuilder, PriorRunFactSelectionCapability, Pure, Read,
    SafeFailureNotApplicable, SafeFailureSuccessOnly, State, StateFrame, StateSettlement,
    StructuredStateCallbacks,
};
use mfm_program_derive::MfmValue;
use mfm_replay::portable::{
    AuthorizedExportClosure, ExportKind, PortableFixation, PortableRunExport, ReplayTrustSnapshot,
    RetainedPhysicalReleaseTrust, StoreCheckpointTrust,
};
use mfm_replay::structured::project_replay_result;
use mfm_runtime::history::{HistoryAppendOutcome, StructuredAdmissionCommand};
use mfm_runtime::structured::{DriveOutcome, Runtime, RuntimeFaultCode, RuntimeStoreFaultKind};
use mfm_spec::structured::{
    ProposedStateOutcome, SecretFreeExecutableIdentity, SecretFreeImplementationDescriptor,
    SecretFreeQualificationArtifact, StructuredComponentKind, StructuredExpansionProfile,
    StructuredFactDescriptor,
};
use mfm_storage_postgres::{
    open_configuration_maintenance, open_structured_authoritative,
    open_structured_authoritative_with_configuration, open_test_application_sessions,
    open_test_combined_sessions, open_test_configuration_sessions, ApplicationTargetSessions,
    CombinedTargetSessions, ConfigurationMaintenanceSessions, PostgresStoreError,
    PostgresStructuredHistoryBackend, TestLoginCredential, TestTargetCredentials,
};
use mfm_store::structured::{
    assemble_in_memory_runtime, AssembledStructuredRuntime, ConfigurationAppendRequest,
    ConfigurationHistoryStore, ConfigurationRevision, ConfigurationStreamKey, ExportRunReader,
    MemoryConfigurationHistoryBackend, PhysicalBindingAuthorization, PhysicalBindingSupersession,
    PhysicalTargetIdentity, ProposedCanonicalValue, PublicPhysicalBindingVerifier,
    RegistryProgramVerifier, RunEvidenceStatus, StructuredAdmissionMaterial,
    StructuredHistoryBackend, StructuredStoreError, StructuredStoreIdentity,
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

impl mfm_authority_seal::PhysicalBindingVerifierSeal for NoPhysicalBindings {}

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

struct AcceptRetainedRelease;

impl mfm_authority_seal::RetainedPhysicalReleaseTrustSeal for AcceptRetainedRelease {}

impl RetainedPhysicalReleaseTrust for AcceptRetainedRelease {
    fn verify(&self, _fixation: &PortableFixation, _kind: ExportKind) -> bool {
        true
    }
}

struct AcceptStoreCheckpoint;

impl mfm_authority_seal::StoreCheckpointTrustSeal for AcceptStoreCheckpoint {}

impl StoreCheckpointTrust for AcceptStoreCheckpoint {
    fn verify(
        &self,
        _fixation: &PortableFixation,
        _kind: ExportKind,
        _closure_reference: &ContentDigest,
    ) -> bool {
        true
    }
}

#[tokio::test]
async fn configured_value_history_is_durable_append_only_and_application_read_only() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured").expect("operation id");
    let (registry, _) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        database.combined_sessions().await,
        registry,
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
             $1, \
             pg_catalog.format('%I.configuration_revisions', pg_catalog.current_schema()), \
             'INSERT' \
         ) AS application_insert, \
         pg_catalog.has_table_privilege( \
             $2, \
             pg_catalog.format('%I.configuration_revisions', pg_catalog.current_schema()), \
             'INSERT' \
         ) AS maintenance_insert, \
         pg_catalog.has_table_privilege( \
             $1, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'SELECT' \
         ) AS application_head_select, \
         (pg_catalog.has_table_privilege( \
             $1, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'INSERT' \
         ) OR pg_catalog.has_table_privilege( \
             $1, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'UPDATE' \
         )) AS application_head_write, \
         (pg_catalog.has_table_privilege( \
             $2, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'SELECT' \
         ) AND pg_catalog.has_table_privilege( \
             $2, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'INSERT' \
         ) AND pg_catalog.has_table_privilege( \
             $2, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'UPDATE' \
         )) AS maintenance_head_access, \
         (pg_catalog.has_table_privilege( \
             $2, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'DELETE' \
         ) OR pg_catalog.has_table_privilege( \
             $2, \
             pg_catalog.format('%I.configuration_heads', pg_catalog.current_schema()), \
             'TRUNCATE' \
         )) AS maintenance_head_destructive",
    )
    .bind(&database.target_roles.configuration_reader)
    .bind(&database.target_roles.configuration_writer)
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
    database.cleanup().await;
}

#[tokio::test]
async fn configuration_acceptance_vectors_match_memory_and_postgres() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured-parity").expect("operation id");
    let (registry, _) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let (_run_history, configuration) = open_structured_authoritative_with_configuration(
        database.combined_sessions().await,
        registry,
        physical_verifier,
    )
    .await
    .expect("qualify PostgreSQL configuration store");
    let (postgres_writer, postgres_reader) = configuration.split();
    let (memory_writer, memory_reader) = ConfigurationHistoryStore::new(
        MemoryConfigurationHistoryBackend::new(database.store_scope_id().await),
    )
    .split();
    let tenant_scope_id =
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "9".repeat(32))).expect("tenant");
    let stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        tenant_scope_id.clone(),
        operation_id.clone(),
        StableId::new("mfm.postgres.fixture/configured-parity-target").expect("target"),
    );
    let contract = admission_object(
        ADMISSION_CONFIGURATION_OBJECT_TYPE,
        "mfm.postgres.fixture.configured-parity-contract",
        39,
    )
    .content_ref;

    let positive_value =
        ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("positive value");
    let postgres_first = postgres_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-parity-positive").expect("append id"),
            contract.clone(),
            positive_value.clone(),
        ))
        .await;
    let memory_first = memory_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-parity-positive").expect("append id"),
            contract.clone(),
            positive_value,
        ))
        .await;
    assert_eq!(postgres_first, memory_first);
    let first = postgres_first.expect("positive append");

    // Derive the fixed serialized-revision overhead from a committed successor so the
    // boundary vector remains exact even if the qualified identifiers change length.
    let boundary_append_ids = [
        "postgres-configured-parity-boundary-a",
        "postgres-configured-parity-boundary-b",
        "postgres-configured-parity-boundary-c",
    ];
    let sizing_value = ProposedCanonicalValue::from_json(r#""sizing""#).expect("sizing value");
    let postgres_sizing = postgres_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new(boundary_append_ids[0]).expect("sizing append id"),
            contract.clone(),
            sizing_value.clone(),
        ))
        .await;
    let memory_sizing = memory_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new(boundary_append_ids[0]).expect("sizing append id"),
            contract.clone(),
            sizing_value,
        ))
        .await;
    assert_eq!(postgres_sizing, memory_sizing);
    let sizing = postgres_sizing.expect("sizing append");
    let revision_overhead = canonical_json(&sizing)
        .expect("canonical sizing revision")
        .as_bytes()
        .len()
        .checked_sub(sizing.canonical_value().len())
        .expect("revision overhead");
    let exact_value_len = MAX_CONFIGURATION_REVISION_BYTES
        .checked_sub(revision_overhead)
        .expect("configuration revision overhead fits its bound");
    assert!(
        exact_value_len >= 2,
        "configuration value must retain JSON quotes"
    );
    let exact_value =
        ProposedCanonicalValue::from_json(&format!("\"{}\"", "x".repeat(exact_value_len - 2)))
            .expect("exact-limit value");
    let postgres_exact = postgres_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(sizing.revision_ref().clone()),
            AppendRequestId::new(boundary_append_ids[1]).expect("exact append id"),
            contract.clone(),
            exact_value.clone(),
        ))
        .await;
    let memory_exact = memory_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(sizing.revision_ref().clone()),
            AppendRequestId::new(boundary_append_ids[1]).expect("exact append id"),
            contract.clone(),
            exact_value,
        ))
        .await;
    assert_eq!(postgres_exact, memory_exact);
    let exact = postgres_exact.expect("exact-limit append");
    assert_eq!(exact.canonical_value().len(), exact_value_len);
    assert_eq!(
        canonical_json(&exact)
            .expect("canonical exact revision")
            .as_bytes()
            .len(),
        MAX_CONFIGURATION_REVISION_BYTES
    );

    let one_over_value =
        ProposedCanonicalValue::from_json(&format!("\"{}\"", "x".repeat(exact_value_len - 1)))
            .expect("one-over value");
    let postgres_one_over = postgres_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(exact.revision_ref().clone()),
            AppendRequestId::new(boundary_append_ids[2]).expect("one-over append id"),
            contract.clone(),
            one_over_value.clone(),
        ))
        .await;
    let memory_one_over = memory_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(exact.revision_ref().clone()),
            AppendRequestId::new(boundary_append_ids[2]).expect("one-over append id"),
            contract.clone(),
            one_over_value,
        ))
        .await;
    assert_eq!(postgres_one_over, memory_one_over);
    assert_eq!(
        postgres_one_over,
        Err(mfm_runtime::history::HistoryError::InvalidHistory)
    );

    // Exercise a deterministic shape corpus through both backends. Each vector uses a separate
    // stream so one invalid value cannot hide a later acceptance decision behind a stale head.
    for ordinal in 0_u32..32 {
        let json = match ordinal % 8 {
            0 => format!(r#"{{"ordinal":{ordinal}}}"#),
            1 => format!(
                r#"{{"enabled":{},"label":"vector-{ordinal}"}}"#,
                ordinal % 2 == 0
            ),
            2 => format!(
                r#"[{}, {}, {{"nested": "vector-{ordinal}"}}]"#,
                ordinal,
                ordinal + 1
            ),
            3 => format!(r#""vector-{ordinal}""#),
            4 => format!(
                r#"{{"items":[{}, {}, {}]}}"#,
                ordinal,
                ordinal + 1,
                ordinal + 2
            ),
            5 => format!(r#"{{"unicode":"café-{ordinal}","ordinal":{ordinal}}}"#),
            6 => format!(r#"{{"nested":{{"ordinal":{ordinal},"ok":true}}}}"#),
            _ => format!(r#"[true,false,null,"vector-{ordinal}"]"#),
        };
        let vector_stream = ConfigurationStreamKey::new(
            database.store_scope_id().await,
            tenant_scope_id.clone(),
            operation_id.clone(),
            StableId::new(format!(
                "mfm.postgres.fixture/configured-parity-vector-{ordinal:02}"
            ))
            .expect("vector target"),
        );
        let append_id =
            AppendRequestId::new(format!("postgres-configured-parity-vector-{ordinal:02}"))
                .expect("vector append id");
        let value = ProposedCanonicalValue::from_json(&json).expect("generated JSON value");
        let postgres_vector = postgres_writer
            .append(ConfigurationAppendRequest::new(
                vector_stream.clone(),
                None,
                append_id.clone(),
                contract.clone(),
                value.clone(),
            ))
            .await;
        let memory_vector = memory_writer
            .append(ConfigurationAppendRequest::new(
                vector_stream,
                None,
                append_id,
                contract.clone(),
                value,
            ))
            .await;
        assert_eq!(postgres_vector, memory_vector, "generated vector {ordinal}");
        assert!(
            postgres_vector.is_ok(),
            "generated vector {ordinal} must append"
        );
    }

    // Keep a bounded sequential run as a scale witness in addition to the separate-shape corpus.
    let scale_stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        tenant_scope_id,
        operation_id,
        StableId::new("mfm.postgres.fixture/configured-parity-scale").expect("scale target"),
    );
    let mut scale_predecessor: Option<ConfigurationRevision> = None;
    for sequence in 0_u32..32 {
        let value = ProposedCanonicalValue::from_json(&format!(
            r#"{{"sequence":{},"payload":"scale-{sequence}"}}"#,
            sequence
        ))
        .expect("scale JSON value");
        let append_id =
            AppendRequestId::new(format!("postgres-configured-parity-scale-{sequence:02}"))
                .expect("scale append id");
        let postgres_scale = postgres_writer
            .append(ConfigurationAppendRequest::new(
                scale_stream.clone(),
                scale_predecessor
                    .as_ref()
                    .map(|revision| revision.revision_ref().clone()),
                append_id.clone(),
                contract.clone(),
                value.clone(),
            ))
            .await;
        let memory_scale = memory_writer
            .append(ConfigurationAppendRequest::new(
                scale_stream.clone(),
                scale_predecessor
                    .as_ref()
                    .map(|revision| revision.revision_ref().clone()),
                append_id,
                contract.clone(),
                value,
            ))
            .await;
        assert_eq!(postgres_scale, memory_scale, "scale append {sequence}");
        scale_predecessor = Some(postgres_scale.expect("scale append"));
    }
    let postgres_scale_current = postgres_reader
        .resolve(&scale_stream, &contract)
        .await
        .expect("resolve PostgreSQL scale head");
    let memory_scale_current = memory_reader
        .resolve(&scale_stream, &contract)
        .await
        .expect("resolve memory scale head");
    assert_eq!(postgres_scale_current, memory_scale_current);
    assert_eq!(
        postgres_scale_current.revision(),
        scale_predecessor.as_ref().unwrap()
    );

    let stale_value = ProposedCanonicalValue::from_json(r#"{"stale":true}"#).expect("stale value");
    let postgres_stale = postgres_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-parity-stale").expect("append id"),
            contract.clone(),
            stale_value.clone(),
        ))
        .await;
    let memory_stale = memory_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            Some(first.revision_ref().clone()),
            AppendRequestId::new("postgres-configured-parity-stale").expect("append id"),
            contract.clone(),
            stale_value,
        ))
        .await;
    assert_eq!(postgres_stale, memory_stale);
    assert_eq!(
        postgres_stale,
        Err(mfm_runtime::history::HistoryError::StaleHead)
    );

    let postgres_replay = postgres_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-parity-positive").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("positive replay value"),
        ))
        .await;
    let memory_replay = memory_writer
        .append(ConfigurationAppendRequest::new(
            stream.clone(),
            None,
            AppendRequestId::new("postgres-configured-parity-positive").expect("append id"),
            contract.clone(),
            ProposedCanonicalValue::from_json(r#"{"revision":1}"#).expect("positive replay value"),
        ))
        .await;
    assert_eq!(postgres_replay, memory_replay);
    assert_eq!(postgres_replay, Ok(first.clone()));

    let postgres_current = postgres_reader
        .resolve(&stream, &contract)
        .await
        .expect("resolve PostgreSQL parity head");
    let memory_current = memory_reader
        .resolve(&stream, &contract)
        .await
        .expect("resolve memory parity head");
    assert_eq!(postgres_current, memory_current);
    assert_eq!(postgres_current.revision(), &exact);

    drop(memory_reader);
    drop(memory_writer);
    drop(postgres_reader);
    drop(postgres_writer);
    database.cleanup().await;
}

#[tokio::test]
async fn maintenance_only_login_can_preflight_and_append_configuration() {
    let database = TestDatabase::create().await;
    let writer = open_configuration_maintenance(database.maintenance_sessions().await)
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
    database.cleanup().await;
}

#[tokio::test]
async fn configured_value_history_linearizes_same_stream_append_races() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured-race").expect("operation id");
    let (registry, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        database.combined_sessions().await,
        registry,
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

    let (reopened_registry, reopened_document) = qualified_program(operation_id.clone());
    assert_eq!(reopened_document, document);
    let (reopened_history, reopened_configuration) =
        open_structured_authoritative_with_configuration(
            database.combined_sessions().await,
            reopened_registry,
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
    let runtime = reopened_history.runtime;
    let history_reader = reopened_history.public_reader;
    let (admitted_run_id, _attempt) = runtime
        .admit_run(StructuredAdmissionCommand::new(
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
        .load_public(&admitted_run_id)
        .await
        .expect("reload admitted race winner");
    assert_eq!(admitted.run_id(), &admitted_run_id);
    assert_eq!(
        admitted.header().tenant_scope_id(),
        stream.tenant_scope_id()
    );

    drop(history_reader);
    drop(runtime);
    drop(reopened_reader);
    drop(reopened_writer);
    database.cleanup().await;
}

#[tokio::test]
async fn configured_value_head_update_is_atomic_and_target_isolated() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured-head").expect("operation id");
    let (registry, _) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        database.combined_sessions().await,
        registry,
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

#[tokio::test]
async fn coordinated_configuration_rollback_is_visible_to_fresh_sessions() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/configured-rollback").expect("operation id");
    let (registry, _) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let (run_history, configuration) = open_structured_authoritative_with_configuration(
        database.combined_sessions().await,
        registry,
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("qualify structured stores");
    let (writer, reader) = configuration.split();
    let stream = ConfigurationStreamKey::new(
        database.store_scope_id().await,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "8".repeat(32))).expect("tenant"),
        operation_id.clone(),
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
    assert_eq!(second.sequence(), 2);
    drop(reader);
    drop(writer);
    drop(run_history);

    let audit_pool = database.independent_pool().await;
    update_configuration_head(&audit_pool, &stream, &first).await;
    sqlx::query(
        "DELETE FROM configuration_revisions           WHERE store_scope_id = $1 AND tenant_scope_id = $2             AND entry_point_operation_id = $3 AND target_id = $4             AND revision_sequence = 2",
    )
    .bind(stream.store_scope_id().as_str())
    .bind(stream.tenant_scope_id().as_str())
    .bind(stream.entry_point_operation_id().as_str())
    .bind(stream.target_id().as_str())
    .execute(&audit_pool)
    .await
    .expect("coordinated owner rollback");
    audit_pool.close().await;

    let (local_registry, _) = qualified_program(operation_id);
    let local = open_structured_authoritative_with_configuration(
        database.combined_sessions().await,
        local_registry,
        physical_verifier,
    )
    .await
    .expect("fresh sessions still open against the retained target authority");
    drop(local);
    database.cleanup().await;
}

async fn qualification_attempt(
    database: &TestDatabase,
) -> mfm_storage_postgres::Result<AssembledStructuredRuntime<PostgresStructuredHistoryBackend>> {
    let (registry, _) =
        qualified_program(stable("mfm.postgres.fixture/qualification").expect("operation id"));
    let sessions = database.try_application_sessions().await?;
    open_structured_authoritative(sessions, registry, Arc::new(NoPhysicalBindings)).await
}

async fn assert_schema_reopen_rejected(database: &TestDatabase) {
    assert!(matches!(
        qualification_attempt(database).await,
        Err(PostgresStoreError::SchemaAuthorityMismatch)
            | Err(PostgresStoreError::TargetSessionRejected)
            | Err(PostgresStoreError::WriterRequired)
    ));
}

async fn assert_session_reopen_rejected(database: &TestDatabase) {
    assert!(matches!(
        qualification_attempt(database).await,
        Err(PostgresStoreError::WriterRequired) | Err(PostgresStoreError::TargetSessionRejected)
    ));
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

    let hostile_role = format!("{}_hostile", database.run_writer_login.role_name);
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

    drop(
        qualification_attempt(&database)
            .await
            .expect("restored exact ACLs qualify"),
    );
    database.cleanup().await;
}

#[tokio::test]
async fn qualification_rejects_extra_membership_inheritance_and_admin_session_substitution() {
    let database = TestDatabase::create().await;
    drop(
        open_configuration_maintenance(database.maintenance_sessions().await)
            .await
            .expect("exact maintenance login qualifies"),
    );

    sqlx::query(AssertSqlSafe(format!(
        "GRANT \"{}\" TO {} WITH INHERIT FALSE, SET TRUE",
        database.target_roles.configuration_writer, database.run_writer_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("inject extra incoming membership");
    assert_session_reopen_rejected(&database).await;
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE \"{}\" FROM {}",
        database.target_roles.configuration_writer, database.run_writer_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("remove extra incoming membership");

    sqlx::query(AssertSqlSafe(format!(
        "GRANT \"{}\" TO {} WITH ADMIN OPTION",
        database.target_roles.run_writer, database.run_writer_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("inject membership administration authority");
    assert_session_reopen_rejected(&database).await;
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE ADMIN OPTION FOR \"{}\" FROM {}",
        database.target_roles.run_writer, database.run_writer_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("remove membership administration authority");

    sqlx::query(AssertSqlSafe(format!(
        "ALTER ROLE {} INHERIT",
        database.run_writer_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("inject inherited session authority");
    assert_session_reopen_rejected(&database).await;
    sqlx::query(AssertSqlSafe(format!(
        "ALTER ROLE {} NOINHERIT",
        database.run_writer_login.role_name
    )))
    .execute(&database.admin_pool)
    .await
    .expect("restore non-inheriting session role");

    // Session substitution via connection options is rejected by the restricted login
    // profile (session_user must equal current_user at issuance). Issue attempts that
    // retain only the exact memberships continue to succeed after the above repairs.
    drop(
        qualification_attempt(&database)
            .await
            .expect("restored exact application login qualifies"),
    );
    database.cleanup().await;
}

#[tokio::test]
async fn structured_history_fresh_process_worker() {
    let Some(mode) = std::env::var_os(FRESH_PROCESS_MODE_ENV) else {
        return;
    };
    let schema = std::env::var(FRESH_PROCESS_SCHEMA_ENV).expect("worker schema is required");
    let sessions = open_test_application_sessions(TestTargetCredentials {
        schema_name: schema,
        run_reader: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_RUN_READER_URL").expect("reader url"),
        },
        run_writer: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_RUN_WRITER_URL").expect("writer url"),
        },
        configuration_reader: TestLoginCredential {
            database_url: std::env::var("MFM_TEST_CONFIG_READER_URL").expect("config reader url"),
        },
        configuration_writer: None,
    })
    .await
    .expect("issue worker sessions");
    let operation_id = stable("mfm.postgres.fixture/reopen").expect("operation id");
    let (registry, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let assembled = open_structured_authoritative(sessions, registry, physical_verifier)
        .await
        .expect("qualify fresh-process structured store");
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let store_scope = reader.store_identity().store_scope_id.clone();
    let run_id = derive_run_id(
        &store_scope,
        &default_tenant(),
        &operation_id,
        &default_invocation(),
    );

    match mode.to_str().expect("worker mode is UTF-8") {
        "admit" => {
            let (admitted_run_id, first_attempt) = runtime
                .admit_run(admission(
                    operation_id.clone(),
                    document.clone(),
                    "postgres-admit",
                ))
                .await
                .expect("persist admission in first process");
            assert_eq!(admitted_run_id, run_id);
            assert!(matches!(
                first_attempt.outcome(),
                HistoryAppendOutcome::NewlyCommitted(_)
            ));
            let (_replay_run_id, replay) = runtime
                .admit_run(admission(operation_id, document, "postgres-admit"))
                .await
                .expect("replay exact admission in first process");
            assert!(matches!(
                replay.outcome(),
                HistoryAppendOutcome::ExistingSame(_)
            ));
            assert!(matches!(
                reader
                    .load_public(&run_id)
                    .await
                    .expect("first-process refold")
                    .status(),
                RunEvidenceStatus::Actionable
            ));
        }
        "continue" => {
            let verified = reader
                .load_public(&run_id)
                .await
                .expect("second-process refold");
            assert!(matches!(verified.status(), RunEvidenceStatus::Actionable));
            assert_eq!(
                runtime
                    .drive_once(&run_id)
                    .await
                    .expect("continue in second process"),
                DriveOutcome::TransitionCommitted { closed: true }
            );
            assert!(matches!(
                reader
                    .load_public(&run_id)
                    .await
                    .expect("second-process closed refold")
                    .status(),
                RunEvidenceStatus::Closed
            ));
        }
        _ => panic!("unknown structured-history worker mode"),
    }

    drop(reader);
    drop(runtime);
}

#[tokio::test]
async fn fresh_process_refolds_and_continues_the_same_structured_run() {
    let database = TestDatabase::create().await;
    run_fresh_process_worker(&database, "admit").await;
    run_fresh_process_worker(&database, "continue").await;

    let operation_id = stable("mfm.postgres.fixture/reopen").expect("operation id");
    let (_registry, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let store_scope = database.store_scope_id().await;
    let run_id = derive_run_id(
        &store_scope,
        &default_tenant(),
        &operation_id,
        &default_invocation(),
    );
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

    let (memory_registry, memory_document) = qualified_program(operation_id.clone());
    assert_eq!(memory_document, document);
    let memory = assemble_in_memory_runtime(
        StructuredStoreIdentity {
            store_scope_id: postgres_admission_batch.store_scope_id.clone(),
            store_epoch: postgres_admission_batch.store_epoch,
            physical_target: Some(PhysicalTargetIdentity {
                target_key: "postgres-memory-fixture-target".to_owned(),
                database_oid: 1,
                fence_generation: 1,
                release_epoch: 1,
                current_incarnation_ref: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(b"postgres-memory-fixture-target"),
                ),
            }),
        },
        memory_registry,
        Arc::clone(&physical_verifier),
    )
    .expect("runtime assembly");
    let memory_runtime = memory.runtime;
    let memory_reader = memory.public_reader;
    let (memory_run_id, memory_admission) = memory_runtime
        .admit_run(admission(
            operation_id.clone(),
            memory_document,
            "postgres-admit",
        ))
        .await
        .expect("persist identical admission through memory");
    assert_eq!(memory_run_id, run_id);
    assert!(matches!(
        memory_admission.outcome(),
        HistoryAppendOutcome::NewlyCommitted(_)
    ));
    assert_eq!(
        memory_runtime
            .drive_once(&run_id)
            .await
            .expect("commit identical transition through memory"),
        DriveOutcome::TransitionCommitted { closed: true }
    );

    let (verification_registry, _) = qualified_program(operation_id.clone());
    let verification = open_structured_authoritative(
        database.application_sessions().await,
        verification_registry,
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("qualify verification structured store");
    let verification_reader = verification.public_reader;
    let verification_replay = verification.replay_reader;
    assert!(matches!(
        verification_reader
            .load_public(&run_id)
            .await
            .expect("verification-process closed refold")
            .status(),
        RunEvidenceStatus::Closed
    ));
    assert!(matches!(
        mfm_replay::structured::verify_recorded_history(&verification_replay, &run_id)
            .await
            .expect("callback-free replay after fresh-process continuation")
            .status(),
        RunEvidenceStatus::Closed
    ));
    assert!(matches!(
        memory_reader
            .load_public(&run_id)
            .await
            .expect("memory parity closed refold")
            .status(),
        RunEvidenceStatus::Closed
    ));
    drop(verification_reader);
    drop(verification_replay);
    drop(verification.runtime);

    // Deployment-issued sessions retain private pools. Ordinary code cannot close them and
    // continue; unavailable credentials fail closed without a memory fallback.
    let rejected = open_test_application_sessions(TestTargetCredentials {
        schema_name: database.schema.clone(),
        run_reader: TestLoginCredential {
            database_url: "postgresql://invalid:invalid@127.0.0.1:1/postgres".to_owned(),
        },
        run_writer: TestLoginCredential {
            database_url: "postgresql://invalid:invalid@127.0.0.1:1/postgres".to_owned(),
        },
        configuration_reader: TestLoginCredential {
            database_url: "postgresql://invalid:invalid@127.0.0.1:1/postgres".to_owned(),
        },
        configuration_writer: None,
    })
    .await;
    assert!(
        matches!(rejected, Err(PostgresStoreError::Connection)),
        "missing deployment credentials must fail closed without memory fallback: {rejected:?}"
    );
    let _ = (physical_verifier, run_id);

    database.cleanup().await;
}

#[tokio::test]
async fn tenant_fact_publications_are_dense_atomic_and_exactly_routed() {
    let database = TestDatabase::create().await;
    let operation_id =
        stable("mfm.postgres.fixture/tenant-fact-publication").expect("operation id");
    let (registry, document) = qualified_fact_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let assembled = open_structured_authoritative(
        database.application_sessions().await,
        registry,
        Arc::clone(&physical_verifier),
    )
    .await
    .expect("qualify fact publication store");
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let store_scope = reader.store_identity().store_scope_id.clone();
    let first_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000040").expect("first inv");
    let second_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000041").expect("second inv");
    let first_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &operation_id,
        &first_invocation,
    );
    let second_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &operation_id,
        &second_invocation,
    );

    let (got_first, _) = runtime
        .admit_run(StructuredAdmissionCommand::new(
            default_tenant(),
            first_invocation,
            operation_id.clone(),
            document.clone(),
            admission_material(9),
            vec![ProposedCanonicalValue::from_value(&Value { value: 7 }).expect("initial value")],
            AppendRequestId::new("first-fact-publication-admit").expect("append id"),
        ))
        .await
        .expect("admit first fact producer");
    assert_eq!(got_first, first_run);
    let (got_second, _) = runtime
        .admit_run(StructuredAdmissionCommand::new(
            default_tenant(),
            second_invocation,
            operation_id.clone(),
            document.clone(),
            admission_material(9),
            vec![ProposedCanonicalValue::from_value(&Value { value: 7 }).expect("initial value")],
            AppendRequestId::new("second-fact-publication-admit").expect("append id"),
        ))
        .await
        .expect("admit second fact producer");
    assert_eq!(got_second, second_run);

    let (first_drive, second_drive) = tokio::join!(
        runtime.drive_once(&first_run),
        runtime.drive_once(&second_run),
    );
    for attempt in [&first_drive, &second_drive] {
        match attempt {
            Ok(DriveOutcome::TransitionCommitted { .. })
            | Ok(DriveOutcome::ConcurrentProgress)
            | Ok(DriveOutcome::Closed)
            | Err(_) => {}
            other => panic!("unexpected racing fact publication outcome: {other:?}"),
        }
    }

    if matches!(
        reader
            .load_public(&first_run)
            .await
            .expect("reload first")
            .status(),
        RunEvidenceStatus::Actionable
    ) {
        assert_eq!(
            runtime
                .drive_once(&first_run)
                .await
                .expect("retry first fact publication"),
            DriveOutcome::TransitionCommitted { closed: true }
        );
    }
    if matches!(
        reader
            .load_public(&second_run)
            .await
            .expect("reload second")
            .status(),
        RunEvidenceStatus::Actionable
    ) {
        assert_eq!(
            runtime
                .drive_once(&second_run)
                .await
                .expect("retry second fact publication"),
            DriveOutcome::TransitionCommitted { closed: true }
        );
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
    let corrupted_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000042").expect("corrupted inv");
    let corrupted_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &operation_id,
        &corrupted_invocation,
    );
    let (got_corrupted, _) = runtime
        .admit_run(StructuredAdmissionCommand::new(
            default_tenant(),
            corrupted_invocation,
            operation_id.clone(),
            document,
            admission_material(9),
            vec![ProposedCanonicalValue::from_value(&Value { value: 7 }).expect("initial value")],
            AppendRequestId::new("corrupted-head-producer-admit").expect("append id"),
        ))
        .await
        .expect("admit producer after tenant head corruption");
    assert_eq!(got_corrupted, corrupted_run);
    let corrupted_drive = runtime.drive_once(&corrupted_run).await;
    assert!(
        matches!(
            corrupted_drive,
            Err(ref error)
                if error.store_fault_kind() == Some(RuntimeStoreFaultKind::InvalidHistory)
        ),
        "missing tenant fact head must fail closed: {corrupted_drive:?}"
    );
    let (corrupted_registry, _) = qualified_fact_program(operation_id);
    let corrupted = match database.try_application_sessions().await {
        Ok(sessions) => {
            open_structured_authoritative(sessions, corrupted_registry, physical_verifier).await
        }
        Err(error) => Err(error),
    };
    assert!(
        matches!(
            corrupted,
            Err(PostgresStoreError::SchemaAuthorityMismatch)
                | Err(PostgresStoreError::TargetSessionRejected)
                | Err(PostgresStoreError::Corruption(_))
        ),
        "missing tenant fact head must fail closed at qualification"
    );

    audit_pool.close().await;
    drop(reader);
    drop(runtime);
    database.cleanup().await;
}

#[tokio::test]
async fn prior_run_fact_scan_survives_reopen_and_matches_memory_bytes() {
    let database = TestDatabase::create().await;
    let postgres_fixture = qualified_fact_scan_fixture();
    let producer_operation = postgres_fixture.producer_operation.clone();
    let consumer_operation = postgres_fixture.consumer_operation.clone();
    let offline_program_verifier = Arc::new(RegistryProgramVerifier::new(
        postgres_fixture.registry.admission_verification_registry(),
    ));
    let postgres_assembled = open_structured_authoritative(
        database.application_sessions().await,
        postgres_fixture.registry,
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("qualify fact scanner store");
    let AssembledStructuredRuntime {
        runtime: postgres_runtime,
        public_reader: postgres_public_reader,
        export_reader: postgres_reader,
        replay_reader: postgres_replay_reader,
        ..
    } = postgres_assembled;
    let postgres_identity = postgres_public_reader.store_identity().clone();
    let postgres_response = drive_qualified_fact_scan(
        postgres_runtime,
        &postgres_reader,
        postgres_fixture.producer_operation,
        postgres_fixture.consumer_operation,
        postgres_fixture.producer_document,
        postgres_fixture.consumer_document,
        postgres_fixture.source_object,
        postgres_fixture.request,
    )
    .await;

    let portable_consumer_run = derive_run_id(
        &postgres_identity.store_scope_id,
        &default_tenant(),
        &consumer_operation,
        &InvocationIdentity::new("00000000-0000-4000-8000-000000000081").expect("consumer inv"),
    );
    let producer_run = derive_run_id(
        &postgres_identity.store_scope_id,
        &default_tenant(),
        &producer_operation,
        &InvocationIdentity::new("00000000-0000-4000-8000-000000000080").expect("producer inv"),
    );
    let producer_export = postgres_reader
        .load_for_export(&producer_run)
        .await
        .expect("load recursively authorized producer export");
    let producer_cutoff = match producer_export.semantic_head() {
        SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
        SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
    };
    let consumer_export = postgres_reader
        .load_for_export(&portable_consumer_run)
        .await
        .expect("load recursively authorized consumer export");
    let consumer_cutoff = match consumer_export.semantic_head() {
        SemanticHead::Genesis { admission_ref, .. } => admission_ref.run_sequence,
        SemanticHead::Transition { transition_ref, .. } => transition_ref.run_sequence,
    };
    let consumer_export = consumer_export
        .with_authorized_sources(
            Some(consumer_cutoff),
            vec![(producer_export, Some(producer_cutoff))],
        )
        .expect("seal recursively authorized consumer sources");
    let consumer_closure = AuthorizedExportClosure::new(
        consumer_export,
        StableId::new("mfm.storage-test/principal").expect("principal"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(b"consumer-export-decision"),
        ),
        BTreeMap::from([(
            producer_run.clone(),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(b"producer-export-decision"),
            ),
        )]),
    )
    .expect("seal recursively authorized consumer closure");
    let portable =
        PortableRunExport::from_authorized_export_closure(&consumer_closure, ExportKind::Semantic)
            .expect("encode recursively authorized portable export");
    assert_eq!(portable.source_run_count(), 1);
    let portable_bytes = portable
        .to_canonical_bytes()
        .expect("encode recursively authorized portable frames");
    let recorded = postgres_replay_reader
        .load_for_recorded_verify(&portable_consumer_run)
        .await
        .expect("load online recorded replay");
    let release = AcceptRetainedRelease;
    let checkpoint = AcceptStoreCheckpoint;
    let trust = ReplayTrustSnapshot::new(&*offline_program_verifier, &NoPhysicalBindings)
        .with_authorized_closure(portable.closure_reference(), &release, &checkpoint);
    let offline = PortableRunExport::verify_offline(&portable_bytes, &trust)
        .expect("fold recursively authorized portable export offline");
    let online = project_replay_result(&recorded).expect("project online recorded replay");
    assert_eq!(offline.as_bytes(), online.as_bytes());
    assert_eq!(offline.schema_id(), online.schema_id());

    let reopened_fixture = qualified_fact_scan_fixture();
    let consumer_run = derive_run_id(
        &postgres_identity.store_scope_id,
        &default_tenant(),
        &reopened_fixture.consumer_operation,
        &InvocationIdentity::new("00000000-0000-4000-8000-000000000081").expect("consumer inv"),
    );
    let reopened_assembled = open_structured_authoritative(
        database.application_sessions().await,
        reopened_fixture.registry,
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("reopen fact scanner store through an independent pool");
    let reopened_response =
        retained_fact_response(&reopened_assembled.export_reader, &consumer_run).await;
    assert_eq!(reopened_response, postgres_response);

    let memory_fixture = qualified_fact_scan_fixture();
    let memory_assembled = assemble_in_memory_runtime(
        postgres_identity,
        memory_fixture.registry,
        Arc::new(NoPhysicalBindings),
    )
    .expect("runtime assembly");
    let AssembledStructuredRuntime {
        runtime: memory_runtime,
        export_reader: memory_reader,
        ..
    } = memory_assembled;
    let memory_response = drive_qualified_fact_scan(
        memory_runtime,
        &memory_reader,
        memory_fixture.producer_operation,
        memory_fixture.consumer_operation,
        memory_fixture.producer_document,
        memory_fixture.consumer_document,
        memory_fixture.source_object,
        memory_fixture.request,
    )
    .await;
    assert_eq!(memory_response, postgres_response);

    drop(memory_reader);
    drop(postgres_reader);
    drop(reopened_assembled);
    database.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prior_run_fact_scan_accepts_empty_frontier_and_excludes_ineligible_source() {
    let database = TestDatabase::create().await;
    let allowed = qualified_fact_scan_fixture();
    let allowed_assembled = open_structured_authoritative(
        database.application_sessions().await,
        allowed.registry,
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("qualify empty-frontier scanner store");
    // Keep each complete fold on a Tokio worker stack; this test exercises sequential
    // empty-frontier and ineligible-source scans without inheriting the small test stack.
    let allowed_runtime = Arc::new(allowed_assembled.runtime);
    let allowed_reader = allowed_assembled.export_reader;
    let store_scope = allowed_reader.store_identity().store_scope_id.clone();
    let empty_tenant =
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "3".repeat(32))).expect("tenant");
    let empty_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000084").expect("empty inv");
    let empty_run = derive_run_id(
        &store_scope,
        &empty_tenant,
        &allowed.consumer_operation,
        &empty_invocation,
    );
    let (got_empty, _) = allowed_runtime
        .admit_run(fact_scan_admission_for_tenant(
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
    assert_eq!(got_empty, empty_run);
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&allowed_runtime);
            let run_id = empty_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("drive empty-frontier fact scan task")
        .expect("drive empty-frontier fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&allowed_runtime);
            let run_id = empty_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("settle empty-frontier fact scan task")
        .expect("settle empty-frontier fact scan"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    let empty_response: PriorRunFactSelectionResponse =
        serde_json::from_str(&retained_fact_response(&allowed_reader, &empty_run).await)
            .expect("decode empty-frontier response");
    assert_eq!(empty_response.attestation.frontier.fact_order, 0);
    assert_eq!(empty_response.query_results.len(), 1);
    assert!(empty_response.query_results[0].selected.is_empty());

    let producer_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000085").expect("producer inv");
    let producer_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &allowed.producer_operation,
        &producer_invocation,
    );
    let (got_producer, _) = allowed_runtime
        .admit_run(fact_scan_admission(
            allowed.producer_operation,
            allowed.producer_document,
            admission_material(85),
            Value { value: 7 },
            "excluded-source-producer-admit",
            85,
        ))
        .await
        .expect("admit excluded-source producer");
    assert_eq!(got_producer, producer_run);
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&allowed_runtime);
            let run_id = producer_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("drive excluded-source producer task")
        .expect("drive excluded-source producer"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    drop(allowed_runtime);
    drop(allowed_reader);

    let excluded = qualified_fact_scan_fixture_excluding_producer();
    let excluded_assembled = open_structured_authoritative(
        database.application_sessions().await,
        excluded.registry,
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("reopen scanner with an excluding manifest");
    let excluded_runtime = Arc::new(excluded_assembled.runtime);
    let excluded_reader = excluded_assembled.export_reader;
    let excluded_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000086").expect("excluded inv");
    let excluded_run = derive_run_id(
        &excluded_reader.store_identity().store_scope_id,
        &default_tenant(),
        &excluded.consumer_operation,
        &excluded_invocation,
    );
    let (got_excluded, _) = excluded_runtime
        .admit_run(fact_scan_admission(
            excluded.consumer_operation,
            excluded.consumer_document,
            admission_material_with_source(86, excluded.source_object),
            Value { value: 7 },
            "excluded-source-consumer-admit",
            86,
        ))
        .await
        .expect("admit excluded-source consumer");
    assert_eq!(got_excluded, excluded_run);
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&excluded_runtime);
            let run_id = excluded_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("drive excluded-source fact scan task")
        .expect("drive excluded-source fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&excluded_runtime);
            let run_id = excluded_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("settle excluded-source fact scan task")
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
    database.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fact_publication_and_selection_barrier_have_one_tenant_linearization() {
    let database = TestDatabase::create().await;
    let fixture = qualified_fact_scan_fixture();
    let assembled = open_structured_authoritative(
        database.application_sessions().await,
        fixture.registry,
        Arc::new(NoPhysicalBindings),
    )
    .await
    .expect("qualify publication-barrier race store");
    let runtime = Arc::new(assembled.runtime);
    let reader = assembled.public_reader;
    let store_scope = reader.store_identity().store_scope_id.clone();
    let producer_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000082").expect("producer inv");
    let consumer_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000083").expect("consumer inv");
    let producer_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &fixture.producer_operation,
        &producer_invocation,
    );
    let consumer_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &fixture.consumer_operation,
        &consumer_invocation,
    );
    let (got_producer, _) = runtime
        .admit_run(fact_scan_admission(
            fixture.producer_operation.clone(),
            fixture.producer_document.clone(),
            admission_material(82),
            Value { value: 7 },
            "publication-barrier-producer-admit",
            82,
        ))
        .await
        .expect("admit racing producer");
    assert_eq!(got_producer, producer_run);
    let (got_consumer, _) = runtime
        .admit_run(fact_scan_admission(
            fixture.consumer_operation.clone(),
            fixture.consumer_document.clone(),
            admission_material_with_source(83, fixture.source_object.clone()),
            Value { value: 7 },
            "publication-barrier-consumer-admit",
            83,
        ))
        .await
        .expect("admit racing consumer");
    assert_eq!(got_consumer, consumer_run);

    // Keep the two complete fold/append drives on independent Tokio worker stacks while
    // preserving their concurrent tenant-lock race.
    let publication_drive = tokio::spawn({
        let runtime = runtime.clone();
        let producer_run = producer_run.clone();
        async move { runtime.drive_once(&producer_run).await }
    });
    let barrier_drive = tokio::spawn({
        let runtime = runtime.clone();
        let consumer_run = consumer_run.clone();
        async move { runtime.drive_once(&consumer_run).await }
    });
    let (publication_drive, barrier_drive) = tokio::join!(publication_drive, barrier_drive);
    let publication_drive = publication_drive.expect("publication drive task");
    let barrier_drive = barrier_drive.expect("selection barrier drive task");
    assert_tenant_race_drive(&publication_drive);
    assert_tenant_race_drive(&barrier_drive);

    if matches!(
        reader
            .load_public(&producer_run)
            .await
            .expect("reload racing producer")
            .status(),
        RunEvidenceStatus::Actionable
    ) {
        let retry = runtime
            .drive_once(&producer_run)
            .await
            .expect("retry racing publication");
        assert!(matches!(
            retry,
            DriveOutcome::TransitionCommitted { .. } | DriveOutcome::ConcurrentProgress
        ));
    }
    if matches!(
        reader
            .load_public(&consumer_run)
            .await
            .expect("reload racing consumer")
            .status(),
        RunEvidenceStatus::Actionable
    ) {
        let retry = runtime
            .drive_once(&consumer_run)
            .await
            .expect("retry racing selection barrier");
        assert!(matches!(
            retry,
            DriveOutcome::AccessObserved
                | DriveOutcome::TransitionCommitted { .. }
                | DriveOutcome::ConcurrentProgress
        ));
    }

    // Finish the consumer if authorization landed but observation/settlement remains open.
    for _ in 0..4 {
        let frontier = reader
            .load_public(&consumer_run)
            .await
            .expect("poll consumer frontier")
            .status();
        if matches!(frontier, RunEvidenceStatus::Closed) {
            break;
        }
        let _ = runtime.drive_once(&consumer_run).await;
    }

    let audit_pool = database.independent_pool().await;
    let producer_batches = load_normalized_batches(&audit_pool, &producer_run).await;
    let consumer_batches = load_normalized_batches(&audit_pool, &consumer_run).await;
    assert!(
        producer_batches.len() >= 2,
        "racing producer must commit one atomic publication"
    );
    let publication_batch = &producer_batches[1];
    assert!(
        consumer_batches.len() >= 2,
        "racing consumer must commit one atomic authorization barrier"
    );
    let barrier_batch = consumer_batches
        .iter()
        .find(|batch| {
            matches!(
                batch.tenant_fact_coordinate,
                TenantFactCoordinate::FactSelectionBarrier { .. }
            )
        })
        .expect("consumer authorization must capture a selection barrier");
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

    drop(reader);
    drop(runtime);
    audit_pool.close().await;
    database.cleanup().await;
}

fn assert_tenant_race_drive(
    attempt: &std::result::Result<DriveOutcome, mfm_runtime::structured::RuntimeError>,
) {
    match attempt {
        Ok(DriveOutcome::TransitionCommitted { .. })
        | Ok(DriveOutcome::AccessObserved)
        | Ok(DriveOutcome::ConcurrentProgress)
        | Ok(DriveOutcome::Closed)
        | Err(_) => {}
        other => panic!("unexpected publication-barrier race outcome: {other:?}"),
    }
}

#[tokio::test]
async fn object_row_failure_rolls_back_batch_objects_and_head() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/object-row-rollback").expect("operation id");
    let (registry, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let assembled = open_structured_authoritative(
        database.application_sessions().await,
        registry,
        physical_verifier,
    )
    .await
    .expect("qualify store before injecting object failure");
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
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

    let admit = runtime
        .admit_run(admission(operation_id, document, "object-row-rollback"))
        .await;
    assert!(
        matches!(
            admit,
            Err(ref error)
                if error.store_fault_kind() == Some(RuntimeStoreFaultKind::BackendUnavailable)
                    || error.code() == RuntimeFaultCode::StoreUnavailable
        ),
        "injected child-row failure must reject the append: {admit:?}"
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

    drop(reader);
    drop(runtime);
    mutation_pool.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn malformed_object_rows_fail_closed_after_qualification() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/malformed-objects").expect("operation id");
    let (registry, document) = qualified_program(operation_id.clone());
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let assembled = open_structured_authoritative(
        database.application_sessions().await,
        registry,
        physical_verifier,
    )
    .await
    .expect("qualify store before hostile object mutations");
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _) = runtime
        .admit_run(admission(operation_id, document, "malformed-object-rows"))
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
        reader.load_public(&run_id).await,
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
        reader.load_public(&run_id).await,
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
        reader.load_public(&run_id).await,
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
        reader.load_public(&run_id).await,
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
            SET canonical_json = '\"' || repeat('a', 33554432) || '\"' \
          WHERE run_id = $1 AND run_sequence = 1 AND object_ordinal = $2",
    )
    .bind(run_id.as_str())
    .bind(first.ordinal)
    .execute(&mutation_pool)
    .await
    .expect("persist oversized hostile object row");
    assert!(matches!(
        reader.load_public(&run_id).await,
        Err(StructuredStoreError::InvalidHistory)
    ));
    update_object_snapshot(&mutation_pool, &run_id, first.ordinal, first).await;
    sqlx::query(
        "ALTER TABLE run_history_batch_objects \
         ADD CONSTRAINT run_history_batch_objects_json_v1 CHECK ( \
             octet_length(canonical_json) BETWEEN 1 AND 33554432 \
         )",
    )
    .execute(&mutation_pool)
    .await
    .expect("restore object frame bound");
    reader
        .load_public(&run_id)
        .await
        .expect("restored object rows must refold");

    drop(reader);
    drop(runtime);
    mutation_pool.close().await;
    database.cleanup().await;
}

#[tokio::test]
async fn numeric_batch_order_refolds_across_the_tenth_append() {
    let database = TestDatabase::create().await;
    let operation_id = stable("mfm.postgres.fixture/numeric-batch-order").expect("operation id");
    let (registry, document) = qualified_program_with_state_count(operation_id.clone(), 10);
    let physical_verifier: Arc<dyn PublicPhysicalBindingVerifier> = Arc::new(NoPhysicalBindings);
    let assembled = open_structured_authoritative(
        database.application_sessions().await,
        registry,
        physical_verifier,
    )
    .await
    .expect("qualify numeric-order store");
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _) = runtime
        .admit_run(admission(operation_id, document, "numeric-order-admit"))
        .await
        .expect("admit numeric-order fixture");
    for _transition in 0..10 {
        match runtime
            .drive_once(&run_id)
            .await
            .expect("commit sequential transition")
        {
            DriveOutcome::TransitionCommitted { closed: false }
            | DriveOutcome::TransitionCommitted { closed: true } => {}
            other => panic!("unexpected numeric-order drive: {other:?}"),
        }
    }
    assert!(matches!(
        reader
            .load_public(&run_id)
            .await
            .expect("refold eleven numeric batches")
            .status(),
        RunEvidenceStatus::Closed
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
    drop(runtime);
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
        .env(
            "MFM_TEST_RUN_READER_URL",
            database
                .run_reader_login
                .database_url(&database.database_url),
        )
        .env(
            "MFM_TEST_RUN_WRITER_URL",
            database
                .run_writer_login
                .database_url(&database.database_url),
        )
        .env(
            "MFM_TEST_CONFIG_READER_URL",
            database
                .configuration_reader_login
                .database_url(&database.database_url),
        )
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
    mfm_certify::structured::QualifiedProgramRegistry,
    mfm_spec::structured::CertifiedProgramDocument,
) {
    qualified_program_with_state_count(operation_id, 1)
}

fn qualified_program_with_state_count(
    operation_id: StableId,
    state_count: usize,
) -> (
    mfm_certify::structured::QualifiedProgramRegistry,
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
    (registry, document)
}

fn qualified_fact_program(
    operation_id: StableId,
) -> (
    mfm_certify::structured::QualifiedProgramRegistry,
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
    (registry, document)
}

struct QualifiedFactScanFixture {
    producer_operation: StableId,
    consumer_operation: StableId,
    producer_document: mfm_spec::structured::CertifiedProgramDocument,
    consumer_document: mfm_spec::structured::CertifiedProgramDocument,
    registry: mfm_certify::structured::QualifiedProgramRegistry,
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
        FactSelectionScanBounds::new(1, 64, 1_048_576, 16, 65_536).expect("fact scan bounds"),
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
                settle_returned: Arc::new(|_frame, returned| {
                    let response: PriorRunFactSelectionResponse =
                        serde_json::from_str(returned.canonical_response_json())
                            .expect("typed fact response");
                    let output = response.query_results[0]
                        .selected
                        .first()
                        .map(|selected| {
                            serde_json::from_str::<Value>(&selected.response_canonical_json)
                                .expect("selected fact response")
                        })
                        .unwrap_or(Value { value: 0 });
                    StateSettlement::Proposed(ProposedStateOutcome::Success(output))
                }),
                settle_safe_failure: Arc::new(|_frame, _failure| {
                    mfm_program::structured::ProposedSuccessOutcome::new(Value { value: 0 })
                }),
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
    QualifiedFactScanFixture {
        producer_operation,
        consumer_operation,
        producer_document,
        consumer_document,
        registry,
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

#[expect(
    clippy::too_many_arguments,
    reason = "the end-to-end fixture keeps each certified input explicit"
)]
async fn drive_qualified_fact_scan<B, P>(
    runtime: Runtime<P>,
    reader: &ExportRunReader<B>,
    producer_operation: StableId,
    consumer_operation: StableId,
    producer_document: mfm_spec::structured::CertifiedProgramDocument,
    consumer_document: mfm_spec::structured::CertifiedProgramDocument,
    source_object: HistoryObject,
    request: FactSelectionRequest,
) -> String
where
    B: StructuredHistoryBackend,
    P: mfm_runtime::history::RuntimeHistoryPort + 'static,
{
    // Keep each complete history fold on a Tokio worker stack; the test still drives the
    // producer and consumer sequentially, but does not inherit the small test-thread stack.
    let runtime = Arc::new(runtime);
    assert_eq!(
        request
            .scan_bounds()
            .expect("fixture scan bounds")
            .maximum_publications(),
        1
    );
    let store_scope = reader.store_identity().store_scope_id.clone();
    let producer_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000080").expect("producer inv");
    let consumer_invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000081").expect("consumer inv");
    let producer_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &producer_operation,
        &producer_invocation,
    );
    let consumer_run = derive_run_id(
        &store_scope,
        &default_tenant(),
        &consumer_operation,
        &consumer_invocation,
    );

    let (got_producer, _) = runtime
        .admit_run(fact_scan_admission(
            producer_operation,
            producer_document,
            admission_material(80),
            Value { value: 7 },
            "fact-scan-producer-admit",
            80,
        ))
        .await
        .expect("admit fact scan producer");
    assert_eq!(got_producer, producer_run);
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&runtime);
            let run_id = producer_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("drive fact scan producer task")
        .expect("drive fact scan producer"),
        DriveOutcome::TransitionCommitted { closed: true }
    );

    let (got_consumer, _) = runtime
        .admit_run(fact_scan_admission(
            consumer_operation,
            consumer_document,
            admission_material_with_source(81, source_object),
            Value { value: 7 },
            "fact-scan-consumer-admit",
            81,
        ))
        .await
        .expect("admit fact scan consumer");
    assert_eq!(got_consumer, consumer_run);
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&runtime);
            let run_id = consumer_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("authorize, invoke, and observe fact scan task")
        .expect("authorize, invoke, and observe fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        tokio::spawn({
            let runtime = Arc::clone(&runtime);
            let run_id = consumer_run.clone();
            async move { runtime.drive_once(&run_id).await }
        })
        .await
        .expect("settle fact scan response task")
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
        .load_for_export(&producer_run)
        .await
        .expect("export-recompute producer prefix");
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
        .load_for_export(&consumer_run)
        .await
        .expect("export-recompute consumer history");
    let sources = consumer
        .direct_source_run_ids()
        .expect("export source discovery");
    assert_eq!(
        sources,
        std::collections::BTreeSet::from([producer_run.clone()]),
        "export discovery must surface the exact prior-run producer identity"
    );
    let producer_sources = producer
        .direct_source_run_ids()
        .expect("producer has no prior-run sources");
    assert!(producer_sources.is_empty());
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
    reader: &ExportRunReader<B>,
    run_id: &RunId,
) -> String {
    let evidence = reader
        .load_for_export(run_id)
        .await
        .expect("export-recompute retained fact response");
    let returned = evidence
        .records()
        .iter()
        .find_map(|assigned| match &assigned.record {
            RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                ObservationOutcome::Returned { value } => evidence.object(&value.value_ref),
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
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    append_id: &str,
) -> StructuredAdmissionCommand {
    StructuredAdmissionCommand::new(
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
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    material: StructuredAdmissionMaterial,
    input: Value,
    append_id: &str,
    invocation_discriminator: u8,
) -> StructuredAdmissionCommand {
    fact_scan_admission_for_tenant(
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
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    material: StructuredAdmissionMaterial,
    input: Value,
    append_id: &str,
    invocation_discriminator: u8,
    tenant_discriminator: char,
) -> StructuredAdmissionCommand {
    StructuredAdmissionCommand::new(
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

fn derive_run_id(
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    entry_point_operation_id: &StableId,
    invocation_identity: &InvocationIdentity,
) -> RunId {
    let preimage = mfm_canonical::CanonicalValue::object([
        (
            "store_scope_id",
            mfm_canonical::CanonicalValue::String(store_scope_id.as_str().to_owned()),
        ),
        (
            "tenant_scope_id",
            mfm_canonical::CanonicalValue::String(tenant_scope_id.as_str().to_owned()),
        ),
        (
            "entry_point_operation_id",
            mfm_canonical::CanonicalValue::String(entry_point_operation_id.as_str().to_owned()),
        ),
        (
            "invocation_identity",
            mfm_canonical::CanonicalValue::String(invocation_identity.as_str().to_owned()),
        ),
    ])
    .expect("run-id preimage");
    let contract = mfm_canonical::RecoverabilityContract::embedded().expect("annex");
    let validated = contract
        .encode("mfm.run-id-preimage.v1", &preimage)
        .expect("validated run-id preimage");
    contract.derive_run_id(&validated).expect("derive run id")
}

fn default_tenant() -> TenantScopeId {
    TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32))).expect("tenant")
}

fn default_invocation() -> InvocationIdentity {
    InvocationIdentity::new("00000000-0000-4000-8000-000000000001").expect("invocation")
}

fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

struct TestDatabase {
    admin_pool: PgPool,
    database_url: String,
    schema: String,
    target_roles: TargetRoleSet,
    run_reader_login: TestLogin,
    run_writer_login: TestLogin,
    configuration_reader_login: TestLogin,
    configuration_writer_login: TestLogin,
}

struct TargetRoleSet {
    owner: String,
    qualification: String,
    run_reader: String,
    run_writer: String,
    configuration_reader: String,
    configuration_writer: String,
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
        let target_roles = load_target_roles(&schema).await;
        let run_reader_login = TestLogin::create(
            &admin_pool,
            &schema,
            "rrd",
            &[&target_roles.qualification, &target_roles.run_reader],
        )
        .await;
        let run_writer_login = TestLogin::create(
            &admin_pool,
            &schema,
            "rwr",
            &[&target_roles.qualification, &target_roles.run_writer],
        )
        .await;
        let configuration_reader_login = TestLogin::create(
            &admin_pool,
            &schema,
            "crd",
            &[
                &target_roles.qualification,
                &target_roles.configuration_reader,
            ],
        )
        .await;
        let configuration_writer_login = TestLogin::create(
            &admin_pool,
            &schema,
            "cwr",
            &[
                &target_roles.qualification,
                &target_roles.configuration_writer,
            ],
        )
        .await;
        Self {
            admin_pool,
            database_url,
            schema,
            target_roles,
            run_reader_login,
            run_writer_login,
            configuration_reader_login,
            configuration_writer_login,
        }
    }

    async fn application_sessions(&self) -> ApplicationTargetSessions {
        self.try_application_sessions()
            .await
            .expect("issue application sessions")
    }

    async fn try_application_sessions(
        &self,
    ) -> Result<ApplicationTargetSessions, PostgresStoreError> {
        open_test_application_sessions(self.application_materials()).await
    }

    async fn combined_sessions(&self) -> CombinedTargetSessions {
        open_test_combined_sessions(self.combined_materials())
            .await
            .expect("issue combined sessions")
    }

    async fn maintenance_sessions(&self) -> ConfigurationMaintenanceSessions {
        open_test_configuration_sessions(
            self.schema.clone(),
            self.login_material(&self.configuration_reader_login),
            self.login_material(&self.configuration_writer_login),
        )
        .await
        .expect("issue maintenance sessions")
    }

    fn application_materials(&self) -> TestTargetCredentials {
        TestTargetCredentials {
            schema_name: self.schema.clone(),
            run_reader: self.login_material(&self.run_reader_login),
            run_writer: self.login_material(&self.run_writer_login),
            configuration_reader: self.login_material(&self.configuration_reader_login),
            configuration_writer: None,
        }
    }

    fn combined_materials(&self) -> TestTargetCredentials {
        TestTargetCredentials {
            schema_name: self.schema.clone(),
            run_reader: self.login_material(&self.run_reader_login),
            run_writer: self.login_material(&self.run_writer_login),
            configuration_reader: self.login_material(&self.configuration_reader_login),
            configuration_writer: Some(self.login_material(&self.configuration_writer_login)),
        }
    }

    fn login_material(&self, login: &TestLogin) -> TestLoginCredential {
        TestLoginCredential {
            database_url: login.database_url(&self.database_url),
        }
    }

    async fn admin_schema_pool(&self) -> PgPool {
        isolated_pool(&self.database_url, &self.schema).await
    }

    async fn independent_pool(&self) -> PgPool {
        self.admin_schema_pool().await
    }

    fn application_database_url(&self) -> String {
        self.run_writer_login.database_url(&self.database_url)
    }

    async fn store_scope_id(&self) -> StoreScopeId {
        let pool = self.admin_schema_pool().await;
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
        for role in [
            self.target_roles.owner.as_str(),
            self.target_roles.qualification.as_str(),
            self.target_roles.run_reader.as_str(),
            self.target_roles.run_writer.as_str(),
            self.target_roles.configuration_reader.as_str(),
            self.target_roles.configuration_writer.as_str(),
        ] {
            let _ = sqlx::query(AssertSqlSafe(format!(
                "REASSIGN OWNED BY \"{role}\" TO CURRENT_USER"
            )))
            .execute(&self.admin_pool)
            .await;
            let _ = sqlx::query(AssertSqlSafe(format!("DROP OWNED BY \"{role}\"")))
                .execute(&self.admin_pool)
                .await;
            let _ = sqlx::query(AssertSqlSafe(format!("DROP ROLE IF EXISTS \"{role}\"")))
                .execute(&self.admin_pool)
                .await;
        }
        self.run_reader_login.drop(&self.admin_pool).await;
        self.run_writer_login.drop(&self.admin_pool).await;
        self.configuration_reader_login.drop(&self.admin_pool).await;
        self.configuration_writer_login.drop(&self.admin_pool).await;
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
        let role_name = format!("mfm_test_{purpose}_{discriminator}");
        let password = format!("MfmTest{discriminator}{}", purpose.len());
        sqlx::query(AssertSqlSafe(format!(
            "CREATE ROLE {role_name} LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE \
             NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD '{password}'"
        )))
        .execute(admin_pool)
        .await
        .expect("create restricted PostgreSQL test login");
        let quoted = memberships
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ");
        sqlx::query(AssertSqlSafe(format!(
            "GRANT {quoted} TO {role_name} WITH INHERIT FALSE, SET TRUE"
        )))
        .execute(admin_pool)
        .await
        .expect("grant exact PostgreSQL test memberships");
        Self {
            role_name,
            password,
        }
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
        sqlx::query(AssertSqlSafe(format!(
            "DROP ROLE IF EXISTS {}",
            self.role_name
        )))
        .execute(admin_pool)
        .await
        .expect("drop restricted PostgreSQL test login");
    }
}

async fn load_target_roles(schema: &str) -> TargetRoleSet {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = isolated_pool(&database_url, schema).await;
    let row = sqlx::query(
        "SELECT owner_role, qualification_role, run_reader_role, run_writer_role, \
                configuration_reader_role, configuration_writer_role \
           FROM target_authority WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .expect("load target authority roles");
    pool.close().await;
    TargetRoleSet {
        owner: row.try_get("owner_role").expect("owner"),
        qualification: row.try_get("qualification_role").expect("qualification"),
        run_reader: row.try_get("run_reader_role").expect("run reader"),
        run_writer: row.try_get("run_writer_role").expect("run writer"),
        configuration_reader: row
            .try_get("configuration_reader_role")
            .expect("configuration reader"),
        configuration_writer: row
            .try_get("configuration_writer_role")
            .expect("configuration writer"),
    }
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
