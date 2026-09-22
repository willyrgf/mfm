//! Admission of the maintained scalar-contract workflow, without IO or Runtime authority.

use mfm_capabilities::{EffectImplementation, ReadImplementation};
use mfm_chain::{
    transaction::{
        CheckedAddConfigurationValue, ConfigurationValue, ContractDeploymentLifecycle,
        ContractExecutionConfig, ContractRead, DeploymentRequest, TransactionEffect,
    },
    CheckedAdd, ContractArtifact, LedgerIdentity,
};
use mfm_evm::{
    EvmContractExecutionConfig, EvmContractReadImplementation, EvmScalarContractArtifact,
    EvmTransactionImplementation,
};
use mfm_program::Pure;
use mfm_values::Object;
use serde::{Deserialize, Serialize};

/// Maintained lifecycle sources plus explicitly published downstream semantics.
///
/// Recomposition needs no publication changes: discovery derives child States, handlers, codecs
/// and injected support. A new State or handler must be reachable through `Additional` for cold
/// loading; automatic downstream code discovery remains a documented limitation.
// See docs/known-gaps.md#downstream-component-discovery-in-the-dsl-refactor.
// Keep this publication site distinct from the configuration-selected native implementation.
pub type ContractResources<Additional = ()> = crate::EvmResources<(
    ContractDeploymentLifecycle,
    Pure<CheckedAddConfigurationValue>,
    Pure<CheckedAdd>,
    Additional,
)>;

/// Public configuration for the exact maintained scalar-contract artifact and ABI.
///
/// Each field is checked on construction or deserialization. Admission derives native identities
/// internally; callers supply neither implementation IDs nor pre-encoded configuration calldata.
/// Live resource correspondence is checked when the Program is constructed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvmContractConfig {
    /// Pinned compiler output supported by the scalar lifecycle, not arbitrary contract bytecode.
    pub artifact: EvmScalarContractArtifact,
    /// Public transaction binding and separate deployment/configuration fee options.
    pub execution: EvmContractExecutionConfig,
    /// Initial configuration value.
    pub requested: ConfigurationValue,
    /// Increment used only when the addition State is selected.
    pub increment: ConfigurationValue,
    /// Optional Operation retry override.
    pub retry_allowance: Option<u32>,
    /// Optional Operation restart override.
    pub restart_allowance: Option<u32>,
}

/// Admission rejection retaining the concrete native identity or encoding cause.
#[derive(Debug, Serialize, thiserror::Error)]
pub enum EvmContractClientError {
    /// The selected native implementation identity could not be constructed.
    #[error("contract capability identity is invalid")]
    Capability(#[from] mfm_capabilities::CapabilityError),
    /// Public native facts could not be encoded with their exact contract.
    #[error("contract configuration encoding failed")]
    Value(#[from] mfm_values::ValueError),
}

impl EvmContractConfig {
    /// Admits semantic input from checked native configuration, without live resource access.
    pub fn into_request(self) -> Result<DeploymentRequest, EvmContractClientError> {
        let route = &self.execution.binding().route;
        let ledger = LedgerIdentity::new(Object::from_value(&route.chain_instance)?);
        let observation_route = Object::from_value(route)?.value_ref().clone();
        Ok(DeploymentRequest::new(
            ContractArtifact::new(ledger, Object::from_value(&self.artifact)?),
            ContractExecutionConfig::new(
                <EvmTransactionImplementation as EffectImplementation<
                    TransactionEffect<DeploymentRequest>,
                >>::implementation_id()?,
                <EvmContractReadImplementation as ReadImplementation<ContractRead>>::implementation_id()?,
                observation_route,
                Object::from_value(&self.execution)?,
            ),
            self.requested,
            self.increment,
            self.retry_allowance,
            self.restart_allowance,
        ))
    }
}
