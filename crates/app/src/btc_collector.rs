use std::sync::Arc;

use mfm_fact_capabilities::{
    FactIndexReadProvider, FactIndexReadRequest, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, AppError, ErrorClass, ProductionRunStore};

pub(crate) fn register_btc_collector_runners_if_configured(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
    btc_configured: bool,
) -> Result<(), AppError> {
    if !btc_configured {
        return Ok(());
    }
    let btc: Arc<dyn mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory> = runtime_config;
    let capabilities =
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(artifacts, btc, fact_index);
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(registry, capabilities)?;
    Ok(())
}

/// Builds the production Platform/Control fact-index provider from a Postgres run store.
///
/// Requires a fact-receipt trust root on the store authority. Production run services and
/// portfolio/BTC collector registration always use this provider (no unavailable stub).
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
