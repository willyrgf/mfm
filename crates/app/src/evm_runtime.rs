use std::sync::Arc;

use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, PublicError};

pub(crate) fn register_evm_balance_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
) -> Result<(), PublicError> {
    let validate_runtime = Arc::clone(&runtime_config);
    let bind_runtime = Arc::clone(&runtime_config);
    let capabilities = mfm_adapters_evm::EvmReadRunnerCapabilities::new(
        artifacts,
        move |binding| {
            let runtime = Arc::clone(&validate_runtime);
            Box::pin(async move { runtime.validate_evm_read_route(binding).await })
        },
        move |binding| {
            let runtime = Arc::clone(&bind_runtime);
            Box::pin(async move { runtime.bind_evm_read_session(binding).await })
        },
    );
    mfm_adapters_evm::register_evm_balance_runners(registry, capabilities)?;
    Ok(())
}
