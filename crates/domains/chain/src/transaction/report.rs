use super::{
    consistency, ConfigurationApplied, ConfigurationValue, ContractValueEvidence,
    DeploymentRequest, LifecycleError, TransactionEvidence, ValidatedConfiguration,
};
use crate::ContractLocator;
use mfm_ids::StableId;
use mfm_program::{Never, ProposedStateOutcome, PureState, State};
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};

/// Flattened checked lifecycle facts, retaining each semantic field and native original once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "contract-deployment-report",
    version = "1",
    schema = "mfm.chain-contract-deployment-report"
)]
pub struct ContractDeploymentReport {
    request: DeploymentRequest,
    effective: ConfigurationValue,
    deployment: TransactionEvidence<ContractLocator>,
    configuration: TransactionEvidence<ConfigurationApplied>,
    observation: ContractValueEvidence,
}
impl ContractDeploymentReport {
    /// Checks internal semantic consistency, without authenticating native evidence or history.
    pub fn new(
        request: DeploymentRequest,
        effective: ConfigurationValue,
        deployment: TransactionEvidence<ContractLocator>,
        configuration: TransactionEvidence<ConfigurationApplied>,
        observation: ContractValueEvidence,
    ) -> Result<Self, LifecycleError> {
        let target = consistency::deployment(&request, &deployment)?;
        consistency::configuration(target, &configuration)?;
        consistency::observation(
            request.execution().observation_route_ref(),
            target,
            &configuration,
            &observation,
        )?;
        consistency::value(&effective, &observation)?;
        Ok(Self {
            request,
            effective,
            deployment,
            configuration,
            observation,
        })
    }
    /// Original complete request.
    pub fn request(&self) -> &DeploymentRequest {
        &self.request
    }
    /// Original requested scalar.
    pub fn requested_value(&self) -> &ConfigurationValue {
        self.request.requested()
    }
    /// Effective scalar selected by deterministic predecessor States.
    pub fn effective_value(&self) -> &ConfigurationValue {
        &self.effective
    }
    /// Scalar proved equal to both effective configuration and retained observed evidence.
    pub fn observed_value(&self) -> &ConfigurationValue {
        &self.effective
    }
    /// Deployment settlement and its native original.
    pub fn deployment(&self) -> &TransactionEvidence<ContractLocator> {
        &self.deployment
    }
    /// Configuration settlement and its native original.
    pub fn configuration(&self) -> &TransactionEvidence<ConfigurationApplied> {
        &self.configuration
    }
    /// Exact observation and its native original.
    pub fn observation(&self) -> &ContractValueEvidence {
        &self.observation
    }
}
impl<'de> Deserialize<'de> for ContractDeploymentReport {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: DeploymentRequest,
            effective: ConfigurationValue,
            deployment: TransactionEvidence<ContractLocator>,
            configuration: TransactionEvidence<ConfigurationApplied>,
            observation: ContractValueEvidence,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.request,
            wire.effective,
            wire.deployment,
            wire.configuration,
            wire.observation,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Flattens a validated lifecycle without duplicating the context or re-encoding native originals.
pub struct Report;
impl State for Report {
    type Input = ValidatedConfiguration;
    type Output = ContractDeploymentReport;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.report@1")?)
    }
}
impl PureState for Report {
    fn evaluate(
        input: ValidatedConfiguration,
    ) -> Result<ProposedStateOutcome<ContractDeploymentReport, Never>, InvocationDiagnostic> {
        let (configured, observation) = input.into_observed().into_parts();
        let (deployed, configuration) = configured.into_parts();
        let (request, effective, deployment) = deployed.into_parts();
        Ok(ProposedStateOutcome::Success {
            output: ContractDeploymentReport {
                request,
                effective,
                deployment,
                configuration,
                observation,
            },
        })
    }
}
