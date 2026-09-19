mod admission;
mod barrier;
mod capacity;
mod native_abis;
mod original_encoding;
mod pending;
mod recovery;

use super::*;
use crate::Runtime;
use mfm_capabilities::{
    AdapterError, CapabilityError, EffectAdapter, EffectCapabilityContract, EffectImplementation,
};
use mfm_ids::{DigestBytes, EntryPointId, StableId};
use mfm_program::{
    BindEffect, CapabilityFamily, Classification, ClassifyError, Effect, EffectSelection,
    EffectState, Identity, InjectEffect, Never, NoParams, OperationDefaults, OperationDefinition,
    Plan, PolicyValues, ProgramEnvironment, ProgramError, ProgramLimits, Resolve, ResolveDefaults,
    ResolveEffectBinding, StandardRecovery, State,
};
use std::{future::Future, pin::Pin};

use mfm_program_derive::MfmValue;
use mfm_store::MemoryStore;
use serde::Deserialize;
use std::sync::Mutex;

// Scope the fault to this test's run; it is consumed once before any projection is constructed.
static PROJECTION_FAULT: Mutex<Option<(RunId, u64, usize)>> = Mutex::new(None);

pub(super) fn projection_fault(driver: &Driver) -> Result<()> {
    let mut fault = PROJECTION_FAULT.lock().unwrap();
    if let Some((run, sequence, skip)) = fault.as_mut() {
        if run == driver.head.run_id() && *sequence == driver.head.head_sequence() {
            if *skip > 0 {
                *skip -= 1;
            } else {
                *fault = None;
                return Err(RuntimeError::at(
                    Operation::Project,
                    Stage::Project,
                    InvocationDiagnostic::from_fields("projection_test", "view", &"injected", None),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Outage {}
impl ClassifyError for Outage {
    fn classify(&self) -> Classification {
        Classification::Retryable
    }
}
struct Mutation;
impl EffectCapabilityContract for Mutation {
    type Command = NoParams;
    type Evidence = NoParams;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.projection/mutation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &EffectId,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &NoParams,
    ) -> std::result::Result<(), InvocationDiagnostic> {
        Ok(())
    }
}
struct Execute;
impl State for Execute {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.projection/execute@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl EffectState<Mutation> for Execute {
    fn prepare(_: &NoParams) -> std::result::Result<NoParams, InvocationDiagnostic> {
        Ok(NoParams)
    }
    fn interpret(
        _: NoParams,
        _: &NoParams,
    ) -> std::result::Result<ProposedStateOutcome<NoParams, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: NoParams })
    }
}
impl EffectSelection<Mutation> for Execute {
    type ExpandedInput = NoParams;
    type ExpandedOutput = NoParams;
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct NativeReceipt {
    accepted: bool,
}
struct Native;
impl EffectImplementation<Mutation> for Native {
    type Binding = NoParams;
    type NativeCommand = NoParams;
    type NativeEvidence = NativeReceipt;
    type OperationalError = Outage;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.projection/native@1")?)
    }
    fn decode_command(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        command_ref: &ContentRef,
        _: &NoParams,
    ) -> std::result::Result<(ContentRef, NoParams), mfm_capabilities::CallbackFailure> {
        Ok((command_ref.clone(), NoParams))
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &EffectId,
        _: &ContentRef,
        _: &NoParams,
        _: &NoParams,
        receipt: &NativeReceipt,
        _: &Object,
    ) -> std::result::Result<NoParams, mfm_capabilities::CallbackFailure> {
        if !receipt.accepted {
            return Err(InvocationDiagnostic::from_fields(
                "rejected_receipt",
                "project_evidence",
                &false,
                None,
            )
            .into());
        }
        Ok(NoParams)
    }
}
impl InjectEffect<Execute, Mutation> for Native {
    type Prefix = Identity<NoParams>;
    type Suffix = Identity<NoParams>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
impl ResolveEffectBinding<bool, Mutation> for Native {
    fn binding(_: &bool) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
}
#[derive(Clone)]
struct Resources {
    calls: Arc<Mutex<Vec<EffectId>>>,
    operational: bool,
    settled: bool,
}
impl ProgramEnvironment for Resources {
    type Sources = (
        mfm_program::Operation<Flow<true>, Policy>,
        mfm_program::Pure<Fail>,
    );
}
impl CapabilityFamily<Mutation> for Resources {
    type Implementations = (Native,);
}
impl Resolve<bool, Mutation> for Resources {
    fn implementation(_: &bool) -> mfm_program::Result<StableId> {
        Ok(Native::implementation_id()?)
    }
}
impl BindEffect<Mutation, Native> for Resources {
    type Adapter = Self;
    fn bind_effect(&self, _: &NoParams) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(self.clone())
    }
}
impl EffectAdapter<NoParams, NativeReceipt, Outage> for Resources {
    fn invoke<'a>(
        &'a self,
        effect_id: &'a EffectId,
        _: &'a ContentRef,
        _: &'a ContentRef,
        _: &'a NoParams,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = std::result::Result<
                        EffectAdapterOutcome<NativeReceipt>,
                        AdapterError<Outage>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.calls.lock().unwrap().push(effect_id.clone());
        Box::pin(async move {
            if self.operational {
                Err(AdapterError::Operational(Outage {}))
            } else if self.settled {
                Ok(EffectAdapterOutcome::Settled(NativeReceipt {
                    accepted: true,
                }))
            } else {
                Ok(EffectAdapterOutcome::Pending)
            }
        })
    }
}
#[derive(Default)]
struct Flow<const RETRY: bool>;
impl<const RETRY: bool> OperationDefinition for Flow<RETRY> {
    type Body = Effect<Execute, Mutation>;
}
impl<const RETRY: bool> Plan<NoParams> for Flow<RETRY> {
    type Config = bool;
    fn plan<'a>(&'a self, _: &'a NoParams) -> mfm_program::Result<(&'a bool, Self::Body)> {
        Ok((&RETRY, Effect::default()))
    }
}
struct Policy;
impl OperationDefaults for Policy {
    type Handler = StandardRecovery;
    type Targets = ();
}
impl ResolveDefaults<bool> for Policy {
    fn resolve(retry: &bool) -> mfm_program::Result<PolicyValues<StandardRecovery>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(u32::from(*retry)),
            restarts: Some(0),
        })
    }
}

#[tokio::test]
async fn projection_acknowledgement_requires_insertion_and_retains_previous_observation() {
    // The last two cases enter both recovered-record projection branches: Retry yields and Stop
    // returns RecoveryStopped. Fault injection begins after the shared admission encoding boundary.
    for (case, fail_sequence, skip, resume, operational, retry, previous_sequence) in [
        (0, 1, 0, false, false, false, None), // initial admission inserted
        (1, 2, 0, false, false, false, Some(1)), // prepared continuation inserted
        (2, 2, 0, true, false, false, None),  // resume's initial projection, no insertion
        (3, 2, 1, true, false, false, Some(2)), // Pending projection, no insertion
        (4, 4, 0, false, true, true, Some(3)), // recovery Retry inserted
        (5, 4, 0, false, true, false, Some(3)), // recovery Stop inserted
    ] {
        let run = RunId::from_digest(DigestBytes::from_array([220 + case; 32]));
        let store = Arc::new(MemoryStore::new());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resources = Resources {
            calls: Arc::clone(&calls),
            operational,
            settled: false,
        };
        let runtime = Runtime::new(store.clone());
        let entry = EntryPointId::new("mfm.test/projection-acknowledgement@1").unwrap();
        let program = if retry {
            mfm_program::compile(
                entry,
                &mfm_program::Operation::<Flow<true>, Policy>::default(),
                &NoParams,
                &resources,
                ProgramLimits::new(1),
            )
        } else {
            mfm_program::compile(
                entry,
                &mfm_program::Operation::<Flow<false>, Policy>::default(),
                &NoParams,
                &resources,
                ProgramLimits::new(1),
            )
        }
        .unwrap();
        let pending = if resume {
            Some(
                admit(
                    store.clone(),
                    run.clone(),
                    program.clone(),
                    Object::from_value(&NoParams).unwrap(),
                    Advancement::Manual,
                )
                .await
                .unwrap(),
            )
        } else {
            None
        };
        let before_calls = calls.lock().unwrap().len();
        *PROJECTION_FAULT.lock().unwrap() = Some((run.clone(), fail_sequence, skip));
        let result = if resume {
            runtime.resume(&run, &program).await
        } else {
            admit(
                store.clone(),
                run.clone(),
                program.clone(),
                Object::from_value(&NoParams).unwrap(),
                Advancement::Manual,
            )
            .await
        };
        let InvocationFailure::Execution {
            run_id,
            error,
            last_observed,
        } = result.err().unwrap()
        else {
            panic!("projection failure must end the invocation")
        };
        assert!(
            PROJECTION_FAULT.lock().unwrap().is_none(),
            "projection fault must be reached"
        );
        assert_eq!(run_id, run);
        let loaded = store
            .load_run(&run, previous_sequence)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.head().head_sequence(), fail_sequence);
        let cause = if resume {
            assert!(
                !matches!(error, RuntimeError::Projection { .. }),
                "no insertion in this invocation"
            );
            let pending = pending.as_ref().unwrap();
            assert_eq!(loaded.head().head_digest(), pending.head_digest());
            &error
        } else {
            let RuntimeError::Projection {
                acknowledged,
                cause,
            } = &error
            else {
                panic!("known insertion must retain its mechanical acknowledgement")
            };
            assert_eq!(acknowledged.as_ref(), loaded.head());
            cause.as_ref()
        };
        let RuntimeError::Native {
            operation: Operation::Project,
            stage: Stage::Project,
            cause,
        } = cause
        else {
            panic!("the producing projection diagnostic must survive")
        };
        assert_eq!(cause.code(), "projection_test");
        assert_eq!(cause.details().as_value(), &serde_json::json!("injected"));
        assert_eq!(
            last_observed.as_ref().map(|view| view.head_sequence()),
            previous_sequence
        );
        if let Some(previous) = last_observed {
            let frame = decode_frame(loaded.probe().unwrap()).unwrap();
            assert_eq!(previous.head_digest(), frame.head_digest());
            if resume {
                assert!(matches!(
                    previous.state(),
                    RunViewState::EffectPending { .. }
                ));
            } else if operational {
                assert!(matches!(
                    previous.state(),
                    RunViewState::AwaitingRecovery { .. }
                ));
            } else {
                assert!(matches!(previous.state(), RunViewState::Runnable { .. }));
            }
        }
        assert_eq!(
            calls.lock().unwrap().len() - before_calls,
            usize::from(operational || (resume && skip == 1))
        );
        // Inspection succeeds after the one-shot failure and cannot advance the acknowledged head.
        assert_eq!(
            runtime.read(&run, &program).await.unwrap().head_digest(),
            loaded.head().head_digest()
        );
    }
}

struct Fail;
impl State for Fail {
    type Input = NoParams;
    type Output = NoParams;
    type Failure = Outage;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.projection/fail@1")?)
    }
}
impl mfm_program::PureState for Fail {
    fn evaluate(
        _: NoParams,
    ) -> std::result::Result<ProposedStateOutcome<NoParams, Outage>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Failure { failure: Outage {} })
    }
}

#[tokio::test]
async fn admission_checks_input_and_cold_reports_retain_the_complete_original_call() {
    let store = Arc::new(MemoryStore::new());
    let runtime = Runtime::new(store.clone());
    let resources = Resources {
        calls: Arc::new(Mutex::new(Vec::new())),
        operational: false,
        settled: false,
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/original-report@1").unwrap(),
        &mfm_program::Pure::<Fail>::default(),
        &NoParams,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([239; 32]));
    let rejected = admit(
        store.clone(),
        run.clone(),
        program.clone(),
        Object::from_value(&Outage {}).unwrap(),
        Advancement::Manual,
    )
    .await;
    assert!(matches!(
        rejected,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Native {
                operation: Operation::Admission,
                stage: Stage::Decode,
                ..
            },
            last_observed: None,
            ..
        })
    ));
    assert!(store.load_run(&run, None).await.unwrap().is_none());
    let hot = admit(
        store.clone(),
        run.clone(),
        program.clone(),
        Object::from_value(&NoParams).unwrap(),
        Advancement::Manual,
    )
    .await
    .unwrap();
    let RunViewState::Failed(report) = hot.state() else {
        panic!("expected terminal original failure")
    };
    let wire: serde_json::Value = serde_json::from_slice(report.canonical_bytes()).unwrap();
    assert_eq!(wire["domain"], "mfm.failure-report.v6");
    assert_eq!(wire["run_id"], serde_json::to_value(&run).unwrap());
    assert_eq!(
        wire["program_ref"],
        serde_json::to_value(program.content_ref()).unwrap()
    );
    assert_eq!(
        wire["state_implementation_ref"],
        serde_json::to_value(program.declarations()[0].state_implementation_ref()).unwrap()
    );
    assert_eq!(wire["execution"], serde_json::json!({"kind":"pure"}));
    assert_eq!(
        wire["failure"],
        serde_json::to_value(report.failure()).unwrap()
    );
    let Failure::Domain {
        call: StateCall::Pure(call),
        original,
    } = report.failure()
    else {
        panic!("expected complete Pure failure")
    };
    assert_eq!(call.input(), &Object::from_value(&NoParams).unwrap());
    original.decode::<Outage>().unwrap();
    assert!(wire.get("root").is_none());
    assert!(wire.get("cause").is_none());
    let document = runtime.program_document(&run).await.unwrap();
    let cold_program = mfm_program::load(document.canonical_bytes(), &resources).unwrap();
    let cold = Runtime::new(store.clone())
        .read(&run, &cold_program)
        .await
        .unwrap();
    let RunViewState::Failed(cold_report) = cold.state() else {
        panic!("expected retained failure")
    };
    assert_eq!(cold_report.canonical_bytes(), report.canonical_bytes());
    assert_eq!(cold_report.value_ref(), report.value_ref());
    assert_eq!(cold.head_digest(), hot.head_digest());
    let different = mfm_program::compile(
        EntryPointId::new("mfm.test/different-program@1").unwrap(),
        &mfm_program::Pure::<Fail>::default(),
        &NoParams,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert!(matches!(
        runtime.resume(&run, &different).await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Native {
                operation: Operation::Restore,
                stage: Stage::Decode,
                ..
            },
            last_observed: None,
            ..
        })
    ));
    assert_eq!(
        store
            .load_run(&run, None)
            .await
            .unwrap()
            .unwrap()
            .head()
            .head_digest(),
        hot.head_digest()
    );
    assert_eq!(resources.calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn acknowledged_settlement_retains_native_evidence_through_cold_inspection() {
    let store = Arc::new(MemoryStore::new());
    let resources = Resources {
        calls: Arc::new(Mutex::new(Vec::new())),
        operational: false,
        settled: true,
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/native-settlement@1").unwrap(),
        &mfm_program::Operation::<Flow<false>, Policy>::default(),
        &NoParams,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([240; 32]));
    let hot = admit(
        store.clone(),
        run.clone(),
        program,
        Object::from_value(&NoParams).unwrap(),
        Advancement::Manual,
    )
    .await
    .unwrap();
    let RunViewState::Succeeded(output) = hot.state() else {
        panic!("expected interpreted settlement")
    };
    output.decode::<NoParams>().unwrap();
    let loaded = store.load_run(&run, Some(3)).await.unwrap().unwrap();
    let settled = decode_record(&decode_frame(loaded.probe().unwrap()).unwrap()).unwrap();
    let RecordedOperation::EffectSettled(settlement) = &settled.operation else {
        panic!("settlement must precede interpretation")
    };
    assert!(
        settlement
            .evidence
            .decode::<NativeReceipt>()
            .unwrap()
            .accepted
    );
    assert!(settlement.evidence.decode::<NoParams>().is_err());
    let completed = decode_record(&decode_frame(loaded.latest()).unwrap()).unwrap();
    let RecordedOperation::Succeeded {
        call: StateCall::Effect(retained),
        ..
    } = &completed.operation
    else {
        panic!("completion retains the acknowledged settlement")
    };
    assert_eq!(retained, settlement);
    let runtime = Runtime::new(store.clone());
    let document = runtime.program_document(&run).await.unwrap();
    let cold_program = mfm_program::load(document.canonical_bytes(), &resources).unwrap();
    for cold in [
        runtime.read(&run, &cold_program).await.unwrap(),
        runtime.resume(&run, &cold_program).await.unwrap(),
    ] {
        assert_eq!(cold.head_digest(), hot.head_digest());
        let RunViewState::Succeeded(cold_output) = cold.state() else {
            panic!("cold result changed")
        };
        assert_eq!(cold_output, output);
    }
    assert_eq!(resources.calls.lock().unwrap().len(), 1);
}
