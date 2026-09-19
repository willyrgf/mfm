use mfm_values::{Object, ValueError};
use serde::Serialize;

use super::{
    ConfigurationApplied, ConfigurationValue, ContractValueEvidence, ContractValueOutcome,
    DeploymentRequest, ObservationFailure, ReadContractValue, ReadContractValueError,
    TransactionEvidence, TransactionResult,
};
use crate::ContractLocator;

/// Rejection of internally inconsistent shared lifecycle facts; native authenticity is separate.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleError {
    /// Deployment did not establish an applied locator.
    #[error("deployment requires applied transaction evidence")]
    DeploymentRejected,
    /// Configuration did not establish application.
    #[error("configuration requires applied transaction evidence")]
    ConfigurationRejected,
    /// Native evidence did not establish an observed scalar.
    #[error("contract observation failed")]
    ObservationFailure(#[from] ObservationFailure),
    /// Retained facts refer to different ledgers.
    #[error("lifecycle ledger mismatch")]
    LedgerMismatch,
    /// Observation used a different point than configuration settlement.
    #[error("configuration observation point mismatch")]
    ObservationPoint,
    /// Observation refers to another semantic target or anchor.
    #[error("configuration observation intent mismatch")]
    ObservationIntent,
    /// Observed configuration differs from the effective value.
    #[error("observed configuration mismatch")]
    ConfigurationMismatch,
    /// Checked semantic intent construction failed.
    #[error("configuration observation intent is invalid")]
    Intent(#[from] ReadContractValueError),
    /// Computing the semantic intent's exact identity failed.
    #[error("configuration observation identity failed")]
    Identity(#[from] ValueError),
}

pub(super) fn deployment<'a>(
    request: &DeploymentRequest,
    evidence: &'a TransactionEvidence<ContractLocator>,
) -> Result<&'a ContractLocator, LifecycleError> {
    let TransactionResult::Applied { output } = evidence.result() else {
        return Err(LifecycleError::DeploymentRejected);
    };
    if output.ledger() != request.artifact().ledger()
        || output.ledger() != evidence.transaction().ledger()
    {
        return Err(LifecycleError::LedgerMismatch);
    }
    Ok(output)
}

pub(super) fn configuration(
    target: &ContractLocator,
    evidence: &TransactionEvidence<ConfigurationApplied>,
) -> Result<(), LifecycleError> {
    if !matches!(evidence.result(), TransactionResult::Applied { .. }) {
        return Err(LifecycleError::ConfigurationRejected);
    }
    if target.ledger() != evidence.transaction().ledger() {
        return Err(LifecycleError::LedgerMismatch);
    }
    Ok(())
}

pub(super) fn observation(
    route_ref: &mfm_ids::ContentRef,
    target: &ContractLocator,
    configuration: &TransactionEvidence<ConfigurationApplied>,
    evidence: &ContractValueEvidence,
) -> Result<(), LifecycleError> {
    let intent = ReadContractValue::new(
        route_ref.clone(),
        target.clone(),
        configuration.observed_at().clone(),
    )?;
    if Object::from_value(&intent)?.value_ref() != evidence.intent_ref() {
        return Err(LifecycleError::ObservationIntent);
    }
    if observed(evidence)?.0 != configuration.observed_at() {
        return Err(LifecycleError::ObservationPoint);
    }
    Ok(())
}

pub(super) fn value(
    effective: &ConfigurationValue,
    observation: &ContractValueEvidence,
) -> Result<(), LifecycleError> {
    if effective != observed(observation)?.1 {
        return Err(LifecycleError::ConfigurationMismatch);
    }
    Ok(())
}

fn observed(
    evidence: &ContractValueEvidence,
) -> Result<(&crate::ObservationPoint, &ConfigurationValue), ObservationFailure> {
    match evidence.outcome() {
        ContractValueOutcome::Observed { observed_at, value } => Ok((observed_at, value)),
        ContractValueOutcome::Rejected => Err(ObservationFailure::Rejected),
        ContractValueOutcome::SafeFailure => Err(ObservationFailure::SafeFailure),
        ContractValueOutcome::IntegrityBlocked => Err(ObservationFailure::IntegrityBlocked),
    }
}
