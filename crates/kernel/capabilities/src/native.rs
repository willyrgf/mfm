//! Selected native protocols and their explicitly bound IO adapters.

use crate::{
    AdapterError, EffectAdapterOutcome, EffectCapabilityContract, ReadCapabilityContract, Result,
};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_values::{MfmValue, Object};
use std::{future::Future, pin::Pin};

/// Native translation and evidence projection for a semantic Effect.
pub trait EffectImplementation<C: EffectCapabilityContract>: Send + Sync + 'static {
    /// Checked public binding retained in the Program.
    type Binding: MfmValue;
    /// Exact native command supplied to the adapter.
    type NativeCommand: MfmValue;
    /// Authoritative native settlement.
    type NativeEvidence: MfmValue;
    /// Original operational failure owned by this implementation.
    type OperationalError: MfmValue;
    /// Stable installed-code selector; cold admission additionally checks the complete ABI.
    fn implementation_id() -> Result<StableId>;
    /// Extracts an already-qualified native command and validates request/binding agreement.
    fn decode_command(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        command_ref: &ContentRef,
        command: &C::Command,
    ) -> std::result::Result<(ContentRef, Self::NativeCommand), crate::CallbackFailure>;
    /// Checks native settlement and projects a semantic view retaining the exact original Object.
    #[allow(clippy::too_many_arguments)]
    fn project_evidence(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        effect_id: &EffectId,
        command_ref: &ContentRef,
        command: &C::Command,
        native_command: &Self::NativeCommand,
        native_evidence: &Self::NativeEvidence,
        original: &Object,
    ) -> std::result::Result<C::Evidence, crate::CallbackFailure>;
}

/// Native translation and evidence projection for a semantic Read.
pub trait ReadImplementation<C: ReadCapabilityContract>: Send + Sync + 'static {
    /// Checked public binding retained in the Program.
    type Binding: MfmValue;
    /// Exact native intent supplied to the adapter.
    type NativeIntent: MfmValue;
    /// Authoritative native observation.
    type NativeEvidence: MfmValue;
    /// Original operational failure owned by this implementation.
    type OperationalError: MfmValue;
    /// Stable installed-code selector; cold admission additionally checks the complete ABI.
    fn implementation_id() -> Result<StableId>;
    /// Translates a retained semantic intent without IO or configuration lookup.
    fn encode_intent(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        intent: &C::Intent,
    ) -> std::result::Result<Self::NativeIntent, crate::CallbackFailure>;
    /// Checks native observation and projects a semantic view retaining the exact original Object.
    #[allow(clippy::too_many_arguments)]
    fn project_evidence(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &Self::Binding,
        intent_ref: &ContentRef,
        intent: &C::Intent,
        native_intent_ref: &ContentRef,
        native_intent: &Self::NativeIntent,
        native_evidence: &Self::NativeEvidence,
        original: &Object,
    ) -> std::result::Result<C::Evidence, crate::CallbackFailure>;
}

/// Explicit async observation capability; binding happens before execution.
pub trait ReadAdapter<Intent, Evidence, Failure>: Send + Sync + 'static {
    /// Invokes a duplicate-safe observation with distinct semantic and native identities.
    fn invoke<'a>(
        &'a self,
        semantic_intent_ref: &'a ContentRef,
        native_intent_ref: &'a ContentRef,
        intent: &'a Intent,
    ) -> Pin<
        Box<dyn Future<Output = std::result::Result<Evidence, AdapterError<Failure>>> + Send + 'a>,
    >;
}

/// Explicit async Effect capability; Runtime retains acknowledgement and retry authority.
pub trait EffectAdapter<Command, Evidence, Failure>: Send + Sync + 'static {
    /// Invokes the retained Effect using the exact selected native command.
    #[allow(clippy::type_complexity)]
    fn invoke<'a>(
        &'a self,
        effect_id: &'a EffectId,
        semantic_command_ref: &'a ContentRef,
        native_command_ref: &'a ContentRef,
        command: &'a Command,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = std::result::Result<
                        EffectAdapterOutcome<Evidence>,
                        AdapterError<Failure>,
                    >,
                > + Send
                + 'a,
        >,
    >;
}
