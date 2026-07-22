use std::sync::Arc;

use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, PublicError};

pub(crate) fn register_evm_balance_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
    read_factory: &mfm_runtime::RunnerFactoryBinding,
    adapter_factory: &mfm_runtime::RunnerFactoryBinding,
) -> Result<(), PublicError> {
    let capabilities =
        mfm_evm_live::EvmReadRunnerCapabilities::new(artifacts, runtime_config.evm_read_sessions());
    mfm_evm_live::register_evm_balance_runners(
        registry,
        capabilities,
        read_factory,
        adapter_factory,
    )?;
    Ok(())
}
