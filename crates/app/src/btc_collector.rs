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
    fact_index: Option<Arc<dyn FactIndexReadProvider>>,
    runtime_config: Arc<LiveTransportRuntime>,
    btc_configured: bool,
) -> Result<(), AppError> {
    let Some(fact_index) = fact_index.filter(|_| btc_configured) else {
        return Ok(());
    };
    let btc: Arc<dyn mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory> = runtime_config;
    let capabilities =
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(artifacts, btc, fact_index);
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(registry, capabilities)?;
    Ok(())
}

pub(crate) fn production_fact_index_read_provider(
    store: ProductionRunStore,
) -> Result<Arc<dyn FactIndexReadProvider>, AppError> {
    let trust_root = store
        .store_authority()
        .fact_receipt_trust_root()
        .ok_or_else(launch_runner_unavailable)?
        .to_material();
    Ok(Arc::new(PostgresFactIndexReadProvider {
        store,
        trust_root,
    }))
}

fn launch_runner_unavailable() -> AppError {
    AppError::backend(
        ErrorClass::BadRequest,
        "LaunchRunnerUnavailable",
        "A required typed runner is unavailable",
    )
}

struct PostgresFactIndexReadProvider {
    store: ProductionRunStore,
    trust_root: FactQueryReceiptTrustRootMaterial,
}

impl FactIndexReadProvider for PostgresFactIndexReadProvider {
    fn read_fact_index<'a>(
        &'a self,
        request: &'a FactIndexReadRequest,
    ) -> mfm_fact_capabilities::FactIndexReadFuture<'a> {
        Box::pin(async move {
            let mut responses = self
                .read_fact_index_batch(std::slice::from_ref(request))
                .await?;
            responses.pop().ok_or_else(|| {
                mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(
                    "fact-index batch returned no response for single request",
                )
            })
        })
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
