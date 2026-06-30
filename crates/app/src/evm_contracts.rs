use std::sync::Arc;

use mfm_artifact_capabilities::ArtifactReadProvider;
use mfm_evm_capabilities::{EvmSourcePolicyId, EvmSourceRef};
use mfm_signers_keystore::{KeystoreSignerProvider, KeystoreSignerRegistryEntry};
use mfm_signing::SignerRef;
use mfm_transports_evm::EvmJsonRpcClient;
use serde::Deserialize;
use uuid::Uuid;

const ENV_EVM_SIGNERS_JSON: &str = "MFM_EVM_SIGNERS_JSON";

#[derive(Clone)]
struct EnvEvmContractRuntimeFactory {
    artifacts: Arc<dyn ArtifactReadProvider>,
}

impl EnvEvmContractRuntimeFactory {
    fn new(artifacts: Arc<dyn ArtifactReadProvider>) -> Self {
        Self { artifacts }
    }
}

impl mfm_adapters_evm_contracts::EvmContractRuntimeFactory for EnvEvmContractRuntimeFactory {
    fn artifacts(&self) -> &dyn ArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn validate_runtime_for(
        &self,
        network_id: &str,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        let source_ref = EvmSourceRef::new(network_id).map_err(runtime_evm_capability_error)?;
        let policy_id = EvmSourcePolicyId::new(network_id).map_err(runtime_evm_capability_error)?;
        EvmJsonRpcClient::from_env()
            .map_err(runtime_evm_transport_error)?
            .validate_route(&policy_id, &source_ref)
            .map_err(runtime_evm_transport_error)?;
        if let Some(signer_ref) = signer_ref {
            let signer = keystore_signer_provider_from_env()?;
            if !signer.contains_signer(signer_ref) {
                return Err(mfm_runtime::RuntimeError::RunnerBinding(
                    "missing EVM signer binding".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn runtime_for(
        &self,
        network_id: &str,
    ) -> mfm_runtime::Result<mfm_adapters_evm_contracts::EvmContractRuntime> {
        let route = mfm_adapters_evm_contracts::EvmContractRuntimeRoute::new(
            EvmSourceRef::new(network_id).map_err(runtime_evm_capability_error)?,
            EvmSourcePolicyId::new(network_id).map_err(runtime_evm_capability_error)?,
        );
        let evm = Arc::new(EvmJsonRpcClient::from_env().map_err(runtime_evm_transport_error)?);
        let signer = Arc::new(keystore_signer_provider_from_env()?);
        Ok(mfm_adapters_evm_contracts::EvmContractRuntime::new(
            route, evm, signer,
        ))
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeSignerConfig {
    signer_ref: String,
    entry_id: Uuid,
    keystore_env: String,
    unlock_file_env: String,
}

fn keystore_signer_provider_from_env() -> mfm_runtime::Result<KeystoreSignerProvider> {
    let raw = std::env::var(ENV_EVM_SIGNERS_JSON).unwrap_or_else(|_| "[]".to_owned());
    let entries = serde_json::from_str::<Vec<RuntimeSignerConfig>>(&raw)
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?
        .into_iter()
        .map(|entry| {
            let signer_ref = SignerRef::new(entry.signer_ref).map_err(runtime_signing_error)?;
            KeystoreSignerRegistryEntry::from_env_sources(
                signer_ref,
                entry.entry_id,
                entry.keystore_env,
                entry.unlock_file_env,
            )
            .map_err(runtime_signing_error)
        })
        .collect::<mfm_runtime::Result<Vec<_>>>()?;
    Ok(KeystoreSignerProvider::new(entries))
}

/// Registers contract lifecycle runners in the process production runner registry.
pub(crate) fn register_contract_lifecycle_runners(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    artifacts: Arc<dyn ArtifactReadProvider>,
) -> mfm_runtime::Result<()> {
    mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
        registry,
        Arc::new(EnvEvmContractRuntimeFactory::new(artifacts)),
    )
}

fn runtime_evm_capability_error(
    error: mfm_evm_capabilities::EvmCapabilityError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_evm_transport_error(
    error: mfm_transports_evm::EvmTransportError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_signing_error(error: mfm_signing::SigningError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

#[cfg(test)]
mod tests;
