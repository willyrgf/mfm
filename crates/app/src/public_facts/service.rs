use std::future::Future;
use std::pin::Pin;

#[cfg(any(test, feature = "test-support"))]
use mfm_store::v1 as store;

use super::catalog::{
    is_public_default_fact_visibility, public_fact_from_query_row, public_query_input,
    FactCatalogService,
};
use super::dto::PublicFactQueryPage;
use super::query::PublicFactQueryRequest;
#[cfg(any(test, feature = "test-support"))]
use crate::async_app_store_error;
use crate::{content_digest_for_bytes, AppError, ErrorClass, PostgresStore};

/// App public fact query service.
#[derive(Debug, Clone)]
pub struct FactPublicQueryService<E> {
    catalog: FactCatalogService,
    executor: E,
    store_scope: mfm_facts::StoreScopeRef,
    scope_decision_evidence: mfm_facts::ScopeDecisionEvidence,
}

impl<E> FactPublicQueryService<E>
where
    E: PublicFactQueryExecutor,
{
    /// Creates a public fact query service from a public catalog and store executor.
    pub fn new(catalog: FactCatalogService, executor: E) -> Result<Self, AppError> {
        Ok(Self {
            catalog,
            executor,
            store_scope: mfm_facts::StoreScopeRef::new("mfm.store.default")?,
            scope_decision_evidence: mfm_facts::ScopeDecisionEvidence::new(
                content_digest_for_bytes(b"mfm.public-facts.default-scope.v1"),
            ),
        })
    }

    /// Executes a public Platform fact query and returns public DTOs only.
    pub async fn query(
        &self,
        request: PublicFactQueryRequest,
    ) -> Result<PublicFactQueryPage, AppError> {
        query_public_facts(
            &self.catalog,
            &self.executor,
            &self.store_scope,
            &self.scope_decision_evidence,
            request,
        )
        .await
    }
}

pub(crate) async fn query_public_facts<E>(
    catalog: &FactCatalogService,
    executor: &E,
    store_scope: &mfm_facts::StoreScopeRef,
    scope_decision_evidence: &mfm_facts::ScopeDecisionEvidence,
    request: PublicFactQueryRequest,
) -> Result<PublicFactQueryPage, AppError>
where
    E: PublicFactQueryExecutor,
{
    request.validate()?;
    let (_descriptor_hash, descriptor) = catalog.resolve_descriptor(&request)?;
    let input = public_query_input(
        &request,
        store_scope.clone(),
        scope_decision_evidence.clone(),
    )?;
    let plan = mfm_facts::compile_fact_query_plan(descriptor, input)?;
    if !is_public_default_fact_visibility(plan.query_scope().audience(), plan.query_scope().scope())
    {
        return Err(AppError::backend(
            ErrorClass::Internal,
            "FactPublicScopeInvalid",
            "Public fact query scope was invalid",
        ));
    }
    let execution = executor.execute_public_fact_query_plan(&plan).await?;
    let facts = execution
        .iter()
        .filter_map(|row| public_fact_from_query_row(catalog, row).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PublicFactQueryPage {
        facts,
        next_cursor: None,
    })
}

/// Boxed future returned by public fact query executors.
pub type PublicFactQueryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PublicFactQueryExecution, AppError>> + Send + 'a>>;

/// Store capability required by app public fact query services.
pub trait PublicFactQueryExecutor: Clone + Send + Sync + 'static {
    /// Executes a compiled canonical public fact query plan and returns raw fact rows.
    fn execute_public_fact_query_plan<'a>(
        &'a self,
        plan: &'a mfm_facts::CanonicalFactQueryPlan,
    ) -> PublicFactQueryFuture<'a>;
}

/// Rows returned from a public fact query executor.
pub type PublicFactQueryExecution = Vec<mfm_facts::FactQueryResultRow>;

pub(crate) type AppFactQueryRow = mfm_facts::FactQueryResultRow;

impl PublicFactQueryExecutor for PostgresStore {
    fn execute_public_fact_query_plan<'a>(
        &'a self,
        plan: &'a mfm_facts::CanonicalFactQueryPlan,
    ) -> PublicFactQueryFuture<'a> {
        Box::pin(async move {
            let result = self
                .execute_fact_query(plan)
                .await
                .map_err(AppError::from)?;
            Ok(result.rows().to_vec())
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PublicFactQueryExecutor for store::AsyncInMemoryRunStore {
    fn execute_public_fact_query_plan<'a>(
        &'a self,
        plan: &'a mfm_facts::CanonicalFactQueryPlan,
    ) -> PublicFactQueryFuture<'a> {
        Box::pin(async move { execute_in_memory_app_fact_query(self, plan) })
    }
}

#[cfg(any(test, feature = "test-support"))]
fn execute_in_memory_app_fact_query(
    store: &store::AsyncInMemoryRunStore,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> Result<PublicFactQueryExecution, AppError> {
    let projection = store.projection_snapshot().map_err(async_app_store_error)?;
    execute_projection_app_fact_query(&projection, plan)
}

#[cfg(any(test, feature = "test-support"))]
fn execute_projection_app_fact_query(
    projection: &store::ProjectionSnapshot,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> Result<PublicFactQueryExecution, AppError> {
    Ok(store::test_support::execute_fact_query_projection_for_test(
        projection, plan,
    )?)
}
