use std::sync::Arc;

use mfm_fact_capabilities::{
    FactIndexReadProvider, FactIndexReadRequest, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_store::v1 as store;

use crate::{AppError, ErrorClass, ProductionRunStore};

pub(crate) fn register_btc_collector_runners_if_configured(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Option<Arc<dyn FactIndexReadProvider>>,
    btc: Option<mfm_runtime_config::BtcRuntimeConfig>,
) -> Result<(), AppError> {
    let (Some(btc), Some(fact_index)) = (btc, fact_index) else {
        return Ok(());
    };
    let btc = Arc::new(btc_json_rpc_read_provider(btc)?);
    let capabilities =
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(artifacts, btc, fact_index);
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(registry, capabilities)?;
    Ok(())
}

pub(crate) fn btc_json_rpc_read_provider(
    btc: mfm_runtime_config::BtcRuntimeConfig,
) -> mfm_runtime::Result<mfm_adapters_btc_jsonrpc::BtcJsonRpcChainHeadProvider> {
    let client = btc_json_rpc_client(btc)?;
    Ok(mfm_adapters_btc_jsonrpc::BtcJsonRpcChainHeadProvider::new(
        Arc::new(client),
    ))
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

fn btc_json_rpc_client(
    btc: mfm_runtime_config::BtcRuntimeConfig,
) -> mfm_runtime::Result<mfm_collectors_btc_jsonrpc_http::BtcJsonRpcClient> {
    let json_rpc = btc.json_rpc();
    let config = mfm_collectors_btc_jsonrpc_http::BtcJsonRpcConfig {
        rpc_url: json_rpc.rpc_url().expose_secret().to_owned(),
        rpc_user: json_rpc
            .rpc_user()
            .map(|value| value.expose_secret().to_owned()),
        rpc_password: json_rpc
            .rpc_password()
            .map(|value| value.expose_secret().to_owned()),
    };
    mfm_collectors_btc_jsonrpc_http::BtcJsonRpcClient::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))
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
            let result = self
                .store
                .execute_fact_query(request.plan())
                .await
                .map_err(mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure)?;
            Ok(FactIndexReadResponse::from_receipt(
                result.receipt().clone(),
                self.trust_root.clone(),
            ))
        })
    }
}
