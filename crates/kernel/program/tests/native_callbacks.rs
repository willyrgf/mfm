//! Framework callback boundary tests, not product lifecycle or Runtime evidence.
#[allow(dead_code)]
#[path = "phase_a/contracts.rs"]
mod contracts;
use contracts::*;
use mfm_capabilities::{
    AdapterError, EffectAdapter, EffectAdapterOutcome, EffectImplementation, ReadAdapter,
    ReadImplementation,
};
use mfm_ids::{
    ContentRef, DigestBytes, EffectId, ExecutionPosition, StableId, StatePosition, VisitId,
};
use mfm_program as source;
use mfm_program::{callback, Never, NoParams, ProposedStateOutcome};
use mfm_values::{InvocationDiagnostic, Object};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

static READ_PROJECTIONS: AtomicUsize = AtomicUsize::new(0);

struct NativeRead<const PHASE: u8 = 0>;
impl<const PHASE: u8> ReadImplementation<Observation> for NativeRead<PHASE> {
    type Binding = NoParams;
    type NativeIntent = Prepared;
    type NativeEvidence = Deployed;
    type OperationalError = Never;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("proof.native-read@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        intent: &Configured,
    ) -> Result<Prepared, mfm_capabilities::CallbackFailure> {
        if PHASE < 10 {
            hook_failure(PHASE)?;
        }
        Ok(Prepared {
            value: intent.value + 1,
        })
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        intent: &Configured,
        _: &ContentRef,
        native: &Prepared,
        evidence: &Deployed,
        original: &Object,
    ) -> Result<Observed, mfm_capabilities::CallbackFailure> {
        if PHASE == 0 {
            READ_PROJECTIONS.fetch_add(1, Ordering::SeqCst);
        }
        if PHASE >= 10 {
            hook_failure(PHASE - 10)?;
        }
        assert_eq!(native.value, intent.value + 1);
        assert_eq!(original.decode::<Deployed>().unwrap().value, evidence.value);
        Ok(Observed {
            value: evidence.value - 1,
        })
    }
}
struct ObserveNative;
impl ReadAdapter<Prepared, Deployed, Never> for ObserveNative {
    fn invoke<'a>(
        &'a self,
        semantic: &'a ContentRef,
        native: &'a ContentRef,
        intent: &'a Prepared,
    ) -> Pin<Box<dyn Future<Output = Result<Deployed, AdapterError<Never>>> + Send + 'a>> {
        Box::pin(async move {
            assert_ne!(semantic, native);
            assert_eq!(native, Object::from_value(intent).unwrap().value_ref());
            Ok(Deployed {
                value: intent.value,
            })
        })
    }
}

#[tokio::test]
async fn read_callbacks_keep_native_evidence_and_interpret_its_semantic_projection() {
    let binding = Object::from_value(&NoParams).unwrap();
    // These low-level callback tests bypass Program association, so the implementation reference
    // is an explicit fixture identity. Exact installed ABI matching belongs to construction tests.
    let implementation = mfm_program::state_implementation_ref::<Observe>().unwrap();
    let callbacks = callback::ReadCallbacks::new::<Observe, Observation, NativeRead>(
        mfm_program::nominal_contract_ref::<Never>().unwrap(),
        implementation.clone(),
        binding.value_ref().clone(),
        Arc::new(NoParams),
    );
    let adapter = callback::read_adapter::<Observation, NativeRead, _>(
        mfm_program::nominal_contract_ref::<Never>().unwrap(),
        implementation,
        binding.value_ref().clone(),
        Arc::new(NoParams),
        ObserveNative,
    );
    let input = Object::from_value(&Configured { value: 7 }).unwrap();
    let intent = (callbacks.prepare)(input.clone()).await.unwrap();
    let position = ExecutionPosition {
        state: StatePosition::new(0).unwrap(),
        visit: VisitId::new(0),
    };
    let evidence = adapter(position, &intent).await.unwrap().unwrap();
    assert_eq!(evidence.decode::<Deployed>().unwrap().value, 8);
    assert!(evidence.decode::<Observed>().is_err());
    let outcome = (callbacks.complete)(input, intent, evidence, position)
        .await
        .unwrap();
    let ProposedStateOutcome::Success { output } = outcome else {
        panic!("unexpected State failure")
    };
    assert_eq!(output.decode::<Observed>().unwrap().value, 7);
    assert_eq!(READ_PROJECTIONS.load(Ordering::SeqCst), 1);
}

struct NativeEffect<const WRONG_SCHEMA: bool>;
impl<const WRONG_SCHEMA: bool> EffectImplementation<Transaction> for NativeEffect<WRONG_SCHEMA> {
    type Binding = NoParams;
    type NativeCommand = Prepared;
    type NativeEvidence = Configured;
    type OperationalError = Never;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("proof.native-effect@1")?)
    }
    fn decode_command(
        _: &ContentRef,
        binding_ref: &ContentRef,
        _: &NoParams,
        command_ref: &ContentRef,
        command: &Prepared,
    ) -> Result<(ContentRef, Prepared), mfm_capabilities::CallbackFailure> {
        Ok((
            if WRONG_SCHEMA {
                binding_ref
            } else {
                command_ref
            }
            .clone(),
            Prepared {
                value: command.value,
            },
        ))
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &EffectId,
        _: &ContentRef,
        command: &Prepared,
        native: &Prepared,
        evidence: &Configured,
        original: &Object,
    ) -> Result<Deployed, mfm_capabilities::CallbackFailure> {
        assert_eq!(command.value, native.value);
        assert_eq!(
            original.decode::<Configured>().unwrap().value,
            evidence.value
        );
        Ok(Deployed {
            value: evidence.value,
        })
    }
}
struct SettleNative(Arc<AtomicUsize>);
impl EffectAdapter<Prepared, Configured, Never> for SettleNative {
    fn invoke<'a>(
        &'a self,
        _: &'a EffectId,
        semantic: &'a ContentRef,
        native: &'a ContentRef,
        command: &'a Prepared,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<EffectAdapterOutcome<Configured>, AdapterError<Never>>>
                + Send
                + 'a,
        >,
    > {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(semantic, native);
            Ok(EffectAdapterOutcome::Settled(Configured {
                value: command.value,
            }))
        })
    }
}

#[tokio::test]
async fn effect_callbacks_validate_before_invocation_and_project_native_settlement() {
    let binding = Object::from_value(&NoParams).unwrap();
    let implementation = mfm_program::state_implementation_ref::<Configure>().unwrap();
    let callbacks = callback::EffectCallbacks::new::<Configure, Transaction, NativeEffect<false>>(
        mfm_program::nominal_contract_ref::<Never>().unwrap(),
        implementation.clone(),
        binding.value_ref().clone(),
        Arc::new(NoParams),
    );
    let invocations = Arc::new(AtomicUsize::new(0));
    let adapter = callback::effect_adapter::<Transaction, NativeEffect<false>, _>(
        mfm_program::nominal_contract_ref::<Never>().unwrap(),
        implementation.clone(),
        binding.value_ref().clone(),
        Arc::new(NoParams),
        SettleNative(Arc::clone(&invocations)),
    );
    let input = Object::from_value(&Deployed { value: 9 }).unwrap();
    let command = (callbacks.prepare)(input.clone()).await.unwrap();
    (callbacks.validate_command)(command.clone()).await.unwrap();
    let position = ExecutionPosition {
        state: StatePosition::new(0).unwrap(),
        visit: VisitId::new(0),
    };
    let effect = EffectId::from_digest(DigestBytes::from_array([1; 32]));
    let EffectAdapterOutcome::Settled(evidence) =
        adapter(position, &effect, &command).await.unwrap().unwrap()
    else {
        panic!("unexpected Pending")
    };
    assert!(evidence.decode::<Deployed>().is_err());
    (callbacks.bind)(effect.clone(), command.clone(), evidence.clone())
        .await
        .unwrap();
    let outcome = (callbacks.interpret)(input, effect.clone(), command.clone(), evidence, position)
        .await
        .unwrap();
    let ProposedStateOutcome::Success { output } = outcome else {
        panic!("unexpected State failure")
    };
    assert_eq!(output.decode::<Configured>().unwrap().value, 9);
    let mismatched = callback::EffectCallbacks::new::<Configure, Transaction, NativeEffect<true>>(
        mfm_program::nominal_contract_ref::<Never>().unwrap(),
        implementation.clone(),
        binding.value_ref().clone(),
        Arc::new(NoParams),
    );
    assert!(matches!(
        (mismatched.validate_command)(command.clone()).await,
        Err(mfm_capabilities::CallbackFailure::Execute(_))
    ));
    let mismatched_adapter = callback::effect_adapter::<Transaction, NativeEffect<true>, _>(
        mfm_program::nominal_contract_ref::<Never>().unwrap(),
        implementation,
        binding.value_ref().clone(),
        Arc::new(NoParams),
        SettleNative(Arc::clone(&invocations)),
    );
    assert!(matches!(
        mismatched_adapter(position, &effect, &command).await,
        Err(mfm_capabilities::CallbackFailure::Execute(_))
    ));
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

impl source::InjectEffect<Configure, Transaction> for NativeEffect<false> {
    type Prefix = source::Identity<Deployed>;
    type Suffix = source::Identity<Configured>;
    fn surround(_: &NoParams) -> source::Result<(Self::Prefix, Self::Suffix)> {
        Ok((source::Identity::default(), source::Identity::default()))
    }
}
impl source::ResolveEffectBinding<Deployed, Transaction> for NativeEffect<false> {
    fn binding(_: &Deployed) -> source::Result<NoParams> {
        Ok(NoParams)
    }
}
struct EffectResources<const COLD: bool>(Arc<AtomicUsize>);
impl<const COLD: bool> source::ProgramEnvironment for EffectResources<COLD> {
    type Sources = source::Effect<Configure, Transaction>;
}
impl<const COLD: bool> source::CapabilityFamily<Transaction> for EffectResources<COLD> {
    type Implementations = (NativeEffect<false>,);
}
impl source::Resolve<Deployed, Transaction> for EffectResources<false> {
    fn implementation(_: &Deployed) -> source::Result<StableId> {
        Ok(NativeEffect::<false>::implementation_id()?)
    }
}
impl<const COLD: bool> source::BindEffect<Transaction, NativeEffect<false>>
    for EffectResources<COLD>
{
    type Adapter = SettleNative;
    fn bind_effect(&self, _: &NoParams) -> Result<SettleNative, InvocationDiagnostic> {
        Ok(SettleNative(Arc::clone(&self.0)))
    }
}

#[tokio::test]
async fn cold_effect_program_contains_bound_native_callbacks_without_configuration_resolution() {
    let invoked = Arc::new(AtomicUsize::new(0));
    let input = Deployed { value: 9 };
    let program = source::compile(
        mfm_ids::EntryPointId::new("mfm.proof/effect@1").unwrap(),
        &source::Effect::<Configure, Transaction>::default(),
        &input,
        &EffectResources::<false>(Arc::clone(&invoked)),
        source::ProgramLimits::new(0),
    )
    .unwrap();
    // The cold environment has no Resolve implementation and invokes no live adapter at load time.
    let cold = source::load(
        program.canonical_bytes(),
        &EffectResources::<true>(Arc::clone(&invoked)),
    )
    .unwrap();
    assert_eq!(cold.content_ref(), program.content_ref());
    assert_eq!(cold.canonical_bytes(), program.canonical_bytes());
    assert_eq!(cold.bindings().len(), 1);
    let source::Execution::Effect { abi, .. } = cold.declarations()[0].execution() else {
        panic!("expected Effect ABI");
    };
    assert_eq!(
        abi.implementation(),
        &source::effect_implementation_ref::<Transaction, NativeEffect<false>>().unwrap()
    );
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    let position = ExecutionPosition {
        state: StatePosition::new(0).unwrap(),
        visit: VisitId::new(0),
    };
    let source::executable::ExecutableMode::Effect { callbacks, adapter } =
        cold.executable(position.state).unwrap().mode()
    else {
        panic!("expected a complete bound Effect occurrence")
    };
    let input = Object::from_value(&input).unwrap();
    let command = (callbacks.prepare)(input.clone()).await.unwrap();
    (callbacks.validate_command)(command.clone()).await.unwrap();
    let effect = EffectId::from_digest(DigestBytes::from_array([2; 32]));
    let EffectAdapterOutcome::Settled(evidence) =
        adapter(position, &effect, &command).await.unwrap().unwrap()
    else {
        panic!("unexpected pending fixture result")
    };
    (callbacks.bind)(effect.clone(), command.clone(), evidence.clone())
        .await
        .unwrap();
    let ProposedStateOutcome::Success { output } =
        (callbacks.interpret)(input, effect, command, evidence, position)
            .await
            .unwrap()
    else {
        panic!("unexpected fixture domain failure")
    };
    assert_eq!(output.decode::<Configured>().unwrap().value, 9);
    assert_eq!(invoked.load(Ordering::SeqCst), 1);
}

fn hook_failure(phase: u8) -> Result<(), mfm_capabilities::CallbackFailure> {
    use mfm_capabilities::{codec, CallbackFailure};
    let cause = || InvocationDiagnostic::from_fields("native_codec", "nested_native", &42, None);
    match phase {
        0 => Ok(()),
        1 => codec::decode(|| Err(cause())),
        2 => codec::encode(|| Err(cause())),
        3 => codec::decode(|| panic!("unretained codec payload")),
        4 => codec::encode(|| panic!("unretained codec payload")),
        5 => panic!("unretained hook payload"),
        _ => Err(CallbackFailure::Execute(cause())),
    }
}

#[tokio::test]
async fn nested_native_hooks_preserve_codec_phases_and_uncaught_hook_panics() {
    async fn check<const PHASE: u8>() {
        use mfm_capabilities::CallbackFailure;
        let binding = Object::from_value(&NoParams).unwrap();
        let implementation = source::state_implementation_ref::<Observe>().unwrap();
        let intent = Object::from_value(&Configured { value: 7 }).unwrap();
        let error = if PHASE < 10 {
            let adapter = callback::read_adapter::<Observation, NativeRead<PHASE>, _>(
                source::nominal_contract_ref::<Never>().unwrap(),
                implementation,
                binding.value_ref().clone(),
                Arc::new(NoParams),
                ObserveNative,
            );
            adapter(
                ExecutionPosition {
                    state: StatePosition::new(0).unwrap(),
                    visit: VisitId::new(0),
                },
                &intent,
            )
            .await
            .unwrap_err()
        } else {
            let callbacks = callback::ReadCallbacks::new::<Observe, Observation, NativeRead<PHASE>>(
                source::nominal_contract_ref::<Never>().unwrap(),
                implementation,
                binding.value_ref().clone(),
                Arc::new(NoParams),
            );
            let callback::ReadCompletionFailure::Bind(error) = (callbacks.complete)(
                intent.clone(),
                intent,
                Object::from_value(&Deployed { value: 8 }).unwrap(),
                ExecutionPosition {
                    state: StatePosition::new(0).unwrap(),
                    visit: VisitId::new(0),
                },
            )
            .await
            .unwrap_err() else {
                panic!("projection retains binding provenance");
            };
            error
        };
        let phase = PHASE % 10;
        let cause = match (phase, error) {
            (1 | 3, CallbackFailure::Decode(cause))
            | (2 | 4, CallbackFailure::Encode(cause))
            | (5, CallbackFailure::Execute(cause)) => cause,
            _ => panic!("native phase changed across Program boundary"),
        };
        if phase <= 2 {
            assert_eq!(cause.code(), "native_codec");
            assert_eq!(cause.operation(), "nested_native");
            assert_eq!(cause.details().as_value(), &serde_json::json!(42));
        } else {
            assert_eq!(cause.code(), "task_failure");
            assert_eq!(cause.details().as_value(), &serde_json::json!("panicked"));
        }
    }
    check::<1>().await;
    check::<2>().await;
    check::<3>().await;
    check::<4>().await;
    check::<5>().await;
    check::<11>().await;
    check::<12>().await;
    check::<13>().await;
    check::<14>().await;
    check::<15>().await;
}
