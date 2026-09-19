use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{ContentRef, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, Object};
use serde::{Deserialize, Serialize};

use super::ConfigurationValue;
use crate::{ContractLocator, ObservationPoint};

/// A contract and observation point cannot belong to different ledgers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("contract read ledger mismatch")]
pub struct ReadContractValueError;

/// Read one deployed contract at an exact observation point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "read-contract-value",
    version = "2",
    schema = "mfm.chain-read-contract-value"
)]
pub struct ReadContractValue {
    route_ref: ContentRef,
    target: ContractLocator,
    at: ObservationPoint,
}
impl ReadContractValue {
    /// Checks common ledger agreement, leaving native protocol qualification to its owner.
    pub fn new(
        route_ref: ContentRef,
        target: ContractLocator,
        at: ObservationPoint,
    ) -> Result<Self, ReadContractValueError> {
        if target.ledger() != at.ledger() {
            return Err(ReadContractValueError);
        }
        Ok(Self {
            route_ref,
            target,
            at,
        })
    }
    /// Caller-selected route whose binding the native owner must qualify before IO.
    pub fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }
    /// Exact deployed contract.
    pub fn target(&self) -> &ContractLocator {
        &self.target
    }
    /// Exact requested observation point.
    pub fn at(&self) -> &ObservationPoint {
        &self.at
    }
}
impl<'de> Deserialize<'de> for ReadContractValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            route_ref: ContentRef,
            target: ContractLocator,
            at: ObservationPoint,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.route_ref, wire.target, wire.at).map_err(serde::de::Error::custom)
    }
}

/// Authenticated scalar observation or native unsuccessful outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "contract-value-outcome",
    version = "1",
    schema = "mfm.chain-contract-value-outcome"
)]
pub enum ContractValueOutcome {
    /// Complete scalar observed at an authenticated point.
    Observed {
        /// Exact authenticated observation point.
        observed_at: ObservationPoint,
        /// Canonical full-width scalar.
        value: ConfigurationValue,
    },
    /// Authenticated native rejection.
    Rejected,
    /// Authenticated absence of the requested anchored observation.
    SafeFailure,
    /// Authenticated evidence conflicts with integrity constraints.
    IntegrityBlocked,
}

/// Native-qualified semantic scalar retaining exact intent and original provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "contract-value-evidence",
    version = "2",
    schema = "mfm.chain-contract-value-evidence"
)]
pub struct ContractValueEvidence {
    intent_ref: ContentRef,
    implementation_ref: ContentRef,
    original: Object,
    outcome: ContractValueOutcome,
}
impl ContractValueEvidence {
    /// Retains the native owner's projection; capability binding checks its request provenance.
    pub fn new(
        intent_ref: ContentRef,
        implementation_ref: ContentRef,
        original: Object,
        outcome: ContractValueOutcome,
    ) -> Self {
        Self {
            intent_ref,
            implementation_ref,
            original,
            outcome,
        }
    }
    /// Exact semantic intent identity.
    pub fn intent_ref(&self) -> &ContentRef {
        &self.intent_ref
    }
    /// Selected native implementation ABI.
    pub fn implementation_ref(&self) -> &ContentRef {
        &self.implementation_ref
    }
    /// Authenticated outcome; unsuccessful observations invent neither point nor scalar.
    pub fn outcome(&self) -> &ContractValueOutcome {
        &self.outcome
    }
    /// Exact native evidence Object, retained without re-encoding.
    pub fn original(&self) -> &Object {
        &self.original
    }
}

/// Semantic anchored contract-value Read, independent of native transport and receipt formats.
pub struct ContractRead;
impl ReadCapabilityContract for ContractRead {
    type Intent = ReadContractValue;
    type Evidence = ContractValueEvidence;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.chain.contract-read@1")?)
    }
    fn bind_evidence(
        intent_ref: &ContentRef,
        intent: &ReadContractValue,
        native_evidence_ref: &ContentRef,
        evidence: &ContractValueEvidence,
    ) -> Result<(), InvocationDiagnostic> {
        let point_matches = match evidence.outcome() {
            ContractValueOutcome::Observed { observed_at, .. } => observed_at == intent.at(),
            ContractValueOutcome::Rejected
            | ContractValueOutcome::SafeFailure
            | ContractValueOutcome::IntegrityBlocked => true,
        };
        if evidence.intent_ref() != intent_ref
            || !point_matches
            || evidence.original().value_ref() != native_evidence_ref
        {
            return Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "bind_contract_value_evidence",
                &CapabilityError::EvidenceBinding,
                None,
            ));
        }
        Ok(())
    }
}
