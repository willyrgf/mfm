use std::sync::Arc;

use mfm_store::v1 as store;

use crate::{live_transports::LiveDispatchRoutes, PublicError};

pub(crate) fn register_btc_collector_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    routes: Arc<LiveDispatchRoutes>,
    read_factory: &mfm_runtime::RunnerFactoryBinding,
    adapter_factory: &mfm_runtime::RunnerFactoryBinding,
) -> Result<(), PublicError> {
    let session = routes.bitcoin_session();
    let implementation_id = session.implementation_id().to_owned();
    mfm_bitcoin_live::register_bitcoin_jsonrpc_runners(
        registry,
        artifacts,
        session,
        read_factory,
        adapter_factory,
    )?;
    registry
        .validate_capability_implementation::<mfm_bitcoin::BitcoinBalanceCollectionReadCapability>(
            &implementation_id,
        )?;
    Ok(())
}
