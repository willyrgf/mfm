use std::sync::Arc;

use mfm_artifact_capabilities::ArtifactReadProvider;
use mfm_fact_capabilities::{
    FactIndexReadProvider, FactIndexReadRequest, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};

use crate::{AppError, ErrorClass, ProductionRunStore, RuntimeConfigLoader};

pub(crate) fn register_btc_collector_runners_if_configured(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn ArtifactReadProvider>,
    fact_index: Option<Arc<dyn FactIndexReadProvider>>,
    runtime_config: RuntimeConfigLoader,
) -> Result<(), AppError> {
    let Some(btc) = runtime_config.load_optional_btc()? else {
        return Ok(());
    };
    let fact_index = fact_index.ok_or_else(|| {
        AppError::backend(
            ErrorClass::BadRequest,
            "LaunchRunnerUnavailable",
            "A required typed runner is unavailable",
        )
    })?;
    let client = btc_json_rpc_client(btc)?;
    let btc = Arc::new(mfm_adapters_btc_jsonrpc::BtcJsonRpcChainHeadProvider::new(
        Arc::new(client),
    ));
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
        .ok_or_else(|| {
            AppError::backend(
                ErrorClass::BadRequest,
                "LaunchRunnerUnavailable",
                "A required typed runner is unavailable",
            )
        })?;
    let trust_root = FactQueryReceiptTrustRootMaterial::new(
        trust_root.store_identity().clone(),
        trust_root.scheme(),
        trust_root.key_id().clone(),
        *trust_root.verifying_key(),
    );
    Ok(Arc::new(PostgresFactIndexReadProvider {
        store,
        trust_root,
    }))
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
            let rows = result
                .rows()
                .iter()
                .map(|row| {
                    mfm_fact_capabilities::FactIndexReadRow::new(
                        row.fact_ref().clone(),
                        row.returned_fields().to_vec(),
                    )
                })
                .collect();
            FactIndexReadResponse::new(rows, result.receipt().clone(), self.trust_root.clone())
        })
    }
}
