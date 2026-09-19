use super::*;
use mfm_capabilities::{ReadAdapter, ReadCapabilityContract, ReadImplementation};
use mfm_program::{
    BindRead, InjectRead, Pure, PureState, Read, ReadSelection, ReadState, ResolveReadBinding,
};
use std::sync::atomic::{AtomicUsize, Ordering};

static ENCODINGS: AtomicUsize = AtomicUsize::new(0);
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Input {
    panics: bool,
}
#[derive(Debug, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Original {
    panics: bool,
}
impl Serialize for Original {
    fn serialize<S: serde::Serializer>(&self, _: S) -> std::result::Result<S::Ok, S::Error> {
        ENCODINGS.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panics, "original panic payload marker");
        Err(serde::ser::Error::custom("original encoder rejected field"))
    }
}
impl ClassifyError for Original {
    fn classify(&self) -> Classification {
        panic!("unrecorded original must never reach classification")
    }
}
struct Reject;
impl State for Reject {
    type Input = Input;
    type Output = Input;
    type Failure = Original;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test/original-encoding@1")?)
    }
}
impl PureState for Reject {
    fn evaluate(
        input: Input,
    ) -> std::result::Result<ProposedStateOutcome<Input, Original>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Failure {
            failure: Original {
                panics: input.panics,
            },
        })
    }
}
struct Observation;
impl ReadCapabilityContract for Observation {
    type Intent = Input;
    type Evidence = Input;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test/original-read@1")?)
    }
    fn bind_evidence(
        _: &ContentRef,
        _: &Input,
        _: &ContentRef,
        _: &Input,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        Ok(())
    }
}
impl ReadState<Observation> for Reject {
    fn prepare(input: &Input) -> std::result::Result<Input, InvocationDiagnostic> {
        Ok(Input {
            panics: input.panics,
        })
    }
    fn interpret(
        _: Input,
        _: &Input,
    ) -> std::result::Result<ProposedStateOutcome<Input, Original>, InvocationDiagnostic> {
        panic!("failed adapter cannot enter interpretation")
    }
}
impl ReadSelection<Observation> for Reject {
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
}
struct NativeRead;
impl ReadImplementation<Observation> for NativeRead {
    type Binding = NoParams;
    type NativeIntent = Input;
    type NativeEvidence = Input;
    type OperationalError = Original;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test/original-native@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        intent: &Input,
    ) -> std::result::Result<Input, mfm_capabilities::CallbackFailure> {
        Ok(Input {
            panics: intent.panics,
        })
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &Input,
        _: &ContentRef,
        _: &Input,
        _: &Input,
        _: &Object,
    ) -> std::result::Result<Input, mfm_capabilities::CallbackFailure> {
        panic!("failed adapter cannot produce evidence")
    }
}
impl InjectRead<Reject, Observation> for NativeRead {
    type Prefix = Identity<Input>;
    type Suffix = Identity<Input>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok(Default::default())
    }
}
impl ResolveReadBinding<Input, Observation> for NativeRead {
    fn binding(_: &Input) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
}
struct Environment;
impl ProgramEnvironment for Environment {
    type Sources = (
        Pure<Reject>,
        Read<Reject, Observation>,
        Effect<Reject, Command>,
    );
}
impl CapabilityFamily<Observation> for Environment {
    type Implementations = (NativeRead,);
}
impl Resolve<Input, Observation> for Environment {
    fn implementation(_: &Input) -> mfm_program::Result<StableId> {
        Ok(NativeRead::implementation_id()?)
    }
}
impl BindRead<Observation, NativeRead> for Environment {
    type Adapter = Self;
    fn bind_read(&self, _: &NoParams) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(Self)
    }
}
impl ReadAdapter<Input, Input, Original> for Environment {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        intent: &'a Input,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Input, AdapterError<Original>>> + Send + 'a>>
    {
        Box::pin(async move {
            Err(AdapterError::Operational(Original {
                panics: intent.panics,
            }))
        })
    }
}

// The same hostile serializer is now a prepared command, rather than a declared failure.
struct Command;
impl EffectCapabilityContract for Command {
    type Command = Original;
    type Evidence = Input;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test/command-encoding@1")?)
    }
    fn bind_evidence(
        _: &EffectId,
        _: &ContentRef,
        _: &Original,
        _: &ContentRef,
        _: &Input,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        panic!("unencoded command cannot reach evidence binding")
    }
}
impl EffectState<Command> for Reject {
    fn prepare(input: &Input) -> std::result::Result<Original, InvocationDiagnostic> {
        Ok(Original {
            panics: input.panics,
        })
    }
    fn interpret(
        _: Input,
        _: &Input,
    ) -> std::result::Result<ProposedStateOutcome<Input, Original>, InvocationDiagnostic> {
        panic!("unencoded command cannot reach interpretation")
    }
}
impl EffectSelection<Command> for Reject {
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
}
struct NativeEffect;
impl EffectImplementation<Command> for NativeEffect {
    type Binding = NoParams;
    type NativeCommand = Input;
    type NativeEvidence = Input;
    type OperationalError = Original;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test/command-native@1")?)
    }
    fn decode_command(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &Original,
    ) -> std::result::Result<(ContentRef, Input), mfm_capabilities::CallbackFailure> {
        panic!("unencoded command cannot reach native materialization")
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &EffectId,
        _: &ContentRef,
        _: &Original,
        _: &Input,
        _: &Input,
        _: &Object,
    ) -> std::result::Result<Input, mfm_capabilities::CallbackFailure> {
        panic!("unencoded command cannot produce evidence")
    }
}
impl InjectEffect<Reject, Command> for NativeEffect {
    type Prefix = Identity<Input>;
    type Suffix = Identity<Input>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok(Default::default())
    }
}
impl ResolveEffectBinding<Input, Command> for NativeEffect {
    fn binding(_: &Input) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
}
impl CapabilityFamily<Command> for Environment {
    type Implementations = (NativeEffect,);
}
impl Resolve<Input, Command> for Environment {
    fn implementation(_: &Input) -> mfm_program::Result<StableId> {
        Ok(NativeEffect::implementation_id()?)
    }
}
impl BindEffect<Command, NativeEffect> for Environment {
    type Adapter = Self;
    fn bind_effect(&self, _: &NoParams) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(Self)
    }
}
impl EffectAdapter<Input, Input, Original> for Environment {
    fn invoke<'a>(
        &'a self,
        _: &'a EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        _: &'a Input,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = std::result::Result<
                        EffectAdapterOutcome<Input>,
                        AdapterError<Original>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        panic!("unencoded command must never invoke the adapter")
    }
}

#[tokio::test]
async fn original_or_command_encoding_fault_preserves_admission_without_retry_or_classification() {
    for read in [false, true] {
        for panics in [false, true] {
            let input = Input { panics };
            let entry = EntryPointId::new("mfm.test/original-encoding@1").unwrap();
            let program = if read {
                mfm_program::compile(
                    entry,
                    &Read::<Reject, Observation>::default(),
                    &input,
                    &Environment,
                    ProgramLimits::new(0),
                )
            } else {
                mfm_program::compile(
                    entry,
                    &Pure::<Reject>::default(),
                    &input,
                    &Environment,
                    ProgramLimits::new(0),
                )
            }
            .unwrap();
            let store = Arc::new(MemoryStore::new());
            let runtime = Runtime::new(store.clone());
            let run = RunId::from_digest(DigestBytes::from_array(
                [150 + u8::from(read) * 2 + u8::from(panics); 32],
            ));
            let before = ENCODINGS.load(Ordering::SeqCst);
            let Err(InvocationFailure::Execution {
                run_id,
                error,
                last_observed: Some(observed),
            }) = runtime.execute(run.clone(), &program, &input).await
            else {
                panic!("first original encoding is invocation-only")
            };
            assert_eq!(run_id, run);
            assert_eq!(ENCODINGS.load(Ordering::SeqCst) - before, 1);
            let RuntimeError::Native {
                operation,
                stage: Stage::Encode,
                cause,
            } = error
            else {
                panic!("retain encoding phase and cause")
            };
            assert!(matches!(
                (read, operation),
                (true, Operation::ReadAdapter) | (false, Operation::PureEvaluate)
            ));
            assert_eq!(cause.operation(), "encode_failure");
            let fields = cause.details().as_value();
            assert_eq!(fields["encoding_target"], "declared_failure");
            assert_eq!(fields["original_detail"], "unavailable");
            assert_eq!(fields["original_identity"], "unavailable");
            assert_eq!(
                fields["failure_contract"],
                serde_json::to_value(mfm_program::nominal_contract_ref::<Original>().unwrap())
                    .unwrap()
            );
            assert_eq!(fields["position"]["state"], 0);
            let rendered = serde_json::to_string(&cause).unwrap();
            if panics {
                assert_eq!(fields["encoding"], "panicked");
                assert!(!rendered.contains("original panic payload marker"));
            } else {
                assert!(rendered.contains("original encoder rejected field"));
            }
            assert_eq!(ENCODINGS.load(Ordering::SeqCst) - before, 1);
            assert_eq!(observed.head_sequence(), 1);
            let document = runtime.program_document(&run).await.unwrap();
            drop(program);
            let cold = mfm_program::load(document.canonical_bytes(), &Environment).unwrap();
            let view = runtime.read(&run, &cold).await.unwrap();
            assert_eq!(view.head_digest(), observed.head_digest());
            assert!(matches!(view.state(), RunViewState::Runnable { .. }));
            assert!(view.failure().is_none());
            assert!(view.success().is_none());
            assert_eq!(ENCODINGS.load(Ordering::SeqCst) - before, 1);
        }
    }
    for panics in [false, true] {
        let input = Input { panics };
        let program = mfm_program::compile(
            EntryPointId::new("mfm.test/command-encoding@1").unwrap(),
            &Effect::<Reject, Command>::default(),
            &input,
            &Environment,
            ProgramLimits::new(0),
        )
        .unwrap();
        let runtime = Runtime::new(Arc::new(MemoryStore::new()));
        let run = RunId::from_digest(DigestBytes::from_array([154 + u8::from(panics); 32]));
        let before = ENCODINGS.load(Ordering::SeqCst);
        let Err(InvocationFailure::Execution {
            run_id,
            error,
            last_observed: Some(observed),
        }) = runtime.execute(run.clone(), &program, &input).await
        else {
            panic!("prepared encoding failure is invocation-only")
        };
        assert_eq!(run_id, run);
        assert_eq!(ENCODINGS.load(Ordering::SeqCst) - before, 1);
        let RuntimeError::Native {
            operation: Operation::EffectPrepare,
            stage: Stage::Encode,
            cause,
        } = error
        else {
            panic!("retain prepared-command encoding stage")
        };
        assert_eq!(cause.operation(), "encode");
        let rendered = serde_json::to_string(&cause).unwrap();
        if panics {
            assert!(rendered.contains("panicked"));
            assert!(!rendered.contains("original panic payload marker"));
        } else {
            assert!(rendered.contains("original encoder rejected field"));
        }
        assert_eq!(observed.head_sequence(), 1);
        let document = runtime.program_document(&run).await.unwrap();
        drop(program);
        let cold = mfm_program::load(document.canonical_bytes(), &Environment).unwrap();
        let view = runtime.read(&run, &cold).await.unwrap();
        assert_eq!(view.head_digest(), observed.head_digest());
        assert!(matches!(view.state(), RunViewState::Runnable { .. }));
        assert!(view.failure().is_none());
        assert!(view.success().is_none());
        assert_eq!(ENCODINGS.load(Ordering::SeqCst) - before, 1);
    }
}
