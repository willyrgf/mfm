use std::sync::{Arc, OnceLock};

use mfm_certify::CertificationRegistry;
use mfm_evm_capabilities::{EvmNetworkBinding, EvmNetworkId};
use mfm_signers_keystore::{KeystoreSignerProvider, KeystoreSignerRegistryEntry};
use mfm_signing::SignerRef;
use mfm_store::v1 as store;

use crate::{evm_json_rpc_client, runtime_evm_transport_error, RuntimeConfigLoader};

struct RuntimeConfigEvmContractRuntimeFactory {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: RuntimeConfigLoader,
    source_run_registry: CertificationRegistry,
    /// Process-local EVM client built once on first use (not reloaded per bind/call).
    evm_client: OnceLock<mfm_transports_evm::EvmJsonRpcClient>,
}

impl RuntimeConfigEvmContractRuntimeFactory {
    fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        runtime_config: RuntimeConfigLoader,
        source_run_registry: CertificationRegistry,
    ) -> Self {
        Self {
            artifacts,
            runtime_config,
            source_run_registry,
            evm_client: OnceLock::new(),
        }
    }

    fn load_runtime_config_with_signers(
        &self,
    ) -> mfm_runtime::Result<mfm_runtime_config::RuntimeConfig> {
        self.runtime_config.load_runtime_config_with_signers()
    }

    fn network_binding(
        &self,
        network_id: &str,
        expected_chain_id: u64,
    ) -> mfm_runtime::Result<EvmNetworkBinding> {
        let network_id = EvmNetworkId::new(network_id)
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
        EvmNetworkBinding::new(network_id, expected_chain_id)
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))
    }

    fn evm_client(&self) -> mfm_runtime::Result<&mfm_transports_evm::EvmJsonRpcClient> {
        if let Some(client) = self.evm_client.get() {
            return Ok(client);
        }
        let client = self
            .runtime_config
            .load_evm()
            .and_then(evm_json_rpc_client)?;
        let _ = self.evm_client.set(client);
        self.evm_client.get().ok_or_else(|| {
            mfm_runtime::RuntimeError::RunnerBinding("EVM client initialization race".to_owned())
        })
    }

    fn bound_evm_provider(
        &self,
        network_id: &str,
        expected_chain_id: u64,
    ) -> mfm_runtime::Result<mfm_transports_evm::EvmJsonRpcNetworkProvider> {
        let binding = self.network_binding(network_id, expected_chain_id)?;
        self.evm_client()?
            .bind_network(binding)
            .map_err(runtime_evm_transport_error)
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
        network_id: &str,
        expected_chain_id: u64,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        let signer_configured = if let Some(signer_ref) = signer_ref {
            let runtime_config = self.load_runtime_config_with_signers()?;
            runtime_config.signers().contains_key(signer_ref)
        } else {
            true
        };
        let binding = self.network_binding(network_id, expected_chain_id)?;
        self.evm_client()?
            .validate_network_binding(&binding)
            .map_err(runtime_evm_transport_error)?;
        if !signer_configured {
            return Err(mfm_runtime::RuntimeError::RunnerBinding(
                "missing EVM signer binding".to_owned(),
            ));
        }
        Ok(())
    }

    fn read_runtime_for(
        &self,
        network_id: &str,
        expected_chain_id: u64,
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractReadRuntime> {
        let evm_provider = self.bound_evm_provider(network_id, expected_chain_id)?;
        Ok(
            mfm_adapters_evm_contracts::EvmContractReadRuntime::new_with_source_run_import_registry(
                Arc::new(evm_provider),
                self.source_run_registry.clone(),
            ),
        )
    }

    fn runtime_for(
        &self,
        network_id: &str,
        expected_chain_id: u64,
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractRuntime> {
        let runtime_config = self.load_runtime_config_with_signers()?;
        let evm_provider = self.bound_evm_provider(network_id, expected_chain_id)?;
        let signer = Arc::new(keystore_signer_provider_from_config(&runtime_config)?);
        Ok(mfm_adapters_evm_contracts::EvmContractRuntime::new(
            Arc::new(evm_provider),
            signer,
        ))
    }
}

fn keystore_signer_provider_from_config(
    runtime_config: &mfm_runtime_config::RuntimeConfig,
) -> mfm_runtime::Result<KeystoreSignerProvider> {
    let entries = runtime_config.signers().iter().map(|(signer_ref, signer)| {
        let signer = signer.as_keystore();
        let keystore = runtime_config
            .keystores()
            .get(signer.keystore_ref())
            .ok_or_else(|| {
                mfm_runtime::RuntimeError::RunnerBinding(
                    "missing keystore profile for signer binding".to_owned(),
                )
            })?;
        Ok(KeystoreSignerRegistryEntry::new(
            signer_ref.clone(),
            signer.entry_id(),
            keystore.keystore_path().expose_path(),
            keystore.unlock_file().expose_path(),
        ))
    });
    Ok(KeystoreSignerProvider::new(
        entries.collect::<mfm_runtime::Result<Vec<_>>>()?,
    ))
}

/// Registers contract lifecycle runners in the process production runner registry.
pub(crate) fn register_contract_lifecycle_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: RuntimeConfigLoader,
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
