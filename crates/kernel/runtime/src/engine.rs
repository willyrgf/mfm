mod fold;
use fold::{Cursor, FoldState};
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{AdapterError, EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition, RunId};
use mfm_journal::{
    EncodedRunFrame, JournalError, JournalHistory, JournalObject, JournalRecord, StoredRunBytes,
};
use mfm_program::{EffectState, Program, ProposedStateOutcome, PureState, ReadState};
use mfm_store::{AppendResult, Store};
use mfm_values::MfmValue;
use serde::Serialize;

use crate::assembly::{
    qualify_hot, AssemblyInner, EffectPendingStart, ErasedEffectAdapterCallback,
    ErasedReadAdapterCallback, ExecutableMode, ExecutableProgram, QualifiedValue, ReadStart,
    RuntimeAssembly, StateStart, ValueCodec,
};
use crate::{
    EffectAdapterOutcome, Result, RunView, RunViewState, RunnableReason, RuntimeError, ValueView,
};

pub(crate) enum DriverDisposition {
    Continue(Accumulator),
    Reload,
    Yield(Accumulator),
    Stopped {
        accumulator: Accumulator,
        incident: Box<crate::AdapterIncidentView>,
        reason: mfm_journal::StopCode,
    },
}

pub(crate) struct DriverContext<'a> {
    store: &'a Arc<dyn Store>,
    accumulator: Accumulator,
}

pub(crate) struct Accumulator {
    admitted_context: Arc<ValueView>,
    executable: ExecutableProgram,
    history: JournalHistory,
    state: FoldState,
}

fn invocation_error(
    run_id: RunId,
    error: RuntimeError,
    last_observed: Option<RunView>,
) -> crate::InvocationFailure {
    crate::InvocationFailure::Execution {
        run_id,
        error,
        last_observed,
    }
}

pub(crate) async fn start<T: MfmValue>(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
    program: Program,
    c0: T,
) -> std::result::Result<RunView, crate::InvocationFailure> {
    let identity = run_id.clone();
    let proposed = run_blocking(move || prepare_admission(assembly, run_id, program, c0))
        .await
        .map_err(|error| invocation_error(identity.clone(), error, None))?;
    let result = store
        .append_run(&proposed.genesis)
        .await
        .map_err(|error| invocation_error(identity.clone(), RuntimeError::from(error), None))?;
    match result {
        AppendResult::Inserted => {
            let accumulator = run_blocking(move || {
                accumulator_from_inserted_genesis(
                    proposed.executable,
                    proposed.c0,
                    proposed.genesis,
                )
            })
            .await
            .map_err(|error| invocation_error(identity.clone(), error, None))?;
            advance_until_stable(store, accumulator).await
        }
        AppendResult::NotInserted => {
            let stored = store
                .load_run(&identity)
                .await
                .map_err(|error| {
                    invocation_error(identity.clone(), RuntimeError::from(error), None)
                })?
                .ok_or_else(|| invocation_error(identity.clone(), RuntimeError::Internal, None))?;
            let run_id = identity.clone();
            run_blocking(move || {
                view(&fold_collision(
                    run_id,
                    stored,
                    proposed.executable,
                    proposed.c0,
                )?)
            })
            .await
            .map_err(|error| invocation_error(identity, error, None))
        }
    }
}

pub(crate) async fn resume(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> std::result::Result<RunView, crate::InvocationFailure> {
    let accumulator = load_and_fold(assembly, &store, &run_id, false)
        .await
        .map_err(|error| invocation_error(run_id, error, None))?;
    advance_until_stable(store, accumulator).await
}

pub(crate) async fn read(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> std::result::Result<RunView, crate::InvocationFailure> {
    let accumulator = load_and_fold(assembly, &store, &run_id, false)
        .await
        .map_err(|error| invocation_error(run_id.clone(), error, None))?;
    run_blocking(move || view(&accumulator))
        .await
        .map_err(|error| invocation_error(run_id, error, None))
}

struct PreparedAdmission {
    executable: ExecutableProgram,
    c0: QualifiedValue,
    genesis: EncodedRunFrame,
}

fn prepare_admission<T: MfmValue>(
    assembly: Arc<AssemblyInner>,
    run_id: RunId,
    program: Program,
    c0: T,
) -> Result<PreparedAdmission> {
    let c0 = qualify_hot(c0).map_err(RuntimeError::from)?;
    if program.admitted_context_contract_ref() != &c0.contract_ref
        || program.initial_value_ref() != &c0.value_ref
    {
        return Err(RuntimeError::Internal);
    }
    let executable = RuntimeAssembly { inner: assembly }.associate(program)?;
    let genesis = EncodedRunFrame::admission(
        &run_id,
        executable.program.content_ref(),
        executable.program.canonical_bytes(),
        &c0.value_ref,
        c0.canonical.as_bytes(),
    )
    .map_err(map_local_journal_error)?;
    validate_admission_bound(&executable.program, genesis.canonical_bytes().len())?;
    Ok(PreparedAdmission {
        executable,
        c0,
        genesis,
    })
}

fn validate_admission_bound(program: &Program, genesis_bytes: usize) -> Result<()> {
    use mfm_program::{ConclusionBound, Execution};
    for declaration in program.declarations() {
        let maximum = match declaration.execution() {
            Execution::Pure { bound } | Execution::Read { bound, .. } => bound.max_frame_bytes(),
            Execution::Effect { bounds, .. } => {
                bounds.prepare_bytes().max(bounds.conclusion_bytes())
            }
        };
        crate::check_size(
            crate::SizeResource::Frame,
            maximum,
            mfm_journal::MAX_FRAME_BYTES as u64,
        )?;
    }
    let bound = program
        .history_bound(
            ConclusionBound::new(genesis_bytes as u64)
                .map_err(|_| RuntimeError::ArithmeticOverflow)?,
        )
        .map_err(|_| RuntimeError::ArithmeticOverflow)?;
    crate::check_size(
        crate::SizeResource::FrameCount,
        bound.frames(),
        mfm_journal::MAX_RUN_FRAMES,
    )?;
    crate::check_size(
        crate::SizeResource::HistoryBytes,
        bound.bytes(),
        mfm_journal::MAX_RUN_BYTES,
    )?;
    Ok(())
}

fn accumulator_from_inserted_genesis(
    executable: ExecutableProgram,
    c0: QualifiedValue,
    genesis: EncodedRunFrame,
) -> Result<Accumulator> {
    let history = JournalHistory::from_genesis(genesis).map_err(map_local_journal_error)?;
    let admission_matches = match history.records().next() {
        Some(JournalRecord::RunAdmitted {
            program,
            admitted_context,
        }) => {
            journal_object_matches_program(program, &executable.program)
                && journal_object_matches_value(admitted_context, &c0)
        }
        _ => false,
    };
    if !admission_matches {
        return Err(RuntimeError::Internal);
    }
    let admitted_context = Arc::new(retained_view(&c0));
    let state = FoldState::initial(&executable, c0)?;
    Ok(Accumulator {
        admitted_context,
        executable,
        history,
        state,
    })
}

async fn load_and_fold(
    assembly: Arc<AssemblyInner>,
    store: &Arc<dyn Store>,
    run_id: &RunId,
    required_reload: bool,
) -> Result<Accumulator> {
    let stored = store
        .load_run(run_id)
        .await
        .map_err(RuntimeError::from)?
        .ok_or(if required_reload {
            RuntimeError::Internal
        } else {
            RuntimeError::Absent
        })?;
    let run_id = run_id.clone();
    run_blocking(move || fold_retained(assembly, run_id, stored)).await
}

async fn advance_until_stable(
    store: Arc<dyn Store>,
    mut accumulator: Accumulator,
) -> std::result::Result<RunView, crate::InvocationFailure> {
    loop {
        let run_id = accumulator.history.run_id().clone();
        if matches!(
            accumulator.state.cursor,
            Cursor::Succeeded(_) | Cursor::Failed(_)
        ) {
            return run_blocking(move || view(&accumulator))
                .await
                .map_err(|error| invocation_error(run_id, error, None));
        }
        let observed =
            view(&accumulator).map_err(|error| invocation_error(run_id.clone(), error, None))?;
        let invocation = match &accumulator.state.cursor {
            Cursor::Runnable { position, .. } => {
                match &accumulator.executable.declarations[position.state.index()].mode {
                    ExecutableMode::Pure { start } => DriverInvocation::State { start: *start },
                    ExecutableMode::Read { start, adapter, .. } => DriverInvocation::Read {
                        start: *start,
                        adapter: Arc::clone(adapter),
                    },
                    ExecutableMode::Effect { prepare, .. } => {
                        DriverInvocation::State { start: *prepare }
                    }
                }
            }
            Cursor::EffectPending { position, .. } => {
                let ExecutableMode::Effect {
                    start_pending,
                    adapter,
                    ..
                } = &accumulator.executable.declarations[position.state.index()].mode
                else {
                    return Err(invocation_error(
                        run_id,
                        RuntimeError::Internal,
                        Some(observed),
                    ));
                };
                DriverInvocation::EffectPending {
                    start: *start_pending,
                    adapter: Arc::clone(adapter),
                }
            }
            Cursor::Succeeded(_) | Cursor::Failed(_) => return Ok(observed),
        };
        let assembly = Arc::clone(&accumulator.executable._assembly);
        let context = DriverContext {
            store: &store,
            accumulator,
        };
        let disposition = match invocation {
            DriverInvocation::State { start } => start(context).await,
            DriverInvocation::Read { start, adapter } => start(context, adapter).await,
            DriverInvocation::EffectPending { start, adapter } => start(context, adapter).await,
        };
        match disposition {
            Err(error) => return Err(invocation_error(run_id, error, Some(observed))),
            Ok(DriverDisposition::Continue(next)) => accumulator = next,
            Ok(DriverDisposition::Reload) => {
                let loaded = load_and_fold(assembly, &store, &run_id, true).await;
                return match loaded {
                    Ok(loaded) => run_blocking(move || view(&loaded))
                        .await
                        .map_err(|error| invocation_error(run_id, error, Some(observed))),
                    Err(error) => Err(invocation_error(run_id, error, Some(observed))),
                };
            }
            Ok(DriverDisposition::Yield(pending)) => {
                return run_blocking(move || view(&pending))
                    .await
                    .map_err(|error| invocation_error(run_id, error, Some(observed)))
            }
            Ok(DriverDisposition::Stopped {
                accumulator,
                incident,
                reason,
            }) => {
                let pending = run_blocking(move || view(&accumulator))
                    .await
                    .map_err(|error| invocation_error(run_id, error, Some(observed)))?;
                return Err(crate::InvocationFailure::RecoveryStopped {
                    observed: pending,
                    incident,
                    reason: crate::report::stop_reason(reason),
                });
            }
        }
    }
}

enum DriverInvocation {
    State {
        start: StateStart,
    },
    Read {
        start: ReadStart,
        adapter: Arc<ErasedReadAdapterCallback>,
    },
    EffectPending {
        start: EffectPendingStart,
        adapter: Arc<ErasedEffectAdapterCallback>,
    },
}

fn current(accumulator: &Accumulator) -> Result<(ExecutionPosition, Arc<QualifiedValue>)> {
    match &accumulator.state.cursor {
        Cursor::Runnable {
            position, input, ..
        } => Ok((*position, Arc::clone(input))),
        _ => Err(RuntimeError::Internal),
    }
}

fn typed_input<T: MfmValue>(input: &QualifiedValue) -> Result<T> {
    if input.contract_ref
        != mfm_program::nominal_contract_ref::<T>().map_err(|_| RuntimeError::Internal)?
    {
        return Err(RuntimeError::Internal);
    }
    serde_json::from_slice(input.canonical.as_bytes()).map_err(|_| RuntimeError::Internal)
}

fn object(value: &QualifiedValue) -> Result<JournalObject<'_>> {
    JournalObject::new(&value.value_ref, value.canonical.as_bytes())
        .map_err(map_local_journal_error)
}

fn copy_value(value: &QualifiedValue, codec: &ValueCodec) -> Result<QualifiedValue> {
    codec
        .qualify(&value.value_ref, value.canonical.as_bytes())
        .map_err(|_| RuntimeError::Internal)
}

fn decide(
    accumulator: &Accumulator,
    position: ExecutionPosition,
    phase: mfm_program::ExecutionPhase,
    incident: crate::assembly::recovery::QualifiedIncident,
) -> Result<mfm_journal::RecoveryDecision> {
    use mfm_journal::{RecoveryDecision, StopCode};
    use mfm_program::{Assessment, RecoveryAllowances, RecoveryContext, RecoveryRequest};
    let declaration = &accumulator.executable.program.declarations()[position.state.index()];
    let usage = accumulator.state.usage(position.state)?;
    let eligible: Vec<_> = declaration
        .recovery_targets()
        .iter()
        .copied()
        .filter(|target| {
            accumulator
                .state
                .eligible(&accumulator.executable, position.state, target.position())
        })
        .collect();
    let context = RecoveryContext::new(
        phase,
        RecoveryAllowances::new(
            declaration
                .allowances()
                .retries()
                .checked_sub(usage.state_retries)
                .ok_or(RuntimeError::Internal)?,
            declaration
                .allowances()
                .restarts()
                .checked_sub(usage.state_restarts)
                .ok_or(RuntimeError::Internal)?,
        ),
        accumulator
            .executable
            .program
            .limits()
            .max_recovery_decisions()
            .checked_sub(usage.run_decisions)
            .ok_or(RuntimeError::Internal)?,
        &eligible,
    );
    let (assessment, request) = accumulator.executable.declarations[position.state.index()]
        .recovery
        .request(incident, &context)?;
    if assessment == Assessment::Nonrecoverable {
        return Ok(RecoveryDecision::Stop {
            reason: StopCode::Nonrecoverable,
        });
    }
    let decision = match request {
        RecoveryRequest::Stop => RecoveryDecision::Stop {
            reason: StopCode::Requested,
        },
        RecoveryRequest::RetryState => RecoveryDecision::Retry,
        RecoveryRequest::Restart(target) => RecoveryDecision::Restart {
            checkpoint: target.position(),
        },
    };
    if phase == mfm_program::ExecutionPhase::EffectPending {
        return Ok(match decision {
            RecoveryDecision::Restart { .. } => RecoveryDecision::Stop {
                reason: StopCode::EffectBarrier,
            },
            decision => decision,
        });
    }
    Ok(
        match accumulator.state.recovery_denial(
            &accumulator.executable,
            position.state,
            decision,
        )? {
            Some(reason) => RecoveryDecision::Stop { reason },
            None => decision,
        },
    )
}

fn conclude<O: MfmValue, F: MfmValue>(
    accumulator: &Accumulator,
    position: ExecutionPosition,
    phase: mfm_program::ExecutionPhase,
    proposed: ProposedStateOutcome<O, F>,
) -> Result<mfm_journal::DomainConclusion<QualifiedValue>> {
    use mfm_journal::{DomainConclusion, DomainDecision, RecoveryDecision, StopCode};
    match proposed {
        ProposedStateOutcome::Success { output } => Ok(DomainConclusion::Success {
            output: qualify_hot(output).map_err(RuntimeError::from)?,
        }),
        ProposedStateOutcome::Failure { failure } => {
            let original = qualify_hot(failure).map_err(RuntimeError::from)?;
            let selected = &accumulator.executable.declarations[position.state.index()];
            let incident = crate::assembly::recovery::QualifiedIncident::Domain(copy_value(
                &original,
                &selected.failure_codec,
            )?);
            let mut decision = decide(accumulator, position, phase, incident)?;
            if phase == mfm_program::ExecutionPhase::EffectSettled
                && !matches!(decision, RecoveryDecision::Stop { .. })
            {
                decision = RecoveryDecision::Stop {
                    reason: StopCode::EffectSettled,
                };
            }
            let decision = match decision {
                RecoveryDecision::Retry => DomainDecision::Retry,
                RecoveryDecision::Restart { checkpoint } => DomainDecision::Restart { checkpoint },
                RecoveryDecision::Stop { reason } => DomainDecision::Stop {
                    reason,
                    root: selected
                        .root_map
                        .apply(copy_value(&original, &selected.failure_codec)?)?,
                },
            };
            Ok(DomainConclusion::Failure { original, decision })
        }
    }
}

fn domain_objects(
    outcome: &mfm_journal::DomainConclusion<QualifiedValue>,
) -> Result<mfm_journal::DomainConclusion<JournalObject<'_>>> {
    use mfm_journal::{DomainConclusion, DomainDecision};
    Ok(match outcome {
        DomainConclusion::Success { output } => DomainConclusion::Success {
            output: object(output)?,
        },
        DomainConclusion::Failure { original, decision } => DomainConclusion::Failure {
            original: object(original)?,
            decision: match decision {
                DomainDecision::Retry => DomainDecision::Retry,
                DomainDecision::Restart { checkpoint } => DomainDecision::Restart {
                    checkpoint: *checkpoint,
                },
                DomainDecision::Stop { reason, root } => DomainDecision::Stop {
                    reason: *reason,
                    root: object(root)?,
                },
            },
        },
    })
}

pub(crate) async fn start_pure<S: PureState>(
    context: DriverContext<'_>,
) -> Result<DriverDisposition> {
    let DriverContext { store, accumulator } = context;
    let prepared = run_blocking(move || {
        let (position, input) = current(&accumulator)?;
        let proposed =
            S::evaluate(typed_input::<S::Input>(&input)?).map_err(|_| RuntimeError::Internal)?;
        let outcome = conclude(
            &accumulator,
            position,
            mfm_program::ExecutionPhase::Pure,
            proposed,
        )?;
        let frame = accumulator
            .history
            .encode_pure_conclusion(position, domain_objects(&outcome)?)
            .map_err(map_local_journal_error)?;
        prepare_append(accumulator, frame)
    })
    .await?;
    finish_append(store, prepared).await
}

pub(crate) async fn start_read<S: ReadState<C>, C: ReadCapabilityContract>(
    context: DriverContext<'_>,
    adapter: Arc<ErasedReadAdapterCallback>,
) -> Result<DriverDisposition> {
    let DriverContext { store, accumulator } = context;
    let (accumulator, position, input, intent) = run_blocking(move || {
        let (position, input) = current(&accumulator)?;
        let typed = input
            .typed
            .downcast_ref::<S::Input>()
            .ok_or(RuntimeError::Internal)?;
        let intent = qualify_hot(S::prepare(typed).map_err(|_| RuntimeError::Internal)?)
            .map_err(RuntimeError::from)?;
        Ok((accumulator, position, input, intent))
    })
    .await?;
    let response = adapter(&intent).await;
    let prepared = run_blocking(move || {
        let frame = match response {
            Ok(qualify) => {
                let evidence = qualify().map_err(RuntimeError::from)?;
                let typed_evidence = evidence
                    .typed
                    .downcast_ref::<C::Evidence>()
                    .ok_or(RuntimeError::Internal)?;
                C::bind_evidence(
                    &intent.value_ref,
                    intent
                        .typed
                        .downcast_ref::<C::Intent>()
                        .ok_or(RuntimeError::Internal)?,
                    typed_evidence,
                )
                .map_err(|_| RuntimeError::Internal)?;
                let proposed = S::interpret(typed_input::<S::Input>(&input)?, typed_evidence)
                    .map_err(|_| RuntimeError::Internal)?;
                let outcome = conclude(
                    &accumulator,
                    position,
                    mfm_program::ExecutionPhase::Read,
                    proposed,
                )?;
                accumulator
                    .history
                    .encode_read_conclusion(
                        position,
                        object(&intent)?,
                        mfm_journal::ReadConclusion::Observed {
                            evidence: object(&evidence)?,
                            outcome: domain_objects(&outcome)?,
                        },
                    )
                    .map_err(map_local_journal_error)?
            }
            Err(AdapterError::Operational(qualify)) => {
                let error = qualify().map_err(RuntimeError::from)?;
                let ExecutableMode::Read { incident, .. } =
                    &accumulator.executable.declarations[position.state.index()].mode
                else {
                    return Err(RuntimeError::Internal);
                };
                let state_context = (incident.context)(&input, &intent, &error)?;
                let decision = decide(
                    &accumulator,
                    position,
                    mfm_program::ExecutionPhase::Read,
                    crate::assembly::recovery::QualifiedIncident::Adapter {
                        original: copy_value(&error, &incident.error_codec)?,
                        context: Box::new(copy_value(&state_context, &incident.context_codec)?),
                    },
                )?;
                accumulator
                    .history
                    .encode_read_conclusion(
                        position,
                        object(&intent)?,
                        mfm_journal::ReadConclusion::AdapterFailed {
                            error: object(&error)?,
                            state_context: object(&state_context)?,
                            decision,
                        },
                    )
                    .map_err(map_local_journal_error)?
            }
            Err(AdapterError::Invariant(_)) => return Err(RuntimeError::Internal),
        };
        prepare_append(accumulator, frame)
    })
    .await?;
    finish_append(store, prepared).await
}

pub(crate) async fn start_effect<S: EffectState<C>, C: EffectCapabilityContract>(
    context: DriverContext<'_>,
) -> Result<DriverDisposition> {
    let DriverContext { store, accumulator } = context;
    let prepared = run_blocking(move || {
        let (position, input) = current(&accumulator)?;
        let command = qualify_hot(
            S::prepare(
                input
                    .typed
                    .downcast_ref::<S::Input>()
                    .ok_or(RuntimeError::Internal)?,
            )
            .map_err(|_| RuntimeError::Internal)?,
        )
        .map_err(RuntimeError::from)?;
        let effect_id = derive_effect_id(
            accumulator.history.run_id(),
            accumulator.executable.program.content_ref(),
            position,
            &command.value_ref,
        )?;
        let frame = accumulator
            .history
            .encode_effect_prepare(position, &effect_id, object(&command)?)
            .map_err(map_local_journal_error)?;
        prepare_append(accumulator, frame)
    })
    .await?;
    finish_append(store, prepared).await
}

pub(crate) async fn start_pending_effect<S: EffectState<C>, C: EffectCapabilityContract>(
    context: DriverContext<'_>,
    adapter: Arc<ErasedEffectAdapterCallback>,
) -> Result<DriverDisposition> {
    let DriverContext { store, accumulator } = context;
    let Cursor::EffectPending {
        position,
        input,
        effect_id,
        command,
    } = &accumulator.state.cursor
    else {
        return Err(RuntimeError::Internal);
    };
    let (position, input, command, effect_id) = (
        *position,
        Arc::clone(input),
        Arc::clone(command),
        effect_id.clone(),
    );
    let response = adapter(&effect_id, &command).await;
    let qualify = match response {
        Ok(EffectAdapterOutcome::Pending) => return Ok(DriverDisposition::Yield(accumulator)),
        Ok(EffectAdapterOutcome::Settled(qualify)) => qualify,
        Err(AdapterError::Invariant(_)) => return Err(RuntimeError::Internal),
        Err(AdapterError::Operational(qualify)) => {
            return run_blocking(move || {
                let error = qualify().map_err(RuntimeError::from)?;
                let ExecutableMode::Effect { incident, .. } =
                    &accumulator.executable.declarations[position.state.index()].mode
                else {
                    return Err(RuntimeError::Internal);
                };
                let state_context = (incident.context)(&input, &command, &error)?;
                let decision = decide(
                    &accumulator,
                    position,
                    mfm_program::ExecutionPhase::EffectPending,
                    crate::assembly::recovery::QualifiedIncident::Adapter {
                        original: copy_value(&error, &incident.error_codec)?,
                        context: Box::new(copy_value(&state_context, &incident.context_codec)?),
                    },
                )?;
                match decision {
                    mfm_journal::RecoveryDecision::Retry => {
                        Ok(DriverDisposition::Yield(accumulator))
                    }
                    mfm_journal::RecoveryDecision::Stop { reason } => {
                        Ok(DriverDisposition::Stopped {
                            accumulator,
                            incident: Box::new(crate::AdapterIncidentView {
                                error: retained_view(&error),
                                state_context: retained_view(&state_context),
                            }),
                            reason,
                        })
                    }
                    mfm_journal::RecoveryDecision::Restart { .. } => Err(RuntimeError::Internal),
                }
            })
            .await;
        }
    };
    let prepared = run_blocking(move || {
        let evidence = qualify().map_err(RuntimeError::from)?;
        let typed_evidence = evidence
            .typed
            .downcast_ref::<C::Evidence>()
            .ok_or(RuntimeError::Internal)?;
        C::bind_evidence(
            &effect_id,
            command
                .typed
                .downcast_ref::<C::Command>()
                .ok_or(RuntimeError::Internal)?,
            typed_evidence,
        )
        .map_err(|_| RuntimeError::Internal)?;
        let proposed = S::interpret(typed_input::<S::Input>(&input)?, typed_evidence)
            .map_err(|_| RuntimeError::Internal)?;
        let outcome = conclude(
            &accumulator,
            position,
            mfm_program::ExecutionPhase::EffectSettled,
            proposed,
        )?;
        let outcome = match domain_objects(&outcome)? {
            mfm_journal::DomainConclusion::Success { output } => {
                mfm_journal::EffectConclusion::Success { output }
            }
            mfm_journal::DomainConclusion::Failure {
                original,
                decision: mfm_journal::DomainDecision::Stop { reason, root },
            } => mfm_journal::EffectConclusion::Failure {
                original,
                root,
                reason,
            },
            _ => return Err(RuntimeError::Internal),
        };
        let frame = accumulator
            .history
            .encode_effect_conclusion(object(&evidence)?, outcome)
            .map_err(map_local_journal_error)?;
        prepare_append(accumulator, frame)
    })
    .await?;
    finish_append(store, prepared).await
}

struct PreparedAppend {
    accumulator: Accumulator,
    frame: EncodedRunFrame,
}

fn current_frame_bound(executable: &ExecutableProgram, state: &FoldState) -> Result<u64> {
    use mfm_program::Execution;
    let (position, pending) = match &state.cursor {
        Cursor::Runnable { position, .. } => (*position, false),
        Cursor::EffectPending { position, .. } => (*position, true),
        _ => return Err(RuntimeError::InvalidHistory),
    };
    Ok(
        match executable.program.declarations()[position.state.index()].execution() {
            Execution::Pure { bound } | Execution::Read { bound, .. } => bound.max_frame_bytes(),
            Execution::Effect { bounds, .. } if pending => bounds.conclusion_bytes(),
            Execution::Effect { bounds, .. } => bounds.prepare_bytes(),
        },
    )
}

fn prepare_append(mut accumulator: Accumulator, frame: EncodedRunFrame) -> Result<PreparedAppend> {
    crate::check_size(
        crate::SizeResource::DeclaredFrame,
        frame.canonical_bytes().len() as u64,
        current_frame_bound(&accumulator.executable, &accumulator.state)?,
    )?;
    accumulator
        .state
        .apply(
            &accumulator.executable,
            accumulator.history.run_id(),
            frame.record(),
        )
        .map_err(|_| RuntimeError::Internal)?;
    // Reports are derived only for terminal failures, before their facts can be acknowledged.
    if let Cursor::Failed(failure) = &accumulator.state.cursor {
        failure_report(&accumulator.state, failure)?;
    }
    Ok(PreparedAppend { accumulator, frame })
}

async fn finish_append(
    store: &Arc<dyn Store>,
    prepared: PreparedAppend,
) -> Result<DriverDisposition> {
    match store
        .append_run(&prepared.frame)
        .await
        .map_err(RuntimeError::from)?
    {
        AppendResult::NotInserted => Ok(DriverDisposition::Reload),
        AppendResult::Inserted => {
            run_blocking(move || {
                let PreparedAppend {
                    mut accumulator,
                    frame,
                } = prepared;
                accumulator
                    .history
                    .extend_inserted(frame)
                    .map_err(map_local_journal_error)?;
                if matches!(
                    accumulator.state.cursor,
                    Cursor::Runnable {
                        reason: RunnableReason::Retry | RunnableReason::Restart { .. },
                        ..
                    }
                ) {
                    Ok(DriverDisposition::Yield(accumulator))
                } else {
                    Ok(DriverDisposition::Continue(accumulator))
                }
            })
            .await
        }
    }
}

fn fold_retained(
    assembly: Arc<AssemblyInner>,
    run_id: RunId,
    stored: StoredRunBytes,
) -> Result<Accumulator> {
    let history =
        JournalHistory::qualify(&run_id, stored).map_err(|_| RuntimeError::InvalidHistory)?;
    let program = decode_retained_program(&history)?;
    let executable = RuntimeAssembly { inner: assembly }.associate(program)?;
    fold(executable, history)
}

fn decode_retained_program(history: &JournalHistory) -> Result<Program> {
    let Some(JournalRecord::RunAdmitted {
        program,
        admitted_context: _,
    }) = history.records().next()
    else {
        return Err(RuntimeError::InvalidHistory);
    };
    let checked_program = Program::decode_canonical(program.canonical_bytes())
        .map_err(|_| RuntimeError::InvalidHistory)?;
    if checked_program.content_ref() != program.content_ref() {
        return Err(RuntimeError::InvalidHistory);
    }
    Ok(checked_program)
}

fn fold_collision(
    run_id: RunId,
    stored: StoredRunBytes,
    executable: ExecutableProgram,
    c0: QualifiedValue,
) -> Result<Accumulator> {
    let history =
        JournalHistory::qualify(&run_id, stored).map_err(|_| RuntimeError::InvalidHistory)?;
    let exact = match history.records().next() {
        Some(JournalRecord::RunAdmitted {
            program,
            admitted_context,
        }) => {
            let retained = Program::decode_canonical(program.canonical_bytes())
                .map_err(|_| RuntimeError::InvalidHistory)?;
            if retained.content_ref() != program.content_ref()
                || retained.admitted_context_contract_ref().schema_id()
                    != admitted_context.content_ref().schema_id()
            {
                return Err(RuntimeError::InvalidHistory);
            }
            journal_object_matches_program(program, &executable.program)
                && journal_object_matches_value(admitted_context, &c0)
        }
        _ => return Err(RuntimeError::InvalidHistory),
    };
    if !exact {
        return Err(RuntimeError::AdmissionConflict);
    }
    fold(executable, history)
}

fn fold(executable: ExecutableProgram, history: JournalHistory) -> Result<Accumulator> {
    let c0 = match history.records().next() {
        Some(JournalRecord::RunAdmitted {
            program,
            admitted_context,
        }) => {
            if !journal_object_matches_program(program, &executable.program)
                || admitted_context.content_ref() != executable.program.initial_value_ref()
            {
                return Err(RuntimeError::InvalidHistory);
            }
            executable
                .admitted_context_codec
                .qualify(
                    admitted_context.content_ref(),
                    admitted_context.canonical_bytes(),
                )
                .map_err(|_| RuntimeError::InvalidHistory)?
        }
        _ => return Err(RuntimeError::InvalidHistory),
    };
    validate_admission_bound(
        &executable.program,
        history
            .frame_lengths()
            .next()
            .ok_or(RuntimeError::InvalidHistory)?,
    )
    .map_err(|_| RuntimeError::InvalidHistory)?;
    let admitted_context = Arc::new(retained_view(&c0));
    let mut state = FoldState::initial(&executable, c0)?;
    let run_id = history.run_id().clone();
    for (record, bytes) in history.records().zip(history.frame_lengths()).skip(1) {
        if bytes as u64 > current_frame_bound(&executable, &state)? {
            return Err(RuntimeError::InvalidHistory);
        }
        state.apply(&executable, &run_id, record)?;
    }
    state.validate_pending(&executable)?;
    Ok(Accumulator {
        admitted_context,
        executable,
        history,
        state,
    })
}

fn qualify_journal_object(codec: &ValueCodec, object: JournalObject<'_>) -> Result<QualifiedValue> {
    codec
        .qualify(object.content_ref(), object.canonical_bytes())
        .map_err(|_| RuntimeError::InvalidHistory)
}

fn journal_object_matches_value(object: JournalObject<'_>, value: &QualifiedValue) -> bool {
    object.content_ref() == &value.value_ref
        && object.canonical_bytes() == value.canonical.as_bytes()
}

fn journal_object_matches_program(object: JournalObject<'_>, program: &Program) -> bool {
    object.content_ref() == program.content_ref()
        && object.canonical_bytes() == program.canonical_bytes()
}

fn view(accumulator: &Accumulator) -> Result<RunView> {
    let state = match &accumulator.state.cursor {
        Cursor::Runnable {
            position, reason, ..
        } => RunViewState::Runnable {
            position: *position,
            reason: *reason,
        },
        Cursor::EffectPending {
            position,
            effect_id,
            ..
        } => RunViewState::EffectPending {
            position: *position,
            effect_id: effect_id.clone(),
        },
        Cursor::Succeeded(value) => RunViewState::Succeeded(retained_view(value)),
        Cursor::Failed(failure) => {
            RunViewState::Failed(failure_report(&accumulator.state, failure)?)
        }
    };
    Ok(RunView {
        run_id: accumulator.history.run_id().clone(),
        head_sequence: accumulator.history.head_sequence(),
        head_digest: accumulator.history.head_digest().clone(),
        state,
        admitted_context: Arc::clone(&accumulator.admitted_context),
        entry_point: accumulator.executable.program.entry_point_id().clone(),
    })
}

fn failure_report(state: &FoldState, failure: &fold::Failure) -> Result<crate::FailureReport> {
    let cause = match &failure.cause {
        fold::FailureCause::Domain { original, root } => crate::FailureCauseView::Domain {
            original: retained_view(original),
            root: retained_view(root),
        },
        fold::FailureCause::Adapter { error, context } => {
            crate::FailureCauseView::Adapter(crate::AdapterIncidentView {
                error: retained_view(error),
                state_context: retained_view(context),
            })
        }
    };
    crate::FailureReport::new(
        failure.position,
        failure.reason,
        state.usage(failure.position.state)?,
        cause,
    )
}

fn retained_view(value: &QualifiedValue) -> ValueView {
    ValueView {
        contract_ref: value.contract_ref.clone(),
        value_ref: value.value_ref.clone(),
        canonical: value.canonical.clone(),
    }
}

#[derive(Serialize)]
struct EffectIdPreimage<'a> {
    command_ref: &'a ContentRef,
    position: ExecutionPosition,
    domain: &'static str,
    program_ref: &'a ContentRef,
    run_id: &'a RunId,
}

fn derive_effect_id(
    run_id: &RunId,
    program_ref: &ContentRef,
    position: ExecutionPosition,
    command_ref: &ContentRef,
) -> Result<EffectId> {
    let preimage = EffectIdPreimage {
        command_ref,
        position,
        domain: "mfm.effect-id.v2",
        program_ref,
        run_id,
    };
    let json = serde_json::to_string(&preimage).map_err(|_| RuntimeError::Internal)?;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| RuntimeError::Internal)?;
    Ok(EffectId::from_digest(canonical.digest_bytes()))
}

async fn run_blocking<T, F>(job: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::runtime::Handle::try_current().map_err(|_| RuntimeError::Internal)?;
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| RuntimeError::Internal)?
}

fn map_local_journal_error(error: JournalError) -> RuntimeError {
    match error {
        JournalError::ObjectSize(size) => RuntimeError::SizeLimit {
            resource: crate::SizeResource::CanonicalObject,
            size,
        },
        JournalError::FrameSize(size) => RuntimeError::SizeLimit {
            resource: crate::SizeResource::Frame,
            size,
        },
        JournalError::EnvelopeSize(size) => RuntimeError::SizeLimit {
            resource: crate::SizeResource::FrameEnvelope,
            size,
        },
        JournalError::HistorySize(size) => RuntimeError::SizeLimit {
            resource: crate::SizeResource::HistoryBytes,
            size,
        },
        JournalError::FrameCount(size) => RuntimeError::SizeLimit {
            resource: crate::SizeResource::FrameCount,
            size,
        },
        JournalError::ArithmeticOverflow => RuntimeError::ArithmeticOverflow,
        JournalError::InvalidFrame | JournalError::InvalidHistory => RuntimeError::Internal,
    }
}
