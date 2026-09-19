use super::{
    consistency, ConfiguredContract, ContractRead, ContractValueEvidence, LifecycleError,
    ReadContractValue,
};
use mfm_ids::StableId;
use mfm_program::{
    Classification, ClassifyError, ProposedStateOutcome, PureState, ReadSelection, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};

/// Configured context and an observation bound to its exact target and settlement point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "observed-configuration",
    version = "1",
    schema = "mfm.chain-observed-configuration"
)]
pub struct ObservedConfiguration {
    configured: ConfiguredContract,
    observation: ContractValueEvidence,
}
impl ObservedConfiguration {
    /// Checks semantic target and anchor binding while allowing a value mismatch for Validate.
    pub fn new(
        configured: ConfiguredContract,
        observation: ContractValueEvidence,
    ) -> Result<Self, LifecycleError> {
        let deployed = configured.configured();
        let target = consistency::deployment(deployed.request(), deployed.deployment())?;
        consistency::observation(
            deployed.request().execution().observation_route_ref(),
            target,
            configured.configuration(),
            &observation,
        )?;
        Ok(Self {
            configured,
            observation,
        })
    }
    /// Retained complete configuration context.
    pub fn configured(&self) -> &ConfiguredContract {
        &self.configured
    }
    /// Exact semantic observation and its native original.
    pub fn observation(&self) -> &ContractValueEvidence {
        &self.observation
    }
    pub(super) fn into_parts(self) -> (ConfiguredContract, ContractValueEvidence) {
        (self.configured, self.observation)
    }
}
impl<'de> Deserialize<'de> for ObservedConfiguration {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            configured: ConfiguredContract,
            observation: ContractValueEvidence,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.configured, wire.observation).map_err(serde::de::Error::custom)
    }
}

/// Observed scalar equality checked against the authoritative effective configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "validated-configuration",
    version = "1",
    schema = "mfm.chain-validated-configuration"
)]
pub struct ValidatedConfiguration {
    observed: ObservedConfiguration,
}
impl ValidatedConfiguration {
    /// Requires exact semantic scalar equality; native integrity is established earlier.
    pub fn new(observed: ObservedConfiguration) -> Result<Self, LifecycleError> {
        consistency::value(
            observed.configured().configured().effective(),
            observed.observation(),
        )?;
        Ok(Self { observed })
    }
    /// Complete observation context whose scalar agrees with effective configuration.
    pub fn observed(&self) -> &ObservedConfiguration {
        &self.observed
    }
    pub(super) fn into_observed(self) -> ObservedConfiguration {
        self.observed
    }
}
impl<'de> Deserialize<'de> for ValidatedConfiguration {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            observed: ObservedConfiguration,
        }
        Self::new(Wire::deserialize(deserializer)?.observed).map_err(serde::de::Error::custom)
    }
}

/// Observed configuration differs from its expected effective scalar; the State call retains both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "configuration-mismatch",
    version = "1",
    schema = "mfm.chain-configuration-mismatch"
)]
pub struct ConfigurationMismatch {}
impl ClassifyError for ConfigurationMismatch {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

/// Permanent native observation failure; the retained State call owns exact native evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "observation-failure",
    version = "1",
    schema = "mfm.chain-observation-failure"
)]
pub enum ObservationFailure {
    /// Authenticated native rejection.
    #[error("contract observation rejected")]
    Rejected,
    /// Authenticated absence of the requested anchored observation.
    #[error("anchored contract observation unavailable")]
    SafeFailure,
    /// Authenticated native evidence conflicts with integrity constraints.
    #[error("contract observation integrity blocked")]
    IntegrityBlocked,
}
impl ClassifyError for ObservationFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

/// Requests the deployed contract at the configuration settlement's exact observation point.
pub struct Observe;
impl State for Observe {
    type Input = ConfiguredContract;
    type Output = ObservedConfiguration;
    type Failure = ObservationFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.observe@1")?)
    }
}
impl ReadSelection<ContractRead> for Observe {
    type ExpandedInput = ConfiguredContract;
    type ExpandedOutput = ObservedConfiguration;
}
impl ReadState<ContractRead> for Observe {
    fn prepare(input: &ConfiguredContract) -> Result<ReadContractValue, InvocationDiagnostic> {
        ReadContractValue::new(
            input
                .configured()
                .request()
                .execution()
                .observation_route_ref()
                .clone(),
            input.configured().contract()?.clone(),
            input.configuration().observed_at().clone(),
        )
        .map_err(|error| {
            InvocationDiagnostic::from_fields("state_internal", "observe_prepare", &error, None)
        })
    }
    fn interpret(
        input: ConfiguredContract,
        evidence: &ContractValueEvidence,
    ) -> Result<ProposedStateOutcome<ObservedConfiguration, ObservationFailure>, InvocationDiagnostic>
    {
        match ObservedConfiguration::new(input, evidence.clone()) {
            Ok(output) => Ok(ProposedStateOutcome::Success { output }),
            Err(LifecycleError::ObservationFailure(failure)) => {
                Ok(ProposedStateOutcome::Failure { failure })
            }
            Err(error) => Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "observe_interpret",
                &error,
                None,
            )),
        }
    }
}

/// Compares the observed scalar with retained effective configuration.
pub struct Validate;
impl State for Validate {
    type Input = ObservedConfiguration;
    type Output = ValidatedConfiguration;
    type Failure = ConfigurationMismatch;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.validate@1")?)
    }
}
impl PureState for Validate {
    fn evaluate(
        input: ObservedConfiguration,
    ) -> Result<
        ProposedStateOutcome<ValidatedConfiguration, ConfigurationMismatch>,
        InvocationDiagnostic,
    > {
        Ok(match ValidatedConfiguration::new(input) {
            Ok(output) => ProposedStateOutcome::Success { output },
            Err(LifecycleError::ConfigurationMismatch) => ProposedStateOutcome::Failure {
                failure: ConfigurationMismatch {},
            },
            Err(error) => {
                return Err(InvocationDiagnostic::from_fields(
                    "state_internal",
                    "validate",
                    &error,
                    None,
                ))
            }
        })
    }
}
