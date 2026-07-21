use std::sync::Arc;

use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, PublicError};

pub(crate) fn register_btc_collector_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
) -> Result<(), PublicError> {
    let btc: Arc<dyn mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory> = runtime_config;
    let capabilities = mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(artifacts, btc);
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(registry, capabilities)?;
    Ok(())
}
