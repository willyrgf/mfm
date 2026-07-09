use std::sync::Arc;

use mfm_fact_capabilities::FactIndexReadProvider;
use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, AppError};

pub(crate) fn register_btc_collector_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
) -> Result<(), AppError> {
    let btc: Arc<dyn mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory> = runtime_config;
    let capabilities =
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(artifacts, btc, fact_index);
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(registry, capabilities)?;
    Ok(())
}
