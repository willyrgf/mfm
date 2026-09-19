use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{ContentRef, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, Object, Unsigned256};
use serde::{Deserialize, Serialize};

use crate::{BalanceTarget, ObservationPoint};

/// A balance target and observation point must belong to the same ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("balance observation ledger mismatch")]
pub struct ReadBalanceAtError;

/// Balance observation at one exact requested point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "read-balance-at",
    version = "2",
    schema = "mfm.chain-read-balance-at"
)]
pub struct ReadBalanceAt {
    route_ref: ContentRef,
    target: BalanceTarget,
    observed_at: ObservationPoint,
}
impl ReadBalanceAt {
    /// Checks common ledger agreement; native protocol qualification belongs to its owner.
    pub fn new(
        target: BalanceTarget,
        observed_at: ObservationPoint,
        route_ref: ContentRef,
    ) -> Result<Self, ReadBalanceAtError> {
        if target.ledger() != observed_at.ledger() {
            return Err(ReadBalanceAtError);
        }
        Ok(Self {
            route_ref,
            target,
            observed_at,
        })
    }
    /// Exact public route selected by the caller's collection.
    pub fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }
    /// Exact native balance target envelope.
    pub fn target(&self) -> &BalanceTarget {
        &self.target
    }
    /// Exact requested observation point.
    pub fn observed_at(&self) -> &ObservationPoint {
        &self.observed_at
    }
}
impl<'de> Deserialize<'de> for ReadBalanceAt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            route_ref: ContentRef,
            target: BalanceTarget,
            observed_at: ObservationPoint,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.target, wire.observed_at, wire.route_ref).map_err(serde::de::Error::custom)
    }
}

/// Native-qualified balance observation or authenticated unsuccessful outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "balance-outcome",
    version = "1",
    schema = "mfm.chain-balance-outcome"
)]
pub enum BalanceOutcome {
    /// Complete unsigned-256 amount observed at the authenticated point.
    Observed {
        /// Exact authenticated observation point.
        observed_at: ObservationPoint,
        /// Raw units before collection scaling.
        raw_units: Unsigned256,
    },
    /// Authenticated rejection under the selected native protocol.
    Rejected,
    /// Authenticated absence under the selected native protocol.
    SafeFailure,
    /// Authenticated evidence conflicts with the requested integrity constraints.
    IntegrityBlocked,
}

/// Semantic view retaining exact intent, selected implementation and native original provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "balance-evidence",
    version = "1",
    schema = "mfm.chain-balance-evidence"
)]
pub struct BalanceEvidence {
    intent_ref: ContentRef,
    implementation_ref: ContentRef,
    original: Object,
    outcome: BalanceOutcome,
}
impl BalanceEvidence {
    /// Retains a native projection; capability admission checks its request and original binding.
    pub fn new(
        intent_ref: ContentRef,
        implementation_ref: ContentRef,
        original: Object,
        outcome: BalanceOutcome,
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
    /// Exact selected native implementation ABI.
    pub fn implementation_ref(&self) -> &ContentRef {
        &self.implementation_ref
    }
    /// Immutable native evidence, retained without re-encoding.
    pub fn original(&self) -> &Object {
        &self.original
    }
    /// Authenticated semantic outcome; unsuccessful evidence supplies no invented amount.
    pub fn outcome(&self) -> &BalanceOutcome {
        &self.outcome
    }
}

/// Shared anchored balance Read, independent of native assets and provider protocols.
pub struct BalanceRead;
impl ReadCapabilityContract for BalanceRead {
    type Intent = ReadBalanceAt;
    type Evidence = BalanceEvidence;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.chain.balance-read@1")?)
    }
    fn bind_evidence(
        intent_ref: &ContentRef,
        intent: &ReadBalanceAt,
        native_evidence_ref: &ContentRef,
        evidence: &BalanceEvidence,
    ) -> Result<(), InvocationDiagnostic> {
        let point_matches = match evidence.outcome() {
            BalanceOutcome::Observed { observed_at, .. } => observed_at == intent.observed_at(),
            BalanceOutcome::Rejected
            | BalanceOutcome::SafeFailure
            | BalanceOutcome::IntegrityBlocked => true,
        };
        if evidence.intent_ref() != intent_ref
            || evidence.original().value_ref() != native_evidence_ref
            || !point_matches
        {
            return Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "bind_balance_evidence",
                &CapabilityError::EvidenceBinding,
                None,
            ));
        }
        Ok(())
    }
}
