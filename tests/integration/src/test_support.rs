#![warn(missing_docs)]
//! Shared helpers for MFM integration tests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use mfm_events::v1::{ArtifactRole, KernelEventPayload};
use mfm_store::v1 as store;
use sqlx::{AssertSqlSafe, PgPool};

#[path = "run_control_support.rs"]
mod run_control_support;

pub use run_control_support::{
    start_counted_portfolio_rpc_mock, start_portfolio_rpc_mock, write_evm_runtime_config_for_test,
    write_portfolio_runtime_config_for_test, PortfolioRpcMock, RuntimeConfigSignerBinding,
};

/// Re-export: merge-safe Platform holding seed for store-backed portfolio report tests.
pub use store::test_support::{
    append_platform_holding_facts_for_test, FactRecordFixtureInputForTest,
    PlatformHoldingFactSeedForTest,
};

/// Store error that can represent a deliberately injected stale-sequence append result.
pub trait InjectedStaleExpectedNextSeqError {
    /// Builds the backend's typed stale-sequence error.
    fn injected_stale_expected_next_seq(
        expected: store::StreamSeq,
        actual: store::StreamSeq,
    ) -> Self;
}

impl InjectedStaleExpectedNextSeqError for store::StoreError {
    fn injected_stale_expected_next_seq(
        expected: store::StreamSeq,
        actual: store::StreamSeq,
    ) -> Self {
        Self::StaleExpectedNextSeq { expected, actual }
    }
}

impl InjectedStaleExpectedNextSeqError for mfm_storage_postgres::PostgresStoreError {
    fn injected_stale_expected_next_seq(
        expected: store::StreamSeq,
        actual: store::StreamSeq,
    ) -> Self {
        Self::Store(store::StoreError::StaleExpectedNextSeq { expected, actual })
    }
}

/// Decorates a store by committing one fact-producing settlement and reporting its result as stale.
///
/// The runtime must reload the committed journal after this injected uncertain result instead of
/// repeating the external read that produced the fact batch.
#[derive(Clone)]
pub struct UncertainFactSettlementStore<S> {
    inner: S,
    injected: Arc<AtomicBool>,
}

impl<S> UncertainFactSettlementStore<S> {
    /// Wraps a concrete store and arms one uncertain fact-settlement result.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            injected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns whether a committed fact settlement has been reported as uncertain.
    pub fn injected(&self) -> bool {
        self.injected.load(Ordering::SeqCst)
    }
}

impl<S> store::RunJournalBackend for UncertainFactSettlementStore<S>
where
    S: store::RunJournalStore + Send + Sync,
    S::Error: InjectedStaleExpectedNextSeqError + From<store::StoreError>,
{
    type Error = S::Error;

    fn backend_append<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let is_fact_settlement = bundle
            .request()
            .payloads()
            .iter()
            .any(|payload| matches!(payload, KernelEventPayload::FactRecorded(_)));
        let expected = bundle.request().expected_next_seq();
        Box::pin(async move {
            let outcome = self.inner.append_prepared_commit_bundle(bundle).await?;
            let inject = is_fact_settlement
                && matches!(&outcome, store::CommitOutcome::Appended(_))
                && self
                    .injected
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok();
            if inject {
                let appended = match &outcome {
                    store::CommitOutcome::Appended(batch) => batch,
                    _ => unreachable!("injection is limited to newly appended fact settlements"),
                };
                let actual = store::StreamSeq::new(
                    appended
                        .seq()
                        .as_u64()
                        .checked_add(1)
                        .expect("an appended test batch has a successor sequence"),
                )
                .expect("an appended test batch has a valid successor sequence");
                return Err(Self::Error::injected_stale_expected_next_seq(
                    expected, actual,
                ));
            }
            Ok(outcome)
        })
    }

    fn backend_load<'a>(
        &'a self,
        verifier: store::JournalLoadVerifier,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunJournal, Self::Error> {
        let run_id = verifier.run_id().clone();
        Box::pin(async move {
            let journal = self.inner.load_committed_journal(&run_id).await?;
            verifier.accept_verified(journal).map_err(Self::Error::from)
        })
    }
}

impl<S> store::CurrentProjectionStore for UncertainFactSettlementStore<S>
where
    S: store::CurrentProjectionStore + Send + Sync,
    S::Error: InjectedStaleExpectedNextSeqError + From<store::StoreError>,
{
    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

impl<S> store::StoreScopeStore for UncertainFactSettlementStore<S>
where
    S: store::StoreScopeStore,
{
    type Error = S::Error;

    fn load_store_scope_id<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, mfm_ids::StoreScopeId, Self::Error> {
        self.inner.load_store_scope_id()
    }
}

impl<S> store::ExecutionClaimStore for UncertainFactSettlementStore<S>
where
    S: store::ExecutionClaimStore,
{
    type Error = S::Error;

    fn acquire_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
        self.inner
            .acquire_execution_claim(scope, holder_run_id, token)
    }

    fn execution_claim_status<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
    ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
        self.inner.execution_claim_status(scope)
    }

    fn renew_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
        self.inner
            .renew_execution_claim(scope, holder_run_id, token)
    }

    fn release_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
        self.inner
            .release_execution_claim(scope, holder_run_id, token)
    }

    fn expired_execution_claims<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, Vec<store::ExpiredExecutionClaim>, Self::Error> {
        self.inner.expired_execution_claims()
    }

    fn reap_expired_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
        self.inner
            .reap_expired_execution_claim(scope, holder_run_id, token)
    }
}

impl<S> store::FactQueryStore for UncertainFactSettlementStore<S>
where
    S: store::FactQueryStore,
{
    type Error = S::Error;

    fn fact_query_implementation_id(&self) -> &'static str {
        self.inner.fact_query_implementation_id()
    }

    fn execute_fact_queries<'a>(
        &'a self,
        plans: &'a [mfm_facts::CanonicalFactQueryPlan],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Vec<mfm_facts::FactQueryResult>, Self::Error>>
                + Send
                + 'a,
        >,
    > {
        self.inner.execute_fact_queries(plans)
    }
}

impl<S> store::RetainedArtifactReadProvider for UncertainFactSettlementStore<S>
where
    S: store::RetainedArtifactReadProvider,
{
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        self.inner.read_retained_artifact(requirement)
    }
}

/// In-memory REST app state used by integration tests.
pub type InMemoryRestAppState = mfm_rest_api::AppState;

/// Creates a unique schema name for an isolated Postgres parity test.
pub fn unique_postgres_schema(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

/// Creates an isolated Postgres schema for a parity test.
pub async fn create_postgres_schema(database_url: &str, schema: &str) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect postgres");
    // Schema names are generated from UUIDs and never come from user input; dynamic DDL is
    // required because PostgreSQL does not parameterize identifiers.
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&pool)
        .await
        .expect("create schema");
    pool.close().await;
}

/// Drops an isolated Postgres schema after a parity test.
pub async fn drop_postgres_schema(database_url: &str, schema: &str) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect postgres");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema} CASCADE"
    )))
    .execute(&pool)
    .await
    .expect("drop schema");
    pool.close().await;
}

/// Adds a Postgres search-path option to an isolated parity-test database URL.
pub fn schema_scoped_database_url(database_url: &str, schema: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}options=-csearch_path%3D{schema}")
}

/// Migrates and connects a Postgres run store, retrying while the managed service becomes ready.
pub async fn connect_postgres_with_retry(
    database_url: &str,
    max_attempts: u32,
    delay_ms: u64,
) -> mfm_storage_postgres::PostgresStore {
    let mut last_err: Option<mfm_storage_postgres::PostgresStoreError> = None;
    for _ in 0..max_attempts {
        match mfm_storage_postgres::PostgresSchema::migrate(database_url).await {
            Ok(()) => match mfm_storage_postgres::PostgresStore::connect(database_url).await {
                Ok(store) => return store,
                Err(err) => {
                    last_err = Some(err);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            },
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    panic!(
        "typed postgres config after retries (DATABASE_URL redacted): {:?}",
        last_err
    );
}

/// Loads and certifies one store-owned committed journal for integration assertions.
pub async fn verified_run_view<S>(
    store: &S,
    registry: &mfm_certify::CertificationRegistry,
    run_id: &mfm_ids::RunId,
) -> store::VerifiedRunView
where
    S: store::RunJournalStore + ?Sized,
{
    let journal = store
        .load_committed_journal(run_id)
        .await
        .unwrap_or_else(|error| panic!("load committed journal for {run_id}: {error}"));
    let (_, spec_bytes) = journal.certified_spec_object();
    let (_, certificate_bytes) = journal.certificate_object();
    let certified = mfm_certify::verify_persisted_spec_certificate_with_trusted_registry(
        spec_bytes,
        certificate_bytes,
        registry,
    )
    .expect("certify stored run spec");
    journal.verify(certified).expect("verify committed journal")
}

/// Builds in-memory REST app state.
pub fn in_memory_rest_app_state() -> InMemoryRestAppState {
    let store = store::AsyncInMemoryRunStore::default();
    mfm_rest_api::AppState::new(
        mfm_rest_api::RestProcessRole::Live,
        mfm_app::in_memory_application_for_test(store, None),
    )
}

/// Loads all fact-query evidence objects referenced by one verified run view.
pub fn fact_query_evidences(view: &store::VerifiedRunView) -> Vec<mfm_facts::FactQueryEvidence> {
    let lifecycle = store::current_lifecycle::read(view);
    let mut evidences = Vec::new();
    let _ = lifecycle.visit_records(|record| {
        let store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(payload) =
            record.kind()
        else {
            return std::ops::ControlFlow::<()>::Continue(());
        };
        if payload.artifact_ref.role != ArtifactRole::FactQueryEvidence {
            return std::ops::ControlFlow::Continue(());
        }
        let mut requirement = None;
        let _ = record.visit_artifact_requirements(|candidate| {
            requirement = Some(candidate.clone());
            std::ops::ControlFlow::<()>::Break(())
        });
        let requirement = requirement.expect("query evidence artifact requirement");
        let artifact = lifecycle
            .object_for_requirement(&requirement)
            .expect("query evidence object");
        evidences.push(
            mfm_facts::parse_canonical_fact_query_evidence_bytes(artifact.bytes())
                .expect("query evidence bytes"),
        );
        std::ops::ControlFlow::Continue(())
    });
    evidences
}

/// Builds a JSON POST request for REST integration tests.
pub fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let payload = serde_json::to_string(&body).expect("request body serializes");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .expect("request")
}

/// Builds an empty-body POST request for REST integration tests.
pub fn empty_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// Parses an Axum response body as JSON for REST integration tests.
pub async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("response json")
}
