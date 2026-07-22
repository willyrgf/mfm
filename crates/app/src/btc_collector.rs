use std::sync::Arc;

use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, PublicError};

pub(crate) fn register_btc_collector_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
    read_factory: &mfm_runtime::RunnerFactoryBinding,
    adapter_factory: &mfm_runtime::RunnerFactoryBinding,
) -> Result<(), PublicError> {
    runtime_config.initialize_bitcoin_routes();
    let session: Arc<dyn mfm_btc_capabilities::BitcoinBalanceSession> = runtime_config;
    mfm_adapters_btc_jsonrpc::register_bitcoin_jsonrpc_runners(
        registry,
        artifacts,
        session,
        read_factory,
        adapter_factory,
    )?;
    Ok(())
}
