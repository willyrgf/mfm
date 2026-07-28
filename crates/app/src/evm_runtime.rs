use std::sync::Arc;

use crate::{live_transports::LiveDispatchRoutes, PublicError};

pub(crate) fn register_evm_balance_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    routes: Arc<LiveDispatchRoutes>,
    read_factory: &mfm_runtime::RunnerFactoryBinding,
    adapter_factory: &mfm_runtime::RunnerFactoryBinding,
) -> Result<(), PublicError> {
    let sessions = routes.evm_read_sessions();
    let implementation_id = sessions.implementation_id().to_owned();
    mfm_evm_live::register_evm_balance_runners(registry, sessions, read_factory, adapter_factory)?;
    registry
        .validate_capability_implementation::<mfm_evm::EvmReadCapability>(&implementation_id)?;
    Ok(())
}
