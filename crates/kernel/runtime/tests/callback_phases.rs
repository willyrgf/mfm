use mfm_capabilities::{AdapterError, ReadCapabilityContract};
use mfm_ids::{ContentRef, DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::{
    Classification, ClassifyError, Identity, Never, NoParams, ProgramLimits, ProposedStateOutcome,
    PureState, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{Failure, InvocationFailure, RunViewState, Runtime, RuntimeError};
use mfm_store::{MemoryStore, Store};
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Input {
    value: u64,
    continuation: String,
}

use mfm_capabilities::EffectCapabilityContract;
use mfm_program::EffectState;
use std::future::Future;

// These faults cross the real adapter boundary; no alternate transition machinery is used.
#[derive(Debug, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(decode_native = "Self::decode_checked")]
struct FaultValue {
    mode: u64,
}
impl Serialize for FaultValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.mode == 13 {
            panic!("callback-payload-marker");
        }
        if self.mode == 14 {
            return Err(serde::ser::Error::custom(
                "reviewed evidence encoder failure",
            ));
        }
        let mut value = serializer.serialize_struct("FaultValue", 1)?;
        value.serialize_field("mode", &self.mode)?;
        value.end()
    }
}
impl FaultValue {
    fn decode_checked(bytes: &[u8]) -> Result<Self, InvocationDiagnostic> {
        let value: Self = serde_json::from_slice(bytes).map_err(|source| {
            InvocationDiagnostic::from_fields(
                "json_error",
                "decode_checked",
                &mfm_canonical::JsonError::new(source),
                None,
            )
        })?;
        match value.mode {
            1 => Err(InvocationDiagnostic::from_fields(
                "fixture_decode",
                "decode_checked",
                &serde_json::json!({"cause": {"code": 73}}),
                None,
            )),
            2 => panic!("callback-payload-marker"),
            _ => Ok(value),
        }
    }
}

struct FaultCapability;
impl ReadCapabilityContract for FaultCapability {
    type Intent = FaultValue;
    type Evidence = FaultValue;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("test.callback.read@1").unwrap())
    }
    fn bind_evidence(
        _: &ContentRef,
        _: &FaultValue,
        _: &ContentRef,
        evidence: &FaultValue,
    ) -> Result<(), InvocationDiagnostic> {
        match evidence.mode {
            26 => panic!("callback-payload-marker"),
            27 => Err(InvocationDiagnostic::from_fields(
                "fixture_bind",
                "bind_evidence",
                &serde_json::json!({"cause": {"code": 89}}),
                None,
            )),
            _ => Ok(()),
        }
    }
}
impl EffectCapabilityContract for FaultCapability {
    type Command = FaultValue;
    type Evidence = FaultValue;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("test.callback.effect@1").unwrap())
    }
    fn bind_evidence(
        _: &mfm_ids::EffectId,
        _: &ContentRef,
        _: &FaultValue,
        _: &ContentRef,
        evidence: &FaultValue,
    ) -> Result<(), InvocationDiagnostic> {
        match evidence.mode {
            26 => panic!("callback-payload-marker"),
            27 => Err(InvocationDiagnostic::from_fields(
                "fixture_bind",
                "bind_evidence",
                &serde_json::json!({"cause": {"code": 89}}),
                None,
            )),
            _ => Ok(()),
        }
    }
}
struct FaultState;
impl State for FaultState {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("test.callback.state@1")?)
    }
}
macro_rules! state {
    ($mode:ident) => {
        impl $mode<FaultCapability> for FaultState {
            fn prepare(input: &Input) -> Result<FaultValue, InvocationDiagnostic> {
                match input.value {
                    20 => panic!("callback-payload-marker"),
                    21 => Err(InvocationDiagnostic::from_fields(
                        "fixture_prepare", "prepare",
                        &serde_json::json!({"cause": {"code": 97}}), None,
                    )),
                    22 => Ok(FaultValue { mode: 13 }),
                    23 => Ok(FaultValue { mode: 14 }),
                    mode => Ok(FaultValue { mode }),
                }
            }
            fn interpret(
                input: Input,
                _: &FaultValue,
            ) -> Result<ProposedStateOutcome<Input, Never>, InvocationDiagnostic> {
                match input.value {
                    28 => panic!("callback-payload-marker"),
                    29 => Err(InvocationDiagnostic::from_fields(
                        "fixture_interpret", "interpret",
                        &serde_json::json!({"cause": {"code": 101}}), None,
                    )),
                    _ => Ok(ProposedStateOutcome::Success { output: input }),
                }
            }
        }
    };
}
state!(ReadState);
state!(EffectState);
impl mfm_program::ReadSelection<FaultCapability> for FaultState {
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
}
impl mfm_program::EffectSelection<FaultCapability> for FaultState {
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
}
// Native hooks nest codec phases inside the generic execution worker.
fn native_codec(mode: u64) -> Result<(), mfm_capabilities::CallbackFailure> {
    match mode {
        40 | 41 => mfm_capabilities::codec::decode(|| {
            FaultValue::decode_checked(if mode == 40 {
                br#"{"mode":1}"#
            } else {
                br#"{"mode":2}"#
            })
        })
        .map(|_| ()),
        42 | 43 => mfm_capabilities::codec::encode(|| {
            mfm_values::Object::from_value(&FaultValue {
                mode: if mode == 42 { 14 } else { 13 },
            })
            .map_err(|cause| cause.into_diagnostic("nested_native_encode"))
        })
        .map(|_| ()),
        _ => Ok(()),
    }
}
struct Native;
impl mfm_capabilities::ReadImplementation<FaultCapability> for Native {
    type Binding = NoParams;
    type NativeIntent = FaultValue;
    type NativeEvidence = FaultValue;
    type OperationalError = custody::FaultOriginal;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("test.callback.native-read@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        intent: &FaultValue,
    ) -> Result<FaultValue, mfm_capabilities::CallbackFailure> {
        native_codec(intent.mode)?;
        Ok(FaultValue { mode: intent.mode })
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &FaultValue,
        _: &ContentRef,
        _: &FaultValue,
        evidence: &FaultValue,
        _: &mfm_values::Object,
    ) -> Result<FaultValue, mfm_capabilities::CallbackFailure> {
        if (50..=53).contains(&evidence.mode) {
            native_codec(evidence.mode - 10)?;
        }
        Ok(FaultValue {
            mode: evidence.mode,
        })
    }
}
impl mfm_capabilities::EffectImplementation<FaultCapability> for Native {
    type Binding = NoParams;
    type NativeCommand = FaultValue;
    type NativeEvidence = FaultValue;
    type OperationalError = custody::FaultOriginal;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("test.callback.native-effect@1")?)
    }
    fn decode_command(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        reference: &ContentRef,
        command: &FaultValue,
    ) -> Result<(ContentRef, FaultValue), mfm_capabilities::CallbackFailure> {
        native_codec(command.mode)?;
        Ok((reference.clone(), FaultValue { mode: command.mode }))
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &mfm_ids::EffectId,
        _: &ContentRef,
        _: &FaultValue,
        _: &FaultValue,
        evidence: &FaultValue,
        _: &mfm_values::Object,
    ) -> Result<FaultValue, mfm_capabilities::CallbackFailure> {
        if (50..=53).contains(&evidence.mode) {
            native_codec(evidence.mode - 10)?;
        }
        Ok(FaultValue {
            mode: evidence.mode,
        })
    }
}
impl mfm_program::InjectRead<FaultState, FaultCapability> for Native {
    type Prefix = Identity<Input>;
    type Suffix = Identity<Input>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
impl mfm_program::InjectEffect<FaultState, FaultCapability> for Native {
    type Prefix = Identity<Input>;
    type Suffix = Identity<Input>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
type ReadSource = mfm_program::ResolvedRead<FaultState, FaultCapability, Native>;
type EffectSource = mfm_program::ResolvedEffect<FaultState, FaultCapability, Native>;
struct Resources<S> {
    entered: Arc<AtomicUsize>,
    original: Option<u64>,
    source: std::marker::PhantomData<fn() -> S>,
}
impl<S> Default for Resources<S> {
    fn default() -> Self {
        Self {
            entered: Arc::new(AtomicUsize::new(0)),
            original: None,
            source: std::marker::PhantomData,
        }
    }
}
impl<S> Clone for Resources<S> {
    fn clone(&self) -> Self {
        Self {
            entered: self.entered.clone(),
            original: self.original,
            source: std::marker::PhantomData,
        }
    }
}
impl<S> mfm_program::ProgramEnvironment for Resources<S> {
    type Sources = S;
}
impl<S: 'static> mfm_program::BindRead<FaultCapability, Native> for Resources<S> {
    type Adapter = Self;
    fn bind_read(&self, _: &NoParams) -> Result<Self, InvocationDiagnostic> {
        Ok(self.clone())
    }
}
impl<S: 'static> mfm_program::BindEffect<FaultCapability, Native> for Resources<S> {
    type Adapter = Self;
    fn bind_effect(&self, _: &NoParams) -> Result<Self, InvocationDiagnostic> {
        Ok(self.clone())
    }
}
impl<S: 'static> mfm_capabilities::ReadAdapter<FaultValue, FaultValue, custody::FaultOriginal>
    for Resources<S>
{
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        intent: &'a FaultValue,
    ) -> std::pin::Pin<
        Box<
            dyn Future<Output = Result<FaultValue, AdapterError<custody::FaultOriginal>>>
                + Send
                + 'a,
        >,
    > {
        self.entered.fetch_add(1, Ordering::SeqCst);
        Box::pin(observe(intent.mode))
    }
}
impl<S: 'static> mfm_capabilities::EffectAdapter<FaultValue, FaultValue, custody::FaultOriginal>
    for Resources<S>
{
    fn invoke<'a>(
        &'a self,
        _: &'a mfm_ids::EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        command: &'a FaultValue,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        mfm_capabilities::EffectAdapterOutcome<FaultValue>,
                        AdapterError<custody::FaultOriginal>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.entered.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.original {
            return Box::pin(async move {
                Err(AdapterError::Operational(custody::FaultOriginal { code }))
            });
        }
        let result = observe(command.mode);
        Box::pin(async move {
            result
                .await
                .map(mfm_capabilities::EffectAdapterOutcome::Settled)
        })
    }
}

// Function construction and future polling are separate failure points.
fn observe(
    mode: u64,
) -> impl Future<Output = Result<FaultValue, AdapterError<custody::FaultOriginal>>> + Send {
    assert_ne!(mode, 3, "callback-payload-marker");
    async move {
        match mode {
            4 => panic!("callback-payload-marker"),
            5 => Err(AdapterError::Invariant(InvocationDiagnostic::from_fields(
                "fixture_invariant",
                "observe",
                &serde_json::json!({"cause": {"code": 79}}),
                None,
            ))),
            24 => Ok(FaultValue { mode: 1 }),
            25 => Ok(FaultValue { mode: 2 }),
            6 => Ok(FaultValue { mode: 13 }),
            7 => Ok(FaultValue { mode: 14 }),
            8 => Err(AdapterError::Operational(custody::FaultOriginal {
                code: 83,
            })),
            _ => Ok(FaultValue { mode }),
        }
    }
}

#[tokio::test]
async fn read_and_effect_callbacks_preserve_operation_phase_and_acknowledged_head() {
    for effect in [false, true] {
        for &(mode, boundary, expected_stage, diagnostic_operation) in CASES {
            // Codec variants are covered directly below. Runtime owns operation provenance,
            // acknowledgement, and inspection of the retained continuation.
            if ![1, 5, 21, 27, 29, 42].contains(&mode) {
                continue;
            }
            let store = Arc::new(MemoryStore::new());
            let entered = Arc::new(AtomicUsize::new(0));
            let runtime = Runtime::new(store.clone());
            let input = Input {
                value: mode,
                continuation: "phase boundary".into(),
            };
            let entry = EntryPointId::new("mfm.test/callback-phases@1").unwrap();
            let program = if effect {
                mfm_program::compile(
                    entry,
                    &EffectSource::new(NoParams),
                    &input,
                    &Resources::<EffectSource> {
                        entered: entered.clone(),
                        ..Default::default()
                    },
                    ProgramLimits::new(0),
                )
            } else {
                mfm_program::compile(
                    entry,
                    &ReadSource::new(NoParams),
                    &input,
                    &Resources::<ReadSource> {
                        entered: entered.clone(),
                        ..Default::default()
                    },
                    ProgramLimits::new(0),
                )
            }
            .unwrap();
            let run = RunId::from_digest(DigestBytes::from_array([181; 32]));
            let Err(InvocationFailure::Execution {
                error:
                    RuntimeError::Native {
                        operation,
                        stage,
                        cause,
                    },
                last_observed: Some(observed),
                ..
            }) = runtime.start(run.clone(), &program, &input).await
            else {
                panic!("expected adapter failure for effect={effect}, mode={mode}")
            };
            // Native command decoding now qualifies preparation before an Effect can be retained.
            let boundary = if effect && (mode <= 2 || (40..=43).contains(&mode)) {
                "prepare"
            } else {
                boundary
            };
            assert_eq!(
                serde_json::to_value(operation).unwrap(),
                format!("{}_{boundary}", if effect { "effect" } else { "read" })
            );
            assert_eq!(serde_json::to_value(stage).unwrap(), expected_stage);
            assert_cause(mode, &cause, diagnostic_operation);
            if mode <= 2 || (40..=43).contains(&mode) || boundary == "prepare" {
                assert_eq!(entered.load(Ordering::SeqCst), 0);
            }
            let head = store.load_run(&run, None).await.unwrap().unwrap();
            let expected_head = if !effect || boundary == "prepare" {
                1
            } else if boundary == "interpret" {
                3
            } else {
                2
            };
            assert_eq!(head.head().head_sequence(), expected_head);
            assert_eq!(head.head().head_digest(), observed.head_digest());
            let calls_before_inspection = entered.load(Ordering::SeqCst);
            let program = if effect {
                mfm_program::load(
                    program.canonical_bytes(),
                    &Resources::<EffectSource> {
                        entered: entered.clone(),
                        ..Default::default()
                    },
                )
            } else {
                mfm_program::load(
                    program.canonical_bytes(),
                    &Resources::<ReadSource> {
                        entered: entered.clone(),
                        ..Default::default()
                    },
                )
            }
            .unwrap();
            let cold = runtime.read(&run, &program).await.unwrap();
            assert_eq!(entered.load(Ordering::SeqCst), calls_before_inspection);
            assert_eq!(cold.head_digest(), observed.head_digest());
            assert!(match expected_head {
                1 => matches!(cold.state(), RunViewState::Runnable { .. }),
                2 => matches!(cold.state(), RunViewState::EffectPending { .. }),
                3 => matches!(cold.state(), RunViewState::AwaitingInterpretation { .. }),
                _ => unreachable!(),
            });
        }
    }
}

#[path = "callback_phases/custody.rs"]
mod custody;

#[path = "callback_phases/pure.rs"]
mod pure;

const CASES: &[(u64, &str, &str, &str)] = &[
    (1, "adapter", "decode", "decode_checked"),
    (2, "adapter", "decode", "decode"),
    (3, "adapter", "execute", "invoke_adapter"),
    (4, "adapter", "execute", "poll"),
    (5, "adapter", "execute", "observe"),
    (6, "adapter", "encode", "encode"),
    (7, "adapter", "encode", "encode"),
    (20, "prepare", "execute", "execute"),
    (21, "prepare", "execute", "prepare"),
    (22, "prepare", "encode", "encode"),
    (23, "prepare", "encode", "encode"),
    (24, "bind", "decode", "decode_checked"),
    (25, "bind", "decode", "decode"),
    (26, "bind", "execute", "execute"),
    (27, "bind", "execute", "bind_evidence"),
    (28, "interpret", "execute", "execute"),
    (29, "interpret", "execute", "interpret"),
    (40, "adapter", "decode", "decode_checked"),
    (41, "adapter", "decode", "decode"),
    (42, "adapter", "encode", "nested_native_encode"),
    (43, "adapter", "encode", "encode"),
    (50, "bind", "decode", "decode_checked"),
    (51, "bind", "decode", "decode"),
    (52, "bind", "encode", "nested_native_encode"),
    (53, "bind", "encode", "encode"),
];

fn assert_cause(mode: u64, cause: &InvocationDiagnostic, diagnostic_operation: &str) {
    assert_eq!(cause.operation(), diagnostic_operation);
    let rendered = serde_json::to_string(&cause).unwrap();
    assert!(!rendered.contains("callback-payload-marker"));
    match mode {
        1 | 24 | 40 | 50 => assert_eq!(cause.details().as_value()["cause"]["code"], 73),
        5 => assert_eq!(cause.details().as_value()["cause"]["code"], 79),
        7 | 23 | 42 | 52 => assert!(rendered.contains("reviewed evidence encoder failure")),
        21 => assert_eq!(cause.details().as_value()["cause"]["code"], 97),
        27 => assert_eq!(cause.details().as_value()["cause"]["code"], 89),
        29 => assert_eq!(cause.details().as_value()["cause"]["code"], 101),
        _ => assert_eq!(cause.code(), "task_failure"),
    }
}

// Exercise every serializer, decoder, nested native hook, invocation and future-poll
// fault at its Program owner without constructing or replaying a run for each variant.
#[tokio::test]
async fn program_callbacks_preserve_all_fault_phases_and_causes() {
    for effect in [false, true] {
        for &(mode, _, expected_stage, diagnostic_operation) in CASES {
            let entered = Arc::new(AtomicUsize::new(0));
            let failure = invoke_fault(effect, mode, entered.clone())
                .await
                .unwrap_err();
            assert_eq!(
                entered.load(Ordering::SeqCst),
                usize::from(!matches!(mode, 1 | 2 | 20..=23 | 40..=43))
            );
            let (stage, cause) = match failure {
                mfm_capabilities::CallbackFailure::Decode(cause) => ("decode", cause),
                mfm_capabilities::CallbackFailure::Execute(cause) => ("execute", cause),
                mfm_capabilities::CallbackFailure::Encode(cause) => ("encode", cause),
            };
            assert_eq!(stage, expected_stage, "effect={effect}, mode={mode}");
            assert_cause(mode, &cause, diagnostic_operation);
        }
    }
}

async fn invoke_fault(
    effect: bool,
    mode: u64,
    entered: Arc<AtomicUsize>,
) -> Result<(), mfm_capabilities::CallbackFailure> {
    use mfm_program::callback;
    use mfm_values::Object;
    let binding = Object::from_value(&NoParams).unwrap();
    // This test enters monomorphized callbacks directly. Program association has its
    // own tests; the explicit reference here is never treated as an admitted ABI.
    let implementation = binding.value_ref().clone();
    let state_failure = mfm_program::nominal_contract_ref::<Never>().unwrap();
    let adapter_failure = mfm_program::nominal_contract_ref::<custody::FaultOriginal>().unwrap();
    let position = mfm_ids::ExecutionPosition {
        state: mfm_ids::StatePosition::new(0).unwrap(),
        visit: mfm_ids::VisitId::new(0),
    };
    let input = Object::from_value(&Input {
        value: mode,
        continuation: "phase boundary".into(),
    })
    .unwrap();
    if effect {
        let callbacks = callback::EffectCallbacks::new::<FaultState, FaultCapability, Native>(
            state_failure,
            implementation.clone(),
            binding.value_ref().clone(),
            Arc::new(NoParams),
        );
        let adapter = callback::effect_adapter::<FaultCapability, Native, _>(
            adapter_failure,
            implementation,
            binding.value_ref().clone(),
            Arc::new(NoParams),
            Resources::<EffectSource> {
                entered,
                ..Default::default()
            },
        );
        let command = (callbacks.prepare)(input.clone()).await?;
        (callbacks.validate_command)(command.clone()).await?;
        let effect_id = mfm_ids::EffectId::from_digest(DigestBytes::from_array([9; 32]));
        let result = adapter(position, &effect_id, &command).await?.unwrap();
        let mfm_capabilities::EffectAdapterOutcome::Settled(evidence) = result else {
            panic!("unexpected pending")
        };
        (callbacks.bind)(effect_id.clone(), command.clone(), evidence.clone()).await?;
        (callbacks.interpret)(input, effect_id, command, evidence, position).await?;
    } else {
        let callbacks = callback::ReadCallbacks::new::<FaultState, FaultCapability, Native>(
            state_failure,
            implementation.clone(),
            binding.value_ref().clone(),
            Arc::new(NoParams),
        );
        let adapter = callback::read_adapter::<FaultCapability, Native, _>(
            adapter_failure,
            implementation,
            binding.value_ref().clone(),
            Arc::new(NoParams),
            Resources::<ReadSource> {
                entered,
                ..Default::default()
            },
        );
        let intent = (callbacks.prepare)(input.clone()).await?;
        let evidence = adapter(position, &intent).await?.unwrap();
        (callbacks.complete)(input, intent, evidence, position)
            .await
            .map_err(|failure| match failure {
                callback::ReadCompletionFailure::Bind(cause)
                | callback::ReadCompletionFailure::Interpret(cause) => cause,
            })?;
    }
    Ok(())
}
