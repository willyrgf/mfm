use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mfm_canonical::{sha256_digest_bytes, CanonicalValue, RecoverabilityContractV3};
use mfm_executor::{
    reference_safe_failure, AccountSequencePolicy, AccountSequenceRequest, AllocationOutcome,
    BoundaryStage, CommittedEffectRequest, ContentRef, DeliveryAttemptOutcome, EvidenceBounds,
    ExecuteTargetOutcome, ExecutorBinding, ExecutorContractDescriptor, ExecutorDeployment,
    ExecutorError, ExecutorLedgerStoreIdentity, ExecutorRetainedClosureContract, FailureClass,
    KeyedExecutorLedger, NonDomainDisposition, NonDomainEntryStatus, NonDomainFailure,
    NonDomainFailureCode, ReferenceFailureCode, ResourceOwnership, ResourcePolicyBinding,
    RetainedValueContract, SchemaQualifiedCanonicalValue, VerifiedExecutorBinding,
};
use mfm_ids::{
    DigestAlgorithm, NodeId, RunId, SemanticTypeId, StableId, StoreScopeId, TenantScopeId,
};
use mfm_storage_executor_postgres::{
    open_executor_store, ExecutorWriterGenerationContext, ExecutorWriterGenerationFence,
    ExecutorWriterGenerationFenceFuture, PostgresExecutorFaultPoint, PostgresExecutorSchema,
    PostgresExecutorStoreError,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use tokio::sync::Barrier;

static NEXT_SCHEMA: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct Fixture {
    binding: VerifiedExecutorBinding,
    tenant_scope_id: TenantScopeId,
}

fn contract() -> &'static RecoverabilityContractV3 {
    RecoverabilityContractV3::embedded().expect("recoverability contract")
}

fn reviewed_ref(label: &str) -> ContentRef {
    reviewed_value(label).reference().expect("reviewed ref")
}

fn reviewed_value(label: &str) -> SchemaQualifiedCanonicalValue {
    let value = contract()
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String(label.to_owned()),
        )
        .expect("reviewed value");
    SchemaQualifiedCanonicalValue::from_validated(&value).expect("schema-qualified value")
}

fn retained_contract(label: &str, schema_contract: &str) -> RetainedValueContract {
    RetainedValueContract::new(
        contract()
            .schema_id(schema_contract)
            .expect("retained schema")
            .clone(),
        SemanticTypeId::new(
            "mfm",
            label,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(label.as_bytes()),
        )
        .expect("semantic type"),
        StableId::new(label).expect("retained role"),
        "application/json",
        reviewed_ref(&format!("{label}.evidence")),
    )
    .expect("retained value contract")
}

fn retained_closure_contract(label: &str) -> ExecutorRetainedClosureContract {
    ExecutorRetainedClosureContract::new(
        retained_contract(
            &format!("{label}.ensure-result"),
            "mfm.executor-ensure-result.v1",
        ),
        retained_contract(
            &format!("{label}.delivery-audit"),
            "mfm.executor-delivery-frontier.v2",
        ),
        retained_contract(
            &format!("{label}.executor-frontier"),
            "mfm.executor-delivery-frontier.v2",
        ),
        retained_contract(
            &format!("{label}.terminal-evidence"),
            "mfm.terminal-effect-evidence.v1",
        ),
        retained_contract(
            &format!("{label}.terminal-tombstone"),
            "mfm.executor-terminal-tombstone.v2",
        ),
        retained_contract(
            &format!("{label}.terminal-proof"),
            "mfm.executor-reference-terminal-proof.v2",
        ),
        retained_contract(
            &format!("{label}.domain-evidence"),
            "mfm.executor-reference-queue-result.v2",
        ),
    )
    .expect("retained closure")
}

fn fixture(label: &str) -> Fixture {
    let tenant_scope_id =
        TenantScopeId::new(format!("mfm.tenant_scope.v1:{:032x}", label.len() + 700))
            .expect("tenant");
    let generation_ref = reviewed_ref(&format!("{label}.generation"));
    let resource_domain_ref = reviewed_ref(&format!("{label}.resource-domain"));
    let ownership = ResourceOwnership::new(
        reviewed_ref(&format!("{label}.coordination")),
        resource_domain_ref.clone(),
        generation_ref.clone(),
        Some(reviewed_ref(&format!("{label}.destination-fence"))),
    )
    .expect("ownership");
    let deployment = ExecutorDeployment::new(
        reviewed_ref(&format!("{label}.namespace")),
        generation_ref,
        tenant_scope_id.clone(),
        reviewed_ref(&format!("{label}.evidence-authority")),
        Some(ownership.reference().expect("ownership ref")),
    )
    .expect("deployment");
    let descriptor = ExecutorContractDescriptor::new(
        reviewed_ref(&format!("{label}.ensure-contract")),
        retained_contract(
            &format!("{label}.semantic-request"),
            "mfm.primitive-stable_id.v1",
        ),
        retained_contract(&format!("{label}.safe-failure"), "mfm.safe-failure.v1"),
        retained_closure_contract(label),
        reviewed_ref(&format!("{label}.safe-failure-contract")),
        reviewed_ref(&format!("{label}.destination-domain")),
        EvidenceBounds::new(64, 256, 4_000_000, 16_384, 2, 16_384).expect("bounds"),
        Some(resource_domain_ref),
        Vec::new(),
    )
    .expect("executor descriptor");
    let binding = ExecutorBinding::new(
        descriptor.reference().expect("descriptor ref"),
        reviewed_ref(&format!("{label}.implementation")),
        deployment.reference().expect("deployment ref"),
    )
    .expect("binding");
    Fixture {
        binding: VerifiedExecutorBinding::verify(
            binding,
            descriptor,
            deployment,
            Some(ownership),
            &tenant_scope_id,
        )
        .expect("verified binding"),
        tenant_scope_id,
    }
}

fn committed(
    fixture: &Fixture,
    seed: u8,
    label: &str,
) -> CommittedEffectRequest<SchemaQualifiedCanonicalValue> {
    let scope = StoreScopeId::new(format!(
        "mfm.store_scope.v1:{:032x}",
        u128::from(seed) + 900
    ))
    .expect("scope");
    let run = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[seed, 11]),
    );
    let node = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[seed, 12]),
    );
    let value = contract()
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String(label.to_owned()),
        )
        .expect("request value");
    CommittedEffectRequest::new(
        fixture.binding.binding_ref().clone(),
        fixture.tenant_scope_id.clone(),
        &scope,
        &run,
        &node,
        SchemaQualifiedCanonicalValue::from_validated(&value).expect("qualified request"),
    )
    .expect("committed effect")
}

fn adversarial_delivery_outcomes(fixture: &Fixture) -> [(&'static str, DeliveryAttemptOutcome); 4] {
    let wrong_outcome_tuple = DeliveryAttemptOutcome::did_not_enter(
        reference_safe_failure(
            fixture
                .binding
                .contract()
                .safe_failure_contract_ref()
                .clone(),
            ReferenceFailureCode::DestinationUnavailable,
            FailureClass::Transport,
            BoundaryStage::BoundaryEntry,
        )
        .expect("structurally valid wrong outcome tuple"),
    )
    .expect("wrong tuple candidate");
    let illegal_fact_layer = DeliveryAttemptOutcome::non_domain_failure(
        NonDomainFailure::new(
            NonDomainEntryStatus::MayHaveEntered,
            NonDomainDisposition::RetryableOperational,
            NonDomainFailureCode::FactStoreUnavailable,
        )
        .expect("globally valid fact-layer failure"),
    )
    .expect("fact-layer candidate");
    let wrong_safe_failure_contract = DeliveryAttemptOutcome::did_not_enter(
        reference_safe_failure(
            reviewed_ref("postgres.hostile.safe-failure-contract"),
            ReferenceFailureCode::GenerationFenced,
            FailureClass::Authorization,
            BoundaryStage::BeforeBoundaryEntry,
        )
        .expect("wrong-contract safe failure"),
    )
    .expect("wrong-contract candidate");
    let wrong_returned_schema =
        DeliveryAttemptOutcome::returned(reviewed_value("postgres.hostile.returned-schema"))
            .expect("wrong-schema returned candidate");
    [
        ("wrong-outcome-tuple", wrong_outcome_tuple),
        ("illegal-fact-layer", illegal_fact_layer),
        ("wrong-safe-failure-contract", wrong_safe_failure_contract),
        ("wrong-returned-schema", wrong_returned_schema),
    ]
}

fn assert_adapter_contract_violation(outcome: &DeliveryAttemptOutcome) {
    assert!(outcome.returned_outcome().is_none());
    assert!(outcome.did_not_enter_failure().is_none());
    assert!(outcome.indeterminate_failure().is_none());
    let fields = outcome
        .non_domain_failure_value()
        .expect("adapter failure outcome")
        .fields();
    assert_eq!(fields.entry_status, NonDomainEntryStatus::MayHaveEntered);
    assert_eq!(fields.disposition, NonDomainDisposition::IntegrityBlocked);
    assert_eq!(fields.code, NonDomainFailureCode::AdapterContractViolation);
}

struct TestDatabase {
    admin_pool: PgPool,
    pool: PgPool,
    database_url: String,
    schema: String,
}

impl TestDatabase {
    async fn create() -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL is required for qualification");
        let admin_pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect PostgreSQL administrator");
        let schema = format!(
            "mfm_executor_qualification_{}_{}",
            std::process::id(),
            NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
        );
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("create executor schema");
        let options = PgConnectOptions::from_str(&database_url)
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(24)
            .connect_with(options)
            .await
            .expect("connect executor schema");
        PostgresExecutorSchema::migrate(&pool)
            .await
            .expect("migrate executor schema");
        Self {
            admin_pool,
            pool,
            database_url,
            schema,
        }
    }

    async fn create_sibling(&self) -> Self {
        let sibling = Self::create().await;
        assert_eq!(sibling.database_url, self.database_url);
        sibling
    }

    async fn set_retired_schema_contract_version(&self) {
        sqlx::query(
            "ALTER TABLE executor_schema_metadata \
             DISABLE TRIGGER ALL, \
             DROP CONSTRAINT executor_schema_metadata_version_v1",
        )
        .execute(&self.pool)
        .await
        .expect("remove executor metadata guards");
        sqlx::query(
            "UPDATE executor_schema_metadata \
             SET schema_contract_version = 'mfm.executor-postgres.v1'",
        )
        .execute(&self.pool)
        .await
        .expect("retain retired executor schema version");
        sqlx::query("ALTER TABLE executor_schema_metadata ENABLE TRIGGER ALL")
            .execute(&self.pool)
            .await
            .expect("restore executor metadata mutation guard");
    }

    async fn database_identity(&self) -> (String, u32) {
        let row = sqlx::query(
            "SELECT current_database()::text AS database_name, \
                    (SELECT oid::bigint FROM pg_catalog.pg_database \
                      WHERE datname = current_database()) AS database_oid",
        )
        .fetch_one(&self.pool)
        .await
        .expect("database identity");
        let oid = u32::try_from(row.get::<i64, _>("database_oid")).expect("database oid");
        (row.get("database_name"), oid)
    }

    async fn copy_authority_from(&self, source: &Self, complete: bool) {
        let tables = if complete {
            &[
                "executor_bindings",
                "executor_content_records",
                "executor_effect_frontiers",
                "executor_resource_records",
                "executor_effect_resource_links",
            ][..]
        } else {
            &["executor_bindings"][..]
        };
        let mut transaction = self.admin_pool.begin().await.expect("copy transaction");
        for table in tables {
            sqlx::query(AssertSqlSafe(format!(
                "INSERT INTO {}.{table} SELECT * FROM {}.{table}",
                self.schema, source.schema
            )))
            .execute(&mut *transaction)
            .await
            .expect("copy executor authority");
        }
        transaction.commit().await.expect("commit copied authority");
    }

    async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin_pool)
        .await
        .expect("drop executor schema");
        self.admin_pool.close().await;
    }
}

#[derive(Clone)]
struct ModeledFence {
    authority: Arc<Mutex<FenceAuthority>>,
    writer: u64,
}

struct FenceAuthority {
    database_name: String,
    database_oid: u32,
    schema: String,
    writer: u64,
    minimum_frontiers: i64,
    identity: ExecutorLedgerStoreIdentity,
}

impl ModeledFence {
    async fn new(database: &TestDatabase, identity: ExecutorLedgerStoreIdentity) -> Self {
        let (database_name, database_oid) = database.database_identity().await;
        Self {
            authority: Arc::new(Mutex::new(FenceAuthority {
                database_name,
                database_oid,
                schema: database.schema.clone(),
                writer: 1,
                minimum_frontiers: 0,
                identity,
            })),
            writer: 1,
        }
    }

    fn sibling(&self, writer: u64) -> Self {
        Self {
            authority: Arc::clone(&self.authority),
            writer,
        }
    }

    fn promote(&self, database: &TestDatabase, writer: u64, minimum_frontiers: i64) -> Self {
        let mut authority = self.authority.lock().expect("fence authority");
        authority.schema.clone_from(&database.schema);
        authority.writer = writer;
        authority.minimum_frontiers = minimum_frontiers;
        drop(authority);
        self.sibling(writer)
    }

    fn set_minimum_frontiers(&self, minimum_frontiers: i64) {
        self.authority
            .lock()
            .expect("fence authority")
            .minimum_frontiers = minimum_frontiers;
    }
}

impl ExecutorWriterGenerationFence for ModeledFence {
    type Error = ();

    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a ExecutorWriterGenerationContext,
    ) -> ExecutorWriterGenerationFenceFuture<'a, Self::Error> {
        Box::pin(async move {
            let frontiers =
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM executor_effect_frontiers")
                    .fetch_one(writer_pool)
                    .await
                    .map_err(|_| ())?;
            let authority = self.authority.lock().map_err(|_| ())?;
            if authority.writer != self.writer
                || authority.database_name != context.database_name()
                || authority.database_oid != context.database_oid()
                || authority.schema != context.schema_name()
                || authority.identity != *context.store_identity()
                || frontiers < authority.minimum_frontiers
            {
                return Err(());
            }
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn fenced_postgres_executor_qualification_matrix() {
    let original = TestDatabase::create().await;
    assert_eq!(
        PostgresExecutorSchema::migrate(&original.admin_pool)
            .await
            .expect_err("executor migration rejects the shared public schema"),
        PostgresExecutorStoreError::SchemaAuthorityMismatch
    );
    assert!(!sqlx::query_scalar::<_, bool>(
        "SELECT pg_catalog.to_regclass('public.executor_schema_metadata') IS NOT NULL",
    )
    .fetch_one(&original.admin_pool)
    .await
    .expect("public-schema isolation probe"));
    let primary_fixture = fixture("postgres-qualified");

    let retired = original.create_sibling().await;
    retired.set_retired_schema_contract_version().await;
    let retired_fence = ModeledFence::new(
        &retired,
        ExecutorLedgerStoreIdentity::from_binding(&primary_fixture.binding),
    )
    .await;
    assert_eq!(
        open_executor_store(
            retired.pool.clone(),
            primary_fixture.binding.clone(),
            retired_fence,
        )
        .await
        .expect_err("retired executor baseline must not qualify"),
        PostgresExecutorStoreError::SchemaAuthorityMismatch
    );
    retired.cleanup().await;

    let identity = ExecutorLedgerStoreIdentity::from_binding(&primary_fixture.binding);
    let fence = ModeledFence::new(&original, identity).await;
    let store = open_executor_store(
        original.pool.clone(),
        primary_fixture.binding.clone(),
        fence.clone(),
    )
    .await
    .expect("open fenced executor store");
    let readiness = store.readiness().await.expect("bounded readiness");
    assert!(readiness.ledger_ready());
    assert!(readiness.fence_ready());

    let engine = KeyedExecutorLedger::new(store.clone(), primary_fixture.binding.clone())
        .expect("shared engine");
    hostile_bind_concurrency(&engine, &primary_fixture, &original).await;
    exact_nonce_resource_cas(&engine, &primary_fixture, &original).await;
    ambiguity_never_returns_target_authority(&engine, &store, &primary_fixture, &original).await;
    oversized_target_result_survives_strict_reopen(&engine, &primary_fixture, &original, &fence)
        .await;
    affine_completion_normalizes_all_unbound_outcomes_across_postgres_reopen(
        &engine,
        &primary_fixture,
        &original,
        &fence,
    )
    .await;
    application_role_cannot_mutate_authority(&original).await;
    hostile_content_inventory_is_rejected(&original, &primary_fixture).await;

    open_executor_store(
        original.pool.clone(),
        primary_fixture.binding.clone(),
        fence.clone(),
    )
    .await
    .expect("strict same-lineage reopen");

    let sibling = fence.sibling(2);
    assert_eq!(
        open_executor_store(
            original.pool.clone(),
            primary_fixture.binding.clone(),
            sibling,
        )
        .await
        .expect_err("sibling writer must be fenced"),
        PostgresExecutorStoreError::WriterFenceRejected
    );

    let alternate = fixture("postgres-stale-generation");
    assert_eq!(
        open_executor_store(original.pool.clone(), alternate.binding, fence.clone(),)
            .await
            .expect_err("another executor identity cannot reuse the fence"),
        PostgresExecutorStoreError::WriterFenceRejected
    );

    let committed_frontiers =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM executor_effect_frontiers")
            .fetch_one(&original.pool)
            .await
            .expect("frontier count");
    fence.set_minimum_frontiers(committed_frontiers);

    let rollback = original.create_sibling().await;
    rollback.copy_authority_from(&original, false).await;
    let rollback_fence = fence.promote(&rollback, 3, committed_frontiers);
    assert_eq!(
        open_executor_store(
            rollback.pool.clone(),
            primary_fixture.binding.clone(),
            rollback_fence,
        )
        .await
        .expect_err("rollback restore must fail the external high-water fence"),
        PostgresExecutorStoreError::WriterFenceRejected
    );

    let restored = original.create_sibling().await;
    restored.copy_authority_from(&original, true).await;
    let restored_fence = fence.promote(&restored, 4, committed_frontiers);
    assert_eq!(
        store
            .readiness()
            .await
            .expect_err("promoted writer permanently fences the stale store"),
        PostgresExecutorStoreError::WriterFenceRejected
    );
    let restored_store = open_executor_store(
        restored.pool.clone(),
        primary_fixture.binding.clone(),
        restored_fence.clone(),
    )
    .await
    .expect("complete promoted restore");
    restored_store
        .readiness()
        .await
        .expect("restored readiness");

    corrupt_frontier_payload(&restored).await;
    assert_eq!(
        open_executor_store(
            restored.pool.clone(),
            primary_fixture.binding,
            restored_fence,
        )
        .await
        .expect_err("strict reopen rejects authoritative corruption"),
        PostgresExecutorStoreError::CorruptLedger
    );

    rollback.cleanup().await;
    restored.cleanup().await;
    original.cleanup().await;
}

async fn hostile_bind_concurrency(
    engine: &KeyedExecutorLedger<mfm_storage_executor_postgres::QualifiedPostgresExecutorStore>,
    fixture: &Fixture,
    database: &TestDatabase,
) {
    let request = committed(fixture, 1, "postgres.concurrent.bind");
    let barrier = Arc::new(Barrier::new(16));
    let mut workers = Vec::new();
    for _ in 0..16 {
        let engine = engine.clone();
        let identity = request.identity().clone();
        let barrier = Arc::clone(&barrier);
        workers.push(tokio::spawn(async move {
            barrier.wait().await;
            engine.bind_effect(&identity).await
        }));
    }
    for worker in workers {
        worker.await.expect("bind worker").expect("idempotent bind");
    }
    let frontier_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM executor_effect_frontiers WHERE effect_key = $1",
    )
    .bind(request.identity().effect_key().as_str())
    .fetch_one(&database.pool)
    .await
    .expect("bound frontier count");
    assert_eq!(frontier_count, 1);
}

async fn exact_nonce_resource_cas(
    engine: &KeyedExecutorLedger<mfm_storage_executor_postgres::QualifiedPostgresExecutorStore>,
    fixture: &Fixture,
    database: &TestDatabase,
) {
    let externally_attested_initial_nonce = 73;
    let policy = AccountSequencePolicy::new(
        ResourcePolicyBinding::new(
            reviewed_ref("postgres.account-policy"),
            reviewed_ref("postgres.account-policy-config"),
        ),
        externally_attested_initial_nonce,
        Some(mfm_executor::FencingRef::from_reviewed(reviewed_ref(
            "postgres.wallet-generation-fence",
        ))),
    );
    let policy_request =
        AccountSequenceRequest::new("sender-42", "eip155-1").expect("account sequence request");
    let requests = [
        committed(fixture, 2, "postgres.resource.first"),
        committed(fixture, 3, "postgres.resource.second"),
    ];
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for request in requests {
        let engine = engine.clone();
        let policy = policy.clone();
        let policy_request = policy_request.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(tokio::spawn(async move {
            barrier.wait().await;
            engine
                .try_bind_and_allocate(request.identity(), None, &policy, &policy_request)
                .await
        }));
    }
    let mut allocated_nonce = None;
    let mut cas_conflicts = 0;
    for worker in workers {
        match worker.await.expect("resource worker") {
            Ok(AllocationOutcome::Allocated { allocation, .. }) => {
                allocated_nonce = Some(allocation.sequence());
            }
            Err(ExecutorError::ResourceCasMismatch) => cas_conflicts += 1,
            other => panic!("unexpected resource CAS result: {other:?}"),
        }
    }
    assert_eq!(allocated_nonce, Some(externally_attested_initial_nonce));
    assert_eq!(cas_conflicts, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM executor_resource_records")
            .fetch_one(&database.pool)
            .await
            .expect("resource rows"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM executor_effect_resource_links")
            .fetch_one(&database.pool)
            .await
            .expect("resource links"),
        1
    );
}

async fn ambiguity_never_returns_target_authority(
    engine: &KeyedExecutorLedger<mfm_storage_executor_postgres::QualifiedPostgresExecutorStore>,
    store: &mfm_storage_executor_postgres::QualifiedPostgresExecutorStore,
    fixture: &Fixture,
    database: &TestDatabase,
) {
    let request = committed(fixture, 4, "postgres.ambiguous.authorization");
    engine
        .bind_effect(request.identity())
        .await
        .expect("bind ambiguous request");
    let target_operation = reviewed_value("postgres.target-operation");
    let head = engine
        .effect_view(request.identity())
        .await
        .expect("load bound request")
        .expect("bound request")
        .delivery_audit()
        .head_ref()
        .expect("head");
    let invoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let invoked_by_target = Arc::clone(&invoked);
    store.inject_qualification_fault(PostgresExecutorFaultPoint::AfterCommitBeforeAcknowledgement);
    assert_eq!(
        engine
            .execute_target_once(
                request.identity(),
                &head,
                target_operation.clone(),
                None,
                move |_| {
                    invoked_by_target.store(true, Ordering::SeqCst);
                    async { unreachable!("ambiguous authorization cannot invoke the target") }
                },
            )
            .await
            .expect_err("ambiguous commit cannot return target authority"),
        ExecutorError::DurableAppendOutcomeUnknown
    );
    assert!(!invoked.load(Ordering::SeqCst));
    let view = engine
        .effect_view(request.identity())
        .await
        .expect("reload ambiguous effect")
        .expect("ambiguous effect exists");
    assert_eq!(view.delivery_audit().attempt_count(), 1);

    let head = view.delivery_audit().head_ref().expect("ambiguous head");
    let safe_failure = reference_safe_failure(
        reviewed_ref("postgres.safe-failure"),
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("safe failure");
    let outcome =
        DeliveryAttemptOutcome::did_not_enter(safe_failure).expect("target failure outcome");
    let attempt_id = Arc::new(Mutex::new(None));
    let target_attempt_id = Arc::clone(&attempt_id);
    let target_invocations = Arc::new(AtomicU64::new(0));
    let invocations = Arc::clone(&target_invocations);
    let store_for_observation = store.clone();
    let target = engine
        .execute_target_once(
            request.identity(),
            &head,
            target_operation.clone(),
            None,
            move |authority| {
                *target_attempt_id.lock().expect("attempt lock") =
                    Some(authority.attempt_id().clone());
                async move {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    store_for_observation.inject_qualification_fault(
                        PostgresExecutorFaultPoint::AfterCommitBeforeAcknowledgement,
                    );
                    outcome
                }
            },
        )
        .await
        .expect("target survives an ambiguous observation acknowledgement");
    assert!(matches!(target, ExecuteTargetOutcome::Observed(_)));
    assert_eq!(target_invocations.load(Ordering::SeqCst), 1);
    let attempt_id = attempt_id
        .lock()
        .expect("attempt lock")
        .clone()
        .expect("target invocation");
    assert_eq!(
        engine
            .target_operation(request.identity(), &attempt_id)
            .await
            .expect("recover retained target operation"),
        target_operation
    );
    let lock_name = format!(
        "mfm.executor-postgres.effect.v1:{}",
        request.identity().effect_key().as_str()
    );
    let acquired = sqlx::query_scalar::<_, bool>(
        "SELECT pg_catalog.pg_try_advisory_lock( \
             pg_catalog.hashtextextended($1 || ':' || $2, 0) \
         )",
    )
    .bind(&database.schema)
    .bind(&lock_name)
    .fetch_one(&database.admin_pool)
    .await
    .expect("probe released effect lock");
    assert!(
        acquired,
        "target authority is returned only after DB lock release"
    );
    sqlx::query(
        "SELECT pg_catalog.pg_advisory_unlock( \
             pg_catalog.hashtextextended($1 || ':' || $2, 0) \
         )",
    )
    .bind(&database.schema)
    .bind(lock_name)
    .execute(&database.admin_pool)
    .await
    .expect("release test advisory lock");
}

async fn oversized_target_result_survives_strict_reopen(
    engine: &KeyedExecutorLedger<mfm_storage_executor_postgres::QualifiedPostgresExecutorStore>,
    fixture: &Fixture,
    database: &TestDatabase,
    fence: &ModeledFence,
) {
    let request = committed(fixture, 5, "postgres.oversized-result");
    let initial = engine
        .bind_effect(request.identity())
        .await
        .expect("bind oversized-result request");
    let expected_head = initial.delivery_audit().head_ref().expect("head");
    let mut oversized_bytes = Vec::with_capacity((32 * 1024) + 2);
    oversized_bytes.push(b'"');
    oversized_bytes.extend(std::iter::repeat_n(b'x', 32 * 1024));
    oversized_bytes.push(b'"');
    let oversized = SchemaQualifiedCanonicalValue::new(
        fixture
            .binding
            .contract()
            .retained_closure_contract()
            .domain_evidence_contract()
            .schema_id()
            .clone(),
        &oversized_bytes,
    )
    .expect("oversized schema-qualified result");
    let oversized = DeliveryAttemptOutcome::returned(oversized).expect("returned outcome");
    let observed = engine
        .execute_target_once(
            request.identity(),
            &expected_head,
            reviewed_value("postgres.oversized-result-target"),
            None,
            move |_authority| async move { oversized },
        )
        .await
        .expect("bounded observation");
    let ExecuteTargetOutcome::Observed(view) = observed else {
        panic!("fresh authorization must be observed")
    };
    assert_result_unrepresentable(&view);

    let reopened_store = open_executor_store(
        database.pool.clone(),
        fixture.binding.clone(),
        fence.clone(),
    )
    .await
    .expect("strict reopen after totalized oversized result");
    let reopened = KeyedExecutorLedger::new(reopened_store, fixture.binding.clone())
        .expect("reopened keyed executor ledger");
    let view = reopened
        .effect_view(request.identity())
        .await
        .expect("reopened effect")
        .expect("persisted effect");
    assert_result_unrepresentable(&view);
}

async fn affine_completion_normalizes_all_unbound_outcomes_across_postgres_reopen(
    engine: &KeyedExecutorLedger<mfm_storage_executor_postgres::QualifiedPostgresExecutorStore>,
    fixture: &Fixture,
    database: &TestDatabase,
    fence: &ModeledFence,
) {
    let invocations = Arc::new(AtomicU64::new(0));
    let mut identities = Vec::new();

    for (index, (label, hostile)) in adversarial_delivery_outcomes(fixture)
        .into_iter()
        .enumerate()
    {
        let seed = u8::try_from(6 + index).expect("test seed");
        let request = committed(fixture, seed, label);
        identities.push(request.identity().clone());
        let initial = engine
            .bind_effect(request.identity())
            .await
            .expect("bind adversarial request");
        let expected_head = initial.delivery_audit().head_ref().expect("head");
        let invoked = Arc::clone(&invocations);
        let observed = engine
            .execute_target_once(
                request.identity(),
                &expected_head,
                reviewed_value(&format!("postgres.{label}.target")),
                None,
                move |_authority| async move {
                    invoked.fetch_add(1, Ordering::SeqCst);
                    hostile
                },
            )
            .await
            .expect("sealed observation");
        let ExecuteTargetOutcome::Observed(view) = observed else {
            panic!("fresh target authority must be observed")
        };
        let attempts = view.delivery_audit().attempts().expect("attempts");
        assert_eq!(attempts.len(), 1, "{label}");
        assert_adapter_contract_violation(attempts[0].outcome().expect("one exact observation"));
    }
    assert_eq!(invocations.load(Ordering::SeqCst), 4);

    let reopened_store = open_executor_store(
        database.pool.clone(),
        fixture.binding.clone(),
        fence.clone(),
    )
    .await
    .expect("strict reopen after adversarial outcome normalization");
    let reopened = KeyedExecutorLedger::new(reopened_store, fixture.binding.clone())
        .expect("reopened keyed executor ledger");
    for identity in identities {
        let view = reopened
            .effect_view(&identity)
            .await
            .expect("reopened view")
            .expect("persisted effect");
        let attempts = view.delivery_audit().attempts().expect("attempts");
        assert_eq!(attempts.len(), 1);
        assert_adapter_contract_violation(
            attempts[0].outcome().expect("reopened exact observation"),
        );
    }
}

fn assert_result_unrepresentable(view: &mfm_executor::EffectEntryView) {
    let attempts = view.delivery_audit().attempts().expect("attempts");
    let failure = attempts
        .last()
        .expect("attempt")
        .outcome()
        .expect("outcome")
        .indeterminate_failure()
        .expect("oversized result must be totalized");
    assert_eq!(
        failure.stable_code(),
        &ReferenceFailureCode::ResultUnrepresentable
    );
}

async fn application_role_cannot_mutate_authority(database: &TestDatabase) {
    for statement in [
        "UPDATE executor_bindings SET tenant_scope_id = tenant_scope_id",
        "DELETE FROM executor_content_records",
        "TRUNCATE executor_effect_frontiers",
        "UPDATE executor_effect_heads SET ordinal = ordinal",
    ] {
        let mut transaction = database.pool.begin().await.expect("privilege transaction");
        sqlx::query("SET LOCAL ROLE mfm_executor_application")
            .execute(&mut *transaction)
            .await
            .expect("set executor application role");
        assert!(
            sqlx::query(statement)
                .execute(&mut *transaction)
                .await
                .is_err(),
            "application role unexpectedly permitted {statement}"
        );
        transaction
            .rollback()
            .await
            .expect("rollback denied statement");
    }
    let executor_is_store_member = sqlx::query_scalar::<_, bool>(
        "SELECT CASE \
             WHEN EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_store_application') \
             THEN pg_catalog.pg_has_role( \
                 'mfm_executor_application', 'mfm_store_application', 'MEMBER' \
             ) \
             ELSE FALSE \
         END",
    )
    .fetch_one(&database.admin_pool)
    .await
    .expect("role separation");
    assert!(!executor_is_store_member);
}

#[derive(Debug, Clone, Copy)]
enum HostileContentMutation {
    Missing,
    Extra,
    Conflicting,
}

async fn hostile_content_inventory_is_rejected(source: &TestDatabase, fixture: &Fixture) {
    for mutation in [
        HostileContentMutation::Missing,
        HostileContentMutation::Extra,
        HostileContentMutation::Conflicting,
    ] {
        let hostile = source.create_sibling().await;
        hostile.copy_authority_from(source, true).await;
        mutate_content_inventory(&hostile, mutation).await;
        let fence = ModeledFence::new(
            &hostile,
            ExecutorLedgerStoreIdentity::from_binding(&fixture.binding),
        )
        .await;
        assert_eq!(
            open_executor_store(hostile.pool.clone(), fixture.binding.clone(), fence,)
                .await
                .expect_err("strict reopen must reject a hostile content inventory"),
            PostgresExecutorStoreError::CorruptLedger,
            "mutation {mutation:?}"
        );
        hostile.cleanup().await;
    }
}

async fn mutate_content_inventory(database: &TestDatabase, mutation: HostileContentMutation) {
    let replacement = reviewed_value("postgres.hostile.content");
    let replacement_ref = replacement.reference().expect("replacement ref");
    match mutation {
        HostileContentMutation::Missing => {
            disable_content_immutability(database).await;
            let result = sqlx::query(AssertSqlSafe(format!(
                "DELETE FROM {}.executor_content_records \
                 WHERE (schema_id, content_digest) = ( \
                     SELECT schema_id, content_digest \
                     FROM {}.executor_content_records \
                     ORDER BY schema_id, content_digest \
                     LIMIT 1 \
                 )",
                database.schema, database.schema
            )))
            .execute(&database.admin_pool)
            .await
            .expect("delete retained content");
            assert_eq!(result.rows_affected(), 1);
            enable_content_immutability(database).await;
        }
        HostileContentMutation::Extra => {
            let result = sqlx::query(AssertSqlSafe(format!(
                "INSERT INTO {}.executor_content_records \
                 (schema_id, content_digest, canonical_bytes) \
                 VALUES ($1, $2, $3)",
                database.schema
            )))
            .bind(replacement_ref.schema_id().as_str())
            .bind(replacement_ref.content_digest().as_str())
            .bind(replacement.as_bytes())
            .execute(&database.admin_pool)
            .await
            .expect("insert unreachable retained content");
            assert_eq!(result.rows_affected(), 1);
        }
        HostileContentMutation::Conflicting => {
            disable_content_immutability(database).await;
            let result = sqlx::query(AssertSqlSafe(format!(
                "UPDATE {}.executor_content_records \
                 SET canonical_bytes = $1 \
                 WHERE (schema_id, content_digest) = ( \
                     SELECT schema_id, content_digest \
                     FROM {}.executor_content_records \
                     WHERE schema_id = $2 \
                       AND content_digest <> $3 \
                     ORDER BY content_digest \
                     LIMIT 1 \
                 )",
                database.schema, database.schema
            )))
            .bind(replacement.as_bytes())
            .bind(replacement_ref.schema_id().as_str())
            .bind(replacement_ref.content_digest().as_str())
            .execute(&database.admin_pool)
            .await
            .expect("replace retained content under a conflicting digest");
            assert_eq!(result.rows_affected(), 1);
            enable_content_immutability(database).await;
        }
    }
}

async fn disable_content_immutability(database: &TestDatabase) {
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {}.executor_content_records \
         DISABLE TRIGGER executor_content_records_no_update",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("disable content immutability trigger");
}

async fn enable_content_immutability(database: &TestDatabase) {
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {}.executor_content_records \
         ENABLE TRIGGER executor_content_records_no_update",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("reenable content immutability trigger");
}

async fn corrupt_frontier_payload(database: &TestDatabase) {
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {}.executor_effect_frontiers \
         DISABLE TRIGGER executor_effect_frontiers_no_update",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("disable immutability trigger for corruption probe");
    sqlx::query(AssertSqlSafe(format!(
        "UPDATE {}.executor_effect_frontiers \
         SET durable_payload = set_byte( \
             durable_payload, 0, get_byte(durable_payload, 0) # 1 \
         ) \
         WHERE (effect_key, ordinal) = ( \
             SELECT effect_key, ordinal \
             FROM {}.executor_effect_frontiers \
             ORDER BY effect_key, ordinal \
             LIMIT 1 \
        )",
        database.schema, database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("corrupt greatest authority");
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {}.executor_effect_frontiers \
         ENABLE TRIGGER executor_effect_frontiers_no_update",
        database.schema
    )))
    .execute(&database.admin_pool)
    .await
    .expect("reenable immutability trigger");
}
