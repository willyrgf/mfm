use std::sync::Arc;

use mfm_store::v1 as store;

use crate::{live_transports::LiveTransportRuntime, PublicError};

pub(crate) fn register_evm_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
) -> Result<(), PublicError> {
    let validate_runtime = Arc::clone(&runtime_config);
    let bind_runtime = Arc::clone(&runtime_config);
    let capabilities = mfm_adapters_evm::EvmValidationRunnerCapabilities::new(
        artifacts.clone(),
        move |binding| validate_runtime.validate_evm_network_binding(binding),
        move |binding| {
            let runtime = Arc::clone(&bind_runtime);
            Box::pin(async move { runtime.bind_evm_read_session(binding).await })
        },
    );
    mfm_adapters_evm::register_evm_validation_runner(registry, capabilities)?;

    let validate_runtime = Arc::clone(&runtime_config);
    let bind_transaction_runtime = Arc::clone(&runtime_config);
    let bind_signer_runtime = runtime_config;
    let transaction_capabilities = mfm_adapters_evm::EvmTransactionRunnerCapabilities::new(
        artifacts,
        mfm_runtime::CapabilityImplementationId::new(
            mfm_signers_keystore::KEYSTORE_SIGNING_IMPLEMENTATION_ID,
        )?,
        move |binding, signer_ref| {
            let runtime = Arc::clone(&validate_runtime);
            Box::pin(async move {
                runtime
                    .validate_evm_mutation_binding(binding, signer_ref)
                    .await
            })
        },
        move |binding| {
            let runtime = Arc::clone(&bind_transaction_runtime);
            Box::pin(async move { runtime.bind_evm_transaction_session(binding).await })
        },
        move |signer_ref| {
            let runtime = Arc::clone(&bind_signer_runtime);
            Box::pin(async move { runtime.bind_evm_signer(signer_ref).await })
        },
    );
    mfm_adapters_evm::register_evm_transaction_runner(registry, transaction_capabilities)?;
    Ok(())
}
