use super::*;
use mfm_capabilities::{AdapterError, ReadCapabilityContract};
use mfm_program::{ReadState, ValueMap};

struct Inflate;
impl ValueMap for Inflate {
    type Input = Failure;
    type Output = Failure;
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.capacity/inflate@1").unwrap())
    }
    fn apply(_: &NoParams, _: Failure) -> Result<Failure, StateExecutionError> {
        Ok(Failure {
            detail: "x".repeat(33 * 1024 * 1024),
        })
    }
}

impl ReadCapabilityContract for Mutation {
    type OperationalError = OperationalFailure;
    type Intent = Input;
    type Evidence = Input;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.capacity/observation@1").unwrap())
    }
    fn bind_evidence(
        _: &mfm_ids::ContentRef,
        intent: &Input,
        evidence: &Input,
    ) -> mfm_capabilities::Result<()> {
        if intent.bytes == evidence.bytes {
            Ok(())
        } else {
            Err(CapabilityError::EvidenceBinding)
        }
    }
}
impl ReadState<Mutation> for Failing {
    type AdapterContext = Failure;
    fn adapter_context(
        input: &Input,
        intent: &Input,
        error: &OperationalFailure,
    ) -> Result<Failure, StateExecutionError> {
        <Self as EffectState<Mutation>>::adapter_context(input, intent, error)
    }
    fn prepare(input: &Input) -> Result<Input, PreparationError> {
        <Self as EffectState<Mutation>>::prepare(input)
    }
    fn interpret(
        input: Input,
        _: &Input,
    ) -> Result<ProposedStateOutcome<Input, Failure>, StateExecutionError> {
        Self::evaluate(input)
    }
}

#[derive(Clone, Copy)]
enum Oversized {
    RootMap,
    ReadContext,
    EffectContext,
}
impl Operation for Oversized {
    type Input = Input;
    type Output = Input;
    type Failure = Failure;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        scope: &mut OperationExpansion<Input, Input, Failure>,
    ) -> mfm_program::Result<()> {
        let bound = ConclusionBound::new(40 * 1024 * 1024)?;
        match self {
            Self::RootMap => scope.pure::<Failing, Inflate>(NoParams, Occurrence::new(), bound),
            Self::ReadContext => scope.read::<Failing, Mutation, Identity<Failure>>(
                &NoParams,
                NoParams,
                Occurrence::new(),
                bound,
            ),
            Self::EffectContext => Plan { effect: true }.expand(scope),
        }
    }
}

#[tokio::test]
async fn mapped_values_and_adapter_contexts_preserve_size_errors_and_acknowledged_heads() {
    for (index, case) in [
        Oversized::RootMap,
        Oversized::ReadContext,
        Oversized::EffectContext,
    ]
    .into_iter()
    .enumerate()
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        match case {
            Oversized::RootMap => builder.register_pure::<Failing>().unwrap(),
            Oversized::ReadContext => builder.register_read::<Failing, Mutation>().unwrap(),
            Oversized::EffectContext => builder.register_effect::<Failing, Mutation>().unwrap(),
        }
        builder.register_map::<Inflate>().unwrap();
        builder
            .register_adapter::<Mutation, _, _>(NoParams, {
                let calls = calls.clone();
                move |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async {
                        Err(AdapterError::Operational(OperationalFailure::Unavailable))
                    })
                }
            })
            .unwrap();
        builder
            .register_effect_adapter::<Mutation, _, _>(NoParams, {
                let calls = calls.clone();
                move |_, _, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async {
                        Err(AdapterError::Operational(OperationalFailure::Unavailable))
                    })
                }
            })
            .unwrap();
        let runtime = Runtime::new(builder.finish(), Arc::new(MemoryStore::new()));
        let id = RunId::from_digest(DigestBytes::from_array([180 + index as u8; 32]));
        let bytes = if matches!(case, Oversized::ReadContext | Oversized::EffectContext) {
            33 * 1024 * 1024
        } else {
            1
        };
        let program = expand_program(
            EntryPointId::new("mfm.test.capacity/qualification@1").unwrap(),
            &case,
            &Input { bytes },
            ProgramLimits::new(0),
        )
        .unwrap();
        let Err(InvocationFailure::Execution {
            error,
            last_observed: Some(observed),
            ..
        }) = runtime.start(id.clone(), program, Input { bytes }).await
        else {
            panic!("size failure")
        };
        let (resource, size) = error.size_limit().expect("specific size error");
        assert_eq!(resource, SizeResource::CanonicalObject);
        assert_eq!(size.limit(), 33_554_432);
        assert!(size.actual() > size.limit());
        let before = calls.load(Ordering::SeqCst);
        let cold = runtime.read(&id).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before);
        assert_eq!(cold.head_digest(), observed.head_digest());
        assert_eq!(
            cold.head_sequence(),
            if matches!(case, Oversized::EffectContext) {
                2
            } else {
                1
            }
        );
        match (observed.state(), cold.state()) {
            (
                RunViewState::EffectPending { effect_id: a, .. },
                RunViewState::EffectPending { effect_id: b, .. },
            ) => assert_eq!(a, b),
            (RunViewState::Runnable { .. }, RunViewState::Runnable { .. }) => {}
            _ => panic!("acknowledged state changed"),
        }
    }
}
