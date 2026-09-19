#![warn(missing_docs)]
//! Closed contracts for observational Read and mutating Effect capabilities.
//!
//! Read evidence binds to both the typed intent and its exact qualified value reference. Effect
//! evidence binds to Runtime's Effect identity and typed command.
//!
//! See the [capability authoring guide](https://github.com/willyrgf/mfm/blob/main/docs/capability-authoring.md)
//! for consumer, State and native implementation responsibilities.

use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_values::{InvocationDiagnostic, MfmValue};

/// Phase-preserving synchronous native codecs.
pub mod codec;

/// Invocation-only failure phase; Runtime supplies the originating execution operation.
#[derive(Debug, thiserror::Error)]
pub enum CallbackFailure {
    /// Exact native materialization failed before invocation.
    #[error("callback decoding failed")]
    Decode(InvocationDiagnostic),
    /// Callback construction, polling or a local invariant failed.
    #[error("callback execution failed")]
    Execute(InvocationDiagnostic),
    /// Canonical encoding failed, including a panicking encoding job.
    #[error("callback encoding failed")]
    Encode(InvocationDiagnostic),
}

mod native;
pub use native::*;

impl CallbackFailure {
    /// Retains a nested codec phase when crossing a construction or ordinary State diagnostic boundary.
    pub fn into_diagnostic(self) -> InvocationDiagnostic {
        let (phase, cause) = match self {
            Self::Execute(cause) => return cause,
            Self::Decode(cause) => ("decode", cause),
            Self::Encode(cause) => ("encode", cause),
        };
        InvocationDiagnostic::from_fields("native_codec", phase, &cause, cause.size())
    }
}

impl From<InvocationDiagnostic> for CallbackFailure {
    fn from(cause: InvocationDiagnostic) -> Self {
        Self::Execute(cause)
    }
}

/// Result type for capability contract operations.
pub type Result<T> = std::result::Result<T, CapabilityError>;

/// Result of one successful Effect adapter invocation.
///
/// Nonterminal progress is represented without fabricating evidence or
/// concluding the durable Effect prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectAdapterOutcome<E> {
    /// The Effect remains pending and can be resumed by a later caller.
    Pending,
    /// The Effect produced terminal evidence that Runtime must bind and interpret.
    Settled(E),
}

/// A capability-owned operational cause or an unrecoverable local invariant failure.
///
/// Typed payloads require no `Error` bound. When the operational type implements `Error`,
/// standard source traversal exposes it without invoking its formatting implementation.
pub enum AdapterError<E> {
    /// Reviewed typed operational cause, retained before recovery policy.
    Operational(E),
    /// Trusted local failure, outside classifier and handler recovery.
    Invariant(mfm_values::InvocationDiagnostic),
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

impl<E: std::error::Error + 'static> std::error::Error for AdapterError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operational(error) => Some(error),
            Self::Invariant(_) => None,
        }
    }
}

/// Redaction-safe capability contract error.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityError {
    /// Invalid checked identity, with its original grammar rejection.
    #[error("capability identity is invalid")]
    Identity(#[from] mfm_ids::CheckedStringError),
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
        native_evidence_ref: &ContentRef,
        evidence: &Self::Evidence,
    ) -> std::result::Result<(), mfm_values::InvocationDiagnostic>;
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
        command_ref: &ContentRef,
        command: &Self::Command,
        native_evidence_ref: &ContentRef,
        evidence: &Self::Evidence,
    ) -> std::result::Result<(), mfm_values::InvocationDiagnostic>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_error_exposes_the_original_nested_source_when_supported() {
        #[derive(Debug, thiserror::Error)]
        #[error("reviewed owner failure")]
        struct Owner(#[source] std::io::Error);
        let error = AdapterError::Operational(Owner(std::io::Error::from_raw_os_error(104)));
        let owner = std::error::Error::source(&error).unwrap();
        assert!(owner.downcast_ref::<Owner>().is_some());
        assert_eq!(
            owner
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(104)
        );
    }

    #[test]
    fn adapter_error_formatting_never_formats_the_operational_cause() {
        struct Unformattable;
        let operational = AdapterError::Operational(Unformattable);
        assert_eq!(format!("{operational:?}"), "Operational(<redacted>)");
        assert_eq!(operational.to_string(), "adapter operational failure");
        let invariant: AdapterError<Unformattable> =
            AdapterError::Invariant(mfm_values::InvocationDiagnostic::from_fields(
                "state_internal",
                "bind_evidence",
                &CapabilityError::EvidenceBinding,
                None,
            ));
        assert_eq!(invariant.to_string(), "adapter invariant failed");
    }
}
