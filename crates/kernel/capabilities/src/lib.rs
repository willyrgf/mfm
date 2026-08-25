#![warn(missing_docs)]
//! Closed contracts for observational Read and mutating Effect capabilities.
//!
//! Read evidence binds to both the typed intent and its exact qualified value reference. Effect
//! evidence binds to Runtime's Effect identity and typed command.

use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_values::MfmValue;

/// Result type for capability contract operations.
pub type Result<T> = std::result::Result<T, CapabilityError>;

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
