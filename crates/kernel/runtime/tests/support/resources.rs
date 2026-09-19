//! Scripted external adapters; all execution uses complete Programs and the shipping Runtime.
use super::program::*;
use mfm_capabilities::{
    AdapterError, CallbackFailure, EffectAdapter, EffectAdapterOutcome, EffectCapabilityContract,
    EffectImplementation, ReadAdapter, ReadCapabilityContract, ReadImplementation,
};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_program::{
    BindEffect, BindRead, EffectSelection, Identity, InjectEffect, InjectRead, ProgramEnvironment,
    ReadSelection, ResolvedEffect, ResolvedRead,
};
use mfm_values::{InvocationDiagnostic, Object};
use std::{future::Future, marker::PhantomData, pin::Pin, sync::Arc};

pub(super) trait NativeIdentity: 'static {
    const ID: &'static str;
}
impl NativeIdentity for () {
    const ID: &'static str = "mfm.test.runtime/native@1";
}
pub(super) struct Native<Injection = ()>(PhantomData<fn() -> Injection>);
impl<C: ReadCapabilityContract, I: NativeIdentity> ReadImplementation<C> for Native<I>
where
    C::Intent: Clone,
    C::Evidence: Clone,
{
    type Binding = Binding;
    type NativeIntent = C::Intent;
    type NativeEvidence = C::Evidence;
    type OperationalError = OperationalFailure;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new(I::ID)?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &Binding,
        intent: &C::Intent,
    ) -> Result<C::Intent, CallbackFailure> {
        Ok(intent.clone())
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &Binding,
        _: &ContentRef,
        _: &C::Intent,
        _: &ContentRef,
        _: &C::Intent,
        evidence: &C::Evidence,
        _: &Object,
    ) -> Result<C::Evidence, CallbackFailure> {
        Ok(evidence.clone())
    }
}
impl<S, C> InjectRead<S, C> for Native
where
    C: ReadCapabilityContract,
    C::Intent: Clone,
    C::Evidence: Clone,
    S: ReadSelection<
        C,
        Input = Number,
        Output = Number,
        ExpandedInput = Number,
        ExpandedOutput = Number,
    >,
{
    type Prefix = Identity<Number>;
    type Suffix = Identity<Number>;
    fn surround(_: &Binding) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Default::default(), Default::default()))
    }
}
impl<C: EffectCapabilityContract, I: NativeIdentity> EffectImplementation<C> for Native<I>
where
    C::Command: Clone,
    C::Evidence: Clone,
{
    type Binding = Binding;
    type NativeCommand = C::Command;
    type NativeEvidence = C::Evidence;
    type OperationalError = OperationalFailure;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new(I::ID)?)
    }
    fn decode_command(
        _: &ContentRef,
        _: &ContentRef,
        _: &Binding,
        reference: &ContentRef,
        command: &C::Command,
    ) -> Result<(ContentRef, C::Command), CallbackFailure> {
        Ok((reference.clone(), command.clone()))
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &Binding,
        _: &EffectId,
        _: &ContentRef,
        _: &C::Command,
        _: &C::Command,
        evidence: &C::Evidence,
        _: &Object,
    ) -> Result<C::Evidence, CallbackFailure> {
        Ok(evidence.clone())
    }
}
impl<S, C> InjectEffect<S, C> for Native
where
    C: EffectCapabilityContract,
    C::Command: Clone,
    C::Evidence: Clone,
    S: EffectSelection<
        C,
        Input = Number,
        Output = Number,
        ExpandedInput = Number,
        ExpandedOutput = Number,
    >,
{
    type Prefix = Identity<Number>;
    type Suffix = Identity<Number>;
    fn surround(_: &Binding) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Default::default(), Default::default()))
    }
}
pub(super) type ReadSource = ResolvedRead<Observe, Observation, Native>;
pub(super) type EffectSource = ResolvedEffect<Mutate, Mutation, Native>;
type ReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Evidence, AdapterError<OperationalFailure>>> + Send + 'a>>;
type EffectFuture<'a> = Pin<
    Box<
        dyn Future<
                Output = Result<
                    EffectAdapterOutcome<EffectEvidence>,
                    AdapterError<OperationalFailure>,
                >,
            > + Send
            + 'a,
    >,
>;
type ReadCall = dyn for<'a> Fn(&'a ContentRef, &'a Intent) -> ReadFuture<'a> + Send + Sync;
type EffectCall =
    dyn for<'a> Fn(&'a EffectId, &'a ContentRef, &'a Command) -> EffectFuture<'a> + Send + Sync;
pub(super) struct Resources<S> {
    pub(super) read: Option<Arc<ReadCall>>,
    pub(super) effect: Option<Arc<EffectCall>>,
    source: PhantomData<fn() -> S>,
}
impl<S> Clone for Resources<S> {
    fn clone(&self) -> Self {
        Self {
            read: self.read.clone(),
            effect: self.effect.clone(),
            source: PhantomData,
        }
    }
}
impl<S> Default for Resources<S> {
    fn default() -> Self {
        Self {
            read: None,
            effect: None,
            source: PhantomData,
        }
    }
}
impl<S> Resources<S> {
    pub(super) fn read(
        call: impl for<'a> Fn(&'a ContentRef, &'a Intent) -> ReadFuture<'a> + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Some(Arc::new(call)),
            ..Self::default()
        }
    }
    pub(super) fn effect(
        call: impl for<'a> Fn(&'a EffectId, &'a ContentRef, &'a Command) -> EffectFuture<'a>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            effect: Some(Arc::new(call)),
            ..Self::default()
        }
    }
}
impl<S> ProgramEnvironment for Resources<S> {
    type Sources = S;
}
impl<S: 'static> BindRead<Observation, Native> for Resources<S> {
    type Adapter = Self;
    fn bind_read(&self, binding: &Binding) -> Result<Self, InvocationDiagnostic> {
        if binding.route != 7 || self.read.is_none() {
            return Err(InvocationDiagnostic::from_fields(
                "test_resources",
                "bind_read",
                &AdapterRejected,
                None,
            ));
        }
        Ok(self.clone())
    }
}
impl<S: 'static, I: NativeIdentity> BindEffect<Mutation, Native<I>> for Resources<S> {
    type Adapter = Self;
    fn bind_effect(&self, binding: &Binding) -> Result<Self, InvocationDiagnostic> {
        if binding.route != 8 || self.effect.is_none() {
            return Err(InvocationDiagnostic::from_fields(
                "test_resources",
                "bind_effect",
                &AdapterRejected,
                None,
            ));
        }
        Ok(self.clone())
    }
}
impl<S: 'static> ReadAdapter<Intent, Evidence, OperationalFailure> for Resources<S> {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        reference: &'a ContentRef,
        intent: &'a Intent,
    ) -> ReadFuture<'a> {
        (self.read.as_ref().expect("bound Read"))(reference, intent)
    }
}
impl<S: 'static> EffectAdapter<Command, EffectEvidence, OperationalFailure> for Resources<S> {
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        reference: &'a ContentRef,
        command: &'a Command,
    ) -> EffectFuture<'a> {
        (self.effect.as_ref().expect("bound Effect"))(effect, reference, command)
    }
}
