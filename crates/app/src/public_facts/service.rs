use std::sync::Arc;

use mfm_store::v1 as store;

use super::catalog::{public_fact_from_query_row, public_query_input, FactCatalogService};
use super::dto::PublicFactQueryPage;
use super::query::PublicFactQueryRequest;
use crate::{ErrorClass, PublicError};

/// App public fact query service backed by the same store used for retained artifacts.
#[derive(Debug, Clone)]
pub struct FactPublicQueryService<S> {
    catalog: FactCatalogService,
    store: Arc<S>,
}

impl<S> FactPublicQueryService<S>
where
    S: store::FactQueryStore + 'static,
{
    /// Creates a public fact query service from a public catalog and its store authority.
    pub fn new(catalog: FactCatalogService, store: Arc<S>) -> Result<Self, PublicError> {
        Ok(Self { catalog, store })
    }

    /// Executes a public Platform fact query and returns public DTOs only.
    pub async fn query(
        &self,
        request: PublicFactQueryRequest,
    ) -> Result<PublicFactQueryPage, PublicError> {
        query_public_facts(&self.catalog, self.store.as_ref(), request).await
    }
}

pub(crate) async fn query_public_facts<S>(
    catalog: &FactCatalogService,
    store: &S,
    request: PublicFactQueryRequest,
) -> Result<PublicFactQueryPage, PublicError>
where
    S: store::FactQueryStore + ?Sized,
{
    request.validate()?;
    let (_descriptor_hash, descriptor) = catalog.resolve_descriptor(&request)?;
    let input = public_query_input(&request)?;
    let plan = mfm_facts::compile_fact_query_plan(descriptor, input)?;
    let mut results = store
        .execute_fact_queries(std::slice::from_ref(&plan))
        .await
        .map_err(|_| fact_query_store_unavailable())?;
    if results.len() != 1 {
        return Err(PublicError::backend(
            ErrorClass::Internal,
            "FactQueryStoreInvalidResponse",
            "Fact query store returned an invalid response",
        ));
    }
    let result = results.pop().expect("one result length was checked");
    let facts = result
        .rows()
        .iter()
        .filter_map(|row| public_fact_from_query_row(catalog, row).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PublicFactQueryPage {
        facts,
        next_cursor: None,
    })
}

pub(crate) type AppFactQueryRow = mfm_facts::FactQueryResultRow;

fn fact_query_store_unavailable() -> PublicError {
    PublicError::backend(
        ErrorClass::Internal,
        "FactQueryStoreUnavailable",
        "Fact query store is unavailable",
    )
}
