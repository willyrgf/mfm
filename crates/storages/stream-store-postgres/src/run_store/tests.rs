use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DigestAlgorithm, DigestBytes, LoweringVersion, NodeId, RunId, SchemaId, ScopeId,
    SemanticTypeId, SpecHash, SpecVersion, StateKind, StateVersion,
};
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualAuthorizationSignatureBytes,
    ManualResolutionAuthorizationProof, ManualResolutionAuthorizationSignature,
    ManualResolutionBlockReason, ManualResolutionEvidenceRef, ManualResolutionPrefixAuthority,
    ManualResolutionProofAuthority, VerifiedManualResolutionForPrefix,
};
use mfm_spec::v1::{
    self as spec, CanonicalizerIdentity, ManualResolutionEvidenceSpec, MediaType,
    ResourceNamespace, SagaPolicySpec, ValueLineageRef,
};
use mfm_store::v1::{
    AdmissionToken, AdmissionWaiter, ArtifactEvidenceRef, AttemptStatus, AttemptTerminal,
    CellTerminalProjection, CommitArtifactEvidenceSet, CommitKey, CommitOutcome,
    CommitPreconditions, ExecutionClaimStatus, ExecutionClaimStore, ManualResolution,
    NowaitSkipAdmissionResult, PreparedCommit, PreparedCommitPlan, RequiredRunState,
    ResourceLaneKey, Retention, RunAdmission, RunState, SagaEngagementReason, SagaTerminal,
    SagaTerminalProof, SideEffectPhase, SideEffectProgress, SideEffectTerminal,
    StateAttemptStarted, StoreError, StreamSeq, TrustScopeId, TrustScopeStore,
};
use sqlx::postgres::PgConnectOptions;
use sqlx::AssertSqlSafe;

use super::*;

static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_schema() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("run_store_{}_{}_{}", std::process::id(), nanos, counter)
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
    crate::schema::validate_pool(&pool)
        .await
        .expect("validate schema");
    let store = PostgresRunStore { pool };
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

#[tokio::test]
async fn schema_validation_proves_append_xid_trigger_contracts() {
    let (store, schema) = test_store().await;
    let mut tx = store.pool.begin().await.expect("begin transaction");
    let expected_xid: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
        .fetch_one(&mut *tx)
        .await
        .expect("current xact id");
    let commit_xid: String = sqlx::query_scalar(
        "INSERT INTO commits \
         (commit_id, run_id, seq, commit_key, commit_purpose, prepared_commit_plan_fingerprint, \
          commit_batch_hash, commit_sort_key, event_count, append_xid) \
         VALUES \
         ('schema-trigger-commit', 'schema-trigger-run', 1, 'schema-trigger-key', \
          'schema-trigger-purpose', $1, $2, decode('01' || repeat('00', 31), 'hex'), 1, \
          '1'::xid8) \
         RETURNING append_xid::text",
    )
    .bind(content_digest(252).as_str())
    .bind(content_digest(253).as_str())
    .fetch_one(&mut *tx)
    .await
    .expect("insert commit with explicit xid");
    assert_eq!(commit_xid, expected_xid);
    tx.rollback().await.expect("rollback manual authority rows");
    crate::schema::validate_pool(&store.pool)
        .await
        .expect("schema validation still passes");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn schema_validation_rejects_disabled_append_xid_trigger() {
    let (store, schema) = test_store().await;

    sqlx::query("ALTER TABLE commits DISABLE TRIGGER commits_set_append_xid")
        .execute(&store.pool)
        .await
        .expect("disable append xid trigger");
    crate::schema::validate_pool(&store.pool)
        .await
        .expect_err("disabled append xid trigger fails schema validation");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn schema_validation_rejects_missing_mutation_guard_trigger() {
    let (store, schema) = test_store().await;

    sqlx::query("DROP TRIGGER run_events_no_update ON run_events")
        .execute(&store.pool)
        .await
        .expect("drop run events mutation guard");
    crate::schema::validate_pool(&store.pool)
        .await
        .expect_err("missing mutation guard fails schema validation");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn store_trust_scope_survives_reconnects_and_rejects_mutation() {
    let (store, schema) = test_store().await;

    let trust_scope = store.load_trust_scope_id().await.expect("load trust scope");
    assert!(trust_scope.as_str().starts_with(TrustScopeId::PREFIX));

    let restarted = PostgresRunStore {
        pool: store.pool.clone(),
    };
    let restarted_trust_scope = restarted
        .load_trust_scope_id()
        .await
        .expect("load restarted trust scope");
    assert_eq!(trust_scope, restarted_trust_scope);

    sqlx::query(
        "UPDATE store_metadata \
         SET trust_scope_id = 'mfm.trust_scope.v1:ffffffffffffffffffffffffffffffff' \
         WHERE singleton",
    )
    .execute(&store.pool)
    .await
    .expect_err("trust scope mutation is rejected");
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

    let admitted = store
        .acquire_execution_claim(&run, holder.clone())
        .await
        .expect("acquire execution claim");
    let NowaitSkipAdmissionResult::Admitted(lease) = admitted else {
        panic!("first execution claim should be admitted");
    };
    assert_eq!(lease.token, holder);
    assert!(matches!(
        store
            .execution_claim_status(&run)
            .await
            .expect("execution claim status"),
        ExecutionClaimStatus::Live(status) if status.token == lease.token
    ));

    let busy = store
        .acquire_execution_claim(&run, other.clone())
        .await
        .expect("busy execution claim");
    let NowaitSkipAdmissionResult::Busy(busy) = busy else {
        panic!("second execution claim should be busy");
    };
    assert_eq!(busy.holder.expect("busy holder").token, lease.token);

    assert!(store
        .renew_execution_claim(&run, &other)
        .await
        .expect("wrong-token renew")
        .is_none());
    assert!(!store
        .release_execution_claim(&run, &other)
        .await
        .expect("wrong-token release"));

    let renewed = store
        .renew_execution_claim(&run, &lease.token)
        .await
        .expect("matching-token renew")
        .expect("matching-token renew returns lease");
    assert_eq!(renewed.token, lease.token);
    assert!(renewed.lease_expires_at_unix_ms >= lease.lease_expires_at_unix_ms);

    assert!(store
        .release_execution_claim(&run, &renewed.token)
        .await
        .expect("matching-token release"));
    assert!(matches!(
        store
            .execution_claim_status(&run)
            .await
            .expect("released execution claim status"),
        ExecutionClaimStatus::Unclaimed
    ));
    assert!(matches!(
        store
            .acquire_execution_claim(&run, other)
            .await
            .expect("acquire after release"),
        NowaitSkipAdmissionResult::Admitted(_)
    ));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn execution_claim_expiry_requires_explicit_reap() {
    let (store, schema) = test_store().await;
    let run = run_id(7);
    let holder = admission_token("mfm.test.execution_claim.expired_holder");
    let other = admission_token("mfm.test.execution_claim.expired_other");

    let admitted = store
        .acquire_execution_claim(&run, holder.clone())
        .await
        .expect("acquire execution claim");
    let NowaitSkipAdmissionResult::Admitted(lease) = admitted else {
        panic!("first execution claim should be admitted");
    };
    expire_execution_claim_row(&store, &run).await;
    assert!(matches!(
        store
            .execution_claim_status(&run)
            .await
            .expect("expired execution claim status"),
        ExecutionClaimStatus::Expired(status) if status.token == lease.token
    ));

    let busy = store
        .acquire_execution_claim(&run, other.clone())
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
        .reap_expired_execution_claim(&run, &other)
        .await
        .expect("wrong-token reap"));
    assert!(matches!(
        store
            .acquire_execution_claim(&run, other.clone())
            .await
            .expect("busy after wrong-token reap"),
        NowaitSkipAdmissionResult::Busy(_)
    ));

    assert!(store
        .reap_expired_execution_claim(&run, &holder)
        .await
        .expect("matching-token reap"));
    assert!(matches!(
        store
            .acquire_execution_claim(&run, other)
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
    let first = admission_token("mfm.test.execution_claim.first");
    let second = admission_token("mfm.test.execution_claim.second");
    let third = admission_token("mfm.test.execution_claim.third");

    assert!(matches!(
        store
            .acquire_execution_claim(&run, first.clone())
            .await
            .expect("first acquire"),
        NowaitSkipAdmissionResult::Admitted(_)
    ));
    expire_execution_claim_row(&store, &run).await;
    assert!(store
        .reap_expired_execution_claim(&run, &first)
        .await
        .expect("first reap"));

    let admitted = store
        .acquire_execution_claim(&run, second.clone())
        .await
        .expect("second acquire");
    let NowaitSkipAdmissionResult::Admitted(second_lease) = admitted else {
        panic!("second execution claim should be admitted");
    };
    expire_execution_claim_row(&store, &run).await;

    assert!(!store
        .reap_expired_execution_claim(&run, &first)
        .await
        .expect("stale-token reap"));
    let busy = store
        .acquire_execution_claim(&run, third)
        .await
        .expect("newer holder still blocks after stale reap");
    let NowaitSkipAdmissionResult::Busy(busy) = busy else {
        panic!("newer holder should remain busy");
    };
    assert_eq!(busy.holder.expect("newer holder").token, second_lease.token);

    assert!(store
        .reap_expired_execution_claim(&run, &second)
        .await
        .expect("newer holder reap"));

    drop_schema(&store, &schema).await;
}

fn digest_bytes(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&test_artifact_bytes(byte)),
    )
}

fn spec_hash(byte: u8) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn test_run_identity_material() -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash: spec_hash(1),
        trust_scope_id: TrustScopeId::new("mfm.trust_scope.v1:40404040404040404040404040404040")
            .expect("test trust scope"),
        distinct_run_key_digest: None,
    }
}

fn artifact_id(byte: u8) -> ArtifactId {
    let digest = content_digest(byte);
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
}

fn test_artifact_bytes(byte: u8) -> Vec<u8> {
    vec![byte; 128]
}

fn test_artifact_bytes_for_digest(digest: &ContentDigest) -> Option<Vec<u8>> {
    (u8::MIN..=u8::MAX).map(test_artifact_bytes).find(|bytes| {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
            == *digest
    })
}

fn test_prepared_artifact_bytes(
    evidence: &ArtifactEvidenceRef,
) -> mfm_store::v1::Result<PreparedArtifactBytes> {
    let bytes = test_artifact_bytes_for_digest(&evidence.digest).ok_or_else(|| {
        StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "bytes",
        }
    })?;
    PreparedArtifactBytes::new(bytes, evidence.clone())
}

fn prepared_artifact_bytes_from_bytes(bytes: Vec<u8>, role: ArtifactRole) -> PreparedArtifactBytes {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let mut evidence = store_artifact_ref(artifact_id, digest, role);
    evidence.byte_len = bytes.len() as u64;
    PreparedArtifactBytes::new(bytes, evidence).expect("prepared artifact bytes")
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn cell_id(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn scope_id(byte: u8) -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn schema_id(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(byte)).expect("schema id")
}

fn semantic_id(name: &str, byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("semantic id")
}

fn state_kind(byte: u8) -> StateKind {
    StateKind::new(
        "mfm.test",
        "state",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("state kind")
}

fn media_type(value: &str) -> MediaType {
    MediaType::new(value).expect("media type")
}

fn admission_token(value: &str) -> AdmissionToken {
    AdmissionToken::new(value).expect("admission token")
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

fn run_admitted(run_id: RunId) -> KernelEventPayload {
    run_admitted_with_saga_policy(run_id, &SagaPolicySpec::NoSideEffects)
}

fn run_admitted_with_saga_policy(
    run_id: RunId,
    saga_policy: &SagaPolicySpec,
) -> KernelEventPayload {
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    let identity_material = test_run_identity_material();
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: entry_point_launch_evidence(),
        spec_hash: spec_hash(1),
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
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
        artifact_id,
        content_digest: digest,
        producer_state_kind: None,
        producer_state_version: None,
    })
}

fn fact_recorded(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::FactRecorded(events::FactRecorded {
        spec_hash: spec_hash(1),
        node_id: node_id(30),
        attempt_id: attempt_id(31),
        capability_kind: CapabilityKind::new(
            "mfm.test",
            "fact",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(32),
        )
        .expect("capability kind"),
        capability_version: CapabilityVersion::new("mfm.test.fact.v1").expect("capability version"),
        adapter_kind: AdapterKind::new(
            "mfm.test",
            "adapter",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(33),
        )
        .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        request_schema_id: schema_id("mfm.test.fact_request", 34),
        request_hash: content_digest(35),
        response_schema_id: schema_id("mfm.test.fact_response", 36),
        response_hash: digest,
        fact_key: events::FactKey::new("fact-key-1").expect("fact key"),
        artifact_id,
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

fn manual_resolution_recorded(verified: &VerifiedManualResolutionForPrefix) -> KernelEventPayload {
    let claim = verified.claim();
    let authorization = verified.authorization();
    KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id: claim.run_id.clone(),
        spec_hash: claim.spec_hash.clone(),
        outcome: claim.outcome,
        evidence_schema_id: claim.evidence.schema_id.clone(),
        evidence_hash: claim.evidence.content_hash.clone(),
        evidence_artifact_id: claim.evidence.artifact_id.clone(),
        authorization_schema_id: authorization.schema_id.clone(),
        authorization_hash: authorization.content_hash.clone(),
        authorization_artifact_id: authorization.artifact_id.clone(),
        note: Some(events::ManualResolutionNote::new("reviewed evidence").expect("note")),
    })
}

fn manual_resolution_artifacts(
    verified: &VerifiedManualResolutionForPrefix,
) -> Vec<ArtifactEvidenceRef> {
    let claim = verified.claim();
    let authorization = verified.authorization();
    vec![
        ArtifactEvidenceRef {
            artifact_id: claim.evidence.artifact_id.clone(),
            digest: claim.evidence.content_hash.clone(),
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(claim.evidence.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionEvidence,
        },
        ArtifactEvidenceRef {
            artifact_id: authorization.artifact_id.clone(),
            digest: authorization.content_hash.clone(),
            byte_len: verified.proof_bytes().len() as u64,
            media_type: media_type("application/json"),
            schema_id: Some(authorization.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionAuthorization,
        },
    ]
}

fn manual_resolution_prepared_artifact_bytes(
    verified: &VerifiedManualResolutionForPrefix,
) -> mfm_store::v1::Result<Vec<PreparedArtifactBytes>> {
    let artifacts = manual_resolution_artifacts(verified);
    Ok(vec![
        test_prepared_artifact_bytes(&artifacts[0])?,
        PreparedArtifactBytes::new(verified.proof_bytes().to_vec(), artifacts[1].clone())?,
    ])
}

fn manual_saga_policy(byte: u8) -> SagaPolicySpec {
    SagaPolicySpec::ManualResolution {
        manual: ManualResolutionEvidenceSpec {
            evidence_schema: schema_id("mfm.test.manual_evidence", byte + 1),
            authorization: manual_authorization(byte),
        },
    }
}

fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
            "mfm.test.manual.verifier.{byte}"
        ))
        .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new(format!(
                "mfm.test.manual.authority.{byte}"
            ))
            .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                    .expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(
                    "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                )
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

fn verified_manual_resolution_for_seq(
    run_id: &RunId,
    expected_next_seq: u64,
    byte: u8,
) -> VerifiedManualResolutionForPrefix {
    let SagaPolicySpec::ManualResolution { manual } = manual_saga_policy(byte) else {
        unreachable!("manual_saga_policy builds manual policy")
    };
    let prefix = ManualResolutionPrefixAuthority::new(
        run_id.clone(),
        spec_hash(1),
        expected_next_seq,
        content_digest(byte + 3),
        ManualResolutionBlockReason::PolicyManualResolution,
        content_digest(byte + 4),
        manual.clone(),
    )
    .expect("manual prefix authority");
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        content_hash: content_digest(byte + 1),
        artifact_id: artifact_id(byte + 1),
    };
    let claim = prefix
        .authorization_claim(
            events::ManualResolutionOutcome::ConfirmRemediated,
            evidence.clone(),
        )
        .expect("manual authorization claim");
    let proof = signed_manual_resolution_proof(&manual.authorization, claim.clone());
    let proof_bytes = proof
        .canonical_json()
        .expect("manual proof canonical json")
        .to_vec();
    let authorization = manual_authorization_ref(&proof_bytes);
    ManualResolutionProofAuthority::new(prefix, claim.outcome, evidence, authorization, proof_bytes)
        .and_then(ManualResolutionProofAuthority::verify)
        .expect("verified manual resolution")
}

fn signed_manual_resolution_proof(
    policy: &spec::ManualResolutionAuthorizationSpec,
    claim: mfm_manual_auth::ManualResolutionAuthorizationClaim,
) -> ManualResolutionAuthorizationProof {
    let operator = policy.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("manual claim digest");
    ManualResolutionAuthorizationProof {
        verifier_id: policy.verifier_id.clone(),
        signing_scheme: policy.signing_scheme.clone(),
        claim,
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("manual signature bytes"),
        }],
    }
}

fn manual_authorization_ref(proof_bytes: &[u8]) -> ManualResolutionEvidenceRef {
    let proof = ManualResolutionAuthorizationProof::from_json_slice(proof_bytes)
        .expect("manual authorization proof");
    let content_hash = proof.content_digest().expect("manual proof content digest");
    ManualResolutionEvidenceRef {
        schema_id: manual_authorization_proof_schema_id().expect("manual authorization schema"),
        artifact_id: ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest()),
        content_hash,
    }
}

fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

fn saga_preconditions(run_id: &RunId, policy: SagaPolicySpec) -> CommitPreconditions {
    CommitPreconditions {
        saga_admit_token: Some(
            mfm_store::v1::SagaAdmitToken::new(run_id.clone(), spec_hash(1), policy)
                .expect("saga admit token"),
        ),
        ..CommitPreconditions::default()
    }
}

fn side_effect_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key")
}

fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

fn side_effect_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        attempt_no: 1,
        state_kind: state_kind(70),
        state_version: StateVersion::new("mfm.test.side_effect_state.v1").expect("state version"),
    })
}

fn side_effect_intent(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectIntentPersisted(events::side_effect::IntentPersisted {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        scope_id: scope_id(71),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.test.side_effect_intent", 70),
        intent_hash: digest,
        intent_artifact_id: artifact_id,
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
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        invocation_epoch: 1,
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
    })
}

fn side_effect_prepared() -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationPrepared(events::side_effect::InvocationPrepared {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
        resource_key: None,
        prepared_artifact_id: None,
        prepared_hash: None,
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
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
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
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
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
    KernelEventPayload::SideEffectSubmissionUnknown(events::side_effect::SubmissionUnknown {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        evidence_schema_id: unknown_schema(),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
    })
}

fn side_effect_submission_observed(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectSubmissionObserved(events::side_effect::SubmissionObserved {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        submission_schema_id: submission_schema(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
    })
}

fn side_effect_ambiguous(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        ambiguity_code: events::AmbiguityCode::new("ambiguous").expect("ambiguity code"),
        evidence_schema_id: schema_id("mfm.test.ambiguity", 84),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
    })
}

fn side_effect_attempt_failed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
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
    KernelEventPayload::SideEffectFailed(events::side_effect::Failed {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
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
    KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
        run_id,
        spec_hash: spec_hash(1),
        refs: vec![events::RetentionRef {
            artifact_id,
            role,
            content_digest: digest,
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
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: Some(node_id(70)),
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
        schema_id: None,
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
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedSpecCertificate,
    }
}

fn run_artifact_ref(artifact: &ArtifactEvidenceRef) -> events::RunArtifactEvidenceRef {
    events::RunArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
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
        vec![retention_refs_appended(
            run_id,
            evidence.artifact_id.clone(),
            evidence.digest.clone(),
            evidence.artifact_role,
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

fn test_prepared_commit_plan(
    request: mfm_store::v1::CommitRequest,
    artifacts: Vec<ArtifactEvidenceRef>,
) -> mfm_store::v1::Result<PreparedCommitPlan> {
    let artifact_set =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), artifacts)?;
    let payloads = request.payloads();
    if payloads
        .iter()
        .all(|payload| matches!(payload, KernelEventPayload::RunAdmitted(_)))
    {
        let mut preconditions = request.preconditions().clone();
        preconditions.required_run_state = RequiredRunState::Absent;
        let request = request.with_preconditions(preconditions);
        return Ok(PreparedCommit::<RunAdmission>::new(request, artifact_set)?.into());
    }
    if payloads
        .iter()
        .all(|payload| matches!(payload, KernelEventPayload::StateAttemptStarted(_)))
    {
        let mut preconditions = request.preconditions().clone();
        preconditions.required_run_state = RequiredRunState::NotCompleted;
        let request = request.with_preconditions(preconditions);
        return Ok(PreparedCommit::<StateAttemptStarted>::new(request, artifact_set)?.into());
    }
    if payloads.iter().any(test_is_run_completed_payload) {
        return Ok(PreparedCommit::<AttemptTerminal>::new(request, artifact_set)?.into());
    }
    if payloads.iter().any(test_is_side_effect_terminal_payload) {
        return Ok(PreparedCommit::<SideEffectTerminal>::new(request, artifact_set)?.into());
    }
    if payloads
        .iter()
        .any(|payload| payload.side_effect_ref().is_some())
    {
        return Ok(PreparedCommit::<SideEffectProgress>::new(request, artifact_set)?.into());
    }
    if payloads.iter().any(test_is_retention_payload) {
        return Ok(PreparedCommit::<Retention>::new(request, artifact_set)?.into());
    }
    Ok(PreparedCommit::<AttemptTerminal>::new(request, artifact_set)?.into())
}

fn test_is_retention_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RetentionRefsAppended(_)
            | KernelEventPayload::RetentionManifestProjected(_)
    )
}

fn test_is_run_completed_payload(payload: &KernelEventPayload) -> bool {
    matches!(payload, KernelEventPayload::RunCompleted(_))
}

fn test_is_side_effect_terminal_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::SideEffectNotSubmittedProven(_)
            | KernelEventPayload::SideEffectSubmissionObserved(_)
            | KernelEventPayload::SideEffectSubmissionUnknown(_)
            | KernelEventPayload::SideEffectReceiptObserved(_)
            | KernelEventPayload::SideEffectConfirmationObserved(_)
            | KernelEventPayload::SideEffectAmbiguous(_)
            | KernelEventPayload::SideEffectFailed(_)
            | KernelEventPayload::ResourceLaneReleaseIntent(_)
            | KernelEventPayload::ResourceLaneReleased(_)
    )
}

async fn append_run_start(
    store: &PostgresRunStore,
    run_id: &RunId,
    commit_key: &str,
) -> Result<CommitOutcome> {
    append_prepared(
        store,
        request(
            run_id.clone(),
            1,
            commit_key,
            vec![run_admitted(run_id.clone())],
        ),
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
        request(
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
        request(
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
    append_prepared(
        store,
        request(
            run_id.clone(),
            next_seq,
            commit_key,
            vec![
                KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
                    spec_hash: spec_hash(1),
                    node_id: lane.node_id.clone(),
                    attempt_id: lane.attempt_id.clone(),
                    ledger_key: side_effect_ledger_key(),
                    ledger_purpose: lane.ledger_purpose.clone(),
                    invocation_epoch: lane.invocation_epoch,
                    claim_id: lane.claim_id.clone(),
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
    assert_eq!(holder.ledger_key, side_effect_ledger_key());
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
            assert_eq!(holder.ledger_key, side_effect_ledger_key());
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

async fn observation_row_cursor_for_commit(
    store: &PostgresRunStore,
    run: &RunId,
    seq: u64,
) -> String {
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let row = sqlx::query(
        "SELECT append_xid::text AS append_xid, commit_sort_key \
         FROM commits WHERE run_id = $1 AND seq = $2",
    )
    .bind(run.as_str())
    .bind(i64::try_from(seq).expect("seq fits i64"))
    .fetch_one(&store.pool)
    .await
    .expect("load commit cursor position");
    encode_observation_cursor(
        &store.pool,
        &metadata,
        &CursorPosition {
            append_xid: row.try_get("append_xid").expect("append xid"),
            commit_sort_key: row.try_get("commit_sort_key").expect("sort key"),
            kind: CursorKind::Row,
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

#[tokio::test]
async fn prepared_commit_idempotency_fingerprint_includes_admitted_artifacts() {
    let (store, schema) = test_store().await;
    let run = run_id(120);
    let artifact = artifact_id(121);
    let digest = content_digest(121);
    let evidence = store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let mut conflicting_evidence = evidence.clone();
    conflicting_evidence.media_type = media_type("application/octet-stream");
    append_prepared(
        &store,
        request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    let request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        store.expected_next_seq(&run).await.expect("next seq"),
        CommitKey::new("prepared-fingerprint").expect("commit key"),
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
    .expect("typed commit request");

    append_prepared(&store, request.clone(), vec![evidence])
        .await
        .expect("append initial prepared commit");
    let retry = request.with_expected_next_seq(StreamSeq::new(99).expect("stale seq"));
    let error = append_prepared(&store, retry, vec![conflicting_evidence])
        .await
        .expect_err("same request with different admitted evidence is not idempotent");
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::CommitConflict { .. })
    ));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn commit_sort_keys_use_v1_rfc_tuple() {
    let (store, schema) = test_store().await;
    let run = run_id(125);
    append_run_start(&store, &run, "sort-key-run-start")
        .await
        .expect("run start");
    append_resource_lane_attempt_start(&store, &run, "sort-key-attempt-start")
        .await
        .expect("attempt start");

    let rows = sqlx::query(
        "SELECT seq, commit_sort_key, commit_key, commit_id, commit_batch_hash \
         FROM commits \
         WHERE run_id = $1 \
         ORDER BY seq",
    )
    .bind(run.as_str())
    .fetch_all(&store.pool)
    .await
    .expect("query commit sort keys");
    assert_eq!(rows.len(), 2);
    let mut previous: Option<Vec<u8>> = None;
    for row in rows {
        let seq: i64 = row.try_get("seq").expect("seq");
        let commit_sort_key: Vec<u8> = row.try_get("commit_sort_key").expect("commit_sort_key");
        let commit_key = CommitKey::new(row.try_get::<String, _>("commit_key").expect("key"))
            .expect("commit key");
        let commit_id: String = row.try_get("commit_id").expect("commit id");
        let commit_batch_hash: String =
            row.try_get("commit_batch_hash").expect("commit batch hash");
        assert_eq!(commit_sort_key.len(), 32);
        assert_eq!(commit_sort_key[0], 1);
        assert_ne!(commit_sort_key, vec![0; 32]);
        assert_eq!(
            commit_sort_key,
            derive_commit_sort_key(
                &run,
                StreamSeq::new(i64_to_positive_u64(seq, "commits.seq").expect("positive seq"))
                    .expect("stream seq"),
                &commit_key,
                &commit_id,
                &commit_batch_hash,
            )
            .expect("expected sort key")
        );
        if let Some(previous) = previous.replace(commit_sort_key.clone()) {
            assert_ne!(previous, commit_sort_key);
        }
    }

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn strict_load_rejects_corrupt_commit_fingerprint() {
    let (store, schema) = test_store().await;
    let run = run_id(126);
    append_run_start(&store, &run, "strict-canonical-run-start")
        .await
        .expect("run start");

    sqlx::query("ALTER TABLE commits DISABLE TRIGGER commits_no_update")
        .execute(&store.pool)
        .await
        .expect("disable commit mutation guard");
    sqlx::query(
        "UPDATE commits SET prepared_commit_plan_fingerprint = $1 WHERE run_id = $2 AND seq = 1",
    )
    .bind(content_digest(251).as_str())
    .bind(run.as_str())
    .execute(&store.pool)
    .await
    .expect("corrupt commit fingerprint");

    let error = store
        .load_run_stream(&run)
        .await
        .expect_err("strict load rejects corrupt commit fingerprint");
    assert_corruption(error, "commit id does not match persisted fingerprint");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn strict_load_rejects_commit_batch_authority_rewritten_away_from_rows() {
    let (store, schema) = test_store().await;
    let run = run_id(144);
    let commit_key = CommitKey::new("strict-batch-authority-run-start").expect("commit key");
    append_run_start(&store, &run, commit_key.as_str())
        .await
        .expect("run start");

    let commit_id: String =
        sqlx::query_scalar("SELECT commit_id FROM commits WHERE run_id = $1 AND seq = 1")
            .bind(run.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("load commit id");
    let forged_batch = canonical_json(serde_json::json!({
        "domain": "mfm.commit.batch.v1",
        "tampered": true,
    }))
    .expect("canonical forged batch");
    let forged_batch_hash = forged_batch.content_digest().as_str().to_owned();
    let forged_sort_key = derive_commit_sort_key(
        &run,
        StreamSeq::new(1).expect("seq"),
        &commit_key,
        &commit_id,
        &forged_batch_hash,
    )
    .expect("forged sort key");

    sqlx::query("ALTER TABLE commits DISABLE TRIGGER commits_no_update")
        .execute(&store.pool)
        .await
        .expect("disable commit mutation guard");
    sqlx::query(
        "UPDATE commits SET commit_batch_hash = $1, commit_sort_key = $2 \
         WHERE run_id = $3 AND seq = 1",
    )
    .bind(&forged_batch_hash)
    .bind(forged_sort_key)
    .bind(run.as_str())
    .execute(&store.pool)
    .await
    .expect("forge commit batch authority");

    let error = store
        .load_run_stream(&run)
        .await
        .expect_err("strict load rejects batch authority that no longer matches rows");
    assert_corruption(
        error,
        "commit batch authority does not match persisted event and artifact bindings",
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn strict_load_rejects_corrupt_commit_sort_key() {
    let (store, schema) = test_store().await;
    let run = run_id(127);
    append_run_start(&store, &run, "strict-sort-key-run-start")
        .await
        .expect("run start");

    sqlx::query("ALTER TABLE commits DISABLE TRIGGER commits_no_update")
        .execute(&store.pool)
        .await
        .expect("disable commit mutation guard");
    sqlx::query("UPDATE commits SET commit_sort_key = $1 WHERE run_id = $2 AND seq = 1")
        .bind(vec![1_u8; 32])
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("corrupt commit sort key");

    let error = store
        .load_run_stream(&run)
        .await
        .expect_err("strict load rejects corrupt commit sort key");
    assert_corruption(error, "commit sort key does not match");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_change_ids_and_cursors_do_not_expose_internal_authority() {
    let (store, schema) = test_store().await;
    let run = run_id(134);
    append_run_start(&store, &run, "opaque-observation-run-start")
        .await
        .expect("run start");

    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let row = sqlx::query(
        "SELECT commit_id, append_xid::text AS append_xid, commit_sort_key \
         FROM commits WHERE run_id = $1 AND seq = 1",
    )
    .bind(run.as_str())
    .fetch_one(&store.pool)
    .await
    .expect("load commit cursor authority");
    let commit_id: String = row.try_get("commit_id").expect("commit id");
    let append_xid: String = row.try_get("append_xid").expect("append xid");
    let commit_sort_key: Vec<u8> = row.try_get("commit_sort_key").expect("commit sort key");
    let public_change_id =
        observation_change_id(&metadata, &commit_id).expect("observation change id");
    let commit_sort_key_hex = bytes_hex(&commit_sort_key);
    let position = CursorPosition {
        append_xid,
        commit_sort_key,
        kind: CursorKind::Row,
    };
    let public_cursor = encode_observation_cursor(&store.pool, &metadata, &position)
        .await
        .expect("encode observation cursor");
    let decoded = decode_observation_cursor(&store.pool, &public_cursor, &metadata)
        .await
        .expect("decode observation cursor");
    assert_eq!(decoded.append_xid, position.append_xid);
    assert_eq!(decoded.commit_sort_key, position.commit_sort_key);
    assert_eq!(decoded.kind, CursorKind::Row);
    assert_eq!(
        bytes_from_hex(&public_cursor)
            .expect("opaque cursor token is hex")
            .len(),
        32
    );
    assert_eq!(
        bytes_from_hex(&public_change_id)
            .expect("opaque change id token is hex")
            .len(),
        32
    );
    for public_value in [public_cursor.as_str(), public_change_id.as_str()] {
        assert!(!public_value.contains('|'));
        assert!(!public_value.contains(CURSOR_VERSION));
        assert!(!public_value.contains(commit_id.as_str()));
        assert!(!public_value.contains(commit_sort_key_hex.as_str()));
        assert!(!public_value.contains(metadata.store_epoch.as_str()));
    }
    assert_ne!(public_change_id.as_str(), commit_id);

    let cursor_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_observation_cursors")
        .fetch_one(&store.pool)
        .await
        .expect("count cursor rows");
    assert_eq!(cursor_rows, 1);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_lifecycle_rejects_malformed_unknown_and_missing_tokens() {
    let (store, schema) = test_store().await;
    let run = run_id(145);
    append_run_start(&store, &run, "cursor-lifecycle-run-start")
        .await
        .expect("run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");

    let malformed = decode_observation_cursor(&store.pool, "not-hex", &metadata)
        .await
        .expect_err("non-hex cursor is invalid");
    assert_invalid_cursor(malformed, "hex");
    let unknown = decode_observation_cursor(&store.pool, &"00".repeat(32), &metadata)
        .await
        .expect_err("unknown cursor is invalid");
    assert_invalid_cursor(unknown, "unknown cursor");

    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    sqlx::query(
        "ALTER TABLE run_observation_cursors DISABLE TRIGGER \
         run_observation_cursors_no_update",
    )
    .execute(&store.pool)
    .await
    .expect("disable cursor mutation guard");
    sqlx::query("DELETE FROM run_observation_cursors WHERE token_hash = $1")
        .bind(observation_cursor_token_hash(&cursor))
        .execute(&store.pool)
        .await
        .expect("delete cursor row");
    let missing = decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect_err("missing cursor row is invalid");
    assert_invalid_cursor(missing, "unknown cursor");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_lifecycle_is_epoch_bound_without_ttl() {
    let (store, schema) = test_store().await;
    let run = run_id(146);
    append_run_start(&store, &run, "cursor-ttl-run-start")
        .await
        .expect("run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    let token_hash = observation_cursor_token_hash(&cursor);

    sqlx::query(
        "ALTER TABLE run_observation_cursors DISABLE TRIGGER \
         run_observation_cursors_no_update",
    )
    .execute(&store.pool)
    .await
    .expect("disable cursor mutation guard");
    sqlx::query(
        "UPDATE run_observation_cursors SET issued_at = '2000-01-01T00:00:00Z'::timestamptz \
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&store.pool)
    .await
    .expect("age cursor");
    decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect("old issued_at does not expire current-epoch cursor");

    sqlx::query(
        "UPDATE run_observation_cursors \
         SET store_epoch = 'mfm.store.epoch.v1:old' \
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&store.pool)
    .await
    .expect("move cursor to old epoch");
    let expired = decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect_err("old epoch cursor expires");
    assert_cursor_expired(expired);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_lifecycle_rejects_stale_format() {
    let (store, schema) = test_store().await;
    let run = run_id(147);
    append_run_start(&store, &run, "cursor-format-run-start")
        .await
        .expect("run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    let token_hash = observation_cursor_token_hash(&cursor);

    sqlx::query(
        "ALTER TABLE run_observation_cursors DISABLE TRIGGER \
         run_observation_cursors_no_update",
    )
    .execute(&store.pool)
    .await
    .expect("disable cursor mutation guard");
    sqlx::query(
        "ALTER TABLE run_observation_cursors DROP CONSTRAINT run_observation_cursors_version_v1",
    )
    .execute(&store.pool)
    .await
    .expect("drop cursor version constraint for stale-format fixture");
    sqlx::query(
        "UPDATE run_observation_cursors \
         SET cursor_version = 'mfm.run_observation.cursor.v0' \
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&store.pool)
    .await
    .expect("stale cursor format");
    let stale = decode_observation_cursor(&store.pool, &cursor, &metadata)
        .await
        .expect_err("stale cursor format is invalid");
    assert_invalid_cursor(stale, "stale cursor format");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_cursor_uses_durable_metadata_across_store_restarts() {
    let (store, schema) = test_store().await;
    let run = run_id(148);
    append_run_start(&store, &run, "cursor-restart-run-start")
        .await
        .expect("run start");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    let restarted = PostgresRunStore {
        pool: store.pool.clone(),
    };
    let page = restarted
        .read_run_observations(RunObservationQuery::new(Some(cursor), 10, 0))
        .await
        .expect("restarted store decodes durable cursor metadata");
    assert!(page.runs.is_empty());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_watch_cursor_pages_without_skipping_rows() {
    let (store, schema) = test_store().await;
    let run = run_id(149);
    append_run_start(&store, &run, "cursor-noskip-run-start")
        .await
        .expect("run start");
    append_retention_commit(&store, &run, 2, "cursor-noskip-retention-a", 150)
        .await
        .expect("append second observation row");
    append_retention_commit(&store, &run, 3, "cursor-noskip-retention-b", 151)
        .await
        .expect("append third observation row");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;

    let first = store
        .read_run_observations(RunObservationQuery::new(Some(cursor), 1, 5_000))
        .await
        .expect("first watch page");
    assert_eq!(first.runs.len(), 1);
    assert_eq!(first.runs[0].head_seq.as_u64(), 2);
    let second = store
        .read_run_observations(RunObservationQuery::new(Some(first.next_cursor), 1, 5_000))
        .await
        .expect("second watch page");
    assert_eq!(second.runs.len(), 1);
    assert_eq!(second.runs[0].head_seq.as_u64(), 3);
    let third = store
        .read_run_observations(RunObservationQuery::new(Some(second.next_cursor), 1, 0))
        .await
        .expect("third watch page");
    assert!(third.runs.is_empty());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_watch_polls_durable_rows_before_waiting_for_notify() {
    let (store, schema) = test_store().await;
    let run = run_id(152);
    append_run_start(&store, &run, "cursor-missed-notify-run-start")
        .await
        .expect("run start");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;
    append_retention_commit(&store, &run, 2, "cursor-missed-notify-retention", 153)
        .await
        .expect("append change before watcher starts");

    let started = Instant::now();
    let page = store
        .read_run_observations(RunObservationQuery::new(Some(cursor), 10, 5_000))
        .await
        .expect("watch polls durable rows before waiting");
    assert_eq!(page.runs.len(), 1);
    assert_eq!(page.runs[0].head_seq.as_u64(), 2);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "watch should return durable rows without waiting for notify"
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn append_commit_emits_observation_notification() {
    let (store, schema) = test_store().await;
    let mut listener = observation_change_listener(&store.pool)
        .await
        .expect("listen for observation changes");
    let run = run_id(141);

    append_run_start(&store, &run, "notify-run-start")
        .await
        .expect("append run start");

    let notification = tokio::time::timeout(Duration::from_secs(2), listener.recv())
        .await
        .expect("append should notify before timeout")
        .expect("receive observation notification");
    assert_eq!(notification.channel(), OBSERVATION_NOTIFY_CHANNEL);
    assert_eq!(notification.payload(), OBSERVATION_NOTIFY_PAYLOAD);
    drop(listener);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_watch_polls_until_frontier_advances_without_notify() {
    let (store, schema) = test_store().await;
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let blocker_options = PgConnectOptions::from_str(&database_url)
        .expect("postgres URL")
        .options([("search_path", schema.as_str())]);
    let blocker_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(blocker_options)
        .await
        .expect("connect blocker pool");

    let run = run_id(154);
    append_run_start(&store, &run, "cursor-frontier-lag-run-start")
        .await
        .expect("run start");
    let cursor = observation_row_cursor_for_commit(&store, &run, 1).await;

    let mut blocker = blocker_pool.begin().await.expect("begin blocker tx");
    let _blocker_xid: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
        .fetch_one(&mut *blocker)
        .await
        .expect("assign blocker xid");
    append_retention_commit(&store, &run, 2, "cursor-frontier-lag-retention", 155)
        .await
        .expect("append row behind blocked frontier");

    let stale_frontier_page = store
        .read_run_observations(RunObservationQuery::new(Some(cursor.clone()), 10, 0))
        .await
        .expect("frontier-lagged watch page");
    assert!(stale_frontier_page.runs.is_empty());

    let watcher_store = store.clone();
    let watcher = tokio::spawn(async move {
        let started = Instant::now();
        let page = watcher_store
            .read_run_observations(RunObservationQuery::new(Some(cursor), 10, 5_000))
            .await
            .expect("watch observes row after frontier advances");
        (page, started.elapsed())
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    blocker.rollback().await.expect("release blocker tx");
    blocker_pool.close().await;

    let (page, elapsed) = watcher.await.expect("watch task joins");
    assert_eq!(page.runs.len(), 1);
    assert_eq!(page.runs[0].head_seq.as_u64(), 2);
    assert!(
        elapsed < Duration::from_secs(2),
        "watch should poll for frontier advancement without waiting for timeout; elapsed={elapsed:?}"
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn observation_watch_wakes_on_notification_before_timeout() {
    let (store, schema) = test_store().await;
    let run = run_id(142);
    append_run_start(&store, &run, "watch-notify-run-start")
        .await
        .expect("append run start");
    let metadata = load_store_metadata(&store.pool)
        .await
        .expect("load store metadata");
    let row = sqlx::query(
        "SELECT append_xid::text AS append_xid, commit_sort_key \
         FROM commits WHERE run_id = $1 AND seq = 1",
    )
    .bind(run.as_str())
    .fetch_one(&store.pool)
    .await
    .expect("load initial commit cursor position");
    let cursor = encode_observation_cursor(
        &store.pool,
        &metadata,
        &CursorPosition {
            append_xid: row.try_get("append_xid").expect("append xid"),
            commit_sort_key: row.try_get("commit_sort_key").expect("sort key"),
            kind: CursorKind::Row,
        },
    )
    .await
    .expect("encode row cursor");
    let watcher_store = store.clone();
    let watcher = tokio::spawn(async move {
        let started = Instant::now();
        let page = watcher_store
            .read_run_observations(RunObservationQuery::new(Some(cursor), 10, 5_000))
            .await
            .expect("watch observations");
        (page, started.elapsed())
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let artifact = artifact_id(143);
    let digest = content_digest(143);
    let evidence = store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let change_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        StreamSeq::new(2).expect("seq"),
        CommitKey::new("watch-notify-retention").expect("commit key"),
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
    append_prepared(&store, change_request, vec![evidence])
        .await
        .expect("append observed change");
    let notify_pool = store.pool.clone();
    let notifier = tokio::spawn(async move {
        for _ in 0..20 {
            let _ = sqlx::query("SELECT pg_notify($1, $2)")
                .bind(OBSERVATION_NOTIFY_CHANNEL)
                .bind(OBSERVATION_NOTIFY_PAYLOAD)
                .execute(&notify_pool)
                .await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });

    let (page, elapsed) = watcher.await.expect("watch task joins");
    notifier.abort();
    let _ = notifier.await;
    assert!(
        elapsed < Duration::from_secs(2),
        "watch should wake from notification before long-poll timeout; elapsed={elapsed:?}"
    );
    assert_eq!(page.runs.len(), 1);
    assert_eq!(page.runs[0].run_id, run);
    assert_eq!(page.runs[0].head_seq.as_u64(), 2);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn artifact_authority_accepts_distinct_evidence_for_same_artifact_id() {
    let (store, schema) = test_store().await;
    let run = run_id(122);
    let artifact = artifact_id(123);
    let digest = content_digest(123);
    let first_evidence =
        store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
    let mut second_evidence = first_evidence.clone();
    second_evidence.schema_id = Some(schema_id("mfm.test.alternate_position", 124));
    assert_ne!(
        first_evidence.evidence_hash().expect("first evidence hash"),
        second_evidence
            .evidence_hash()
            .expect("second evidence hash")
    );

    append_prepared(
        &store,
        request(
            run.clone(),
            1,
            "same-id-run-start",
            vec![run_admitted(run.clone())],
        ),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");

    let first_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        store.expected_next_seq(&run).await.expect("next seq"),
        CommitKey::new("same-id-first-evidence").expect("commit key"),
        vec![retention_refs_appended(
            run.clone(),
            artifact.clone(),
            digest.clone(),
            ArtifactRole::StateOutput,
        )],
        vec![first_evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("typed first request");
    append_prepared(&store, first_request, vec![first_evidence])
        .await
        .expect("append first evidence");

    let second_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        store.expected_next_seq(&run).await.expect("next seq"),
        CommitKey::new("same-id-second-evidence").expect("commit key"),
        vec![retention_refs_appended_with_reason(
            run.clone(),
            artifact,
            digest,
            ArtifactRole::StateOutput,
            events::RetentionReason::PublicOutput,
        )],
        vec![second_evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("typed second request");
    append_prepared(&store, second_request, vec![second_evidence])
        .await
        .expect("append second evidence for same artifact id");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn artifact_precondition_failure_rolls_back_blob_insert() {
    let (store, schema) = test_store().await;
    let run = run_id(135);
    append_run_start(&store, &run, "blob-rollback-run-start")
        .await
        .expect("run start");
    let artifact =
        prepared_artifact_bytes_from_bytes(vec![0x5a; 1024 * 1024 + 1], ArtifactRole::StateOutput);
    let evidence = artifact.evidence().clone();
    let bundle = retention_artifact_bundle(
        run.clone(),
        2,
        "blob-rollback-retention",
        artifact,
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            ..CommitPreconditions::default()
        },
    )
    .expect("large retention bundle");

    let error = store
        .append_prepared_commit_bundle(bundle)
        .await
        .expect_err("precondition fails before blob commit");
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::RunStatePreconditionFailed { .. })
    ));
    let blob_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_blobs WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count rolled-back blob");
    assert_eq!(blob_count, 0);
    let admission_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_admissions WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count rolled-back admissions");
    assert_eq!(admission_count, 0);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn artifact_blob_success_is_admitted_in_append_transaction() {
    let (store, schema) = test_store().await;
    let run = run_id(136);
    append_run_start(&store, &run, "blob-success-run-start")
        .await
        .expect("run start");
    let artifact =
        prepared_artifact_bytes_from_bytes(vec![0x6b; 1024 * 1024 + 1], ArtifactRole::StateOutput);
    let evidence = artifact.evidence().clone();
    let bundle = retention_artifact_bundle(
        run.clone(),
        2,
        "blob-success-retention",
        artifact,
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("large retention bundle");

    let outcome = store
        .append_prepared_commit_bundle(bundle)
        .await
        .expect("large blob append");
    assert!(matches!(outcome, CommitOutcome::Appended(_)));
    let blob_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_blobs WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count committed blob");
    assert_eq!(blob_count, 1);
    let admission_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_admissions WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count committed admissions");
    assert_eq!(admission_count, 1);
    sqlx::query("DELETE FROM artifact_blobs WHERE artifact_id = $1")
        .bind(evidence.artifact_id.as_str())
        .execute(&store.pool)
        .await
        .expect_err("direct artifact blob delete is blocked");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn oversized_artifact_blob_is_rejected_before_authority_rows() {
    let (store, schema) = test_store().await;
    let run = run_id(137);
    append_run_start(&store, &run, "oversize-run-start")
        .await
        .expect("run start");
    let artifact = prepared_artifact_bytes_from_bytes(
        vec![0x7c; MAX_ARTIFACT_BLOB_BYTES as usize + 1],
        ArtifactRole::StateOutput,
    );
    let evidence = artifact.evidence().clone();
    let bundle = retention_artifact_bundle(
        run.clone(),
        2,
        "oversize-retention",
        artifact,
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("oversize retention bundle");

    let error = store
        .append_prepared_commit_bundle(bundle)
        .await
        .expect_err("oversized artifact rejected");
    assert!(matches!(
        error,
        PostgresStoreError::Store(StoreError::ArtifactEvidenceMismatch {
            field: "byte_len",
            ..
        })
    ));
    let blob_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM artifact_blobs WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_one(&store.pool)
            .await
            .expect("count oversized blob rows");
    assert_eq!(blob_count, 0);
    let commit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM commits WHERE run_id = $1")
        .bind(run.as_str())
        .fetch_one(&store.pool)
        .await
        .expect("count run commits");
    assert_eq!(commit_count, 1);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn commit_key_sequence_and_projection_rebuild_contract() {
    let (store, schema) = test_store().await;
    let run = run_id(7);
    let artifact = artifact_id(8);
    let digest = content_digest(8);

    append_prepared(
        &store,
        request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "attempt-start",
            vec![state_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt start");
    let terminal_request = request(
        run.clone(),
        3,
        "terminal",
        vec![
            cell_produced(artifact.clone(), digest.clone()),
            state_attempt_completed(),
        ],
    );
    let appended = append_prepared(
        &store,
        terminal_request.clone(),
        vec![store_artifact_ref(
            artifact.clone(),
            digest.clone(),
            ArtifactRole::StateOutput,
        )],
    )
    .await
    .expect("terminal commit");
    let CommitOutcome::Appended(appended_batch) = appended else {
        panic!("terminal commit should append");
    };
    let appended_fingerprint = appended_batch.fingerprint().clone();

    let stale_retry =
        terminal_request.with_expected_next_seq(StreamSeq::new(1).expect("stale seq"));
    let idempotent = append_prepared(
        &store,
        stale_retry,
        vec![store_artifact_ref(
            artifact,
            digest,
            ArtifactRole::StateOutput,
        )],
    )
    .await
    .expect("idempotent retry before stale seq");
    let CommitOutcome::Idempotent(idempotent_batch) = idempotent else {
        panic!("terminal retry should be idempotent");
    };
    assert_eq!(idempotent_batch.fingerprint(), &appended_fingerprint);
    assert_eq!(
        store.expected_next_seq(&run).await.expect("next seq"),
        StreamSeq::new(4).expect("seq")
    );

    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    assert!(matches!(
        before.cell_terminal(&cell_id(21)),
        Some(CellTerminalProjection::Produced { .. })
    ));
    let stream = store.load_run_stream(&run).await.expect("typed run stream");
    assert_eq!(stream.len(), 4);
    assert_eq!(
        ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("payload rebuild"),
        before
    );

    assert_eq!(
        store
            .status_projection_snapshot(&run)
            .await
            .expect("stream-authoritative projection rebuilds from events"),
        before
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn interrupted_attempt_projection_rebuilds_from_events() {
    let (store, schema) = test_store().await;
    let run = run_id(17);

    append_prepared(
        &store,
        request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "attempt-start",
            vec![state_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt start");
    append_prepared(
        &store,
        request(
            run.clone(),
            3,
            "attempt-interrupted",
            vec![state_attempt_interrupted()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt interrupted");

    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let attempt = before
        .attempt(&node_id(20), &attempt_id(23))
        .expect("attempt projection");
    assert!(matches!(&attempt.status, AttemptStatus::Interrupted));
    assert!(before.saga_engagement(&run).is_none());

    let rebuilt = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection rebuilds from events");
    assert_eq!(rebuilt, before);
    assert!(matches!(
        &rebuilt
            .attempt(&node_id(20), &attempt_id(23))
            .expect("stream-derived attempt projection")
            .status,
        AttemptStatus::Interrupted
    ));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn resource_lane_projection_rebuilds_from_events() {
    let (store, schema) = test_store().await;
    let run = run_id(21);
    let intent_artifact = artifact_id(22);
    let intent_digest = content_digest(22);
    let lane_key = resource_lane_key("wallet-1");

    append_prepared(
        &store,
        request(
            run.clone(),
            1,
            "resource-run-start",
            vec![run_admitted(run.clone())],
        ),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "resource-attempt-start",
            vec![side_effect_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt start");
    append_prepared(
        &store,
        request(
            run.clone(),
            3,
            "resource-prepare",
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                resource_lane_claim_intent(resource_key("wallet-1", 201)),
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
    .expect("prepare with resource key");

    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let lane = before.resource_lane(&lane_key).expect("resource lane");
    assert_eq!(lane.holder.run_id, run);
    assert_eq!(lane.holder.ledger_key, side_effect_ledger_key());

    let stream = store.load_run_stream(&run).await.expect("typed run stream");
    assert_eq!(
        ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("payload rebuild"),
        before
    );

    let peer_run = run_id(24);
    append_prepared(
        &store,
        request(
            peer_run.clone(),
            1,
            "resource-peer-run-start",
            vec![run_admitted(peer_run.clone())],
        ),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("peer run start");
    let peer_before = store
        .status_projection_snapshot(&peer_run)
        .await
        .expect("peer projection");
    let peer_lane = peer_before
        .resource_lane(&lane_key)
        .expect("peer snapshot includes cross-run lane");
    assert_eq!(&peer_lane.holder.run_id, &run);
    assert_eq!(peer_before.resource_lanes().count(), 1);

    let peer_after_reload = store
        .status_projection_snapshot(&peer_run)
        .await
        .expect("peer stream-authoritative projection");
    let peer_lane = peer_after_reload
        .resource_lane(&lane_key)
        .expect("peer snapshot includes cross-run lane from stream");
    assert_eq!(&peer_lane.holder.run_id, &run);
    assert_eq!(&peer_lane.holder.ledger_key, &side_effect_ledger_key());
    assert_eq!(peer_after_reload.run_state(&peer_run), RunState::Started);
    assert_eq!(
        store
            .status_projection_snapshot(&run)
            .await
            .expect("holder stream-authoritative projection"),
        before
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn resource_lane_append_admission_uses_stream_authority() {
    let (store, schema) = test_store().await;
    let holder_run = run_id(25);
    let contender = run_id(26);
    let lane_value = "wallet-admission";
    let lane_key = resource_lane_key(lane_value);

    append_run_start(&store, &holder_run, "admission-holder-run-start")
        .await
        .expect("holder run start");
    append_resource_lane_attempt_start(&store, &holder_run, "admission-holder-attempt-start")
        .await
        .expect("holder attempt start");
    append_resource_lane_prepare(
        &store,
        &holder_run,
        "admission-holder-prepare",
        lane_value,
        28,
    )
    .await
    .expect("holder resource lane prepare");
    let holder_projection = store
        .status_projection_snapshot(&holder_run)
        .await
        .expect("holder projection");
    assert_eq!(
        holder_projection
            .resource_lane(&lane_key)
            .expect("holder resource lane")
            .holder
            .run_id,
        holder_run
    );

    append_run_start(&store, &contender, "contender-run-start")
        .await
        .expect("contender run start");
    append_resource_lane_attempt_start(&store, &contender, "contender-attempt-start")
        .await
        .expect("contender attempt start");
    let stream_before = store
        .load_run_stream(&contender)
        .await
        .expect("contender stream before conflict");
    let outcome =
        append_resource_lane_prepare(&store, &contender, "contender-prepare", lane_value, 30)
            .await
            .expect("resource lane stream authority blocks contender");
    assert_resource_lane_blocked(outcome, &lane_key, &holder_run);
    assert_eq!(
        store
            .load_run_stream(&contender)
            .await
            .expect("contender stream after conflict"),
        stream_before
    );
    assert_eq!(
        store
            .expected_next_seq(&contender)
            .await
            .expect("contender next seq"),
        StreamSeq::new(3).expect("contender prepare seq")
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn admission_waiters_enforce_single_lane_fifo_after_release() {
    let (store, schema) = test_store().await;
    let holder_run = run_id(32);
    let first_waiter = run_id(33);
    let second_waiter = run_id(34);
    let lane_value = "wallet-fifo-admission";
    let lane_key = resource_lane_key(lane_value);

    for (run, run_key, attempt_key) in [
        (
            &holder_run,
            "fifo-holder-run-start",
            "fifo-holder-attempt-start",
        ),
        (&first_waiter, "fifo-b-run-start", "fifo-b-attempt-start"),
        (&second_waiter, "fifo-c-run-start", "fifo-c-attempt-start"),
    ] {
        append_run_start(&store, run, run_key)
            .await
            .expect("run start");
        append_resource_lane_attempt_start(&store, run, attempt_key)
            .await
            .expect("attempt start");
    }

    append_resource_lane_prepare(&store, &holder_run, "fifo-holder-prepare", lane_value, 32)
        .await
        .expect("holder resource lane prepare");

    let first_waiter_block = assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &first_waiter, "fifo-b-prepare", lane_value, 33)
            .await
            .expect("first waiter blocked by holder"),
        &lane_key,
        Some(&holder_run),
    );
    let second_waiter_block = assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &second_waiter, "fifo-c-prepare", lane_value, 34)
            .await
            .expect("second waiter blocked by holder"),
        &lane_key,
        Some(&holder_run),
    );
    assert!(first_waiter_block.lane_ticket < second_waiter_block.lane_ticket);

    append_resource_lane_release(&store, &holder_run, "fifo-holder-release", lane_value)
        .await
        .expect("holder release");

    let second_retry_block = assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &second_waiter, "fifo-c-retry-1", lane_value, 34)
            .await
            .expect("second waiter cannot bypass first waiter"),
        &lane_key,
        None,
    );
    assert_eq!(second_retry_block.waiter_id, second_waiter_block.waiter_id);
    assert_eq!(
        second_retry_block.lane_ticket,
        second_waiter_block.lane_ticket
    );

    append_resource_lane_prepare(&store, &first_waiter, "fifo-b-retry-1", lane_value, 33)
        .await
        .expect("first waiter claims after holder release");
    assert_eq!(
        store
            .status_projection_snapshot(&first_waiter)
            .await
            .expect("first waiter projection")
            .resource_lane(&lane_key)
            .expect("first waiter lane")
            .holder
            .run_id,
        first_waiter
    );

    assert_wait_fifo_admission_blocked(
        append_resource_lane_prepare(&store, &second_waiter, "fifo-c-retry-2", lane_value, 34)
            .await
            .expect("second waiter blocked by first waiter holder"),
        &lane_key,
        Some(&first_waiter),
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn saga_projection_rebuilds_from_events() {
    let (store, schema) = test_store().await;
    let run = run_id(41);
    let saga_policy = manual_saga_policy(42);

    append_prepared(
        &store,
        request(
            run.clone(),
            1,
            "saga-run-start",
            vec![run_admitted_with_saga_policy(run.clone(), &saga_policy)],
        ),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "saga-side-effect-attempt-start",
            vec![side_effect_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("side-effect attempt start");
    let intent_artifact = artifact_id(40);
    let intent_digest = content_digest(40);
    append_prepared(
        &store,
        request(
            run.clone(),
            3,
            "saga-side-effect-prepare",
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                side_effect_prepared(),
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
    .expect("side-effect prepare");
    let ambiguity_artifact = artifact_id(140);
    let ambiguity_digest = content_digest(140);
    append_prepared(
        &store,
        request(
            run.clone(),
            4,
            "saga-side-effect-ambiguous",
            vec![
                side_effect_started(),
                side_effect_ambiguous(ambiguity_artifact.clone(), ambiguity_digest.clone()),
                side_effect_attempt_failed(),
            ],
        ),
        vec![side_effect_artifact_ref(
            ambiguity_artifact,
            ambiguity_digest,
            schema_id("mfm.test.ambiguity", 84),
            ArtifactRole::AmbiguityEvidence,
        )],
    )
    .await
    .expect("side-effect ambiguous");
    let verified = verified_manual_resolution_for_seq(&run, 5, 42);
    let manual_artifacts = manual_resolution_artifacts(&verified);
    let manual_request = mfm_store::v1::CommitRequest::from_payloads(
        run.clone(),
        StreamSeq::new(5).expect("manual resolution seq"),
        CommitKey::new("saga-manual-resolution").expect("commit key"),
        vec![manual_resolution_recorded(&verified)],
        manual_artifacts.clone(),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            ..saga_preconditions(&run, saga_policy)
        },
    )
    .expect("manual resolution request");
    let manual_commit = PreparedCommit::<ManualResolution>::new(
        manual_request,
        CommitArtifactEvidenceSet::new(manual_artifacts.clone(), manual_artifacts)
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect("proof-backed manual resolution prepared commit");
    store
        .append_prepared_commit_bundle(
            PreparedCommitBundle::new(
                manual_commit.into(),
                manual_resolution_prepared_artifact_bytes(&verified)
                    .expect("manual artifact bytes"),
                Vec::new(),
            )
            .expect("manual bundle"),
        )
        .await
        .expect("manual resolution");
    let before = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let engagement = before.saga_engagement(&run).expect("saga engagement");
    assert!(matches!(
        engagement.reason,
        SagaEngagementReason::ForwardAmbiguous { .. }
    ));
    let manual = before
        .manual_resolution(&run)
        .expect("manual resolution projection");
    assert_eq!(
        manual.outcome,
        events::ManualResolutionOutcome::ConfirmRemediated
    );
    assert_eq!(
        manual
            .note
            .as_ref()
            .map(events::ManualResolutionNote::as_str),
        Some("reviewed evidence")
    );
    assert!(before.run_completion(&run).is_none());

    let stream = store.load_run_stream(&run).await.expect("typed run stream");
    assert_eq!(
        ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("payload rebuild"),
        before
    );

    let terminal_run = run_id(43);
    let terminal_policy = SagaPolicySpec::FailWithoutAcdcClaim;
    append_prepared(
        &store,
        request(
            terminal_run.clone(),
            1,
            "saga-terminal-run-start",
            vec![run_admitted_with_saga_policy(
                terminal_run.clone(),
                &terminal_policy,
            )],
        ),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("terminal run start");
    append_prepared(
        &store,
        request(
            terminal_run.clone(),
            2,
            "saga-terminal-side-effect-attempt-start",
            vec![side_effect_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("terminal side-effect attempt start");
    let terminal_intent_artifact = artifact_id(150);
    let terminal_intent_digest = content_digest(150);
    append_prepared(
        &store,
        request(
            terminal_run.clone(),
            3,
            "saga-terminal-side-effect-prepare",
            vec![
                side_effect_intent(
                    terminal_intent_artifact.clone(),
                    terminal_intent_digest.clone(),
                ),
                side_effect_claim(),
                side_effect_prepared(),
            ],
        ),
        vec![side_effect_artifact_ref(
            terminal_intent_artifact,
            terminal_intent_digest,
            schema_id("mfm.test.side_effect_intent", 70),
            ArtifactRole::SideEffectIntent,
        )],
    )
    .await
    .expect("terminal side-effect prepare");
    let terminal_ambiguity_artifact = artifact_id(152);
    let terminal_ambiguity_digest = content_digest(152);
    append_prepared(
        &store,
        request(
            terminal_run.clone(),
            4,
            "saga-terminal-side-effect-ambiguous",
            vec![
                side_effect_started(),
                side_effect_ambiguous(
                    terminal_ambiguity_artifact.clone(),
                    terminal_ambiguity_digest.clone(),
                ),
                side_effect_attempt_failed(),
            ],
        ),
        vec![side_effect_artifact_ref(
            terminal_ambiguity_artifact,
            terminal_ambiguity_digest,
            schema_id("mfm.test.ambiguity", 84),
            ArtifactRole::AmbiguityEvidence,
        )],
    )
    .await
    .expect("terminal side-effect ambiguous");
    let terminal_before_completion = store
        .status_projection_snapshot(&terminal_run)
        .await
        .expect("terminal projection before completion");
    let terminal_saga =
        terminal_before_completion.derive_saga_projection(&terminal_run, &terminal_policy);
    let terminal_proof = SagaTerminalProof::new(
        &terminal_policy,
        &terminal_saga,
        store
            .expected_next_seq(&terminal_run)
            .await
            .expect("terminal next seq"),
        None,
    )
    .expect("terminal proof");
    let terminal_request = request(
        terminal_run.clone(),
        5,
        "saga-terminal-run-completed",
        vec![run_completed(
            terminal_run.clone(),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
        )],
    );
    let terminal_request =
        terminal_request.with_preconditions(saga_preconditions(&terminal_run, terminal_policy));
    let terminal_commit = PreparedCommit::<SagaTerminal>::new(
        terminal_request,
        CommitArtifactEvidenceSet::empty(),
        &terminal_proof,
    )
    .expect("proof-backed terminal commit");
    store
        .append_prepared_commit_bundle(
            test_prepared_commit_bundle(terminal_commit.into()).expect("terminal bundle"),
        )
        .await
        .expect("terminal run completed");

    let terminal_before = store
        .status_projection_snapshot(&terminal_run)
        .await
        .expect("terminal projection");
    assert!(matches!(
        terminal_before
            .run_completion(&terminal_run)
            .map(|projection| &projection.outcome),
        Some(events::RunCompletionOutcome::FailedWithoutAcdcClaim)
    ));
    let terminal_stream = store
        .load_run_stream(&terminal_run)
        .await
        .expect("terminal typed run stream");
    assert_eq!(
        ProjectionSnapshot::rebuild_from_run_stream(&terminal_stream)
            .expect("terminal payload rebuild"),
        terminal_before
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn required_artifacts_and_fact_projection_are_atomic() {
    let (store, schema) = test_store().await;
    let run = run_id(10);
    append_prepared(
        &store,
        request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "fact-attempt-start",
            vec![fact_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("fact attempt start");

    let missing_artifact = artifact_id(11);
    let missing_digest = content_digest(11);
    let missing_ref = store_artifact_ref(
        missing_artifact.clone(),
        missing_digest.clone(),
        ArtifactRole::FactResponse,
    );
    let fact_request = request(
        run.clone(),
        3,
        "fact",
        vec![fact_recorded(
            missing_artifact.clone(),
            missing_digest.clone(),
        )],
    );
    let fact_request = fact_request.with_preconditions(CommitPreconditions {
        required_run_state: RequiredRunState::Started,
        ..CommitPreconditions::default()
    });
    let fact_request = fact_request.with_required_artifacts(vec![missing_ref.clone()]);
    let err = append_prepared(&store, fact_request.clone(), Vec::new())
        .await
        .expect_err("missing fact artifact");
    assert!(matches!(
        err,
        PostgresStoreError::Store(StoreError::MissingArtifact { .. })
    ));
    assert_eq!(
        store.expected_next_seq(&run).await.expect("next seq"),
        StreamSeq::new(3).expect("seq")
    );

    append_prepared(&store, fact_request, vec![missing_ref])
        .await
        .expect("fact commit after artifact");
    let projection = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    assert!(projection
        .fact(
            &node_id(30),
            &attempt_id(31),
            &events::FactKey::new("fact-key-1").unwrap()
        )
        .is_some());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn side_effect_unknown_recovery_updates_submission_result_slot() {
    let (store, schema) = test_store().await;
    let run = run_id(13);

    append_prepared(
        &store,
        request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
        vec![spec_artifact_ref(), certificate_artifact_ref()],
    )
    .await
    .expect("run start");

    let intent_artifact = artifact_id(14);
    let intent_digest = content_digest(14);
    append_prepared(
        &store,
        request(
            run.clone(),
            2,
            "sidefx-attempt-start",
            vec![side_effect_attempt_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("attempt start");
    append_prepared(
        &store,
        request(
            run.clone(),
            3,
            "sidefx-prepare",
            vec![
                side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                side_effect_claim(),
                side_effect_prepared(),
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
    .expect("prepare");
    append_prepared(
        &store,
        request(
            run.clone(),
            4,
            "sidefx-started",
            vec![side_effect_started()],
        ),
        Vec::new(),
    )
    .await
    .expect("started");

    let unknown_artifact = artifact_id(16);
    let unknown_digest = content_digest(16);
    let unknown_outcome = append_prepared(
        &store,
        request(
            run.clone(),
            5,
            "sidefx-submission-unknown",
            vec![side_effect_submission_unknown(
                unknown_artifact.clone(),
                unknown_digest.clone(),
            )],
        ),
        vec![side_effect_artifact_ref(
            unknown_artifact,
            unknown_digest,
            unknown_schema(),
            ArtifactRole::SubmissionUnknownEvidence,
        )],
    )
    .await
    .expect("submission unknown");
    let CommitOutcome::Appended(unknown) = unknown_outcome else {
        panic!("submission unknown should append");
    };
    let submission_result_key = format!(
        "sidefx:forward:{}:invocation:1:submission_result",
        side_effect_ledger_key()
    );
    assert_eq!(
        unknown.events()[0].logical_key().as_str(),
        submission_result_key
    );

    let submission_artifact = artifact_id(18);
    let submission_digest = content_digest(18);
    let observed_outcome = append_prepared(
        &store,
        request(
            run.clone(),
            6,
            "sidefx-submission-observed-after-unknown",
            vec![side_effect_submission_observed(
                submission_artifact.clone(),
                submission_digest.clone(),
            )],
        ),
        vec![side_effect_artifact_ref(
            submission_artifact,
            submission_digest,
            submission_schema(),
            ArtifactRole::Submission,
        )],
    )
    .await
    .expect("submission observed recovery");
    let CommitOutcome::Appended(observed) = observed_outcome else {
        panic!("submission observed should append");
    };
    assert_eq!(
        observed.events()[0].logical_key().as_str(),
        submission_result_key
    );

    let projection = store
        .status_projection_snapshot(&run)
        .await
        .expect("projection");
    let side_effect = projection
        .side_effect(&side_effect_ledger_key())
        .expect("side-effect projection");
    assert!(matches!(
        side_effect.phase,
        SideEffectPhase::SubmissionObserved {
            invocation_epoch: 1
        }
    ));

    let stored_payload_hash = sqlx::query_scalar::<_, String>(
        "SELECT payload_hash FROM run_events \
         WHERE run_id = $1 AND logical_key = $2",
    )
    .bind(run.as_str())
    .bind(submission_result_key.as_str())
    .fetch_one(&store.pool)
    .await
    .expect("run event payload row");
    assert_eq!(
        stored_payload_hash,
        observed.events()[0].payload_hash().as_str()
    );

    drop_schema(&store, &schema).await;
}
