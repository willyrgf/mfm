use mfm_capabilities::EffectAdapterOutcome;
use mfm_capabilities::{AdapterError, EffectCapabilityContract};
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_journal::{decode_frame, seal_frame};
use mfm_program::*;
use mfm_program_derive::MfmValue;
use mfm_runtime::{InvocationFailure, RecoveryOutcome, RunViewState, Runtime, RuntimeError};
use mfm_store::{MemoryStore, Store};
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Number {
    value: u64,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum Cause {
    Timeout { deadline_ms: u64 },
}
impl ClassifyError for Cause {
    fn classify(&self) -> Classification {
        Classification::OutcomeUnknown
    }
}
struct Submit;
impl EffectCapabilityContract for Submit {
    type Command = Number;
    type Evidence = Number;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.audited-submit@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &EffectId,
        _: &ContentRef,
        command: &Number,
        _: &ContentRef,
        evidence: &Number,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        if command.value == evidence.value {
            Ok(())
        } else {
            Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "bind_evidence",
                &(mfm_capabilities::CapabilityError::EvidenceBinding),
                None,
            ))
        }
    }
}
struct Execute;
impl State for Execute {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.audited-execute@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl EffectState<Submit> for Execute {
    fn prepare(input: &Number) -> std::result::Result<Number, InvocationDiagnostic> {
        Ok(Number { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl EffectSelection<Submit> for Execute {
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
}
struct RetryUnknown;
impl Handler for RetryUnknown {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.retry-unknown@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        classification: Classification,
        _: &RecoveryContext<'_>,
    ) -> std::result::Result<RecoveryRequest, InvocationDiagnostic> {
        assert_eq!(classification, Classification::OutcomeUnknown);
        Ok(RecoveryRequest::RetryState)
    }
}
struct SubmitFlow;
impl OperationDefinition for SubmitFlow {
    type Body = ResolvedEffect<Execute, Submit, Native<Cause>>;
}
impl Plan<Number> for SubmitFlow {
    type Config = Number;
    fn plan<'a>(&'a self, input: &'a Number) -> mfm_program::Result<(&'a Number, Self::Body)> {
        Ok((input, ResolvedEffect::new(Number { value: 1 })))
    }
}
impl Default for SubmitFlow {
    fn default() -> Self {
        Self
    }
}
struct Policy<H, const RETRIES: u32>(std::marker::PhantomData<fn() -> H>);
impl<H: Handler, const RETRIES: u32> OperationDefaults for Policy<H, RETRIES> {
    type Handler = H;
    type Targets = ();
}
impl<C: ?Sized, H: Handler<Params = NoParams>, const RETRIES: u32> ResolveDefaults<C>
    for Policy<H, RETRIES>
{
    fn resolve(_: &C) -> mfm_program::Result<PolicyValues<H>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(RETRIES),
            restarts: Some(0),
        })
    }
}
type Flow = Operation<SubmitFlow, Policy<RetryUnknown, 1>>;
type StopFlow = Operation<SubmitFlow, Policy<StandardRecovery, 0>>;

// The native protocol is an identity codec here; the observable boundary is the scripted IO.
struct Native<E>(std::marker::PhantomData<fn() -> E>);
impl<C, E> mfm_capabilities::EffectImplementation<C> for Native<E>
where
    C: EffectCapabilityContract<Command = Number, Evidence = Number>,
    E: mfm_values::MfmValue,
{
    type Binding = Number;
    type NativeCommand = Number;
    type NativeEvidence = Number;
    type OperationalError = E;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.pending-native@1")?)
    }
    fn decode_command(
        _: &ContentRef,
        _: &ContentRef,
        _: &Number,
        reference: &ContentRef,
        command: &Number,
    ) -> std::result::Result<(ContentRef, Number), mfm_capabilities::CallbackFailure> {
        Ok((
            reference.clone(),
            Number {
                value: command.value,
            },
        ))
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &Number,
        effect: &EffectId,
        command_ref: &ContentRef,
        command: &Number,
        _: &Number,
        evidence: &Number,
        original: &mfm_values::Object,
    ) -> std::result::Result<Number, mfm_capabilities::CallbackFailure> {
        C::bind_evidence(effect, command_ref, command, original.value_ref(), evidence)?;
        Ok(Number {
            value: evidence.value,
        })
    }
}
impl<S, C, E> InjectEffect<S, C> for Native<E>
where
    C: EffectCapabilityContract<Command = Number, Evidence = Number>,
    S: EffectSelection<
        C,
        Input = Number,
        Output = Number,
        ExpandedInput = Number,
        ExpandedOutput = Number,
    >,
    E: mfm_values::MfmValue,
{
    type Prefix = Identity<Number>;
    type Suffix = Identity<Number>;
    fn surround(_: &Number) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
type EffectFuture<E> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = std::result::Result<EffectAdapterOutcome<Number>, AdapterError<E>>,
            > + Send,
    >,
>;
type Invocation<E> = dyn Fn(EffectId, ContentRef, Number) -> EffectFuture<E> + Send + Sync;
struct Resources<S, E = Cause> {
    invoke: Arc<Invocation<E>>,
    source: std::marker::PhantomData<fn() -> S>,
}
impl<S, E> Resources<S, E> {
    fn new(
        invoke: impl Fn(EffectId, ContentRef, Number) -> EffectFuture<E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            invoke: Arc::new(invoke),
            source: std::marker::PhantomData,
        }
    }
}
impl<S: 'static, E: 'static> ProgramEnvironment for Resources<S, E> {
    type Sources = S;
}
impl<S, C, E> BindEffect<C, Native<E>> for Resources<S, E>
where
    S: 'static,
    C: EffectCapabilityContract<Command = Number, Evidence = Number>,
    E: mfm_values::MfmValue,
{
    type Adapter = Self;
    fn bind_effect(&self, binding: &Number) -> std::result::Result<Self, InvocationDiagnostic> {
        assert_eq!(binding.value, 1);
        Ok(Self {
            invoke: Arc::clone(&self.invoke),
            source: std::marker::PhantomData,
        })
    }
}
impl<S: 'static, E: 'static> mfm_capabilities::EffectAdapter<Number, Number, E>
    for Resources<S, E>
{
    fn invoke<'a>(
        &'a self,
        effect: &'a EffectId,
        _: &'a ContentRef,
        reference: &'a ContentRef,
        command: &'a Number,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<EffectAdapterOutcome<Number>, AdapterError<E>>,
                > + Send
                + 'a,
        >,
    > {
        (self.invoke)(
            effect.clone(),
            reference.clone(),
            Number {
                value: command.value,
            },
        )
    }
}

#[allow(dead_code)]
#[path = "support/scripted_store.rs"]
mod scripted_store;

#[path = "pending_failure/audit.rs"]
mod audit;
#[path = "pending_failure/phase.rs"]
mod phase;
#[path = "pending_failure/protocol.rs"]
mod protocol;

#[path = "pending_failure/capacity.rs"]
mod capacity;
