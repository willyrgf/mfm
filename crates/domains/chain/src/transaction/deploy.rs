use mfm_ids::StableId;
use mfm_program::{
    Classification, ClassifyError, EffectSelection, EffectState, ProposedStateOutcome, PureState,
    State,
};
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};

use super::{
    ConfigurationValue, DeploymentRequest, PreparedTransaction, TransactionEffect,
    TransactionEvidence, TransactionResult,
};
use crate::ContractLocator;

/// Authoritative deployment context, retaining its request and authenticated applied evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "deployed-contract",
    version = "1",
    schema = "mfm.chain-deployed-contract"
)]
pub struct DeployedContract {
    request: DeploymentRequest,
    effective: ConfigurationValue,
    deployment: TransactionEvidence<ContractLocator>,
}

/// Adds the admitted increment to effective configuration while preserving deployment context.
pub struct CheckedAddConfigurationValue;
impl State for CheckedAddConfigurationValue {
    type Input = DeployedContract;
    type Output = DeployedContract;
    type Failure = crate::AdditionOverflow;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new(
            "mfm.chain.checked-add-configuration-value@1",
        )?)
    }
}
impl PureState for CheckedAddConfigurationValue {
    fn evaluate(
        mut input: DeployedContract,
    ) -> Result<ProposedStateOutcome<DeployedContract, crate::AdditionOverflow>, InvocationDiagnostic>
    {
        Ok(
            match input.effective.checked_add(input.request.increment()) {
                Some(value) => {
                    input.effective = value;
                    ProposedStateOutcome::Success { output: input }
                }
                None => ProposedStateOutcome::Failure {
                    failure: crate::AdditionOverflow {},
                },
            },
        )
    }
}

impl DeployedContract {
    /// Checks that settlement establishes a deployment; native integrity is checked before this step.
    pub fn new(
        request: DeploymentRequest,
        effective: ConfigurationValue,
        deployment: TransactionEvidence<ContractLocator>,
    ) -> Result<Self, super::LifecycleError> {
        super::consistency::deployment(&request, &deployment)?;
        Ok(Self {
            request,
            effective,
            deployment,
        })
    }
    pub(super) fn into_parts(
        self,
    ) -> (
        DeploymentRequest,
        ConfigurationValue,
        TransactionEvidence<ContractLocator>,
    ) {
        (self.request, self.effective, self.deployment)
    }
    /// Original complete request, including the requested value and admitted increment.
    pub fn request(&self) -> &DeploymentRequest {
        &self.request
    }
    /// Current authoritative configuration value.
    pub fn effective(&self) -> &ConfigurationValue {
        &self.effective
    }
    /// Deployment settlement and its exact native original.
    pub fn deployment(&self) -> &TransactionEvidence<ContractLocator> {
        &self.deployment
    }
    /// Projects the checked deployed locator without native decoding or panicking.
    pub fn contract(&self) -> Result<&ContractLocator, InvocationDiagnostic> {
        match self.deployment.result() {
            TransactionResult::Applied { output } => Ok(output),
            TransactionResult::Rejected { .. } => Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "deployment_contract",
                &super::LifecycleError::DeploymentRejected,
                None,
            )),
        }
    }
}

impl<'de> Deserialize<'de> for DeployedContract {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            request: DeploymentRequest,
            effective: ConfigurationValue,
            deployment: TransactionEvidence<ContractLocator>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.request, wire.effective, wire.deployment).map_err(serde::de::Error::custom)
    }
}

/// Authenticated deployment rejection; the State call retains its complete settlement and request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "deployment-failure",
    version = "1",
    schema = "mfm.chain-deployment-failure"
)]
pub struct DeploymentFailure {}
impl ClassifyError for DeploymentFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

/// Concrete semantic deployment State, independent of native preparation and execution.
pub struct Deploy;
impl State for Deploy {
    type Input = PreparedTransaction<DeploymentRequest>;
    type Output = DeployedContract;
    type Failure = DeploymentFailure;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.deploy@1")?)
    }
}
impl EffectSelection<TransactionEffect<DeploymentRequest>> for Deploy {
    type ExpandedInput = DeploymentRequest;
    type ExpandedOutput = DeployedContract;
}
impl EffectState<TransactionEffect<DeploymentRequest>> for Deploy {
    fn prepare(input: &Self::Input) -> Result<Self::Input, InvocationDiagnostic> {
        Ok(input.clone())
    }
    fn interpret(
        input: Self::Input,
        evidence: &TransactionEvidence<ContractLocator>,
    ) -> Result<ProposedStateOutcome<DeployedContract, DeploymentFailure>, InvocationDiagnostic>
    {
        Ok(match evidence.result() {
            TransactionResult::Applied { .. } => {
                let request = input.into_request();
                let effective = request.requested().clone();
                let output = DeployedContract::new(request, effective, evidence.clone()).map_err(
                    |error| {
                        InvocationDiagnostic::from_fields("state_internal", "deploy", &error, None)
                    },
                )?;
                ProposedStateOutcome::Success { output }
            }
            TransactionResult::Rejected { .. } => ProposedStateOutcome::Failure {
                failure: DeploymentFailure {},
            },
        })
    }
}
