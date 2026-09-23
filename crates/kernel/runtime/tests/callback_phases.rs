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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum Fault {
    None,
    DecodeFailure,
    DecodePanic,
    AdapterCreatePanic,
    AdapterPollPanic,
    AdapterFailure,
    EvidenceEncodePanic,
    EvidenceEncodeFailure,
    Operational,
    ValueEncodePanic,
    ValueEncodeFailure,
    ExecutePanic,
    ExecuteFailure,
    ResultEncodePanic,
    ResultEncodeFailure,
    EvidenceDecodeFailure,
    EvidenceDecodePanic,
    BindPanic,
    BindFailure,
    InterpretPanic,
    InterpretFailure,
    OriginalEncodeFailure,
    OriginalEncodePanic,
    NativeDecodeFailure,
    NativeDecodePanic,
    NativeEncodeFailure,
    NativeEncodePanic,
    ProjectionDecodeFailure,
    ProjectionDecodePanic,
    ProjectionEncodeFailure,
    ProjectionEncodePanic,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Input {
    value: Fault,
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
    mode: Fault,
}
impl Serialize for FaultValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.mode == Fault::ValueEncodePanic {
            panic!("callback-payload-marker");
        }
        if self.mode == Fault::ValueEncodeFailure {
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
            Fault::DecodeFailure => Err(InvocationDiagnostic::from_fields(
                "fixture_decode",
                "decode_checked",
                &serde_json::json!({"cause": {"code": 73}}),
                None,
            )),
            Fault::DecodePanic => panic!("callback-payload-marker"),
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
            Fault::BindPanic => panic!("callback-payload-marker"),
            Fault::BindFailure => Err(InvocationDiagnostic::from_fields(
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
            Fault::BindPanic => panic!("callback-payload-marker"),
            Fault::BindFailure => Err(InvocationDiagnostic::from_fields(
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
                    Fault::ExecutePanic => panic!("callback-payload-marker"),
                    Fault::ExecuteFailure => Err(InvocationDiagnostic::from_fields(
                        "fixture_prepare", "prepare",
                        &serde_json::json!({"cause": {"code": 97}}), None,
                    )),
                    Fault::ResultEncodePanic => Ok(FaultValue { mode: Fault::ValueEncodePanic }),
                    Fault::ResultEncodeFailure => Ok(FaultValue { mode: Fault::ValueEncodeFailure }),
                    mode => Ok(FaultValue { mode }),
                }
            }
            fn interpret(
                input: Input,
                _: &FaultValue,
            ) -> Result<ProposedStateOutcome<Input, Never>, InvocationDiagnostic> {
                match input.value {
                    Fault::InterpretPanic => panic!("callback-payload-marker"),
                    Fault::InterpretFailure => Err(InvocationDiagnostic::from_fields(
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
fn native_codec(mode: Fault, projection: bool) -> Result<(), mfm_capabilities::CallbackFailure> {
    let fault = match (mode, projection) {
        (Fault::NativeDecodeFailure, false) | (Fault::ProjectionDecodeFailure, true) => {
            Fault::DecodeFailure
        }
        (Fault::NativeDecodePanic, false) | (Fault::ProjectionDecodePanic, true) => {
            Fault::DecodePanic
        }
        (Fault::NativeEncodeFailure, false) | (Fault::ProjectionEncodeFailure, true) => {
            Fault::ValueEncodeFailure
        }
        (Fault::NativeEncodePanic, false) | (Fault::ProjectionEncodePanic, true) => {
            Fault::ValueEncodePanic
        }
        _ => return Ok(()),
    };
    if matches!(fault, Fault::DecodeFailure | Fault::DecodePanic) {
        mfm_capabilities::codec::decode(|| {
            let bytes = serde_json::to_vec(&FaultValue { mode: fault }).unwrap();
            FaultValue::decode_checked(&bytes)
        })
        .map(|_| ())
    } else {
        mfm_capabilities::codec::encode(|| {
            mfm_values::Object::from_value(&FaultValue { mode: fault })
                .map_err(|cause| cause.into_diagnostic("nested_native_encode"))
        })
        .map(|_| ())
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
        native_codec(intent.mode, false)?;
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
        native_codec(evidence.mode, true)?;
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
        native_codec(command.mode, false)?;
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
        native_codec(evidence.mode, true)?;
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
    original: Option<custody::OriginalFault>,
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
    mode: Fault,
) -> impl Future<Output = Result<FaultValue, AdapterError<custody::FaultOriginal>>> + Send {
    assert_ne!(mode, Fault::AdapterCreatePanic, "callback-payload-marker");
    async move {
        match mode {
            Fault::AdapterPollPanic => panic!("callback-payload-marker"),
            Fault::AdapterFailure => {
                Err(AdapterError::Invariant(InvocationDiagnostic::from_fields(
                    "fixture_invariant",
                    "observe",
                    &serde_json::json!({"cause": {"code": 79}}),
                    None,
                )))
            }
            Fault::EvidenceDecodeFailure => Ok(FaultValue {
                mode: Fault::DecodeFailure,
            }),
            Fault::EvidenceDecodePanic => Ok(FaultValue {
                mode: Fault::DecodePanic,
            }),
            Fault::EvidenceEncodePanic => Ok(FaultValue {
                mode: Fault::ValueEncodePanic,
            }),
            Fault::EvidenceEncodeFailure => Ok(FaultValue {
                mode: Fault::ValueEncodeFailure,
            }),
            Fault::Operational => Err(AdapterError::Operational(custody::FaultOriginal {
                code: custody::OriginalFault::Operational,
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
            if ![
                Fault::DecodeFailure,
                Fault::AdapterFailure,
                Fault::ExecuteFailure,
                Fault::BindFailure,
                Fault::InterpretFailure,
                Fault::NativeEncodeFailure,
            ]
            .contains(&mode)
            {
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
                panic!("expected adapter failure for effect={effect}, mode={mode:?}")
            };
            // Native command decoding now qualifies preparation before an Effect can be retained.
            let boundary = if effect
                && matches!(
                    mode,
                    Fault::DecodeFailure
                        | Fault::DecodePanic
                        | Fault::NativeDecodeFailure
                        | Fault::NativeDecodePanic
                        | Fault::NativeEncodeFailure
                        | Fault::NativeEncodePanic
                ) {
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
            if matches!(
                mode,
                Fault::DecodeFailure
                    | Fault::DecodePanic
                    | Fault::NativeDecodeFailure
                    | Fault::NativeDecodePanic
                    | Fault::NativeEncodeFailure
                    | Fault::NativeEncodePanic
            ) || boundary == "prepare"
            {
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

const CASES: &[(Fault, &str, &str, &str)] = &[
    (Fault::DecodeFailure, "adapter", "decode", "decode_checked"),
    (Fault::DecodePanic, "adapter", "decode", "decode"),
    (
        Fault::AdapterCreatePanic,
        "adapter",
        "execute",
        "invoke_adapter",
    ),
    (Fault::AdapterPollPanic, "adapter", "execute", "poll"),
    (Fault::AdapterFailure, "adapter", "execute", "observe"),
    (Fault::EvidenceEncodePanic, "adapter", "encode", "encode"),
    (Fault::EvidenceEncodeFailure, "adapter", "encode", "encode"),
    (Fault::ExecutePanic, "prepare", "execute", "execute"),
    (Fault::ExecuteFailure, "prepare", "execute", "prepare"),
    (Fault::ResultEncodePanic, "prepare", "encode", "encode"),
    (Fault::ResultEncodeFailure, "prepare", "encode", "encode"),
    (
        Fault::EvidenceDecodeFailure,
        "bind",
        "decode",
        "decode_checked",
    ),
    (Fault::EvidenceDecodePanic, "bind", "decode", "decode"),
    (Fault::BindPanic, "bind", "execute", "execute"),
    (Fault::BindFailure, "bind", "execute", "bind_evidence"),
    (Fault::InterpretPanic, "interpret", "execute", "execute"),
    (Fault::InterpretFailure, "interpret", "execute", "interpret"),
    (
        Fault::NativeDecodeFailure,
        "adapter",
        "decode",
        "decode_checked",
    ),
    (Fault::NativeDecodePanic, "adapter", "decode", "decode"),
    (
        Fault::NativeEncodeFailure,
        "adapter",
        "encode",
        "nested_native_encode",
    ),
    (Fault::NativeEncodePanic, "adapter", "encode", "encode"),
    (
        Fault::ProjectionDecodeFailure,
        "bind",
        "decode",
        "decode_checked",
    ),
    (Fault::ProjectionDecodePanic, "bind", "decode", "decode"),
    (
        Fault::ProjectionEncodeFailure,
        "bind",
        "encode",
        "nested_native_encode",
    ),
    (Fault::ProjectionEncodePanic, "bind", "encode", "encode"),
];

fn assert_cause(mode: Fault, cause: &InvocationDiagnostic, diagnostic_operation: &str) {
    assert_eq!(cause.operation(), diagnostic_operation);
    let rendered = serde_json::to_string(&cause).unwrap();
    assert!(!rendered.contains("callback-payload-marker"));
    match mode {
        Fault::DecodeFailure
        | Fault::EvidenceDecodeFailure
        | Fault::NativeDecodeFailure
        | Fault::ProjectionDecodeFailure => {
            assert_eq!(cause.details().as_value()["cause"]["code"], 73)
        }
        Fault::AdapterFailure => assert_eq!(cause.details().as_value()["cause"]["code"], 79),
        Fault::EvidenceEncodeFailure
        | Fault::ResultEncodeFailure
        | Fault::NativeEncodeFailure
        | Fault::ProjectionEncodeFailure => {
            assert!(rendered.contains("reviewed evidence encoder failure"))
        }
        Fault::ExecuteFailure => assert_eq!(cause.details().as_value()["cause"]["code"], 97),
        Fault::BindFailure => assert_eq!(cause.details().as_value()["cause"]["code"], 89),
        Fault::InterpretFailure => assert_eq!(cause.details().as_value()["cause"]["code"], 101),
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
                usize::from(!matches!(
                    mode,
                    Fault::DecodeFailure
                        | Fault::DecodePanic
                        | Fault::ExecutePanic
                        | Fault::ExecuteFailure
                        | Fault::ResultEncodePanic
                        | Fault::ResultEncodeFailure
                        | Fault::NativeDecodeFailure
                        | Fault::NativeDecodePanic
                        | Fault::NativeEncodeFailure
                        | Fault::NativeEncodePanic
                ))
            );
            let (stage, cause) = match failure {
                mfm_capabilities::CallbackFailure::Decode(cause) => ("decode", cause),
                mfm_capabilities::CallbackFailure::Execute(cause) => ("execute", cause),
                mfm_capabilities::CallbackFailure::Encode(cause) => ("encode", cause),
            };
            assert_eq!(stage, expected_stage, "effect={effect}, mode={mode:?}");
            assert_cause(mode, &cause, diagnostic_operation);
        }
    }
}

async fn invoke_fault(
    effect: bool,
    mode: Fault,
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
