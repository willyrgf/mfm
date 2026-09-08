#![warn(missing_docs)]
//! Closed contracts for observational Read and mutating Effect capabilities.
//!
//! Read evidence binds to both the typed intent and its exact qualified value reference. Effect
//! evidence binds to Runtime's Effect identity and typed command.

use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_values::MfmValue;

/// Result type for capability contract operations.
pub type Result<T> = std::result::Result<T, CapabilityError>;

/// A trusted adapter invariant failed; this carries no provider or secret detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("adapter invariant failed")]
pub struct AdapterInvariantError;

/// A capability-owned operational cause or an unrecoverable local invariant failure.
#[derive(PartialEq, Eq)]
pub enum AdapterError<E> {
    /// Reviewed typed operational cause, eligible for State contextualization.
    Operational(E),
    /// Trusted local failure, outside classifier and handler recovery.
    Invariant(AdapterInvariantError),
}

impl<E> std::fmt::Debug for AdapterError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operational(_) => formatter.write_str("Operational(<redacted>)"),
            Self::Invariant(error) => std::fmt::Debug::fmt(error, formatter),
        }
    }
}

impl<E> std::fmt::Display for AdapterError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Operational(_) => "adapter operational failure",
            Self::Invariant(_) => "adapter invariant failed",
        })
    }
}

impl<E> std::error::Error for AdapterError<E> {}

/// Redaction-safe capability contract error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    /// A static capability identity is invalid.
    #[error("capability contract is invalid")]
    InvalidContract,
    /// Evidence does not bind to the exact intent.
    #[error("capability evidence does not bind to intent")]
    EvidenceBinding,
}

/// One observational capability with a closed intent/evidence contract.
pub trait ReadCapabilityContract: Send + Sync + 'static {
    /// Reviewed bounded operational failure returned by the adapter.
    type OperationalError: MfmValue;
    /// Canonical intent passed to the trusted adapter.
    type Intent: MfmValue;
    /// Closed evidence returned by the trusted adapter.
    type Evidence: MfmValue;

    /// Returns the stable capability contract identity.
    fn contract_id() -> Result<StableId>;

    /// Proves that evidence answers the exact qualified intent value.
    fn bind_evidence(
        intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> Result<()>;
}

/// One mutating capability with a closed command/evidence contract.
pub trait EffectCapabilityContract: Send + Sync + 'static {
    /// Reviewed bounded operational failure returned by the adapter.
    type OperationalError: MfmValue;
    /// Complete nonce-free command passed to the trusted adapter.
    type Command: MfmValue;
    /// Closed evidence returned by the trusted adapter.
    type Evidence: MfmValue;

    /// Returns the stable capability contract identity.
    fn contract_id() -> Result<StableId>;

    /// Proves that evidence settles the exact Effect and command.
    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_error_formatting_never_formats_the_operational_cause() {
        struct Unformattable;
        let operational = AdapterError::Operational(Unformattable);
        assert_eq!(format!("{operational:?}"), "Operational(<redacted>)");
        assert_eq!(operational.to_string(), "adapter operational failure");
        let invariant: AdapterError<Unformattable> = AdapterError::Invariant(AdapterInvariantError);
        assert_eq!(invariant.to_string(), "adapter invariant failed");
    }
}
