use std::sync::Arc;

use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBlockReadProvider, EvmCallReadProvider, EvmNetworkBinding,
};
use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, PublicError};

pub(crate) fn register_evm_collector_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
) -> Result<(), PublicError> {
    let evm: Arc<dyn mfm_adapters_evm::EvmProviderFactory> = runtime_config;
    let capabilities = mfm_adapters_evm::EvmRunnerCapabilities::new(artifacts, evm);
    mfm_adapters_evm::register_evm_collectors_runners(registry, capabilities)?;
    Ok(())
}

impl mfm_adapters_evm::EvmProviderFactory for LiveTransportRuntime {
    fn validate_network_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()> {
        self.validate_evm_network_binding(binding)
    }

    fn bind_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn mfm_adapters_evm::EvmBoundProvider>> {
        let provider = self.evm_provider(binding)?;
        Ok(Arc::new(BoundLiveEvmProvider { provider }))
    }
}

struct BoundLiveEvmProvider {
    provider: mfm_transports_evm::EvmJsonRpcNetworkProvider,
}

impl EvmBlockReadProvider for BoundLiveEvmProvider {
    fn read_block<'a>(
        &'a self,
        request: &'a mfm_evm_capabilities::EvmBlockReadRequest,
    ) -> mfm_evm_capabilities::EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmBlockReadResponse>
    {
        self.provider.read_block(request)
    }
}

impl EvmBalanceReadProvider for BoundLiveEvmProvider {
    fn read_balance<'a>(
        &'a self,
        request: &'a mfm_evm_capabilities::EvmBalanceReadRequest,
    ) -> mfm_evm_capabilities::EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmBalanceReadResponse>
    {
        self.provider.read_balance(request)
    }
}

impl EvmCallReadProvider for BoundLiveEvmProvider {
    fn read_call<'a>(
        &'a self,
        request: &'a mfm_evm_capabilities::EvmCallReadRequest,
    ) -> mfm_evm_capabilities::EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmCallReadResponse>
    {
        self.provider.read_call(request)
    }
}
