use std::sync::Arc;

use mfm_artifact_capabilities::ArtifactReadProvider;
use mfm_evm_capabilities::{EvmChainGuard, EvmNetworkId};
use mfm_signers_keystore::{
    KeystorePasswordSource, KeystorePathSource, KeystoreSignerProvider, KeystoreSignerRegistryEntry,
};
use mfm_signing::SignerRef;

use crate::{evm_json_rpc_client, runtime_evm_transport_error, RuntimeConfigLoader};

#[derive(Clone)]
struct RuntimeConfigEvmContractRuntimeFactory {
    artifacts: Arc<dyn ArtifactReadProvider>,
    runtime_config: RuntimeConfigLoader,
}

impl RuntimeConfigEvmContractRuntimeFactory {
    fn new(artifacts: Arc<dyn ArtifactReadProvider>, runtime_config: RuntimeConfigLoader) -> Self {
        Self {
            artifacts,
            runtime_config,
        }
    }

    fn load_evm(&self) -> mfm_runtime::Result<mfm_runtime_config::EvmRuntimeConfig> {
        self.runtime_config.load_evm()
    }
}

impl mfm_adapters_evm_contracts::EvmContractRuntimeFactory
    for RuntimeConfigEvmContractRuntimeFactory
{
    fn artifacts(&self) -> &dyn ArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn validate_runtime_for(
        &self,
        network_id: &str,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        let evm = self.load_evm()?;
        let network_id = EvmNetworkId::new(network_id)
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
        let client = evm_json_rpc_client(evm.clone())?;
        client
            .validate_guard(&EvmChainGuard::new(network_id, 0))
            .map_err(runtime_evm_transport_error)?;
        if let Some(signer_ref) = signer_ref {
            if !evm.signers().contains_key(signer_ref) {
                return Err(mfm_runtime::RuntimeError::RunnerBinding(
                    "missing EVM signer binding".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn runtime_for(
        &self,
        _network_id: &str,
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractRuntime> {
        let evm = self.load_evm()?;
        let evm_provider = Arc::new(evm_json_rpc_client(evm.clone())?);
        let signer = Arc::new(keystore_signer_provider_from_config(&evm));
        Ok(mfm_adapters_evm_contracts::EvmContractRuntime::new(
            evm_provider,
            signer,
        ))
    }
}

fn keystore_signer_provider_from_config(
    evm: &mfm_runtime_config::EvmRuntimeConfig,
) -> KeystoreSignerProvider {
    let entries = evm.signers().iter().map(|(signer_ref, signer)| {
        let signer = signer.as_keystore();
        KeystoreSignerRegistryEntry::new(
            signer_ref.clone(),
            signer.entry_id(),
            KeystorePathSource::path(signer.keystore_path().expose_path()),
            KeystorePasswordSource::file(signer.unlock_file().expose_path()),
        )
    });
    KeystoreSignerProvider::new(entries)
}

/// Registers contract lifecycle runners in the process production runner registry.
pub(crate) fn register_contract_lifecycle_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn ArtifactReadProvider>,
    runtime_config: RuntimeConfigLoader,
) -> mfm_runtime::Result<()> {
    mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
        registry,
        Arc::new(RuntimeConfigEvmContractRuntimeFactory::new(
            artifacts,
            runtime_config,
        )),
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
fn test_run_store() -> mfm_store::v1::AsyncInMemoryRunStore {
    mfm_store::v1::AsyncInMemoryRunStore::default()
}
