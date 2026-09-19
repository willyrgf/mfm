#[cfg(test)]
mod tests;

use crate::state::*;
use crate::{
    CandidatePresence, InvocationFailure, Operation, RecordingFailure, Result, RunView,
    RunViewState, RuntimeError, Stage,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::EffectAdapterOutcome;
use mfm_ids::{ContentRef, EffectId, ExecutionPosition, RunId};
use mfm_journal::{decode_frame, seal_frame, EncodedRunFrame};
use mfm_program::executable::ExecutableMode;
use mfm_program::{callback, Program, ProposedStateOutcome};
use mfm_store::{AppendResult, LoadedRun, RunSummary, Store};
use mfm_values::{InvocationDiagnostic, Object};
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Driver {
    admission: Arc<RunRecord>,
    current: Arc<RunRecord>,
    program: Program,
    head: RunSummary,
}
struct DriverContext<'a> {
    store: &'a Arc<dyn Store>,
    driver: Driver,
}
enum DriverDisposition {
    Continue(Driver),
    Yield(Driver, RunView),
    Failed(InvocationFailure),
}
#[derive(Clone, Copy)]
pub(crate) enum Advancement {
    Manual,
    Terminal,
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
fn canonical<T: Serialize>(value: &T, operation: Operation) -> Result<PlainCanonicalJsonBytes> {
    let error = |source: mfm_canonical::CanonicalError| {
        let size = source
            .serialization_bound()
            .map(
                |(limit, observed_at_least)| mfm_values::SizeViolation::SerializationBound {
                    resource: mfm_values::SizeResource::Frame,
                    limit: limit as u64,
                    observed_at_least: observed_at_least as u64,
                },
            );
        RuntimeError::at(
            operation,
            Stage::Encode,
            InvocationDiagnostic::from_fields("canonical_error", "canonical", &source, size),
        )
    };
    let json =
        mfm_canonical::to_json_bounded(value, mfm_journal::MAX_FRAME_BYTES).map_err(error)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(error)
}
pub(crate) async fn run_blocking<T: Send + 'static, F: FnOnce() -> Result<T> + Send + 'static>(
    operation: Operation,
    job: F,
) -> Result<T> {
    tokio::task::spawn_blocking(job).await.map_err(|source| {
        RuntimeError::at(
            operation,
            Stage::Execute,
            InvocationDiagnostic::from_fields(
                "task_failure",
                "run_blocking",
                if source.is_panic() {
                    "panicked"
                } else {
                    "cancelled"
                },
                None,
            ),
        )
    })?
}

pub(crate) async fn admit(
    store: Arc<dyn Store>,
    run_id: RunId,
    program: Program,
    initial: Object,
    advancement: Advancement,
) -> std::result::Result<RunView, InvocationFailure> {
    let identity = run_id.clone();
    let (program, commit, candidate) = run_blocking(Operation::Admission, move || {
        program
            .admit(&initial, program.admitted_context_contract_ref())
            .map_err(|cause| RuntimeError::at(Operation::Admission, Stage::Decode, cause))?;
        if initial.value_ref() != program.initial_value_ref() {
            return Err(RuntimeError::at(
                Operation::Admission,
                Stage::Execute,
                InvocationDiagnostic::from_fields(
                    "runtime_invariant",
                    "start",
                    &StateInvariant::Identity {
                        field: "initial_value_ref",
                        expected: Box::new(program.initial_value_ref().clone()),
                        actual: Box::new(initial.value_ref().clone()),
                    },
                    None,
                ),
            ));
        }
        let document =
            Object::from_canonical(program.content_ref().clone(), program.canonical_bytes())
                .map_err(RuntimeError::from)?;
        let commit = Arc::new(RunRecord::initial(&program, document, initial)?);
        let candidate = seal(&identity, 1, None, &commit, &program)?;
        Ok((program, commit, candidate))
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
                    RuntimeError::at(
                        Operation::Admission,
                        Stage::Append,
                        InvocationDiagnostic::from_fields(
                            "runtime_invariant",
                            "start",
                            &source,
                            None,
                        ),
                    ),
                    None,
                )
            })?;
            let driver = Driver {
                admission: Arc::clone(&commit),
                current: commit,
                program,
                head,
            };
            let observed = project(&driver).await.map_err(|cause| {
                invocation(
                    run_id,
                    RuntimeError::Projection {
                        acknowledged: Box::new(driver.head.clone()),
                        cause: Box::new(cause),
                    },
                    None,
                )
            })?;
            advance(store, driver, observed, advancement).await
        }
        Ok(AppendResult::NotInserted) => {
            let loaded = store
                .load_run(&run_id, None)
                .await
                .map_err(|source| invocation(run_id.clone(), RuntimeError::Store(source), None))?
                .ok_or_else(|| invocation(run_id.clone(), RuntimeError::Absent, None))?;
            let proposed = Arc::clone(&commit);
            let (driver, observed) = run_blocking(Operation::Restore, move || {
                let driver = restore(program, &run_id, loaded, None)?;
                if driver.admission.operation != proposed.operation {
                    return Err(RuntimeError::AdmissionConflict);
                }
                let observed = view(&driver)?;
                Ok((driver, observed))
            })
            .await
            .map_err(|error| invocation(candidate.run_id().clone(), error, None))?;
            match advancement {
                Advancement::Manual => Ok(observed),
                Advancement::Terminal => advance(store, driver, observed, advancement).await,
            }
        }
        Err(source) => Err(invocation(
            run_id,
            RuntimeError::Recording {
                operation: Operation::Admission,
                failure: Box::new(RecordingFailure::Store {
                    original: None,
                    candidate,
                    cause: source,
                }),
            },
            None,
        )),
    }
}
pub(crate) async fn resume(
    program: Program,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> std::result::Result<RunView, InvocationFailure> {
    let driver = load(program, &store, &run_id)
        .await
        .map_err(|error| invocation(run_id.clone(), error, None))?;
    let observed = project(&driver)
        .await
        .map_err(|error| invocation(run_id, error, None))?;
    advance(store, driver, observed, Advancement::Manual).await
}
pub(crate) async fn read(
    program: Program,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> std::result::Result<RunView, InvocationFailure> {
    let driver = load(program, &store, &run_id)
        .await
        .map_err(|error| invocation(run_id.clone(), error, None))?;
    run_blocking(Operation::Project, move || view(&driver))
        .await
        .map_err(|error| invocation(run_id, error, None))
}
async fn load(program: Program, store: &Arc<dyn Store>, run_id: &RunId) -> Result<Driver> {
    let loaded = store
        .load_run(run_id, None)
        .await
        .map_err(RuntimeError::Store)?
        .ok_or(RuntimeError::Absent)?;
    let run_id = run_id.clone();
    run_blocking(Operation::Restore, move || {
        restore(program, &run_id, loaded, None)
    })
    .await
}
fn bound_frames(
    run_id: &RunId,
    loaded: &LoadedRun,
    probe: Option<u64>,
) -> Result<(EncodedRunFrame, EncodedRunFrame, Option<EncodedRunFrame>)> {
    let frame = |bytes| {
        decode_frame(bytes).map_err(|source| {
            RuntimeError::at(
                Operation::Restore,
                Stage::Decode,
                InvocationDiagnostic::from_fields(
                    "journal_error",
                    "bound_frames",
                    &source,
                    match &source {
                        mfm_journal::JournalError::FrameSize(size) => {
                            Some(mfm_values::SizeViolation::measured(
                                mfm_values::SizeResource::Frame,
                                *size,
                            ))
                        }
                        mfm_journal::JournalError::FrameCount(size) => {
                            Some(mfm_values::SizeViolation::measured(
                                mfm_values::SizeResource::FrameCount,
                                *size,
                            ))
                        }
                        _ => None,
                    },
                ),
            )
        })
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
    program: Program,
    run_id: &RunId,
    loaded: LoadedRun,
    probe: Option<u64>,
) -> Result<Driver> {
    let (admission_frame, latest_frame, _) = bound_frames(run_id, &loaded, probe)?;
    restore_frames(
        program,
        run_id,
        loaded.head().clone(),
        admission_frame,
        latest_frame,
    )
}
fn decode_record(frame: &EncodedRunFrame) -> Result<Arc<RunRecord>> {
    serde_json::from_slice::<RunRecord>(frame.payload().as_bytes())
        .map(Arc::new)
        .map_err(|source| {
            RuntimeError::at(
                Operation::Restore,
                Stage::Decode,
                InvocationDiagnostic::from_fields(
                    "json_error",
                    "decode_record",
                    &mfm_canonical::JsonError::new(source),
                    None,
                ),
            )
        })
}

fn admitted_program<'a>(frame: &EncodedRunFrame, record: &'a RunRecord) -> Result<&'a Object> {
    let RecordedOperation::Admitted { program, .. } = &record.operation else {
        return Err(RuntimeError::InvalidHistory);
    };
    if program.value_ref() != &record.program_ref {
        return Err(RuntimeError::at(
            Operation::Restore,
            Stage::Decode,
            InvocationDiagnostic::from_fields(
                "runtime_invariant",
                "admitted_program",
                &StateInvariant::Identity {
                    field: "program_ref",
                    expected: Box::new(record.program_ref.clone()),
                    actual: Box::new(program.value_ref().clone()),
                },
                None,
            ),
        ));
    }
    check_metadata(frame, record)?;
    Ok(program)
}

pub(crate) async fn program_document(
    store: &Arc<dyn Store>,
    run_id: RunId,
) -> std::result::Result<Object, InvocationFailure> {
    let loaded = store
        .load_run(&run_id, None)
        .await
        .map_err(|cause| invocation(run_id.clone(), RuntimeError::Store(cause), None))?
        .ok_or_else(|| invocation(run_id.clone(), RuntimeError::Absent, None))?;
    let identity = run_id.clone();
    run_blocking(Operation::Restore, move || {
        let (admission_frame, _, _) = bound_frames(&identity, &loaded, None)?;
        let admission = decode_record(&admission_frame)?;
        admitted_program(&admission_frame, &admission).cloned()
    })
    .await
    .map_err(|cause| invocation(run_id, cause, None))
}

fn restore_frames(
    program: Program,
    run_id: &RunId,
    head: RunSummary,
    admission_frame: EncodedRunFrame,
    latest_frame: EncodedRunFrame,
) -> Result<Driver> {
    let admission = decode_record(&admission_frame)?;
    let retained = admitted_program(&admission_frame, &admission)?;
    if retained.value_ref() != program.content_ref() {
        return Err(RuntimeError::at(
            Operation::Restore,
            Stage::Decode,
            InvocationDiagnostic::from_fields(
                "runtime_invariant",
                "restore_frames",
                &StateInvariant::Identity {
                    field: "program_ref",
                    expected: Box::new(program.content_ref().clone()),
                    actual: Box::new(retained.value_ref().clone()),
                },
                None,
            ),
        ));
    }
    admission.validate_current(run_id, 1, &program, true)?;
    let current = if head.head_sequence() == 1 {
        Arc::clone(&admission)
    } else {
        let commit = decode_record(&latest_frame)?;
        commit.validate_current(run_id, latest_frame.run_sequence(), &program, true)?;
        check_metadata(&latest_frame, &commit)?;
        commit
    };
    Ok(Driver {
        admission,
        current,
        program,
        head,
    })
}
async fn advance(
    store: Arc<dyn Store>,
    mut driver: Driver,
    mut observed: RunView,
    advancement: Advancement,
) -> std::result::Result<RunView, InvocationFailure> {
    loop {
        let run_id = driver.head.run_id().clone();
        let continuation = driver
            .current
            .continuation(&driver.program)
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
        let program = context.driver.program.clone();
        let Some(executable) = program.executable(position.state) else {
            return Err(invocation(
                run_id,
                RuntimeError::InvalidHistory,
                Some(observed),
            ));
        };
        let result = match executable.mode() {
            ExecutableMode::Pure { callbacks } => start_pure(context, callbacks).await,
            ExecutableMode::Read { callbacks, adapter } => {
                start_read(context, callbacks, Arc::clone(adapter)).await
            }
            ExecutableMode::Effect { callbacks, adapter } => {
                if runnable {
                    start_effect(context, callbacks).await
                } else {
                    start_pending_effect(context, callbacks, Arc::clone(adapter)).await
                }
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => return Err(invocation(run_id, error, Some(observed))),
        };
        match result {
            DriverDisposition::Continue(next) => {
                observed = project(&next).await.map_err(|cause| {
                    invocation(
                        run_id,
                        RuntimeError::Projection {
                            acknowledged: Box::new(next.head.clone()),
                            cause: Box::new(cause),
                        },
                        Some(observed),
                    )
                })?;
                driver = next;
            }
            DriverDisposition::Yield(next, checked) => match advancement {
                Advancement::Manual => return Ok(checked),
                Advancement::Terminal => {
                    driver = next;
                    observed = checked;
                }
            },
            DriverDisposition::Failed(failure) => return Err(failure),
        }
    }
}
fn seal(
    run_id: &RunId,
    sequence: u64,
    previous: Option<&mfm_ids::ContentDigest>,
    commit: &RunRecord,
    program: &Program,
) -> Result<EncodedRunFrame> {
    commit.validate_current(run_id, sequence, program, false)?;
    if let Continuation::Failed { failure, reason } = commit.continuation(program)? {
        failure_report(run_id, program, commit, failure, reason)?;
    }
    let payload = canonical(commit, Operation::Record)?;
    let candidate = seal_frame(run_id, sequence, previous, &payload).map_err(|source| {
        RuntimeError::at(
            Operation::Record,
            Stage::Seal,
            InvocationDiagnostic::from_fields(
                "journal_error",
                "seal",
                &source,
                match &source {
                    mfm_journal::JournalError::FrameSize(size) => Some(
                        mfm_values::SizeViolation::measured(mfm_values::SizeResource::Frame, *size),
                    ),
                    mfm_journal::JournalError::FrameCount(size) => {
                        Some(mfm_values::SizeViolation::measured(
                            mfm_values::SizeResource::FrameCount,
                            *size,
                        ))
                    }
                    _ => None,
                },
            ),
        )
    })?;
    check_metadata(&candidate, commit)?;
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
    run_blocking(Operation::Project, move || view(&projected)).await
}
async fn record(
    context: DriverContext<'_>,
    next: RunRecord,
    operation: Operation,
) -> Result<DriverDisposition> {
    let DriverContext { store, driver } = context;
    let commit = Arc::new(next);
    let next_commit = Arc::clone(&commit);
    let program = driver.program.clone();
    let head = driver.head.clone();
    let candidate = run_blocking(operation, move || {
        seal(
            head.run_id(),
            head.head_sequence()
                .checked_add(1)
                .ok_or(RuntimeError::ArithmeticOverflow)?,
            Some(head.head_digest()),
            &next_commit,
            &program,
        )
    })
    .await
    .map_err(|cause| match &commit.operation {
        RecordedOperation::Failed(original)
        | RecordedOperation::Recovered {
            failure: original, ..
        } => RuntimeError::Recording {
            operation,
            failure: Box::new(RecordingFailure::BeforeAppend {
                original: original.clone(),
                cause: Box::new(cause),
            }),
        },
        _ => cause,
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
            .map_err(|source| {
                RuntimeError::at(
                    operation,
                    Stage::Append,
                    InvocationDiagnostic::from_fields("runtime_invariant", "record", &source, None),
                )
            })?;
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
                let observed = project(&next)
                    .await
                    .map_err(|cause| RuntimeError::Projection {
                        acknowledged: Box::new(next.head.clone()),
                        cause: Box::new(cause),
                    })?;
                return Ok(DriverDisposition::Failed(
                    InvocationFailure::RecoveryStopped { observed },
                ));
            }
            if matches!(next.current.operation, RecordedOperation::Recovered { .. }) {
                let observed = project(&next)
                    .await
                    .map_err(|cause| RuntimeError::Projection {
                        acknowledged: Box::new(next.head.clone()),
                        cause: Box::new(cause),
                    })?;
                Ok(DriverDisposition::Yield(next, observed))
            } else {
                Ok(DriverDisposition::Continue(next))
            }
        }
        Ok(AppendResult::NotInserted) => {
            let original = match &commit.operation {
                RecordedOperation::Failed(original)
                | RecordedOperation::Recovered {
                    failure: original, ..
                } => Some(original.clone()),
                _ => None,
            };
            reconcile(store, driver, candidate, original, operation).await
        }
        Err(cause) => {
            let original = match &commit.operation {
                RecordedOperation::Failed(original)
                | RecordedOperation::Recovered {
                    failure: original, ..
                } => Some(original.clone()),
                _ => None,
            };
            Err(RuntimeError::Recording {
                operation,
                failure: Box::new(RecordingFailure::Store {
                    original,
                    candidate,
                    cause,
                }),
            })
        }
    }
}

async fn reconcile(
    store: &Arc<dyn Store>,
    driver: Driver,
    candidate: EncodedRunFrame,
    original: Option<Failure>,
    operation: Operation,
) -> Result<DriverDisposition> {
    let sequence = candidate.run_sequence();
    let loaded = store.load_run(driver.head.run_id(), Some(sequence)).await;
    let run_id = driver.head.run_id().clone();
    let program = driver.program.clone();
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
                let restored = restore_frames(program, &run_id, summary.clone(), admission, latest)
                    .and_then(|restored| {
                        let observed = view(&restored)?;
                        Ok((restored, observed))
                    });
                Ok((summary, presence, restored))
            })
            .await
        }
        Ok(None) => Err(RuntimeError::Absent),
        Err(source) => Err(RuntimeError::Store(source)),
    };
    let (observation, checked_view, reload_cause) = match result {
        Ok((_, CandidatePresence::Present, Ok((restored, observed)))) => {
            if matches!(
                &restored.current.operation,
                RecordedOperation::Recovered {
                    failure: Failure::PendingEffect { .. },
                    outcome: RecoveryOutcome::Stop { .. },
                    ..
                }
            ) {
                return Ok(DriverDisposition::Failed(
                    InvocationFailure::RecoveryStopped { observed },
                ));
            }
            return Ok(DriverDisposition::Yield(restored, observed));
        }
        Ok((summary, presence, restored)) => match restored {
            Ok((_, observed)) => (Some((Box::new(summary), presence)), Some(observed), None),
            Err(cause) => (
                Some((Box::new(summary), presence)),
                None,
                Some(Box::new(cause)),
            ),
        },
        Err(cause) => (None, None, Some(Box::new(cause))),
    };
    let error = RuntimeError::Recording {
        operation,
        failure: Box::new(RecordingFailure::NotInserted {
            original,
            candidate,
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
    match driver.current.continuation(&driver.program)? {
        Continuation::Runnable {
            position, input, ..
        } => Ok(Call {
            position,
            input: input.clone(),
        }),
        _ => Err(RuntimeError::InvalidHistory),
    }
}
async fn conclude(
    context: DriverContext<'_>,
    call: StateCall,
    outcome: callback::Outcome,
    operation: Operation,
) -> Result<DriverDisposition> {
    let mut next = (*context.driver.current).clone();
    match outcome {
        ProposedStateOutcome::Success { output } => {
            next.operation = RecordedOperation::Succeeded { call, output };
            next.enter(&context.driver.program)?;
        }
        ProposedStateOutcome::Failure { failure: original } => {
            next.operation = RecordedOperation::Failed(Failure::Domain { call, original });
        }
    };
    record(context, next, operation).await
}
async fn operational(
    context: DriverContext<'_>,
    failure: Failure,
    operation: Operation,
) -> Result<DriverDisposition> {
    let mut next = (*context.driver.current).clone();
    next.operation = RecordedOperation::Failed(failure);
    record(context, next, operation).await
}
async fn start_pure(
    context: DriverContext<'_>,
    callbacks: &callback::PureCallbacks,
) -> Result<DriverDisposition> {
    if matches!(
        context
            .driver
            .current
            .continuation(&context.driver.program)?,
        Continuation::AwaitingRecovery(_)
    ) {
        return recover(context, callbacks.classify, callbacks.classify).await;
    }
    let call = runnable(&context.driver)?;
    let outcome = (callbacks.evaluate)(call.input.clone(), call.position)
        .await
        .map_err(|error| RuntimeError::callback(Operation::PureEvaluate, error))?;
    conclude(
        context,
        StateCall::Pure(call),
        outcome,
        Operation::PureEvaluate,
    )
    .await
}
async fn start_read(
    context: DriverContext<'_>,
    callbacks: &callback::ReadCallbacks,
    adapter: Arc<callback::ErasedReadAdapterCallback>,
) -> Result<DriverDisposition> {
    if matches!(
        context
            .driver
            .current
            .continuation(&context.driver.program)?,
        Continuation::AwaitingRecovery(_)
    ) {
        return recover(context, callbacks.classify, callbacks.classify_operational).await;
    }
    let call = runnable(&context.driver)?;
    let intent = (callbacks.prepare)(call.input.clone())
        .await
        .map_err(|error| RuntimeError::callback(Operation::ReadPrepare, error))?;
    match adapter(call.position, &intent)
        .await
        .map_err(|error| RuntimeError::callback(Operation::ReadAdapter, error))?
    {
        Err(original) => {
            operational(
                context,
                Failure::Read {
                    call,
                    intent,
                    original,
                },
                Operation::ReadAdapter,
            )
            .await
        }
        Ok(evidence) => {
            (callbacks.bind)(intent.clone(), evidence.clone())
                .await
                .map_err(|error| RuntimeError::callback(Operation::ReadBind, error))?;
            let outcome = (callbacks.interpret)(
                call.input.clone(),
                intent.clone(),
                evidence.clone(),
                call.position,
            )
            .await
            .map_err(|error| RuntimeError::callback(Operation::ReadInterpret, error))?;
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
async fn start_effect(
    context: DriverContext<'_>,
    callbacks: &callback::EffectCallbacks,
) -> Result<DriverDisposition> {
    let call = runnable(&context.driver)?;
    let command = (callbacks.prepare)(call.input.clone())
        .await
        .map_err(|error| RuntimeError::callback(Operation::EffectPrepare, error))?;
    let effect_id = derive_effect_id(
        context.driver.head.run_id(),
        &context.driver.current.program_ref,
        call.position,
        command.value_ref(),
    )?;
    (callbacks.validate_command)(command.clone())
        .await
        .map_err(|error| RuntimeError::callback(Operation::EffectPrepare, error))?;
    let mut next = (*context.driver.current).clone();
    next.effect_barrier = Some(call.position.state);
    next.operation = RecordedOperation::EffectPrepared(EffectCall {
        call,
        effect_id,
        command,
    });
    record(context, next, Operation::EffectPrepare).await
}
async fn start_pending_effect(
    context: DriverContext<'_>,
    callbacks: &callback::EffectCallbacks,
    adapter: Arc<callback::ErasedEffectAdapterCallback>,
) -> Result<DriverDisposition> {
    match context
        .driver
        .current
        .continuation(&context.driver.program)?
    {
        Continuation::AwaitingRecovery(_) => {
            return recover(context, callbacks.classify, callbacks.classify_operational).await;
        }
        Continuation::AwaitingInterpretation(settlement) => {
            let settlement = settlement.clone();
            let outcome = (callbacks.interpret)(
                settlement.effect.call.input.clone(),
                settlement.effect.effect_id.clone(),
                settlement.effect.command.clone(),
                settlement.evidence.clone(),
                settlement.effect.call.position,
            )
            .await
            .map_err(|error| RuntimeError::callback(Operation::EffectInterpret, error))?;
            return conclude(
                context,
                StateCall::Effect(settlement),
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
        .continuation(&context.driver.program)?
    else {
        return Err(RuntimeError::InvalidHistory);
    };
    let effect = effect.clone();
    let prepared = (callbacks.prepare)(effect.call.input.clone())
        .await
        .map_err(|error| RuntimeError::callback(Operation::EffectPrepare, error))?;
    if prepared != effect.command {
        return Err(RuntimeError::native(
            Operation::EffectPrepare,
            InvocationDiagnostic::from_fields(
                "runtime_invariant",
                "start_pending_effect",
                &StateInvariant::Contract,
                None,
            ),
        ));
    }
    match adapter(effect.call.position, &effect.effect_id, &effect.command)
        .await
        .map_err(|error| RuntimeError::callback(Operation::EffectAdapter, error))?
    {
        Err(original) => {
            operational(
                context,
                Failure::PendingEffect { effect, original },
                Operation::EffectAdapter,
            )
            .await
        }
        Ok(EffectAdapterOutcome::Pending) => {
            let observed = project(&context.driver).await?;
            Ok(DriverDisposition::Yield(context.driver, observed))
        }
        Ok(EffectAdapterOutcome::Settled(evidence)) => {
            (callbacks.bind)(
                effect.effect_id.clone(),
                effect.command.clone(),
                evidence.clone(),
            )
            .await
            .map_err(|error| RuntimeError::callback(Operation::EffectBind, error))?;
            let mut next = (*context.driver.current).clone();
            next.operation = RecordedOperation::EffectSettled(Settlement { effect, evidence });
            record(context, next, Operation::EffectBind).await
        }
    }
}
async fn recover(
    context: DriverContext<'_>,
    classify: callback::Classifier,
    classify_operational: callback::Classifier,
) -> Result<DriverDisposition> {
    let current = Arc::clone(&context.driver.current);
    let program = context.driver.program.clone();
    let Continuation::AwaitingRecovery(failure) = current.continuation(&program)? else {
        return Err(RuntimeError::InvalidHistory);
    };
    let classification = match failure {
        Failure::Domain { original, .. } => classify(original.clone()).await,
        Failure::Read { original, .. } | Failure::PendingEffect { original, .. } => {
            classify_operational(original.clone()).await
        }
    }
    .map_err(|error| RuntimeError::callback(Operation::Recovery, error))?;
    let next = run_blocking(Operation::Recovery, move || {
        let Continuation::AwaitingRecovery(failure) = current.continuation(&program)? else {
            return Err(RuntimeError::InvalidHistory);
        };
        let position = failure.call().position;
        let declaration = &program.declarations()[position.state.index()];
        let used = current.usage(position.state)?;
        let declared = declaration.recovery_targets();
        let eligible: Vec<_> = declared
            .iter()
            .copied()
            .filter(|target| current.eligible(&program, position.state, target.position()))
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
            program
                .limits()
                .max_recovery_decisions()
                .checked_sub(used.run_decisions)
                .ok_or(RuntimeError::InvalidHistory)?,
            declared,
            &eligible,
        );
        let request = program
            .executable(position.state)
            .ok_or(RuntimeError::InvalidHistory)?
            .request(classification, &policy_context)
            .map_err(|cause| RuntimeError::callback(Operation::Recovery, cause))?;
        let mut outcome = current.authorize(&program, failure, request)?;
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
            RecoveryOutcome::Stop { .. } => {}
        }
        next.operation = RecordedOperation::Recovered {
            failure: failure.clone(),
            classification,
            request,
            outcome,
        };
        next.enter(&program)?;
        Ok(next)
    })
    .await?;
    record(context, next, Operation::Recovery).await
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
    run_id: &RunId,
    program: &Program,
    record: &RunRecord,
    failure: &Failure,
    reason: mfm_program::StopReason,
) -> Result<crate::FailureReport> {
    let declaration = program
        .declarations()
        .get(failure.call().position.state.index())
        .ok_or(RuntimeError::InvalidHistory)?;
    crate::FailureReport::new(
        run_id,
        program.content_ref(),
        declaration,
        failure.clone(),
        reason,
        record.usage(failure.call().position.state)?,
    )
}
fn view(driver: &Driver) -> Result<RunView> {
    #[cfg(test)]
    tests::projection_fault(driver)?;
    let state = match driver.current.continuation(&driver.program)? {
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
        Continuation::Succeeded { output, .. } => {
            driver
                .program
                .admit(output, driver.program.root_success_contract_ref())
                .map_err(|cause| RuntimeError::at(Operation::Project, Stage::Decode, cause))?;
            RunViewState::Succeeded(output.clone())
        }
        Continuation::Failed { failure, reason } => RunViewState::Failed(failure_report(
            driver.head.run_id(),
            &driver.program,
            &driver.current,
            failure,
            reason,
        )?),
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
        entry_point: driver.program.entry_point_id().clone(),
    })
}
