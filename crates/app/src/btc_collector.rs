use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_btc_capabilities::{BtcChainHeadReadProvider, BtcSourceBinding};
use mfm_fact_capabilities::{
    FactIndexReadProvider, FactIndexReadRequest, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_store::v1 as store;

use crate::{runtime_btc_capability_error, AppError, ErrorClass, ProductionRunStore};

pub(crate) fn register_btc_collector_runners_if_configured(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Option<Arc<dyn FactIndexReadProvider>>,
    btc: Option<mfm_runtime_config::BtcRuntimeConfig>,
) -> Result<(), AppError> {
    let (Some(btc), Some(fact_index)) = (btc, fact_index) else {
        return Ok(());
    };
    let btc = Arc::new(btc_json_rpc_provider_factory(btc)?);
    let capabilities =
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(artifacts, btc, fact_index);
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(registry, capabilities)?;
    Ok(())
}

#[derive(Clone)]
pub(crate) struct RuntimeBtcJsonRpcProviderFactory {
    router: mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter,
}

pub(crate) fn btc_json_rpc_provider_factory(
    btc: mfm_runtime_config::BtcRuntimeConfig,
) -> mfm_runtime::Result<RuntimeBtcJsonRpcProviderFactory> {
    Ok(RuntimeBtcJsonRpcProviderFactory {
        router: btc_json_rpc_router(btc)?,
    })
}

fn btc_json_rpc_router(
    btc: mfm_runtime_config::BtcRuntimeConfig,
) -> mfm_runtime::Result<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter> {
    let mut routes = BTreeMap::new();
    for (source_identity, route) in btc.routes() {
        let client = btc_json_rpc_client(route)?;
        routes.insert(
            source_identity.clone(),
            Arc::new(client)
                as Arc<dyn mfm_transports_btc_jsonrpc_http::BtcJsonRpcChainHeadTransport>,
        );
    }
    Ok(mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter::new(
        routes,
    ))
}

impl mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory for RuntimeBtcJsonRpcProviderFactory {
    fn validate_source_binding(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<()> {
        self.router.validate_source_binding(binding)
    }

    fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcChainHeadReadProvider>> {
        self.router
            .bind_source(binding)
            .map(|provider| Arc::new(provider) as Arc<dyn BtcChainHeadReadProvider>)
    }
}

impl mfm_adapters_portfolio::PortfolioBtcProviderFactory for RuntimeBtcJsonRpcProviderFactory {
    fn validate_btc_source_binding(&self, binding: &BtcSourceBinding) -> mfm_runtime::Result<()> {
        self.router
            .validate_source_binding(binding)
            .map_err(runtime_btc_capability_error)
    }

    fn bind_btc_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn mfm_adapters_portfolio::PortfolioBtcProvider>> {
        self.router.bind_source(binding).map(|provider| {
            Arc::new(provider) as Arc<dyn mfm_adapters_portfolio::PortfolioBtcProvider>
        })
    }
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
    json_rpc: &mfm_runtime_config::BtcJsonRpcRuntimeConfig,
) -> mfm_runtime::Result<mfm_transports_btc_jsonrpc_http::BtcJsonRpcClient> {
    let config = mfm_transports_btc_jsonrpc_http::BtcJsonRpcConfig {
        rpc_url: json_rpc.rpc_url().expose_secret().to_owned(),
        rpc_user: json_rpc
            .rpc_user()
            .map(|value| value.expose_secret().to_owned()),
        rpc_password: json_rpc
            .rpc_password()
            .map(|value| value.expose_secret().to_owned()),
    };
    mfm_transports_btc_jsonrpc_http::BtcJsonRpcClient::new(config)
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
