//! Process-bound Platform/Control fact-index providers.
//!
//! Production assembly always uses [`production_fact_index_read_provider`] (Postgres).
//! Store-backed tests use [`ProjectionFactIndexProvider`] over an in-memory projection.
//! Adapter unit tests keep local plan fixtures (`MockFactIndex` in adapter crates).

use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use std::sync::Mutex;

use mfm_fact_capabilities::{FactIndexReadProvider, FactIndexReadRequest};
#[cfg(any(test, feature = "test-support"))]
use mfm_store::v1 as store;

use crate::PostgresStore;

/// Builds the production Platform/Control fact-index provider from a Postgres run store.
///
/// Fact queries are evaluated against the validated Postgres store authority.
pub fn production_fact_index_read_provider(store: PostgresStore) -> Arc<dyn FactIndexReadProvider> {
    Arc::new(PostgresFactIndexReadProvider { store })
}

struct PostgresFactIndexReadProvider {
    store: PostgresStore,
}

impl FactIndexReadProvider for PostgresFactIndexReadProvider {
    fn implementation_id(&self) -> &'static str {
        "mfm.app.postgres.fact-index.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> mfm_fact_capabilities::FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            if requests.is_empty() {
                return Ok(Vec::new());
            }
            let plans: Vec<_> = requests
                .iter()
                .map(|request| request.plan().clone())
                .collect();
            let results =
                self.store.execute_fact_queries(&plans).await.map_err(
                    mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure,
                )?;
            Ok(results)
        })
    }
}

/// Projection-backed fact-index over an in-memory store snapshot.
///
/// Single store-backed test/process-assembly provider (not production Postgres). Freezes one
/// projection for the whole batch (shared selection frontier).
#[cfg(any(test, feature = "test-support"))]
pub struct ProjectionFactIndexProvider {
    store: store::AsyncInMemoryRunStore,
    returned_row_counts: Mutex<Vec<usize>>,
}

#[cfg(any(test, feature = "test-support"))]
impl ProjectionFactIndexProvider {
    /// Builds a provider that reads fact rows from the supplied in-memory store projection.
    pub fn new(store: store::AsyncInMemoryRunStore) -> Self {
        Self {
            store,
            returned_row_counts: Mutex::new(Vec::new()),
        }
    }

    /// Empty in-memory store + projection provider for registry-only process assembly tests.
    pub fn empty() -> Self {
        Self::new(store::AsyncInMemoryRunStore::default())
    }

    /// Dyn provider over the given store (shared by integration and app unit tests).
    pub fn arc(store: store::AsyncInMemoryRunStore) -> Arc<dyn FactIndexReadProvider> {
        Arc::new(Self::new(store))
    }

    /// Dyn provider over an empty in-memory store.
    pub fn empty_arc() -> Arc<dyn FactIndexReadProvider> {
        Arc::new(Self::empty())
    }

    /// Returns one row count per fact-index plan in batch call order (integration assertions).
    pub fn returned_row_counts(&self) -> Vec<usize> {
        self.returned_row_counts.lock().expect("row counts").clone()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl FactIndexReadProvider for ProjectionFactIndexProvider {
    fn implementation_id(&self) -> &'static str {
        "mfm.app.projection.fact-index.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> mfm_fact_capabilities::FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            if requests.is_empty() {
                return Ok(Vec::new());
            }
            let projection = self.store.projection_snapshot().map_err(|error| {
                mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
            })?;
            let mut responses = Vec::with_capacity(requests.len());
            for request in requests {
                let rows = store::test_support::execute_fact_query_projection_for_test(
                    &projection,
                    request.plan(),
                )
                .map_err(|error| {
                    mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                })?;
                self.returned_row_counts
                    .lock()
                    .expect("row counts")
                    .push(rows.len());
                let shape = mfm_facts::parse_canonical_fact_query_shape(request.plan()).map_err(
                    |error| {
                        mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                    },
                )?;
                let receipt = mfm_facts::FactQueryReceipt::from_rows(
                    mfm_facts::StoreReadFrontier::new(
                        request.plan().store_scope().clone(),
                        request.plan().query_scope().clone(),
                        mfm_facts::DescriptorCatalogWatermark::new(
                            projection.fact_descriptors().count() as u64,
                        ),
                        mfm_facts::StoreCommitOrder::new(
                            projection
                                .fact_index_entries()
                                .map(|(_, entry)| entry.store_commit_order)
                                .max()
                                .unwrap_or_default(),
                        ),
                    ),
                    mfm_facts::StoreReadFrontierType::Snapshot,
                    &rows,
                    !shape.return_fields().is_empty(),
                    request.plan().limit(),
                )
                .map_err(|error| {
                    mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                })?;
                let result = mfm_facts::FactQueryResult::new(rows, receipt).map_err(|error| {
                    mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                })?;
                responses.push(result);
            }
            Ok(responses)
        })
    }
}
