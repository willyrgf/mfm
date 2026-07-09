//! Process-bound Platform/Control fact-index providers.
//!
//! Production assembly always uses [`production_fact_index_read_provider`] (Postgres).
//! Store-backed tests use [`ProjectionFactIndexProvider`] over an in-memory projection.
//! Adapter unit tests keep local plan fixtures (`MockFactIndex` in adapter crates).

use std::sync::{Arc, Mutex};

use mfm_fact_capabilities::{
    FactIndexReadProvider, FactIndexReadRequest, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_store::v1 as store;

use crate::{AppError, ErrorClass, ProductionRunStore};

/// Builds the production Platform/Control fact-index provider from a Postgres run store.
///
/// Requires a fact-receipt trust root on the store authority. Production run services,
/// REST live services, and portfolio/BTC collector registration use this provider only.
pub fn production_fact_index_read_provider(
    store: ProductionRunStore,
) -> Result<Arc<dyn FactIndexReadProvider>, AppError> {
    let trust_root = store
        .store_authority()
        .fact_receipt_trust_root()
        .ok_or_else(missing_fact_receipt_trust_root)?
        .to_material();
    Ok(Arc::new(PostgresFactIndexReadProvider {
        store,
        trust_root,
    }))
}

fn missing_fact_receipt_trust_root() -> AppError {
    AppError::backend(
        ErrorClass::BadRequest,
        "MissingFactReceiptTrustRoot",
        "Postgres run store is missing fact receipt trust root required for Platform fact-index",
    )
}

struct PostgresFactIndexReadProvider {
    store: ProductionRunStore,
    trust_root: FactQueryReceiptTrustRootMaterial,
}

impl FactIndexReadProvider for PostgresFactIndexReadProvider {
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
            Ok(results
                .into_iter()
                .map(|result| {
                    FactIndexReadResponse::from_receipt(
                        result.receipt().clone(),
                        self.trust_root.clone(),
                    )
                })
                .collect())
        })
    }
}

/// Projection-backed fact-index over an in-memory store snapshot.
///
/// Single store-backed test/process-assembly provider (not production Postgres). Freezes one
/// projection for the whole batch (shared selection frontier), signs receipts with a fixed test
/// key, and verifies authentication before returning rows.
pub struct ProjectionFactIndexProvider {
    store: store::AsyncInMemoryRunStore,
    signing_key: ed25519_dalek::SigningKey,
    store_identity: mfm_facts::StoreIdentity,
    key_id: mfm_facts::StoreKeyId,
    returned_row_counts: Mutex<Vec<usize>>,
}

impl ProjectionFactIndexProvider {
    /// Builds a provider that reads fact rows from the supplied in-memory store projection.
    pub fn new(store: store::AsyncInMemoryRunStore) -> Self {
        Self {
            store,
            signing_key: ed25519_dalek::SigningKey::from_bytes(&[0x43; 32]),
            store_identity: mfm_facts::StoreIdentity::new("mfm.app.projection.fact_index")
                .expect("store identity"),
            key_id: mfm_facts::StoreKeyId::new("app.projection.fact.read").expect("store key id"),
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

    /// Store trust root matching this provider's signed receipts.
    pub fn receipt_trust_root(&self) -> store::FactQueryReceiptTrustRoot {
        store::test_support::fact_query_receipt_trust_root_for_test(
            &self.signing_key,
            self.store_identity.clone(),
            self.key_id.clone(),
        )
    }

    fn trust_root_material(&self) -> FactQueryReceiptTrustRootMaterial {
        FactQueryReceiptTrustRootMaterial::new(
            self.store_identity.clone(),
            mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            self.key_id.clone(),
            self.signing_key.verifying_key().to_bytes(),
        )
    }
}

impl FactIndexReadProvider for ProjectionFactIndexProvider {
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
            let trust_root = self.receipt_trust_root();
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
                let receipt =
                    store::test_support::signed_fact_query_receipt_for_projection_for_test(
                        request.plan(),
                        &projection,
                        &self.signing_key,
                        self.store_identity.clone(),
                        self.key_id.clone(),
                        &rows,
                    );
                let plan_hash =
                    mfm_facts::fact_query_plan_hash(request.plan()).map_err(|error| {
                        mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                    })?;
                store::verify_fact_query_receipt_authentication(&plan_hash, &receipt, &trust_root)
                    .map_err(|error| {
                        mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                    })?;
                responses.push(FactIndexReadResponse::from_receipt(
                    receipt,
                    self.trust_root_material(),
                ));
            }
            Ok(responses)
        })
    }
}
