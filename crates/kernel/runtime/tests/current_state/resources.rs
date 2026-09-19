use super::*;
use mfm_capabilities::{
    EffectAdapter, EffectAdapterOutcome, EffectImplementation, ReadAdapter, ReadImplementation,
};
use mfm_program::{
    BindEffect, BindRead, InjectEffect, InjectRead, ProgramEnvironment, ReadSelection,
};
use mfm_values::{MfmValue as Value, Object};
use std::{future::Future, marker::PhantomData, pin::Pin};

pub(super) struct Native<E>(PhantomData<fn() -> E>);
impl<C, E> ReadImplementation<C> for Native<E>
where
    C: ReadCapabilityContract<Intent = Request, Evidence = Request>,
    E: Value,
{
    type Binding = NoParams;
    type NativeIntent = Request;
    type NativeEvidence = Request;
    type OperationalError = E;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.current-native-read@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        intent: &Request,
    ) -> Result<Request, mfm_capabilities::CallbackFailure> {
        Ok(Request {
            value: intent.value,
        })
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &Request,
        _: &ContentRef,
        _: &Request,
        evidence: &Request,
        _: &Object,
    ) -> Result<Request, mfm_capabilities::CallbackFailure> {
        Ok(Request {
            value: evidence.value,
        })
    }
}
impl<S, C, E> InjectRead<S, C> for Native<E>
where
    C: ReadCapabilityContract<Intent = Request, Evidence = Request>,
    E: Value,
    S: ReadSelection<
        C,
        Input = Input,
        Output = Input,
        ExpandedInput = Input,
        ExpandedOutput = Input,
    >,
{
    type Prefix = Identity<Input>;
    type Suffix = Identity<Input>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Default::default(), Default::default()))
    }
}
impl EffectImplementation<Submit> for Native<Never> {
    type Binding = NoParams;
    type NativeCommand = Request;
    type NativeEvidence = Request;
    type OperationalError = Never;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.current-native-effect@1")?)
    }
    fn decode_command(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        reference: &ContentRef,
        command: &Request,
    ) -> Result<(ContentRef, Request), mfm_capabilities::CallbackFailure> {
        Ok((
            reference.clone(),
            Request {
                value: command.value,
            },
        ))
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &mfm_ids::EffectId,
        _: &ContentRef,
        _: &Request,
        _: &Request,
        evidence: &Request,
        _: &Object,
    ) -> Result<Request, mfm_capabilities::CallbackFailure> {
        Ok(Request {
            value: evidence.value,
        })
    }
}
impl InjectEffect<Effect, Submit> for Native<Never> {
    type Prefix = Identity<Input>;
    type Suffix = Identity<Input>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Default::default(), Default::default()))
    }
}

pub(super) struct Resources<S> {
    pub(super) available: Arc<AtomicBool>,
    pub(super) calls: Arc<AtomicUsize>,
    pub(super) source: PhantomData<fn() -> S>,
}
impl<S> Clone for Resources<S> {
    fn clone(&self) -> Self {
        Self {
            available: self.available.clone(),
            calls: self.calls.clone(),
            source: PhantomData,
        }
    }
}
impl<S> ProgramEnvironment for Resources<S> {
    type Sources = S;
}
impl<S, C, E> BindRead<C, Native<E>> for Resources<S>
where
    S: 'static,
    C: ReadCapabilityContract<Intent = Request, Evidence = Request>,
    E: Value,
    Self: ReadAdapter<Request, Request, E>,
{
    type Adapter = Self;
    fn bind_read(&self, _: &NoParams) -> Result<Self, InvocationDiagnostic> {
        Ok(self.clone())
    }
}
impl<S: 'static> BindEffect<Submit, Native<Never>> for Resources<S> {
    type Adapter = Self;
    fn bind_effect(&self, _: &NoParams) -> Result<Self, InvocationDiagnostic> {
        Ok(self.clone())
    }
}
impl<S: 'static> ReadAdapter<Request, Request, Outage> for Resources<S> {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        intent: &'a Request,
    ) -> Pin<Box<dyn Future<Output = Result<Request, AdapterError<Outage>>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let available = self.available.load(Ordering::SeqCst);
        Box::pin(async move {
            if available {
                Ok(Request {
                    value: intent.value,
                })
            } else {
                Err(AdapterError::Operational(Outage { deadline_ms: 731 }))
            }
        })
    }
}
impl<S: 'static> ReadAdapter<Request, Request, Invalidated> for Resources<S> {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        intent: &'a Request,
    ) -> Pin<Box<dyn Future<Output = Result<Request, AdapterError<Invalidated>>> + Send + 'a>> {
        let available = self.available.load(Ordering::SeqCst);
        Box::pin(async move {
            if available {
                Ok(Request {
                    value: intent.value,
                })
            } else {
                Err(AdapterError::Operational(Invalidated { anchor: 29 }))
            }
        })
    }
}
impl<S: 'static> EffectAdapter<Request, Request, Never> for Resources<S> {
    fn invoke<'a>(
        &'a self,
        _: &'a mfm_ids::EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        command: &'a Request,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<EffectAdapterOutcome<Request>, AdapterError<Never>>>
                + Send
                + 'a,
        >,
    > {
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(if attempt == 0 {
                EffectAdapterOutcome::Pending
            } else {
                EffectAdapterOutcome::Settled(Request {
                    value: command.value,
                })
            })
        })
    }
}
