//! Shared semantic transaction contracts, independent of native network and Runtime types.

mod authoring;
mod capability;
pub use authoring::{
    ConfigureAndObserve, ContractDeploymentLifecycle, LifecycleDefaults, LifecyclePlanning,
};
mod configure;
mod consistency;
pub use consistency::LifecycleError;
mod deploy;
mod observation;
mod read;
mod report;
pub use capability::{
    PreparedTransaction, TransactionEffect, TransactionEvidence, TransactionEvidenceError,
    TransactionRequest, TransactionResult,
};
pub use configure::{ConfigurationApplied, ConfigurationFailure, Configure, ConfiguredContract};
pub use deploy::{CheckedAddConfigurationValue, Deploy, DeployedContract, DeploymentFailure};
pub use observation::{
    ConfigurationMismatch, ObservationFailure, Observe, ObservedConfiguration, Validate,
    ValidatedConfiguration,
};
pub use read::{
    ContractRead, ContractValueEvidence, ContractValueOutcome, ReadContractValue,
    ReadContractValueError,
};
pub use report::{ContractDeploymentReport, Report};

use mfm_ids::{ContentRef, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::{Object, Unsigned256, Unsigned256Error};
use serde::{Deserialize, Serialize};

/// Authoritative deployment request, including configuration and local recovery allowances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "deployment-request",
    version = "1",
    schema = "mfm.chain-deployment-request"
)]
pub struct DeploymentRequest {
    artifact: crate::ContractArtifact,
    execution: ContractExecutionConfig,
    requested: ConfigurationValue,
    increment: ConfigurationValue,
    retry_allowance: Option<u32>,
    restart_allowance: Option<u32>,
}
impl DeploymentRequest {
    /// Retains the complete checked public request without native resource resolution.
    pub fn new(
        artifact: crate::ContractArtifact,
        execution: ContractExecutionConfig,
        requested: ConfigurationValue,
        increment: ConfigurationValue,
        retry_allowance: Option<u32>,
        restart_allowance: Option<u32>,
    ) -> Self {
        Self {
            artifact,
            execution,
            requested,
            increment,
            retry_allowance,
            restart_allowance,
        }
    }
    /// Public artifact and its claimed ledger.
    pub fn artifact(&self) -> &crate::ContractArtifact {
        &self.artifact
    }
    /// Native implementation selectors and public options.
    pub fn execution(&self) -> &ContractExecutionConfig {
        &self.execution
    }
    /// Original requested configuration scalar.
    pub fn requested(&self) -> &ConfigurationValue {
        &self.requested
    }
    /// Admitted increment for composed configuration arithmetic.
    pub fn increment(&self) -> &ConfigurationValue {
        &self.increment
    }
    /// Optional local retry override.
    pub fn retry_allowance(&self) -> Option<u32> {
        self.retry_allowance
    }
    /// Optional local restart override.
    pub fn restart_allowance(&self) -> Option<u32> {
        self.restart_allowance
    }
}
impl TransactionRequest for DeploymentRequest {
    type Applied = crate::ContractLocator;
}

/// Full-width canonical scalar used by the maintained contract workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.chain",
    name = "configuration-value",
    version = "1",
    schema = "mfm.chain-configuration-value"
)]
pub struct ConfigurationValue(Unsigned256);
impl ConfigurationValue {
    /// Checks canonical decimal syntax and the complete unsigned-256 range.
    pub fn new(decimal: impl Into<String>) -> Result<Self, Unsigned256Error> {
        Unsigned256::new(decimal).map(Self)
    }
    /// Adds two checked values, returning absence only for arithmetic overflow.
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        self.0.checked_add(&other.0).map(Self)
    }
    /// Whether this checked value is zero.
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }
}
impl std::fmt::Display for ConfigurationValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Public implementation selectors and their closed native binding/options descriptor.
/// Native protocol validation and live resource binding remain separate construction steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "contract-execution-config",
    version = "2",
    schema = "mfm.chain-contract-execution-config"
)]
pub struct ContractExecutionConfig {
    transaction_implementation: StableId,
    read_implementation: StableId,
    observation_route_ref: ContentRef,
    native: Object,
}
impl ContractExecutionConfig {
    /// Wraps checked selectors and a public native descriptor without resolving code or live handles.
    pub fn new(
        transaction_implementation: StableId,
        read_implementation: StableId,
        observation_route_ref: ContentRef,
        native: Object,
    ) -> Self {
        Self {
            transaction_implementation,
            read_implementation,
            observation_route_ref,
            native,
        }
    }
    /// Selected native transaction family member.
    pub fn transaction_implementation(&self) -> &StableId {
        &self.transaction_implementation
    }
    /// Selected native observation family member.
    pub fn read_implementation(&self) -> &StableId {
        &self.read_implementation
    }
    /// Caller-selected public observation route, qualified by the native owner.
    pub fn observation_route_ref(&self) -> &ContentRef {
        &self.observation_route_ref
    }
    /// Native binding/options Object for exact owner-qualified decoding.
    pub fn native(&self) -> &Object {
        &self.native
    }
}
