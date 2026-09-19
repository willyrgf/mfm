use mfm_capabilities::{AdapterError, ReadCapabilityContract};
use mfm_ids::{ContentRef, DigestBytes, EntryPointId, RunId, StableId};
use mfm_program::{
    Classification, ClassifyError, Handler, Identity, Never, NoParams, ProgramError, ProgramLimits,
    ProposedStateOutcome, PureState, ReadState, RecoveryContext, RecoveryRequest, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{Failure, InvocationFailure, RunViewState, Runtime, RuntimeError};
use mfm_store::{MemoryStore, Store};
use mfm_values::InvocationDiagnostic;
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
    ) -> Result<RecoveryRequest, InvocationDiagnostic> {
        if HANDLER_AVAILABLE.load(Ordering::SeqCst) {
            Ok(RecoveryRequest::RetryState)
        } else {
            Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "handle",
                &(PolicyUnavailable {
                    operation: "select_retry",
                }),
                None,
            ))
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
    fn evaluate(
        mut input: Input,
    ) -> Result<ProposedStateOutcome<Input, Never>, InvocationDiagnostic> {
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
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.current-observation@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &ContentRef,
        intent: &Request,
        _: &ContentRef,
        evidence: &Request,
    ) -> Result<(), InvocationDiagnostic> {
        if intent.value == evidence.value {
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
impl mfm_program::ReadSelection<Observation> for Read {
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
}
impl ReadState<Observation> for Read {
    fn prepare(input: &Input) -> Result<Request, InvocationDiagnostic> {
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
#[derive(Default)]
struct FlowDefinition;
impl mfm_program::OperationDefinition for FlowDefinition {
    type Body = (
        mfm_program::Pure<Increment>,
        mfm_program::ResolvedRead<Read, Observation, Native<Outage>>,
    );
}
impl mfm_program::Plan<Input> for FlowDefinition {
    type Config = Input;
    fn plan<'a>(&'a self, input: &'a Input) -> mfm_program::Result<(&'a Input, Self::Body)> {
        Ok((
            input,
            (Default::default(), mfm_program::ResolvedRead::new(NoParams)),
        ))
    }
}
struct FlowPolicy;
impl mfm_program::OperationDefaults for FlowPolicy {
    type Handler = Policy;
    type Targets = ();
}
impl mfm_program::ResolveDefaults<Input> for FlowPolicy {
    fn resolve(_: &Input) -> mfm_program::Result<mfm_program::PolicyValues<Policy>> {
        Ok(mfm_program::PolicyValues {
            handler: Some(NoParams),
            retries: Some(1),
            restarts: Some(0),
        })
    }
}
type Flow = mfm_program::Operation<FlowDefinition, FlowPolicy>;
fn runtime(
    store: Arc<dyn Store>,
    available: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
) -> (Runtime, Resources<Flow>) {
    (
        Runtime::new(store),
        Resources {
            available,
            calls,
            source: std::marker::PhantomData,
        },
    )
}
#[path = "current_state/resources.rs"]
mod resources;
use resources::{Native, Resources};

// A broken recovery handler must not lose the provider failure; resume must retry policy before
// allowing another provider call.
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
    let (runtime, resources) = runtime(store.clone(), available.clone(), calls.clone());
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/current@1").unwrap(),
        &Flow::default(),
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([121; 32]));
    let error = runtime
        .start(run.clone(), &program, &input)
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
    assert_eq!(cause.details().as_value()["operation"], "select_retry");
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
    let program = mfm_program::load(program.canonical_bytes(), &resources).unwrap();
    let cold = runtime.read(&run, &program).await.unwrap();
    assert_eq!(cold.head_digest(), observed.head_digest());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(CLASSIFICATIONS.load(Ordering::SeqCst), 1);
    HANDLER_AVAILABLE.store(true, Ordering::SeqCst);
    let retry = runtime.resume(&run, &program).await.unwrap();
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
    let done = runtime.resume(&run, &program).await.unwrap();
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
            Box::pin(async {
                Err(mfm_store::StoreError::Unavailable(
                    mfm_values::DiagnosticEvidence::from_value(
                        serde_json::json!({"operation": "test.store", "injected": "Unavailable"}),
                    ),
                ))
            })
        } else {
            self.inner.append_run(frame)
        }
    }
}
// When recording fails, return the original failure and attempted frame without claiming they
// were stored or performing a speculative reload.
#[tokio::test]
async fn recording_failure_retains_admitted_original_and_exact_candidate_without_probing() {
    let store = Arc::new(RefuseFailure {
        inner: MemoryStore::new(),
        loads: AtomicUsize::new(0),
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (runtime, resources) = runtime(
        store.clone(),
        Arc::new(AtomicBool::new(false)),
        calls.clone(),
    );
    let input = Input {
        value: 9,
        continuation: "retained before recording".into(),
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/current-recording@1").unwrap(),
        &Flow::default(),
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([122; 32]));
    let error = runtime
        .start(run.clone(), &program, &input)
        .await
        .err()
        .unwrap();
    let projected = match &error {
        InvocationFailure::Execution { error, .. } => serde_json::to_value(error).unwrap(),
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
    let mfm_runtime::RecordingFailure::Store {
        original: Some(original),
        candidate,
        cause: mfm_store::StoreError::Unavailable(evidence),
    } = failure.as_ref()
    else {
        panic!("unavailable without automatic probe")
    };
    assert_eq!(
        evidence.as_value(),
        &serde_json::json!({"operation": "test.store", "injected": "Unavailable"})
    );
    assert_eq!(
        projected["store"]["cause"]["unavailable"],
        *evidence.as_value()
    );
    assert_eq!(
        original.original().decode::<Outage>().unwrap().deadline_ms,
        731
    );
    assert_eq!(candidate.run_sequence(), 3);
    assert_eq!(
        candidate.previous_head_digest(),
        Some(observed.head_digest())
    );
    assert_eq!(
        projected["store"]["original"]["read"]["original"]["canonical"]["deadline_ms"],
        731
    );
    assert!(projected["store"]["candidate"].get("payload").is_none());
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    let program = mfm_program::load(program.canonical_bytes(), &resources).unwrap();
    let cold = runtime.read(&run, &program).await.unwrap();
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
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.current-submit@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        _: &mfm_ids::EffectId,
        _: &ContentRef,
        command: &Request,
        evidence_ref: &ContentRef,
        evidence: &Request,
    ) -> Result<(), InvocationDiagnostic> {
        Observation::bind_evidence(
            &mfm_program::nominal_contract_ref::<Request>().unwrap(),
            command,
            evidence_ref,
            evidence,
        )
    }
}
impl mfm_program::EffectState<Submit> for Effect {
    fn prepare(input: &Input) -> Result<Request, InvocationDiagnostic> {
        PREPARATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, InvocationDiagnostic> {
        if INTERPRETER_AVAILABLE.load(Ordering::SeqCst) {
            Ok(ProposedStateOutcome::Success { output: input })
        } else {
            Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "interpret",
                &(PolicyUnavailable {
                    operation: "interpret_settlement",
                }),
                None,
            ))
        }
    }
}
impl mfm_program::EffectSelection<Submit> for Effect {
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
}
type EffectFlow = mfm_program::ResolvedEffect<Effect, Submit, Native<Never>>;
// Once settlement is stored, an interpreter failure must be recoverable without submitting the
// external command again.
#[tokio::test]
async fn pending_command_settles_before_interpretation_and_resume_enters_no_adapter() {
    INTERPRETER_AVAILABLE.store(false, Ordering::SeqCst);
    PREPARATIONS.store(0, Ordering::SeqCst);
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let resources = Resources::<EffectFlow> {
        available: Arc::new(AtomicBool::new(true)),
        calls: calls.clone(),
        source: std::marker::PhantomData,
    };
    let build = || Runtime::new(store.clone());
    let input = Input {
        value: 17,
        continuation: "settled input".into(),
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/current-effect@1").unwrap(),
        &EffectFlow::new(NoParams),
        &input,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([123; 32]));
    let runtime = build();
    let pending = runtime.start(run.clone(), &program, &input).await.unwrap();
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(
        pending.state(),
        RunViewState::EffectPending { .. }
    ));
    assert_eq!(PREPARATIONS.load(Ordering::SeqCst), 2);
    let program = mfm_program::load(program.canonical_bytes(), &resources).unwrap();
    let failure = build().resume(&run, &program).await.err().unwrap();
    let InvocationFailure::Execution {
        error: RuntimeError::Native { cause, .. },
        last_observed: Some(observed),
        ..
    } = failure
    else {
        panic!("interpreter failure")
    };
    assert_eq!(
        cause.details().as_value()["operation"],
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
    let inspected = build().read(&run, &program).await.unwrap();
    assert_eq!(inspected.head_digest(), observed.head_digest());
    INTERPRETER_AVAILABLE.store(true, Ordering::SeqCst);
    let finished = build().resume(&run, &program).await.unwrap();
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
    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.current-refresh@1")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }
    fn bind_evidence(
        reference: &ContentRef,
        intent: &Request,
        _: &ContentRef,
        evidence: &Request,
    ) -> Result<(), InvocationDiagnostic> {
        Observation::bind_evidence(reference, intent, reference, evidence)
    }
}
impl ReadState<Refresh> for Read {
    fn prepare(input: &Input) -> Result<Request, InvocationDiagnostic> {
        Ok(Request { value: input.value })
    }
    fn interpret(
        input: Input,
        _: &Request,
    ) -> Result<ProposedStateOutcome<Input, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
impl mfm_program::ReadSelection<Refresh> for Read {
    type ExpandedInput = Input;
    type ExpandedOutput = Input;
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
    ) -> Result<RecoveryRequest, InvocationDiagnostic> {
        Ok(context
            .eligible_restart_targets()
            .first()
            .copied()
            .map(RecoveryRequest::Restart)
            .unwrap_or(RecoveryRequest::Stop))
    }
}
struct Outer;
struct Inner;
impl mfm_program::CheckpointMarker for Outer {
    type Context = Input;
}
impl mfm_program::CheckpointMarker for Inner {
    type Context = Input;
}
struct NoRecovery;
impl mfm_program::OperationDefaults for NoRecovery {
    type Handler = mfm_program::Stop;
    type Targets = ();
}
impl mfm_program::ResolveDefaults<Input> for NoRecovery {
    fn resolve(_: &Input) -> mfm_program::Result<mfm_program::PolicyValues<mfm_program::Stop>> {
        Ok(mfm_program::PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(0),
        })
    }
}
type IncrementOnly = mfm_program::Operation<(mfm_program::Pure<Increment>,), NoRecovery>;
#[derive(Default)]
struct CheckpointDefinition;
impl mfm_program::OperationDefinition for CheckpointDefinition {
    type Body = (
        mfm_program::Checkpoint<Outer>,
        IncrementOnly,
        mfm_program::Checkpoint<Inner>,
        IncrementOnly,
        mfm_program::ResolvedRead<Read, Refresh, Native<Invalidated>>,
    );
}
impl mfm_program::Plan<Input> for CheckpointDefinition {
    type Config = Input;
    fn plan<'a>(&'a self, input: &'a Input) -> mfm_program::Result<(&'a Input, Self::Body)> {
        Ok((
            input,
            (
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                mfm_program::ResolvedRead::new(NoParams),
            ),
        ))
    }
}
struct CheckpointPolicy;
impl mfm_program::OperationDefaults for CheckpointPolicy {
    type Handler = Rewind;
    type Targets = (Outer, Inner);
}
impl mfm_program::ResolveDefaults<Input> for CheckpointPolicy {
    fn resolve(_: &Input) -> mfm_program::Result<mfm_program::PolicyValues<Rewind>> {
        Ok(mfm_program::PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(1),
        })
    }
}
type Checkpoints = mfm_program::Operation<CheckpointDefinition, CheckpointPolicy>;
// Restart must restore the chosen checkpoint, discard later checkpoints and retain spent
// recovery allowance.
#[tokio::test]
async fn restart_restores_its_complete_input_prunes_later_checkpoints_and_keeps_usage() {
    let store = Arc::new(MemoryStore::new());
    let available = Arc::new(AtomicBool::new(false));
    let resources = Resources::<Checkpoints> {
        available: available.clone(),
        calls: Arc::new(AtomicUsize::new(0)),
        source: std::marker::PhantomData,
    };
    let build = || Runtime::new(store.clone());
    let input = Input {
        value: 9,
        continuation: "checkpoint input".into(),
    };
    let program = mfm_program::compile(
        EntryPointId::new("mfm.test/current-checkpoints@1").unwrap(),
        &Checkpoints::default(),
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let run = RunId::from_digest(DigestBytes::from_array([124; 32]));
    let restarted = build().start(run.clone(), &program, &input).await.unwrap();
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
        build().read(&run, &program).await.unwrap().head_digest(),
        restarted.head_digest()
    );
    let program = mfm_program::load(program.canonical_bytes(), &resources).unwrap();
    available.store(true, Ordering::SeqCst);
    let finished = build().resume(&run, &program).await.unwrap();
    assert_eq!(finished.head_sequence(), 8);
    let RunViewState::Succeeded(output) = finished.state() else {
        panic!("success after restore")
    };
    assert_eq!(output.decode::<Input>().unwrap().value, 11);
}

#[path = "current_state/collision.rs"]
mod collision;

#[path = "current_state/validation.rs"]
mod validation;

#[path = "current_state/bootstrap.rs"]
mod bootstrap;
