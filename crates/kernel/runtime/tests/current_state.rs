use mfm_capabilities::{AdapterError, ReadCapabilityContract};
use mfm_ids::{ContentRef, DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::{
    Classification, ClassifyError, Handler, HandlerBinding, Identity, Never, NoParams, Occurrence,
    Operation, OperationExpansion, ProgramError, ProgramLimits, ProposedStateOutcome, PureState,
    ReadState, RecoveryAllowances, RecoveryContext, RecoveryRequest, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{
    Failure, InvocationFailure, RunViewState, Runtime, RuntimeAssemblyBuilder, RuntimeError,
};
use mfm_store::{MemoryStore, Store};
use mfm_values::NativeCause;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Input {
    value: u64,
    continuation: String,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Request {
    value: u64,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Outage {
    deadline_ms: u64,
}
static CLASSIFICATIONS: AtomicUsize = AtomicUsize::new(0);
impl ClassifyError for Outage {
    fn classify(&self) -> Classification {
        CLASSIFICATIONS.fetch_add(1, Ordering::SeqCst);
        Classification::Retryable
    }
}
#[derive(Debug, Serialize, thiserror::Error)]
#[error("policy unavailable")]
struct PolicyUnavailable {
    operation: &'static str,
}
static HANDLER_AVAILABLE: AtomicBool = AtomicBool::new(false);
struct Policy;
impl Handler for Policy {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-policy@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        _: Classification,
        _: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, NativeCause> {
        if HANDLER_AVAILABLE.load(Ordering::SeqCst) {
            Ok(RecoveryRequest::RetryState)
        } else {
            Err(NativeCause::from_error(PolicyUnavailable {
                operation: "select_retry",
            }))
        }
    }
}
struct Increment;
impl State for Increment {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-increment@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Increment {
    fn evaluate(mut input: Input) -> Result<ProposedStateOutcome<Input, Never>, NativeCause> {
        input.value += 1;
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
struct Read;
struct Observation;
impl State for Read {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-read@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl ReadCapabilityContract for Observation {
    type Intent = Request;
    type Evidence = Request;
    type OperationalError = Outage;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.current-observation@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &ContentRef,
        intent: &Request,
        evidence: &Request,
    ) -> Result<(), NativeCause> {
        if intent.value == evidence.value {
            Ok(())
        } else {
            Err(NativeCause::from_error(
                mfm_capabilities::CapabilityError::EvidenceBinding,
            ))
        }
    }
}
impl mfm_program::CapabilityInjection<Read> for Observation {
    type Setup = NoParams;
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &NoParams) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
impl ReadState<Observation> for Read {
    fn prepare(input: &Input) -> Result<Request, NativeCause> {
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, NativeCause> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
struct Flow;
impl Operation for Flow {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Input, Input, Never>,
    ) -> mfm_program::Result<()> {
        body.handler(HandlerBinding::new::<Policy>(NoParams)?)?;
        body.allowances(RecoveryAllowances::new(1, 0))?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())?;
        body.read::<Read, Observation, Identity<Never>>(&NoParams, NoParams, Occurrence::new())
    }
}
fn runtime(store: Arc<dyn Store>, available: Arc<AtomicBool>, calls: Arc<AtomicUsize>) -> Runtime {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder.register_pure::<Increment>().unwrap();
    builder.register_read::<Read, Observation>().unwrap();
    builder.register_handler::<Policy>().unwrap();
    builder
        .register_adapter::<Observation, _, _>(NoParams, move |_, intent| {
            calls.fetch_add(1, Ordering::SeqCst);
            let available = available.load(Ordering::SeqCst);
            let value = intent.value;
            Box::pin(async move {
                if available {
                    Ok(Request { value })
                } else {
                    Err(AdapterError::Operational(Outage { deadline_ms: 731 }))
                }
            })
        })
        .unwrap();
    Runtime::new(builder.finish(), store)
}

#[tokio::test]
async fn original_commits_before_policy_failure_and_cold_resume_retries_only_recovery() {
    CLASSIFICATIONS.store(0, Ordering::SeqCst);
    HANDLER_AVAILABLE.store(false, Ordering::SeqCst);
    let store = Arc::new(MemoryStore::new());
    let available = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let input = Input {
        value: 9,
        continuation: "complete caller continuation".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/current@1").unwrap(),
        &Flow,
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([121; 32]));
    let runtime = runtime(store.clone(), available.clone(), calls.clone());
    let error = runtime
        .start(run.clone(), program, input)
        .await
        .err()
        .expect("handler failure");
    let InvocationFailure::Execution {
        error: RuntimeError::Native { cause, .. },
        last_observed: Some(observed),
        ..
    } = error
    else {
        panic!("native handler failure")
    };
    assert_eq!(
        cause.downcast_ref::<PolicyUnavailable>().unwrap().operation,
        "select_retry"
    );
    assert_eq!(observed.head_sequence(), 3);
    let RunViewState::AwaitingRecovery {
        failure: failure @ Failure::Read { intent, .. },
    } = observed.state()
    else {
        panic!("retained original")
    };
    assert_eq!(
        failure.original().decode::<Outage>().unwrap().deadline_ms,
        731
    );
    let input = failure.call().input().decode::<Input>().unwrap();
    assert_eq!(input.value, 10);
    assert_eq!(input.continuation, "complete caller continuation");
    assert_eq!(intent.decode::<Request>().unwrap().value, 10);
    let rows = store.load_run(&run, None).await.unwrap().unwrap();
    let admission = mfm_journal::decode_frame(rows.admission()).unwrap();
    let latest = mfm_journal::decode_frame(rows.latest()).unwrap();
    assert_eq!(admission.run_sequence(), 1);
    assert_eq!(latest.run_sequence(), 3);
    assert!(rows.probe().is_none());
    let payload: serde_json::Value = serde_json::from_slice(latest.payload().as_bytes()).unwrap();
    assert!(payload.get("state").is_none());
    assert!(payload["operation"].get("failed").is_some());
    assert!(payload["operation"].get("recovered").is_none());
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_digest(), observed.head_digest());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(CLASSIFICATIONS.load(Ordering::SeqCst), 1);
    HANDLER_AVAILABLE.store(true, Ordering::SeqCst);
    let retry = runtime.resume(&run).await.unwrap();
    assert_eq!(retry.head_sequence(), 4);
    assert!(matches!(
        retry.state(),
        RunViewState::Runnable {
            reason: mfm_runtime::RunnableReason::Retry,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    available.store(true, Ordering::SeqCst);
    let done = runtime.resume(&run).await.unwrap();
    assert_eq!(done.head_sequence(), 5);
    let RunViewState::Succeeded(output) = done.state() else {
        panic!("success")
    };
    assert_eq!(output.decode::<Input>().unwrap().value, 10);
    assert_eq!(CLASSIFICATIONS.load(Ordering::SeqCst), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

struct RefuseFailure {
    inner: MemoryStore,
    loads: AtomicUsize,
}
impl Store for RefuseFailure {
    fn load_run<'a>(
        &'a self,
        run: &'a RunId,
        probe: Option<u64>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Option<mfm_store::LoadedRun>, mfm_store::StoreError>,
                > + Send
                + 'a,
        >,
    > {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load_run(run, probe)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<mfm_store::AppendResult, mfm_store::StoreError>>
                + Send
                + 'a,
        >,
    > {
        if frame.run_sequence() == 3 {
            Box::pin(async { Err(mfm_store::StoreError::Unavailable) })
        } else {
            self.inner.append_run(frame)
        }
    }
}
#[tokio::test]
async fn recording_failure_retains_native_original_and_exact_candidate_without_probing() {
    let store = Arc::new(RefuseFailure {
        inner: MemoryStore::new(),
        loads: AtomicUsize::new(0),
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = runtime(
        store.clone(),
        Arc::new(AtomicBool::new(false)),
        calls.clone(),
    );
    let input = Input {
        value: 9,
        continuation: "retained before recording".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/current-recording@1").unwrap(),
        &Flow,
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([122; 32]));
    let error = runtime
        .start(run.clone(), program, input)
        .await
        .err()
        .unwrap();
    let projected = match &error {
        InvocationFailure::Execution { error, .. } => {
            serde_json::from_str::<serde_json::Value>(error.project().unwrap().get()).unwrap()
        }
        _ => panic!("recording invocation"),
    };
    let projected = &projected["recording"]["failure"];
    let InvocationFailure::Execution {
        error: RuntimeError::Recording { failure, .. },
        last_observed: Some(observed),
        ..
    } = error
    else {
        panic!("recording custody")
    };
    assert_eq!(observed.head_sequence(), 2);
    let mfm_runtime::RecordingFailure::Append {
        original: Some(original),
        candidate,
        outcome: mfm_runtime::AppendFailure::Store(mfm_store::StoreError::Unavailable),
        observation: None,
        reload_cause: None,
    } = failure.as_ref()
    else {
        panic!("unavailable without automatic probe")
    };
    assert_eq!(original.downcast_ref::<Outage>().unwrap().deadline_ms, 731);
    assert_eq!(candidate.run_sequence(), 3);
    assert_eq!(
        candidate.previous_head_digest(),
        Some(observed.head_digest())
    );
    assert_eq!(projected["append"]["original"]["deadline_ms"], 731);
    assert!(projected["append"]["candidate"].get("payload").is_none());
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    let cold = runtime.read(&run).await.unwrap();
    assert_eq!(cold.head_digest(), observed.head_digest());
    assert!(matches!(cold.state(), RunViewState::Runnable { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

static INTERPRETER_AVAILABLE: AtomicBool = AtomicBool::new(false);
static PREPARATIONS: AtomicUsize = AtomicUsize::new(0);
struct Effect;
struct Submit;
impl State for Effect {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-effect@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl mfm_capabilities::EffectCapabilityContract for Submit {
    type Command = Request;
    type Evidence = Request;
    type OperationalError = Never;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.current-submit@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &mfm_ids::EffectId,
        command: &Request,
        evidence: &Request,
    ) -> Result<(), NativeCause> {
        Observation::bind_evidence(
            &mfm_program::nominal_contract_ref::<Request>().unwrap(),
            command,
            evidence,
        )
    }
}
impl mfm_program::EffectState<Submit> for Effect {
    fn prepare(input: &Input) -> Result<Request, NativeCause> {
        PREPARATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, NativeCause> {
        if INTERPRETER_AVAILABLE.load(Ordering::SeqCst) {
            Ok(ProposedStateOutcome::Success { output: input })
        } else {
            Err(NativeCause::from_error(PolicyUnavailable {
                operation: "interpret_settlement",
            }))
        }
    }
}
impl mfm_program::CapabilityInjection<Effect> for Submit {
    type Setup = NoParams;
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &NoParams) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct EffectFlow;
impl Operation for EffectFlow {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Input, Input, Never>,
    ) -> mfm_program::Result<()> {
        body.effect::<Effect, Submit, Identity<Never>>(&NoParams, NoParams, Occurrence::new())
    }
}
#[tokio::test]
async fn pending_command_settles_before_interpretation_and_resume_enters_no_adapter() {
    INTERPRETER_AVAILABLE.store(false, Ordering::SeqCst);
    PREPARATIONS.store(0, Ordering::SeqCst);
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let build = || {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_effect::<Effect, Submit>().unwrap();
        let calls = Arc::clone(&calls);
        builder
            .register_effect_adapter::<Submit, _, _>(NoParams, move |_, _, command| {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                let value = command.value;
                Box::pin(async move {
                    Ok(if attempt == 0 {
                        mfm_runtime::EffectAdapterOutcome::Pending
                    } else {
                        mfm_runtime::EffectAdapterOutcome::Settled(Request { value })
                    })
                })
            })
            .unwrap();
        Runtime::new(builder.finish(), store.clone())
    };
    let input = Input {
        value: 17,
        continuation: "settled input".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/current-effect@1").unwrap(),
        &EffectFlow,
        &input,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([123; 32]));
    let runtime = build();
    let pending = runtime.start(run.clone(), program, input).await.unwrap();
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(
        pending.state(),
        RunViewState::EffectPending { .. }
    ));
    assert_eq!(PREPARATIONS.load(Ordering::SeqCst), 2);
    let failure = build().resume(&run).await.err().unwrap();
    let InvocationFailure::Execution {
        error: RuntimeError::Native { cause, .. },
        last_observed: Some(observed),
        ..
    } = failure
    else {
        panic!("interpreter failure")
    };
    assert_eq!(
        cause.downcast_ref::<PolicyUnavailable>().unwrap().operation,
        "interpret_settlement"
    );
    assert_eq!(observed.head_sequence(), 3);
    let RunViewState::AwaitingInterpretation { settlement } = observed.state() else {
        panic!("durable settlement")
    };
    assert_eq!(
        settlement
            .effect()
            .command()
            .decode::<Request>()
            .unwrap()
            .value,
        17
    );
    assert_eq!(settlement.evidence().decode::<Request>().unwrap().value, 17);
    assert_eq!(PREPARATIONS.load(Ordering::SeqCst), 3);
    let inspected = build().read(&run).await.unwrap();
    assert_eq!(inspected.head_digest(), observed.head_digest());
    INTERPRETER_AVAILABLE.store(true, Ordering::SeqCst);
    let finished = build().resume(&run).await.unwrap();
    assert!(matches!(finished.state(), RunViewState::Succeeded(_)));
    assert_eq!(finished.head_sequence(), 4);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(PREPARATIONS.load(Ordering::SeqCst), 3);
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
struct Invalidated {
    anchor: u64,
}
impl ClassifyError for Invalidated {
    fn classify(&self) -> Classification {
        Classification::InputInvalidated
    }
}
struct Refresh;
impl ReadCapabilityContract for Refresh {
    type Intent = Request;
    type Evidence = Request;
    type OperationalError = Invalidated;
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.current-refresh@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        reference: &ContentRef,
        intent: &Request,
        evidence: &Request,
    ) -> Result<(), NativeCause> {
        Observation::bind_evidence(reference, intent, evidence)
    }
}
impl ReadState<Refresh> for Read {
    fn prepare(input: &Input) -> Result<Request, NativeCause> {
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, NativeCause> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl mfm_program::CapabilityInjection<Read> for Refresh {
    type Setup = NoParams;
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
    type ExpandedFailure = Never;
    type FailureMap = Identity<Never>;
    fn failure_map_params(_: &NoParams) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    fn original_binding_ref(setup: &NoParams) -> mfm_program::Result<ContentRef> {
        mfm_values::canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}
struct Rewind;
impl Handler for Rewind {
    type Params = NoParams;
    fn implementation_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.current-rewind@1").map_err(|_| ProgramError::InvalidContract)
    }
    fn handle(
        _: &NoParams,
        _: Classification,
        context: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, NativeCause> {
        Ok(context
            .eligible_restart_targets()
            .first()
            .copied()
            .map(RecoveryRequest::Restart)
            .unwrap_or(RecoveryRequest::Stop))
    }
}
struct Checkpoints;
impl Operation for Checkpoints {
    type Input = Input;
    type Output = Input;
    type Failure = Never;
    fn validate_input(&self, _: &Input) -> mfm_program::Result<()> {
        Ok(())
    }
    fn expand(
        &self,
        body: &mut OperationExpansion<Input, Input, Never>,
    ) -> mfm_program::Result<()> {
        let outer = body.checkpoint::<Input>()?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())?;
        let inner = body.checkpoint::<Input>()?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())?;
        body.handler(
            HandlerBinding::new::<Rewind>(NoParams)?
                .checkpoint(&outer)?
                .checkpoint(&inner)?,
        )?;
        body.allowances(RecoveryAllowances::new(0, 1))?;
        body.read::<Read, Refresh, Identity<Never>>(&NoParams, NoParams, Occurrence::new())
    }
}
#[tokio::test]
async fn restart_restores_its_complete_input_prunes_later_checkpoints_and_keeps_usage() {
    let store = Arc::new(MemoryStore::new());
    let available = Arc::new(AtomicBool::new(false));
    let build = || {
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder.register_pure::<Increment>().unwrap();
        builder.register_read::<Read, Refresh>().unwrap();
        builder.register_handler::<Rewind>().unwrap();
        let available = Arc::clone(&available);
        builder
            .register_adapter::<Refresh, _, _>(NoParams, move |_, intent| {
                let available = available.load(Ordering::SeqCst);
                let value = intent.value;
                Box::pin(async move {
                    if available {
                        Ok(Request { value })
                    } else {
                        Err(AdapterError::Operational(Invalidated { anchor: 29 }))
                    }
                })
            })
            .unwrap();
        Runtime::new(builder.finish(), store.clone())
    };
    let input = Input {
        value: 9,
        continuation: "checkpoint input".into(),
    };
    let program = mfm_program::expand_program(
        EntryPointId::new("mfm.test/current-checkpoints@1").unwrap(),
        &Checkpoints,
        &input,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([124; 32]));
    let restarted = build().start(run.clone(), program, input).await.unwrap();
    assert_eq!(restarted.head_sequence(), 5);
    let RunViewState::Runnable {
        position,
        reason: mfm_runtime::RunnableReason::Restart { checkpoint },
    } = restarted.state()
    else {
        panic!("restart grant")
    };
    assert_eq!(checkpoint.index(), 0);
    assert_eq!(position.visit.value(), 3);
    let loaded = store.load_run(&run, Some(4)).await.unwrap().unwrap();
    let original = mfm_journal::decode_frame(loaded.probe().unwrap()).unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(original.payload().as_bytes()).unwrap();
    assert_eq!(original["checkpoints"].as_array().unwrap().len(), 2);
    assert_eq!(original["checkpoints"][0]["input"]["canonical"]["value"], 9);
    assert_eq!(
        original["checkpoints"][1]["input"]["canonical"]["value"],
        10
    );
    let current = mfm_journal::decode_frame(loaded.latest()).unwrap();
    let current: serde_json::Value = serde_json::from_slice(current.payload().as_bytes()).unwrap();
    assert_eq!(current["checkpoints"].as_array().unwrap().len(), 1);
    assert_eq!(current["checkpoints"][0]["input"]["canonical"]["value"], 9);
    assert_eq!(current["usage"][2]["restarts"], 1);
    assert_eq!(
        build().read(&run).await.unwrap().head_digest(),
        restarted.head_digest()
    );
    available.store(true, Ordering::SeqCst);
    let finished = build().resume(&run).await.unwrap();
    assert_eq!(finished.head_sequence(), 8);
    let RunViewState::Succeeded(output) = finished.state() else {
        panic!("success after restore")
    };
    assert_eq!(output.decode::<Input>().unwrap().value, 11);
}

#[path = "current_state/collision.rs"]
mod collision;

#[path = "current_state/constructors.rs"]
mod constructors;

#[path = "current_state/validation.rs"]
mod validation;

#[path = "current_state/sizes.rs"]
mod sizes;

#[path = "current_state/terminal.rs"]
mod terminal;

#[path = "current_state/capacity.rs"]
mod capacity;
