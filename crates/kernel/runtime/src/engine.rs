use crate::assembly::{
    AdapterReturn, AssemblyInner, ErasedEffectAdapterCallback, ErasedReadAdapterCallback,
    ExecutableMode, ExecutableProgram, RuntimeAssembly,
};
use crate::state::*;
use crate::{
    AppendFailure, CandidatePresence, EffectAdapterOutcome, InvocationFailure, Operation,
    RecordingFailure, Result, RunView, RunViewState, RuntimeError, Stage,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition, RunId};
use mfm_journal::{decode_frame, seal_frame, EncodedRunFrame};
use mfm_program::{
    ClassifyError, EffectState, Program, ProposedStateOutcome, PureState, ReadState,
};
use mfm_store::{AppendResult, LoadedRun, RunSummary, Store};
use mfm_values::{MfmValue, NativeCause, Object};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize, thiserror::Error)]
#[error("initial value does not match the supplied Program")]
struct InitialValueMismatch {
    expected: ContentRef,
    actual: ContentRef,
}

pub(crate) struct ReturnedFailure {
    pub(crate) object: Object,
    pub(crate) original: NativeCause,
}
#[derive(Clone)]
pub(crate) struct Driver {
    admission: Arc<RunRecord>,
    current: Arc<RunRecord>,
    executable: Arc<ExecutableProgram>,
    head: RunSummary,
}
pub(crate) struct DriverContext<'a> {
    store: &'a Arc<dyn Store>,
    driver: Driver,
}
pub(crate) enum DriverDisposition {
    Continue(Driver),
    Yield(RunView),
    Failed(InvocationFailure),
}
fn invocation(
    run_id: RunId,
    error: RuntimeError,
    last_observed: Option<RunView>,
) -> InvocationFailure {
    InvocationFailure::Execution {
        run_id,
        error,
        last_observed,
    }
}
fn native<E: std::error::Error + Serialize + Send + Sync + 'static>(
    operation: Operation,
    stage: Stage,
    source: E,
) -> RuntimeError {
    RuntimeError::at(operation, stage, NativeCause::from_error(source))
}
fn decode<T: MfmValue>(object: &Object, operation: Operation) -> Result<T> {
    object
        .decode::<T>()
        .map_err(|cause| RuntimeError::at(operation, Stage::Decode, cause))
}
fn canonical<T: Serialize>(value: &T, operation: Operation) -> Result<PlainCanonicalJsonBytes> {
    let json = mfm_canonical::to_json_bounded(value, mfm_journal::MAX_FRAME_BYTES)
        .map_err(|source| native(operation, Stage::Encode, source))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|source| native(operation, Stage::Encode, source))
}
pub(crate) async fn run_blocking<T: Send + 'static, F: FnOnce() -> Result<T> + Send + 'static>(
    operation: Operation,
    job: F,
) -> Result<T> {
    tokio::task::spawn_blocking(job).await.map_err(|source| {
        native(
            operation,
            Stage::Execute,
            if source.is_panic() {
                crate::TaskFailure::Panicked
            } else {
                crate::TaskFailure::Cancelled
            },
        )
    })?
}
pub(crate) async fn encode<T: MfmValue>(value: T, operation: Operation) -> Result<Object> {
    run_blocking(operation, move || {
        Object::from_value(&value).map_err(|source| native(operation, Stage::Encode, source))
    })
    .await
}
pub(crate) async fn encode_failure<E: MfmValue>(
    error: E,
    operation: Operation,
) -> Result<ReturnedFailure> {
    let error = Arc::new(error);
    let original = NativeCause::from_original(Arc::clone(&error));
    match run_blocking(operation, move || {
        Object::from_value(error.as_ref())
            .map_err(|source| native(operation, Stage::Encode, source))
    })
    .await
    {
        Ok(object) => Ok(ReturnedFailure { object, original }),
        Err(cause) => Err(RuntimeError::Recording {
            operation,
            failure: Box::new(RecordingFailure::BeforeAppend {
                original: Some(original),
                candidate: None,
                cause: cause.into_native(),
            }),
        }),
    }
}

pub(crate) async fn start<T: MfmValue>(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
    program: Program,
    c0: T,
) -> std::result::Result<RunView, InvocationFailure> {
    let identity = run_id.clone();
    let (executable, commit, candidate) = run_blocking(Operation::Admission, move || {
        let initial = Object::from_value(&c0).map_err(RuntimeError::from)?;
        if initial.value_ref() != program.initial_value_ref() {
            return Err(native(
                Operation::Admission,
                Stage::Execute,
                InitialValueMismatch {
                    expected: program.initial_value_ref().clone(),
                    actual: initial.value_ref().clone(),
                },
            ));
        }
        let executable = Arc::new(RuntimeAssembly { inner: assembly }.associate(program)?);
        let program = Object::from_canonical(
            executable.program.content_ref().clone(),
            executable.program.canonical_bytes(),
        )
        .map_err(RuntimeError::from)?;
        let commit = Arc::new(RunRecord::initial(&executable, program, initial)?);
        let candidate = seal(&identity, 1, None, &commit, &executable)?;
        Ok((executable, commit, candidate))
    })
    .await
    .map_err(|error| invocation(run_id.clone(), error, None))?;
    let outcome = store.append_run(&candidate).await;
    match outcome {
        Ok(AppendResult::Inserted) => {
            let head = RunSummary::new(
                run_id.clone(),
                1,
                candidate.head_digest().clone(),
                candidate.canonical_bytes().len() as u64,
            )
            .map_err(|source| {
                invocation(
                    run_id.clone(),
                    native(Operation::Admission, Stage::Append, source),
                    None,
                )
            })?;
            advance(
                store,
                Driver {
                    admission: Arc::clone(&commit),
                    current: commit,
                    executable,
                    head,
                },
            )
            .await
        }
        Ok(AppendResult::NotInserted) => {
            let loaded = store
                .load_run(&run_id, None)
                .await
                .map_err(|source| invocation(run_id.clone(), RuntimeError::Store(source), None))?
                .ok_or_else(|| invocation(run_id.clone(), RuntimeError::Absent, None))?;
            let proposed = Arc::clone(&commit);
            run_blocking(Operation::Restore, move || {
                let driver = restore(executable._assembly.clone(), &run_id, loaded, None)?;
                if driver.admission.operation != proposed.operation {
                    return Err(RuntimeError::AdmissionConflict);
                }
                view(&driver)
            })
            .await
            .map_err(|error| invocation(candidate.run_id().clone(), error, None))
        }
        Err(source) => Err(invocation(
            run_id,
            RuntimeError::Recording {
                operation: Operation::Admission,
                failure: Box::new(RecordingFailure::Append {
                    original: None,
                    candidate,
                    outcome: AppendFailure::Store(source),
                    observation: None,
                    reload_cause: None,
                }),
            },
            None,
        )),
    }
}
pub(crate) async fn resume(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> std::result::Result<RunView, InvocationFailure> {
    let driver = load(assembly, &store, &run_id)
        .await
        .map_err(|error| invocation(run_id, error, None))?;
    advance(store, driver).await
}
pub(crate) async fn read(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> std::result::Result<RunView, InvocationFailure> {
    let driver = load(assembly, &store, &run_id)
        .await
        .map_err(|error| invocation(run_id.clone(), error, None))?;
    run_blocking(Operation::Project, move || view(&driver))
        .await
        .map_err(|error| invocation(run_id, error, None))
}
async fn load(
    assembly: Arc<AssemblyInner>,
    store: &Arc<dyn Store>,
    run_id: &RunId,
) -> Result<Driver> {
    let loaded = store
        .load_run(run_id, None)
        .await
        .map_err(RuntimeError::Store)?
        .ok_or(RuntimeError::Absent)?;
    let run_id = run_id.clone();
    run_blocking(Operation::Restore, move || {
        restore(assembly, &run_id, loaded, None)
    })
    .await
}
fn bound_frames(
    run_id: &RunId,
    loaded: &LoadedRun,
    probe: Option<u64>,
) -> Result<(EncodedRunFrame, EncodedRunFrame, Option<EncodedRunFrame>)> {
    let frame = |bytes| {
        decode_frame(bytes).map_err(|source| native(Operation::Restore, Stage::Decode, source))
    };
    let admission = frame(loaded.admission())?;
    let latest = if Arc::ptr_eq(loaded.admission(), loaded.latest()) {
        admission.clone()
    } else {
        frame(loaded.latest())?
    };
    let probed = loaded
        .probe()
        .map(|bytes| {
            if Arc::ptr_eq(bytes, loaded.admission()) {
                Ok(admission.clone())
            } else if Arc::ptr_eq(bytes, loaded.latest()) {
                Ok(latest.clone())
            } else {
                frame(bytes)
            }
        })
        .transpose()?;
    if loaded.head().run_id() != run_id
        || admission.run_id() != run_id
        || admission.run_sequence() != 1
        || latest.run_id() != run_id
        || latest.run_sequence() != loaded.head().head_sequence()
        || latest.head_digest() != loaded.head().head_digest()
        || loaded.head().head_sequence() == 1 && loaded.admission() != loaded.latest()
        || loaded.probe().is_some_and(|bytes| {
            probe == Some(1) && bytes != loaded.admission()
                || probe == Some(loaded.head().head_sequence()) && bytes != loaded.latest()
        })
        || probed
            .as_ref()
            .is_some_and(|frame| frame.run_id() != run_id || Some(frame.run_sequence()) != probe)
        || probe.is_some_and(|sequence| sequence <= loaded.head().head_sequence())
            && probed.is_none()
    {
        return Err(RuntimeError::InvalidHistory);
    }
    Ok((admission, latest, probed))
}
fn restore(
    assembly: Arc<AssemblyInner>,
    run_id: &RunId,
    loaded: LoadedRun,
    probe: Option<u64>,
) -> Result<Driver> {
    let (admission_frame, latest_frame, _) = bound_frames(run_id, &loaded, probe)?;
    restore_frames(
        assembly,
        run_id,
        loaded.head().clone(),
        admission_frame,
        latest_frame,
    )
}
fn restore_frames(
    assembly: Arc<AssemblyInner>,
    run_id: &RunId,
    head: RunSummary,
    admission_frame: EncodedRunFrame,
    latest_frame: EncodedRunFrame,
) -> Result<Driver> {
    let decode_commit = |frame: &EncodedRunFrame| -> Result<Arc<RunRecord>> {
        serde_json::from_slice::<RunRecord>(frame.payload().as_bytes())
            .map(Arc::new)
            .map_err(|source| {
                native(
                    Operation::Restore,
                    Stage::Decode,
                    mfm_canonical::JsonError::new(source),
                )
            })
    };
    let admission = decode_commit(&admission_frame)?;
    let RecordedOperation::Admitted { program, .. } = &admission.operation else {
        return Err(RuntimeError::InvalidHistory);
    };
    let program = Program::decode_canonical(program.canonical_bytes())
        .map_err(|source| native(Operation::Restore, Stage::Decode, source))?;
    let executable = Arc::new(RuntimeAssembly { inner: assembly }.associate(program)?);
    admission.validate_current(run_id, 1, &executable, true)?;
    check_metadata(&admission_frame, &admission)?;
    let current = if head.head_sequence() == 1 {
        Arc::clone(&admission)
    } else {
        let commit = decode_commit(&latest_frame)?;
        commit.validate_current(run_id, latest_frame.run_sequence(), &executable, true)?;
        check_metadata(&latest_frame, &commit)?;
        commit
    };
    Ok(Driver {
        admission,
        current,
        executable,
        head,
    })
}
async fn advance(
    store: Arc<dyn Store>,
    mut driver: Driver,
) -> std::result::Result<RunView, InvocationFailure> {
    let mut last_observed = None;
    loop {
        let run_id = driver.head.run_id().clone();
        let observed = project(&driver)
            .await
            .map_err(|error| invocation(run_id.clone(), error, last_observed.take()))?;
        let continuation = driver
            .current
            .continuation(&driver.executable)
            .map_err(|error| invocation(run_id.clone(), error, None))?;
        let position = match &continuation {
            Continuation::Succeeded { .. } | Continuation::Failed { .. } => return Ok(observed),
            _ => {
                continuation
                    .active()
                    .ok_or_else(|| invocation(run_id.clone(), RuntimeError::InvalidHistory, None))?
                    .0
            }
        };
        let runnable = matches!(continuation, Continuation::Runnable { .. });
        let context = DriverContext {
            store: &store,
            driver,
        };
        let executable = Arc::clone(&context.driver.executable);
        let result = match &executable.declarations[position.state.index()].mode {
            ExecutableMode::Pure { start } => start(context).await,
            ExecutableMode::Read { start, adapter, .. } => {
                start(context, Arc::clone(adapter)).await
            }
            ExecutableMode::Effect {
                prepare,
                start_pending,
                adapter,
                ..
            } => {
                if runnable {
                    prepare(context).await
                } else {
                    start_pending(context, Arc::clone(adapter)).await
                }
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => return Err(invocation(run_id, error, Some(observed))),
        };
        match result {
            DriverDisposition::Continue(next) => {
                last_observed = Some(observed);
                driver = next;
            }
            DriverDisposition::Yield(observed) => return Ok(observed),
            DriverDisposition::Failed(failure) => return Err(failure),
        }
    }
}
fn seal(
    run_id: &RunId,
    sequence: u64,
    previous: Option<&mfm_ids::ContentDigest>,
    commit: &RunRecord,
    executable: &ExecutableProgram,
) -> Result<EncodedRunFrame> {
    commit.validate_current(run_id, sequence, executable, false)?;
    if let Continuation::Failed {
        failure,
        reason,
        root,
    } = commit.continuation(executable)?
    {
        failure_report(commit, failure, reason, root)?;
    }
    let payload = canonical(commit, Operation::Record)?;
    let candidate = seal_frame(run_id, sequence, previous, &payload)
        .map_err(|source| native(Operation::Record, Stage::Seal, source))?;
    if let Err(cause) = check_metadata(&candidate, commit) {
        return Err(RuntimeError::Recording {
            operation: Operation::Record,
            failure: Box::new(RecordingFailure::BeforeAppend {
                original: None,
                candidate: Some(candidate),
                cause: cause.into_native(),
            }),
        });
    }
    Ok(candidate)
}
fn check_metadata(frame: &EncodedRunFrame, commit: &RunRecord) -> Result<()> {
    let metadata = (frame.canonical_bytes().len() as u64)
        .checked_sub(commit.object_payload_bytes()?)
        .ok_or(RuntimeError::ArithmeticOverflow)?;
    crate::check_size(
        crate::SizeResource::FrameEnvelope,
        metadata,
        mfm_journal::MAX_FRAME_NON_PAYLOAD_ENVELOPE as u64,
    )
}
async fn project(driver: &Driver) -> Result<RunView> {
    let projected = driver.clone();
    run_blocking(Operation::Project, move || view(&projected))
        .await
        .map_err(|cause| RuntimeError::Projection {
            acknowledged: Box::new(driver.head.clone()),
            cause: cause.into_native(),
        })
}
async fn record(
    context: DriverContext<'_>,
    next: RunRecord,
    original: Option<NativeCause>,
    operation: Operation,
) -> Result<DriverDisposition> {
    let DriverContext { store, driver } = context;
    let commit = Arc::new(next);
    let next_commit = Arc::clone(&commit);
    let executable = Arc::clone(&driver.executable);
    let head = driver.head.clone();
    let candidate = run_blocking(operation, move || {
        seal(
            head.run_id(),
            head.head_sequence()
                .checked_add(1)
                .ok_or(RuntimeError::ArithmeticOverflow)?,
            Some(head.head_digest()),
            &next_commit,
            &executable,
        )
    })
    .await
    .map_err(|cause| match cause {
        RuntimeError::Recording { failure, .. } => match *failure {
            RecordingFailure::BeforeAppend {
                candidate, cause, ..
            } => RuntimeError::Recording {
                operation,
                failure: Box::new(RecordingFailure::BeforeAppend {
                    original: original.clone(),
                    candidate,
                    cause,
                }),
            },
            failure => RuntimeError::Recording {
                operation,
                failure: Box::new(failure),
            },
        },
        cause => RuntimeError::Recording {
            operation,
            failure: Box::new(RecordingFailure::BeforeAppend {
                original: original.clone(),
                candidate: None,
                cause: cause.into_native(),
            }),
        },
    })?;
    match store.append_run(&candidate).await {
        Ok(AppendResult::Inserted) => {
            let head = RunSummary::new(
                driver.head.run_id().clone(),
                candidate.run_sequence(),
                candidate.head_digest().clone(),
                driver
                    .head
                    .total_bytes()
                    .checked_add(candidate.canonical_bytes().len() as u64)
                    .ok_or(RuntimeError::ArithmeticOverflow)?,
            )
            .map_err(|source| native(operation, Stage::Append, source))?;
            let next = Driver {
                current: commit,
                head,
                ..driver
            };
            if let RecordedOperation::Recovered {
                failure: Failure::PendingEffect { .. },
                outcome: RecoveryOutcome::Stop { .. },
                ..
            } = &next.current.operation
            {
                let observed = project(&next).await?;
                return Ok(DriverDisposition::Failed(
                    InvocationFailure::RecoveryStopped { observed },
                ));
            }
            Ok(
                if matches!(next.current.operation, RecordedOperation::Recovered { .. }) {
                    DriverDisposition::Yield(project(&next).await?)
                } else {
                    DriverDisposition::Continue(next)
                },
            )
        }
        Ok(AppendResult::NotInserted) => {
            reconcile(store, driver, candidate, original, operation).await
        }
        Err(source) => Err(RuntimeError::Recording {
            operation,
            failure: Box::new(RecordingFailure::Append {
                original,
                candidate,
                outcome: AppendFailure::Store(source),
                observation: None,
                reload_cause: None,
            }),
        }),
    }
}
async fn reconcile(
    store: &Arc<dyn Store>,
    driver: Driver,
    candidate: EncodedRunFrame,
    original: Option<NativeCause>,
    operation: Operation,
) -> Result<DriverDisposition> {
    let sequence = candidate.run_sequence();
    let loaded = store.load_run(driver.head.run_id(), Some(sequence)).await;
    let run_id = driver.head.run_id().clone();
    let assembly = Arc::clone(&driver.executable._assembly);
    let compared = candidate.clone();
    let result = match loaded {
        Ok(Some(loaded)) => {
            run_blocking(Operation::Restore, move || {
                let (admission, latest, probe) = bound_frames(&run_id, &loaded, Some(sequence))?;
                let presence = match probe {
                    Some(frame) if frame.canonical_bytes() == compared.canonical_bytes() => {
                        CandidatePresence::Present
                    }
                    Some(_) => CandidatePresence::Excluded,
                    None => CandidatePresence::Absent,
                };
                let summary = loaded.head().clone();
                let restored =
                    restore_frames(assembly, &run_id, summary.clone(), admission, latest)
                        .and_then(|restored| view(&restored));
                Ok((summary, presence, restored))
            })
            .await
        }
        Ok(None) => Err(RuntimeError::Absent),
        Err(source) => Err(RuntimeError::Store(source)),
    };
    let (observation, checked_view, reload_cause) = match result {
        Ok((_, CandidatePresence::Present, Ok(observed))) => {
            return Ok(DriverDisposition::Yield(observed))
        }
        Ok((summary, presence, restored)) => match restored {
            Ok(observed) => (Some((Box::new(summary), presence)), Some(observed), None),
            Err(cause) => (
                Some((Box::new(summary), presence)),
                None,
                Some(cause.into_native()),
            ),
        },
        Err(cause) => (None, None, Some(cause.into_native())),
    };
    let error = RuntimeError::Recording {
        operation,
        failure: Box::new(RecordingFailure::Append {
            original,
            candidate,
            outcome: AppendFailure::NotInserted,
            observation,
            reload_cause,
        }),
    };
    if let Some(observed) = checked_view {
        Ok(DriverDisposition::Failed(invocation(
            driver.head.run_id().clone(),
            error,
            Some(observed),
        )))
    } else {
        Err(error)
    }
}

fn runnable(driver: &Driver) -> Result<Call> {
    match driver.current.continuation(&driver.executable)? {
        Continuation::Runnable {
            position, input, ..
        } => Ok(Call {
            position,
            input: input.clone(),
        }),
        _ => Err(RuntimeError::InvalidHistory),
    }
}
async fn conclude<O: MfmValue, F: MfmValue>(
    context: DriverContext<'_>,
    call: StateCall,
    outcome: ProposedStateOutcome<O, F>,
    operation: Operation,
) -> Result<DriverDisposition> {
    let mut next = (*context.driver.current).clone();
    let original = match outcome {
        ProposedStateOutcome::Success { output } => {
            next.operation = RecordedOperation::Succeeded {
                call,
                output: encode(output, operation).await?,
            };
            next.enter(&context.driver.executable)?;
            None
        }
        ProposedStateOutcome::Failure { failure } => {
            let returned = encode_failure(failure, operation).await?;
            next.operation = RecordedOperation::Failed(Failure::Domain {
                call,
                original: returned.object,
            });
            Some(returned.original)
        }
    };
    record(context, next, original, operation).await
}
async fn operational(
    context: DriverContext<'_>,
    failure: Failure,
    original: NativeCause,
    operation: Operation,
) -> Result<DriverDisposition> {
    let mut next = (*context.driver.current).clone();
    next.operation = RecordedOperation::Failed(failure);
    record(context, next, Some(original), operation).await
}
pub(crate) async fn start_pure<S: PureState>(
    context: DriverContext<'_>,
) -> Result<DriverDisposition> {
    if matches!(
        context
            .driver
            .current
            .continuation(&context.driver.executable)?,
        Continuation::AwaitingRecovery(_)
    ) {
        return recover::<S::Failure, mfm_program::Never>(context).await;
    }
    let call = runnable(&context.driver)?;
    let input = call.input.clone();
    let outcome = run_blocking(Operation::PureEvaluate, move || {
        S::evaluate(decode::<S::Input>(&input, Operation::PureEvaluate)?)
            .map_err(|cause| RuntimeError::native(Operation::PureEvaluate, cause))
    })
    .await?;
    conclude(
        context,
        StateCall::Pure(call),
        outcome,
        Operation::PureEvaluate,
    )
    .await
}
pub(crate) async fn start_read<S: ReadState<C>, C: ReadCapabilityContract>(
    context: DriverContext<'_>,
    adapter: Arc<ErasedReadAdapterCallback>,
) -> Result<DriverDisposition>
where
    C::OperationalError: ClassifyError,
{
    if matches!(
        context
            .driver
            .current
            .continuation(&context.driver.executable)?,
        Continuation::AwaitingRecovery(_)
    ) {
        return recover::<S::Failure, C::OperationalError>(context).await;
    }
    let call = runnable(&context.driver)?;
    let input = call.input.clone();
    let intent = run_blocking(Operation::ReadPrepare, move || {
        let input = decode::<S::Input>(&input, Operation::ReadPrepare)?;
        let intent = S::prepare(&input)
            .map_err(|cause| RuntimeError::native(Operation::ReadPrepare, cause))?;
        Object::from_value(&intent)
            .map_err(|source| native(Operation::ReadPrepare, Stage::Encode, source))
    })
    .await?;
    match adapter(&intent).await? {
        AdapterReturn::Operational(returned) => {
            let failure = Failure::Read {
                call,
                intent,
                original: returned.object,
            };
            operational(context, failure, returned.original, Operation::ReadAdapter).await
        }
        AdapterReturn::Observed(evidence) => {
            let input = call.input.clone();
            let retained_intent = intent.clone();
            let retained_evidence = evidence.clone();
            let outcome = run_blocking(Operation::ReadInterpret, move || {
                let input = decode::<S::Input>(&input, Operation::ReadInterpret)?;
                let intent = decode::<C::Intent>(&retained_intent, Operation::ReadBind)?;
                let evidence = decode::<C::Evidence>(&retained_evidence, Operation::ReadBind)?;
                C::bind_evidence(retained_intent.value_ref(), &intent, &evidence)
                    .map_err(|cause| RuntimeError::native(Operation::ReadBind, cause))?;
                S::interpret(input, &evidence)
                    .map_err(|cause| RuntimeError::native(Operation::ReadInterpret, cause))
            })
            .await?;
            conclude(
                context,
                StateCall::Read {
                    call,
                    intent,
                    evidence,
                },
                outcome,
                Operation::ReadInterpret,
            )
            .await
        }
    }
}
pub(crate) async fn start_effect<S: EffectState<C>, C: EffectCapabilityContract>(
    context: DriverContext<'_>,
) -> Result<DriverDisposition> {
    let call = runnable(&context.driver)?;
    let input = call.input.clone();
    let command = run_blocking(Operation::EffectPrepare, move || {
        let input = decode::<S::Input>(&input, Operation::EffectPrepare)?;
        let command = S::prepare(&input)
            .map_err(|cause| RuntimeError::native(Operation::EffectPrepare, cause))?;
        Object::from_value(&command)
            .map_err(|source| native(Operation::EffectPrepare, Stage::Encode, source))
    })
    .await?;
    let effect_id = derive_effect_id(
        context.driver.head.run_id(),
        &context.driver.current.program_ref,
        call.position,
        command.value_ref(),
    )?;
    let mut next = (*context.driver.current).clone();
    next.effect_barrier = Some(call.position.state);
    let effect = EffectCall {
        call,
        effect_id,
        command,
    };
    next.operation = RecordedOperation::EffectPrepared(effect);
    record(context, next, None, Operation::EffectPrepare).await
}
pub(crate) async fn start_pending_effect<S: EffectState<C>, C: EffectCapabilityContract>(
    context: DriverContext<'_>,
    adapter: Arc<ErasedEffectAdapterCallback>,
) -> Result<DriverDisposition>
where
    C::OperationalError: ClassifyError,
{
    match context
        .driver
        .current
        .continuation(&context.driver.executable)?
    {
        Continuation::AwaitingRecovery(_) => {
            return recover::<S::Failure, C::OperationalError>(context).await
        }
        Continuation::AwaitingInterpretation(settlement) => {
            let retained = settlement.clone();
            let settlement = settlement.clone();
            let outcome = run_blocking(Operation::EffectInterpret, move || {
                let input =
                    decode::<S::Input>(&settlement.effect.call.input, Operation::EffectInterpret)?;
                let command =
                    decode::<C::Command>(&settlement.effect.command, Operation::EffectBind)?;
                let evidence = decode::<C::Evidence>(&settlement.evidence, Operation::EffectBind)?;
                C::bind_evidence(&settlement.effect.effect_id, &command, &evidence)
                    .map_err(|cause| RuntimeError::native(Operation::EffectBind, cause))?;
                S::interpret(input, &evidence)
                    .map_err(|cause| RuntimeError::native(Operation::EffectInterpret, cause))
            })
            .await?;
            return conclude(
                context,
                StateCall::Effect(retained),
                outcome,
                Operation::EffectInterpret,
            )
            .await;
        }
        _ => {}
    }
    let Continuation::EffectPending { effect, .. } = context
        .driver
        .current
        .continuation(&context.driver.executable)?
    else {
        return Err(RuntimeError::InvalidHistory);
    };
    let effect = effect.clone();
    let retained = effect.clone();
    run_blocking(Operation::EffectPrepare, move || {
        let input = decode::<S::Input>(&retained.call.input, Operation::EffectPrepare)?;
        let command = S::prepare(&input)
            .map_err(|cause| RuntimeError::native(Operation::EffectPrepare, cause))?;
        let prepared = Object::from_value(&command)
            .map_err(|source| native(Operation::EffectPrepare, Stage::Encode, source))?;
        if prepared != retained.command {
            return Err(RuntimeError::native(
                Operation::EffectPrepare,
                NativeCause::from_error(StateInvariant::Contract),
            ));
        }
        Ok(())
    })
    .await?;
    match adapter(&effect.effect_id, &effect.command).await? {
        AdapterReturn::Operational(returned) => {
            operational(
                context,
                Failure::PendingEffect {
                    effect,
                    original: returned.object,
                },
                returned.original,
                Operation::EffectAdapter,
            )
            .await
        }
        AdapterReturn::Observed(EffectAdapterOutcome::Pending) => {
            Ok(DriverDisposition::Yield(project(&context.driver).await?))
        }
        AdapterReturn::Observed(EffectAdapterOutcome::Settled(evidence)) => {
            let settlement = Settlement { effect, evidence };
            let retained = settlement.clone();
            run_blocking(Operation::EffectBind, move || {
                let command =
                    decode::<C::Command>(&retained.effect.command, Operation::EffectBind)?;
                let evidence = decode::<C::Evidence>(&retained.evidence, Operation::EffectBind)?;
                C::bind_evidence(&retained.effect.effect_id, &command, &evidence)
                    .map_err(|cause| RuntimeError::native(Operation::EffectBind, cause))
            })
            .await?;
            let mut next = (*context.driver.current).clone();
            next.operation = RecordedOperation::EffectSettled(settlement);
            record(context, next, None, Operation::EffectBind).await
        }
    }
}
async fn recover<D: ClassifyError, E: ClassifyError>(
    context: DriverContext<'_>,
) -> Result<DriverDisposition> {
    let current = Arc::clone(&context.driver.current);
    let executable = Arc::clone(&context.driver.executable);
    let next = run_blocking(Operation::Recovery, move || {
        let Continuation::AwaitingRecovery(failure) = current.continuation(&executable)? else {
            return Err(RuntimeError::InvalidHistory);
        };
        let classification = match failure {
            Failure::Domain { original, .. } => {
                decode::<D>(original, Operation::Recovery)?.classify()
            }
            Failure::Read { original, .. } => {
                decode::<E>(original, Operation::Recovery)?.classify()
            }
            Failure::PendingEffect { original, .. } => {
                decode::<E>(original, Operation::Recovery)?.classify()
            }
        };
        let position = failure.call().position;
        let declaration = &executable.program.declarations()[position.state.index()];
        let used = current.usage(position.state)?;
        let declared = declaration.recovery_targets();
        let eligible: Vec<_> = declared
            .iter()
            .copied()
            .filter(|target| current.eligible(&executable, position.state, target.position()))
            .collect();
        let policy_context = mfm_program::RecoveryContext::new(
            failure.phase(),
            mfm_program::RecoveryAllowances::new(
                declaration
                    .allowances()
                    .retries()
                    .checked_sub(used.state_retries)
                    .ok_or(RuntimeError::InvalidHistory)?,
                declaration
                    .allowances()
                    .restarts()
                    .checked_sub(used.state_restarts)
                    .ok_or(RuntimeError::InvalidHistory)?,
            ),
            executable
                .program
                .limits()
                .max_recovery_decisions()
                .checked_sub(used.run_decisions)
                .ok_or(RuntimeError::InvalidHistory)?,
            declared,
            &eligible,
        );
        let request = executable.declarations[position.state.index()]
            .recovery
            .request(classification, &policy_context)?;
        let mut outcome = current.authorize(&executable, failure, request)?;
        let mut next = (*current).clone();
        match &mut outcome {
            RecoveryOutcome::Retry => {
                next.usage[position.state.index()].retries = used
                    .state_retries
                    .checked_add(1)
                    .ok_or(RuntimeError::ArithmeticOverflow)?;
            }
            RecoveryOutcome::Restart { checkpoint } => {
                next.usage[position.state.index()].restarts = used
                    .state_restarts
                    .checked_add(1)
                    .ok_or(RuntimeError::ArithmeticOverflow)?;
                next.checkpoints
                    .retain(|entry| entry.position <= *checkpoint);
            }
            RecoveryOutcome::Stop { root, .. } => {
                if let Failure::Domain { original, .. } = failure {
                    *root = Some(
                        executable.declarations[position.state.index()]
                            .root_map
                            .apply(original.clone())?,
                    );
                }
            }
        }
        next.operation = RecordedOperation::Recovered {
            failure: failure.clone(),
            classification,
            request,
            outcome,
        };
        next.enter(&executable)?;
        Ok(next)
    })
    .await?;
    record(context, next, None, Operation::Recovery).await
}

pub(crate) fn derive_effect_id(
    run_id: &RunId,
    program_ref: &ContentRef,
    position: ExecutionPosition,
    command_ref: &ContentRef,
) -> Result<EffectId> {
    #[derive(Serialize)]
    struct Preimage<'a> {
        command_ref: &'a ContentRef,
        position: ExecutionPosition,
        domain: &'static str,
        program_ref: &'a ContentRef,
        run_id: &'a RunId,
    }
    let canonical = canonical(
        &Preimage {
            command_ref,
            position,
            domain: "mfm.effect-id.v2",
            program_ref,
            run_id,
        },
        Operation::EffectPrepare,
    )?;
    Ok(EffectId::from_digest(canonical.digest_bytes()))
}
fn failure_report(
    record: &RunRecord,
    failure: &Failure,
    reason: mfm_program::StopReason,
    root: Option<&Object>,
) -> Result<crate::FailureReport> {
    crate::FailureReport::new(
        failure.clone(),
        reason,
        record.usage(failure.call().position.state)?,
        root.cloned(),
    )
}
fn view(driver: &Driver) -> Result<RunView> {
    let state = match driver.current.continuation(&driver.executable)? {
        Continuation::Runnable {
            position, reason, ..
        } => RunViewState::Runnable { position, reason },
        Continuation::EffectPending {
            effect,
            latest_failure,
        } => RunViewState::EffectPending {
            effect: effect.clone(),
            latest_failure: latest_failure
                .map(|(original, outcome)| (original.clone(), outcome.clone())),
        },
        Continuation::AwaitingRecovery(failure) => RunViewState::AwaitingRecovery {
            failure: failure.clone(),
        },
        Continuation::AwaitingInterpretation(settlement) => RunViewState::AwaitingInterpretation {
            settlement: settlement.clone(),
        },
        Continuation::Succeeded { output, .. } => RunViewState::Succeeded(output.clone()),
        Continuation::Failed {
            failure,
            reason,
            root,
        } => RunViewState::Failed(failure_report(&driver.current, failure, reason, root)?),
    };
    let RecordedOperation::Admitted { initial, .. } = &driver.admission.operation else {
        return Err(RuntimeError::InvalidHistory);
    };
    Ok(RunView {
        run_id: driver.head.run_id().clone(),
        head_sequence: driver.head.head_sequence(),
        head_digest: driver.head.head_digest().clone(),
        state,
        admitted_context: Arc::new(initial.clone()),
        entry_point: driver.executable.program.entry_point_id().clone(),
    })
}
