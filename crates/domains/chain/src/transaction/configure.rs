use mfm_ids::StableId;
use mfm_program::{
    Classification, ClassifyError, EffectSelection, EffectState, ProposedStateOutcome, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};

use super::{
    DeployedContract, PreparedTransaction, TransactionEffect, TransactionEvidence,
    TransactionRequest, TransactionResult,
};

/// Authenticated configuration application needs no additional domain result fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.chain",
    name = "configuration-applied",
    version = "1",
    schema = "mfm.chain-configuration-applied"
)]
pub struct ConfigurationApplied;

impl TransactionRequest for DeployedContract {
    type Applied = ConfigurationApplied;
}

/// Retained deployment context and authenticated configuration settlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "configured-contract",
    version = "1",
    schema = "mfm.chain-configured-contract"
)]
pub struct ConfiguredContract {
    configured: DeployedContract,
    configuration: TransactionEvidence<ConfigurationApplied>,
}
impl ConfiguredContract {
    /// Requires applied settlement; native integrity is established before semantic interpretation.
    pub fn new(
        configured: DeployedContract,
        configuration: TransactionEvidence<ConfigurationApplied>,
    ) -> Result<Self, super::LifecycleError> {
        let target = super::consistency::deployment(configured.request(), configured.deployment())?;
        super::consistency::configuration(target, &configuration)?;
        Ok(Self {
            configured,
            configuration,
        })
    }
    pub(super) fn into_parts(
        self,
    ) -> (DeployedContract, TransactionEvidence<ConfigurationApplied>) {
        (self.configured, self.configuration)
    }
    /// Authoritative configured request, effective value and original deployment settlement.
    pub fn configured(&self) -> &DeployedContract {
        &self.configured
    }
    /// Configuration settlement and its exact observation point and native original.
    pub fn configuration(&self) -> &TransactionEvidence<ConfigurationApplied> {
        &self.configuration
    }
}
impl<'de> Deserialize<'de> for ConfiguredContract {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            configured: DeployedContract,
            configuration: TransactionEvidence<ConfigurationApplied>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.configured, wire.configuration).map_err(serde::de::Error::custom)
    }
}

/// Authenticated configuration rejection; the State call retains its complete settlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "configuration-failure",
    version = "1",
    schema = "mfm.chain-configuration-failure"
)]
pub struct ConfigurationFailure {}
impl ClassifyError for ConfigurationFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

/// Concrete semantic configuration State, consuming the authoritative deployed context.
pub struct Configure;
impl State for Configure {
    type Input = PreparedTransaction<DeployedContract>;
    type Output = ConfiguredContract;
    type Failure = ConfigurationFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.configure@1")?)
    }
}
impl EffectSelection<TransactionEffect<DeployedContract>> for Configure {
    type ExpandedInput = DeployedContract;
    type ExpandedOutput = ConfiguredContract;
}
impl EffectState<TransactionEffect<DeployedContract>> for Configure {
    fn prepare(input: &Self::Input) -> Result<Self::Input, InvocationDiagnostic> {
        Ok(input.clone())
    }
    fn interpret(
        input: Self::Input,
        evidence: &TransactionEvidence<ConfigurationApplied>,
    ) -> Result<ProposedStateOutcome<ConfiguredContract, ConfigurationFailure>, InvocationDiagnostic>
    {
        Ok(match evidence.result() {
            TransactionResult::Applied { .. } => {
                let output = ConfiguredContract::new(input.into_request(), evidence.clone())
                    .map_err(|error| {
                        InvocationDiagnostic::from_fields(
                            "state_internal",
                            "configure",
                            &error,
                            None,
                        )
                    })?;
                ProposedStateOutcome::Success { output }
            }
            TransactionResult::Rejected { .. } => ProposedStateOutcome::Failure {
                failure: ConfigurationFailure {},
            },
        })
    }
}
