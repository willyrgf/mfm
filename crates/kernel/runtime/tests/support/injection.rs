use super::*;

struct Injected;
impl State for Injected {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.injection/main@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl EffectState<Mutation> for Injected {
    fn prepare(input: &Number) -> Result<Command, PreparationError> {
        Ok(Command { value: input.value })
    }
    fn interpret(
        input: Number,
        _: &EffectEvidence,
    ) -> std::result::Result<ProposedStateOutcome<Number, Never>, mfm_program::StateExecutionError>
    {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl CapabilityInjection<Injected> for Mutation {
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = Number;
    fn original_binding_ref(setup: &Binding) -> mfm_program::Result<ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
    fn write_before(
        setup: &Binding,
        body: &mut OperationExpansion<Number, Number, Number>,
    ) -> mfm_program::Result<()> {
        body.operation(&IncrementProgram)?;
        body.read::<Observe, Observation>(setup)?;
        body.pure::<Choose>()?;
        body.match_join::<Choice, Choice>(|arms| {
            arms.arm::<Number>(StableId::new("left").unwrap(), |body| {
                body.effect::<Mutate, Mutation>(setup)
            })?;
            arms.arm::<Number>(StableId::new("right").unwrap(), |body| {
                body.effect::<Mutate, Mutation>(setup)
            })
        })
    }
    fn write_after(
        _: &Binding,
        body: &mut OperationExpansion<Number, Number, Number>,
    ) -> mfm_program::Result<()> {
        body.pure::<Project>()
    }
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Choice {
    Left(Number),
    Right(Number),
}
struct Choose;
impl State for Choose {
    type Input = Number;
    type Output = Choice;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.injection/choose@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Choose {
    fn evaluate(
        input: Number,
    ) -> std::result::Result<ProposedStateOutcome<Choice, Never>, mfm_program::StateExecutionError>
    {
        Ok(ProposedStateOutcome::Success {
            output: if input.value.is_multiple_of(2) {
                Choice::Left(input)
            } else {
                Choice::Right(input)
            },
        })
    }
}
struct Project;
impl State for Project {
    type Input = Number;
    type Output = Number;
    type Failure = Number;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.injection/project@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Project {
    fn evaluate(
        input: Number,
    ) -> std::result::Result<ProposedStateOutcome<Number, Number>, mfm_program::StateExecutionError>
    {
        if input.value == 2 {
            Ok(ProposedStateOutcome::Failure { failure: input })
        } else {
            Ok(ProposedStateOutcome::Success { output: input })
        }
    }
}
struct IncrementProgram;
impl Operation for IncrementProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn expand(
        &self,
        body: &mut OperationExpansion<Number, Number, Never>,
    ) -> mfm_program::Result<()> {
        body.pure::<Increment>()
    }
}
struct InjectedProgram;
impl Operation for InjectedProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;
    fn expand(
        &self,
        body: &mut OperationExpansion<Number, Number, Never>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Number, Number>(
            |body| body.effect::<Injected, Mutation>(&Binding { route: 9 }),
            |body| body.pure::<Increment>(),
        )
    }
}

#[tokio::test]
async fn injected_effects_resume_and_post_failure_reaches_the_outer_handler() {
    let store = Arc::new(MemoryStore::new());
    let pending = Arc::new(AtomicBool::new(true));
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_pure::<Increment>().unwrap();
    builder.register_pure::<Project>().unwrap();
    builder.register_pure::<Choose>().unwrap();
    builder.register_read::<Observe, Observation>().unwrap();
    builder.register_effect::<Mutate, Mutation>().unwrap();
    builder.register_effect::<Injected, Mutation>().unwrap();
    builder
        .register_adapter::<Observation, _, _>(Binding { route: 9 }, |reference, intent| {
            let reference = reference.clone();
            let value = intent.value;
            Box::pin(async move {
                Ok(Evidence {
                    intent_value_ref: reference,
                    value,
                    accepted: true,
                })
            })
        })
        .unwrap();
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 9 }, move |id, _, command| {
            let id = id.clone();
            let value = command.value;
            let pending = pending.swap(false, Ordering::SeqCst);
            Box::pin(async move {
                if pending {
                    Ok(EffectAdapterOutcome::Pending)
                } else {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id: id,
                        value,
                        accepted: true,
                    }))
                }
            })
        })
        .unwrap();
    let assembly = builder.finish();
    let runtime = Runtime::new(assembly, store);
    let run_id = RunId::from_digest(DigestBytes::from_array([197; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.injection/program@1").unwrap(),
        &InjectedProgram,
    )
    .unwrap();
    let first = runtime
        .start(run_id.clone(), program, Number { value: 1 })
        .await
        .unwrap();
    assert!(matches!(first.state(), RunViewState::Runnable));
    let finished = runtime.resume(&run_id).await.unwrap();
    let RunViewState::Succeeded(value) = finished.state() else {
        panic!("handler must succeed")
    };
    assert_eq!(value.canonical_bytes(), br#"{"value":3}"#);
    assert_eq!(
        runtime.read(&run_id).await.unwrap().head_digest(),
        finished.head_digest()
    );
}

struct Recursive;
impl CapabilityInjection<Injected> for Recursive {
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = Number;
    fn original_binding_ref(setup: &Binding) -> mfm_program::Result<ContentRef> {
        <Mutation as CapabilityInjection<Injected>>::original_binding_ref(setup)
    }
    fn write_before(
        setup: &Binding,
        body: &mut OperationExpansion<Number, Number, Number>,
    ) -> mfm_program::Result<()> {
        body.effect::<Injected, Recursive>(setup)
    }
}
impl EffectCapabilityContract for Recursive {
    type Command = Command;
    type Evidence = EffectEvidence;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Mutation::contract_id()
    }
    fn bind_evidence(
        id: &EffectId,
        command: &Command,
        evidence: &EffectEvidence,
    ) -> mfm_capabilities::Result<()> {
        Mutation::bind_evidence(id, command, evidence)
    }
}
impl EffectState<Recursive> for Injected {
    fn prepare(input: &Number) -> Result<Command, PreparationError> {
        <Self as EffectState<Mutation>>::prepare(input)
    }
    fn interpret(
        input: Number,
        evidence: &EffectEvidence,
    ) -> std::result::Result<ProposedStateOutcome<Number, Never>, mfm_program::StateExecutionError>
    {
        <Self as EffectState<Mutation>>::interpret(input, evidence)
    }
}
struct RecursionProgram;
impl Operation for RecursionProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;
    fn expand(
        &self,
        body: &mut OperationExpansion<Number, Number, Number>,
    ) -> mfm_program::Result<()> {
        assert_eq!(
            body.effect::<Injected, Recursive>(&Binding { route: 9 }),
            Err(ProgramError::Capacity)
        );
        body.pure::<Increment>()
    }
}
#[test]
fn recursive_injection_is_bounded_and_does_not_merge_partial_states() {
    let program = expand_program(
        EntryPointId::new("mfm.test.injection/recursion@1").unwrap(),
        &RecursionProgram,
    )
    .unwrap();
    assert_eq!(program.declarations().len(), 1);
}
