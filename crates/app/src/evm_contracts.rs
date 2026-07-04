use std::sync::Arc;

use mfm_certify::CertificationRegistry;
use mfm_evm_capabilities::EvmNetworkId;
use mfm_signers_keystore::{KeystoreSignerProvider, KeystoreSignerRegistryEntry};
use mfm_signing::SignerRef;
use mfm_store::v1 as store;

use crate::{evm_json_rpc_client, runtime_evm_transport_error, RuntimeConfigLoader};

#[cfg(test)]
use mfm_evm_capabilities::EvmChainGuard;

#[derive(Clone)]
struct RuntimeConfigEvmContractRuntimeFactory {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    runtime_config: RuntimeConfigLoader,
    source_run_registry: CertificationRegistry,
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
        }
    }

    fn load_evm(&self) -> mfm_runtime::Result<mfm_runtime_config::EvmRuntimeConfig> {
        self.runtime_config.load_evm()
    }

    fn load_runtime_config_with_signers(
        &self,
    ) -> mfm_runtime::Result<mfm_runtime_config::RuntimeConfig> {
        self.runtime_config.load_runtime_config_with_signers()
    }
}

impl mfm_adapters_evm_contracts::EvmContractRuntimeFactory
    for RuntimeConfigEvmContractRuntimeFactory
{
    fn artifacts(&self) -> &dyn store::RetainedArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn source_run_import_registry(&self) -> Option<&CertificationRegistry> {
        Some(&self.source_run_registry)
    }

    fn validate_runtime_for(
        &self,
        network_id: &str,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        let (evm, signer_configured) = if let Some(signer_ref) = signer_ref {
            let runtime_config = self.load_runtime_config_with_signers()?;
            let evm = runtime_config.evm().cloned().ok_or_else(|| {
                mfm_runtime::RuntimeError::RunnerBinding("missing EVM runtime config".to_owned())
            })?;
            (evm, runtime_config.signers().contains_key(signer_ref))
        } else {
            (self.load_evm()?, true)
        };
        let network_id = EvmNetworkId::new(network_id)
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
        let client = evm_json_rpc_client(evm.clone())?;
        client
            .validate_route_binding(&network_id)
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
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractReadRuntime> {
        let evm = self.load_evm()?;
        let network_id = EvmNetworkId::new(network_id)
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
        let evm_provider = evm_json_rpc_client(evm)?;
        evm_provider
            .validate_route_binding(&network_id)
            .map_err(runtime_evm_transport_error)?;
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
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractRuntime> {
        let runtime_config = self.load_runtime_config_with_signers()?;
        let evm = runtime_config.evm().cloned().ok_or_else(|| {
            mfm_runtime::RuntimeError::RunnerBinding("missing EVM runtime config".to_owned())
        })?;
        let network_id = EvmNetworkId::new(network_id)
            .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
        let evm_client = evm_json_rpc_client(evm.clone())?;
        evm_client
            .validate_route_binding(&network_id)
            .map_err(runtime_evm_transport_error)?;
        let evm_provider = Arc::new(evm_client);
        let signer = Arc::new(keystore_signer_provider_from_config(&runtime_config)?);
        Ok(mfm_adapters_evm_contracts::EvmContractRuntime::new(
            evm_provider,
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

#[cfg(test)]
fn test_run_store() -> mfm_store::v1::AsyncInMemoryRunStore {
    mfm_store::v1::AsyncInMemoryRunStore::default()
}
