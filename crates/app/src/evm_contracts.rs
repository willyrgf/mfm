use std::sync::Arc;

use mfm_certify::CertificationRegistry;
use mfm_evm_capabilities::EvmNetworkBinding;
use mfm_signing::SignerRef;
use mfm_store::v1 as store;

use crate::live_transports::LiveTransportRuntime;

#[derive(Clone)]
struct RuntimeConfigEvmContractRuntimeFactory {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
    source_run_registry: CertificationRegistry,
}

impl RuntimeConfigEvmContractRuntimeFactory {
    fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        runtime_config: Arc<LiveTransportRuntime>,
        source_run_registry: CertificationRegistry,
    ) -> Self {
        Self {
            artifacts,
            runtime_config,
            source_run_registry,
        }
    }

    fn evm_provider_for(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_runtime::Result<mfm_transports_evm::EvmJsonRpcNetworkProvider> {
        self.runtime_config.evm_provider(binding)
    }
}

impl mfm_adapters_evm_contracts::EvmContractRuntimeFactory
    for RuntimeConfigEvmContractRuntimeFactory
{
    fn artifacts(&self) -> &dyn store::RetainedArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn validate_runtime_for(
        &self,
        binding: &EvmNetworkBinding,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        self.runtime_config.validate_evm_network_binding(binding)?;
        if signer_ref
            .map(|signer_ref| {
                self.runtime_config
                    .signer_provider()
                    .map(|provider| provider.contains_signer(signer_ref))
            })
            .transpose()?
            == Some(false)
        {
            return Err(mfm_runtime::RuntimeError::RunnerBinding(
                "missing EVM signer binding".to_owned(),
            ));
        }
        Ok(())
    }

    fn read_runtime_for(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractReadRuntime> {
        let evm_provider = self.evm_provider_for(binding)?;
        Ok(
            mfm_adapters_evm_contracts::EvmContractReadRuntime::new_with_source_run_import_registry(
                Arc::new(evm_provider),
                self.source_run_registry.clone(),
            ),
        )
    }

    fn runtime_for(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractRuntime> {
        let evm_provider = Arc::new(self.evm_provider_for(binding)?);
        let signer = self.runtime_config.signer_provider()?;
        Ok(mfm_adapters_evm_contracts::EvmContractRuntime::new(
            evm_provider,
            signer,
        ))
    }
}

/// Registers contract lifecycle runners in the process production runner registry.
pub(crate) fn register_contract_lifecycle_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: Arc<LiveTransportRuntime>,
    source_run_registry: CertificationRegistry,
) -> mfm_runtime::Result<()> {
    mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
        registry,
        Arc::new(RuntimeConfigEvmContractRuntimeFactory::new(
            artifacts,
            runtime_config,
            source_run_registry,
        )),
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
fn test_run_store() -> mfm_store::v1::AsyncInMemoryRunStore {
    mfm_store::v1::AsyncInMemoryRunStore::default()
}
