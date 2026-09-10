use super::*;
use mfm_program::StateExecutionError;

static FAIL_EXECUTION: AtomicBool = AtomicBool::new(true);

struct FailingPure;
struct FailingRead;
struct FailingEffect;

macro_rules! error_state {
    ($state:ident, $id:literal) => {
        impl State for $state {
            type Input = Number;
            type Output = Number;
            type Failure = Number;
            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($id).map_err(|_| ProgramError::InvalidContract)
            }
        }
    };
}
error_state!(FailingPure, "mfm.test.runtime/failing-pure@1");
error_state!(FailingRead, "mfm.test.runtime/failing-read@1");
error_state!(FailingEffect, "mfm.test.runtime/failing-effect@1");

fn execution(input: Number) -> Result<ProposedStateOutcome<Number, Number>, StateExecutionError> {
    if FAIL_EXECUTION.load(Ordering::SeqCst) {
        Err(StateExecutionError)
    } else {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}

impl PureState for FailingPure {
    fn evaluate(
        input: Number,
    ) -> Result<ProposedStateOutcome<Number, Number>, StateExecutionError> {
        execution(input)
    }
}
impl ReadState<Observation> for FailingRead {
    type AdapterContext = NoContext;
    fn adapter_context(
        _: &Self::Input,
        _: &Intent,
        _: &OperationalFailure,
    ) -> Result<NoContext, mfm_program::StateExecutionError> {
        Ok(NoContext)
    }
    fn prepare(input: &Number) -> Result<Intent, PreparationError> {
        Ok(Intent { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &Evidence,
    ) -> Result<ProposedStateOutcome<Number, Number>, StateExecutionError> {
        execution(input)
    }
}
impl EffectState<Mutation> for FailingEffect {
    type AdapterContext = NoContext;
    fn adapter_context(
        _: &Self::Input,
        _: &Command,
        _: &OperationalFailure,
    ) -> Result<NoContext, mfm_program::StateExecutionError> {
        Ok(NoContext)
    }
    fn prepare(input: &Number) -> Result<Command, PreparationError> {
        Ok(Command { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &EffectEvidence,
    ) -> Result<ProposedStateOutcome<Number, Number>, StateExecutionError> {
        execution(input)
    }
}
impl CapabilityInjection<FailingRead> for Observation {
    type FailureMap = Identity<Number>;
    fn failure_map_params(_: &Self::Setup) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = Number;
    fn original_binding_ref(setup: &Binding) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl CapabilityInjection<FailingEffect> for Mutation {
    type FailureMap = Identity<Number>;
    fn failure_map_params(_: &Self::Setup) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = Number;
    fn original_binding_ref(setup: &Binding) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

enum ErrorProgram {
    Pure,
    Read,
    Effect,
}
impl Operation for ErrorProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;
    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Number, Number, Number>,
    ) -> mfm_program::Result<()> {
        match self {
            Self::Pure => body.pure::<FailingPure, Identity<Number>>(
                NoParams,
                Occurrence::new(),
                ConclusionBound::new(65536)?,
            ),
            Self::Read => body.read::<FailingRead, Observation, Identity<Number>>(
                &Binding { route: 7 },
                NoParams,
                Occurrence::new(),
                ConclusionBound::new(65536)?,
            ),
            Self::Effect => body.effect::<FailingEffect, Mutation, Identity<Number>>(
                &Binding { route: 8 },
                NoParams,
                Occurrence::new(),
                EffectBounds::new(65536, 65536, 8, 65536)?,
            ),
        }
    }
}

#[tokio::test]
async fn internal_callback_errors_preserve_heads_and_effect_retry_identity() {
    let store = Arc::new(MemoryStore::new());
    let identities = Arc::new(std::sync::Mutex::new(Vec::new()));
    let read_calls = Arc::new(AtomicUsize::new(0));
    let build_runtime = || {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_pure::<FailingPure>().unwrap();
        builder.register_read::<FailingRead, Observation>().unwrap();
        builder
            .register_effect::<FailingEffect, Mutation>()
            .unwrap();
        builder
            .register_adapter::<Observation, _, _>(Binding { route: 7 }, {
                let calls = read_calls.clone();
                move |reference, intent| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let evidence = Evidence {
                        intent_value_ref: reference.clone(),
                        value: intent.value,
                        accepted: true,
                    };
                    Box::pin(async move { Ok(evidence) })
                }
            })
            .unwrap();
        builder
            .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
                let identities = identities.clone();
                move |effect_id, reference, command| {
                    identities.lock().unwrap().push((
                        effect_id.clone(),
                        reference.clone(),
                        canonicalize_mfm_value(command).unwrap().0,
                    ));
                    let evidence = EffectEvidence {
                        effect_id: effect_id.clone(),
                        value: command.value,
                        accepted: true,
                    };
                    Box::pin(async move { Ok(EffectAdapterOutcome::Settled(evidence)) })
                }
            })
            .unwrap();
        Runtime::new(builder.finish(), store.clone())
    };
    let runtime = build_runtime();
    for (index, operation, expected_head) in [
        (70, ErrorProgram::Pure, 1),
        (71, ErrorProgram::Read, 1),
        (72, ErrorProgram::Effect, 2),
    ] {
        FAIL_EXECUTION.store(true, Ordering::SeqCst);
        let id = RunId::from_digest(DigestBytes::from_array([index; 32]));
        let program = expand_program(
            EntryPointId::new("mfm.test.runtime/callback-error@1").unwrap(),
            &operation,
            &Number { value: 12 },
            ProgramLimits::new(0),
        )
        .unwrap();
        assert!(matches!(
            runtime
                .start(id.clone(), program, Number { value: 12 })
                .await,
            Err(InvocationFailure::Execution {
                error: RuntimeError::Internal,
                ..
            })
        ));
        let pending = runtime.read(&id).await.unwrap();
        assert_eq!(pending.head_sequence(), expected_head);
        if expected_head == 2 {
            assert!(matches!(
                pending.state(),
                RunViewState::EffectPending { .. }
            ));
        } else {
            assert!(matches!(pending.state(), RunViewState::Runnable { .. }));
        }
        let cold = build_runtime();
        assert!(matches!(
            cold.resume(&id).await,
            Err(InvocationFailure::Execution {
                error: RuntimeError::Internal,
                ..
            })
        ));
        assert_eq!(
            cold.read(&id).await.unwrap().head_digest(),
            pending.head_digest()
        );
        FAIL_EXECUTION.store(false, Ordering::SeqCst);
        let completed = cold.resume(&id).await.unwrap();
        assert!(matches!(completed.state(), RunViewState::Succeeded(_)));
        assert_eq!(completed.head_sequence(), expected_head + 1);
        FAIL_EXECUTION.store(true, Ordering::SeqCst);
        let terminal = build_runtime();
        let reloaded = terminal.read(&id).await.unwrap();
        assert_eq!(reloaded.head_digest(), completed.head_digest());
        let resumed = terminal.resume(&id).await.unwrap();
        assert_eq!(resumed.head_digest(), completed.head_digest());
        let (RunViewState::Succeeded(actual), RunViewState::Succeeded(expected)) =
            (resumed.state(), completed.state())
        else {
            panic!("terminal success")
        };
        assert_eq!(actual.canonical_bytes(), expected.canonical_bytes());
        assert_eq!(actual.value_ref(), expected.value_ref());
    }
    assert_eq!(read_calls.load(Ordering::SeqCst), 3);
    let retained = identities.lock().unwrap();
    assert_eq!(retained.len(), 3);
    assert!(retained.windows(2).all(|pair| pair[0] == pair[1]));
}
