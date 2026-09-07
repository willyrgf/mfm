use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::{EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{ContentRef, EffectId, RunId};
use mfm_journal::{
    EncodedRunFrame, JournalError, JournalHistory, JournalObject, JournalRecord, OutcomeKind,
    StoredRunBytes,
};
use mfm_program::{Declaration, EffectState, Program, ProposedStateOutcome, PureState, ReadState};
use mfm_store::{AppendResult, Store, StoreError};
use mfm_values::{MfmValue, ValueError};
use serde::Serialize;

use crate::assembly::{
    qualify_hot, AssemblyInner, EffectPendingStart, ErasedEffectAdapterCallback,
    ErasedReadAdapterCallback, ExecutableDeclaration, ExecutableMode, ExecutableProgram,
    QualifiedValue, ReadStart, RuntimeAssembly, StateStart, ValueCodec,
};
use crate::{
    AdapterError, EffectAdapterOutcome, Result, RetainedValueView, RunView, RunViewState,
    RuntimeError,
};

pub(crate) enum DriverDisposition {
    Continue(Accumulator),
    Reload,
    Yield(Accumulator),
}

pub(crate) struct DriverContext<'a> {
    store: &'a Arc<dyn Store>,
    accumulator: Accumulator,
}

pub(crate) struct Accumulator {
    executable: ExecutableProgram,
    history: JournalHistory,
    state: FoldState,
}

enum FoldState {
    Runnable {
        declaration_index: u16,
        input: QualifiedValue,
    },
    EffectPending {
        declaration_index: u16,
        input: QualifiedValue,
        effect_id: EffectId,
        command: Box<QualifiedValue>,
    },
    Succeeded(QualifiedValue),
    Failed(QualifiedValue),
}

pub(crate) async fn start<T: MfmValue>(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
    program: Program,
    c0: T,
) -> Result<RunView> {
    let proposed = run_blocking(move || prepare_admission(assembly, run_id, program, c0)).await?;
    match store
        .append_run(&proposed.genesis)
        .await
        .map_err(map_append_error)?
    {
        AppendResult::Inserted => {
            let PreparedAdmission {
                executable,
                c0,
                genesis,
            } = proposed;
            let accumulator =
                run_blocking(move || accumulator_from_inserted_genesis(executable, c0, genesis))
                    .await?;
            advance_until_stable(store, accumulator).await
        }
        AppendResult::NotInserted => {
            let stored = store
                .load_run(proposed.genesis.run_id())
                .await
                .map_err(map_load_error)?
                .ok_or(RuntimeError::Internal)?;
            let expected_run_id = proposed.genesis.run_id().clone();
            let accumulator = run_blocking(move || {
                fold_collision(expected_run_id, stored, proposed.executable, proposed.c0)
            })
            .await?;
            advance_until_stable(store, accumulator).await
        }
    }
}

pub(crate) async fn resume(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> Result<RunView> {
    let accumulator = load_and_fold(assembly, &store, &run_id, false).await?;
    advance_until_stable(store, accumulator).await
}

pub(crate) async fn read(
    assembly: Arc<AssemblyInner>,
    store: Arc<dyn Store>,
    run_id: RunId,
) -> Result<RunView> {
    let stored = store
        .load_run(&run_id)
        .await
        .map_err(map_load_error)?
        .ok_or(RuntimeError::Absent)?;
    run_blocking(move || view(fold_retained(assembly, run_id, stored)?)).await
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
    let c0 = qualify_hot(c0).map_err(map_hot_value_error)?;
    if program.admitted_context_contract_ref() != &c0.contract_ref {
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
    Ok(PreparedAdmission {
        executable,
        c0,
        genesis,
    })
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
    let state = initial_state(&executable, c0)?;
    Ok(Accumulator {
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
        .map_err(map_load_error)?
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
) -> Result<RunView> {
    loop {
        let invocation = match &accumulator.state {
            FoldState::Runnable {
                declaration_index, ..
            } => {
                let selected = executable_state(&accumulator.executable, *declaration_index)
                    .map_err(|_| RuntimeError::Internal)?;
                match &selected.mode {
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
            FoldState::EffectPending {
                declaration_index, ..
            } => {
                let selected = executable_state(&accumulator.executable, *declaration_index)
                    .map_err(|_| RuntimeError::Internal)?;
                let ExecutableMode::Effect {
                    start_pending,
                    adapter,
                    ..
                } = &selected.mode
                else {
                    return Err(RuntimeError::Internal);
                };
                DriverInvocation::EffectPending {
                    start: *start_pending,
                    adapter: Arc::clone(adapter),
                }
            }
            FoldState::Succeeded(_) | FoldState::Failed(_) => return view(accumulator),
        };
        let reload_run_id = accumulator.history.run_id().clone();
        let reload_assembly = Arc::clone(&accumulator.executable._assembly);
        let context = DriverContext {
            store: &store,
            accumulator,
        };
        let disposition = match invocation {
            DriverInvocation::State { start } => start(context).await?,
            DriverInvocation::Read { start, adapter } => start(context, adapter).await?,
            DriverInvocation::EffectPending { start, adapter } => start(context, adapter).await?,
        };
        match disposition {
            DriverDisposition::Continue(next) => accumulator = next,
            DriverDisposition::Reload => {
                accumulator = load_and_fold(reload_assembly, &store, &reload_run_id, true).await?;
            }
            DriverDisposition::Yield(pending) => return view(pending),
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

pub(crate) async fn start_pure<S: PureState>(
    context: DriverContext<'_>,
) -> Result<DriverDisposition> {
    let DriverContext { store, accumulator } = context;
    let Accumulator {
        executable,
        history,
        state,
    } = accumulator;
    let FoldState::Runnable {
        declaration_index,
        input,
    } = state
    else {
        return Err(RuntimeError::Internal);
    };
    let typed = input
        .typed
        .downcast::<S::Input>()
        .map_err(|_| RuntimeError::Internal)?;
    let prepared = run_blocking(move || {
        let (kind, outcome) =
            qualify_outcome(S::evaluate(*typed).map_err(|_| RuntimeError::Internal)?)?;
        let frame = history
            .encode_pure_conclusion(kind, &outcome.value_ref, outcome.canonical.as_bytes())
            .map_err(map_local_journal_error)?;
        let next_state = apply_outcome(&executable, declaration_index, kind, outcome)?;
        Ok(PreparedAppend {
            executable,
            history,
            frame,
            next_state,
        })
    })
    .await?;
    finish_append(store, prepared).await
}

pub(crate) async fn start_read<S, C>(
    context: DriverContext<'_>,
    adapter: Arc<ErasedReadAdapterCallback>,
) -> Result<DriverDisposition>
where
    S: ReadState<C>,
    C: ReadCapabilityContract,
{
    let DriverContext { store, accumulator } = context;
    let Accumulator {
        executable,
        history,
        state,
    } = accumulator;
    let FoldState::Runnable {
        declaration_index,
        input,
    } = state
    else {
        return Err(RuntimeError::Internal);
    };
    let typed_input = input
        .typed
        .downcast::<S::Input>()
        .map_err(|_| RuntimeError::Internal)?;
    let (typed_input, intent) = run_blocking(move || {
        let intent = S::prepare(&typed_input).map_err(|_| RuntimeError::Internal)?;
        let intent = qualify_hot(intent).map_err(map_hot_value_error)?;
        Ok((typed_input, intent))
    })
    .await?;

    let qualify_evidence = adapter(&intent).await.map_err(map_adapter_error)?;

    let prepared = run_blocking(move || {
        let evidence = qualify_evidence().map_err(map_hot_value_error)?;
        let typed_intent = intent
            .typed
            .downcast_ref::<C::Intent>()
            .ok_or(RuntimeError::Internal)?;
        let typed_evidence = evidence
            .typed
            .downcast_ref::<C::Evidence>()
            .ok_or(RuntimeError::Internal)?;
        C::bind_evidence(&intent.value_ref, typed_intent, typed_evidence)
            .map_err(|_| RuntimeError::Internal)?;
        let (kind, outcome) = qualify_outcome(
            S::interpret(*typed_input, typed_evidence).map_err(|_| RuntimeError::Internal)?,
        )?;
        let frame = history
            .encode_read_conclusion(
                &intent.value_ref,
                intent.canonical.as_bytes(),
                &evidence.value_ref,
                evidence.canonical.as_bytes(),
                kind,
                &outcome.value_ref,
                outcome.canonical.as_bytes(),
            )
            .map_err(map_local_journal_error)?;
        let next_state = apply_outcome(&executable, declaration_index, kind, outcome)?;
        Ok(PreparedAppend {
            executable,
            history,
            frame,
            next_state,
        })
    })
    .await?;
    finish_append(store, prepared).await
}

pub(crate) async fn start_effect<S, C>(context: DriverContext<'_>) -> Result<DriverDisposition>
where
    S: EffectState<C>,
    C: EffectCapabilityContract,
{
    let DriverContext { store, accumulator } = context;
    let Accumulator {
        executable,
        history,
        state,
    } = accumulator;
    let FoldState::Runnable {
        declaration_index,
        input,
    } = state
    else {
        return Err(RuntimeError::Internal);
    };
    let QualifiedValue {
        contract_ref,
        value_ref,
        canonical,
        typed,
    } = input;
    let typed_input = typed
        .downcast::<S::Input>()
        .map_err(|_| RuntimeError::Internal)?;
    let prepared = run_blocking(move || {
        let command = S::prepare(&typed_input).map_err(|_| RuntimeError::Internal)?;
        let command = qualify_hot(command).map_err(map_hot_value_error)?;
        let effect_id = derive_effect_id(
            history.run_id(),
            executable.program.content_ref(),
            declaration_index,
            &command.value_ref,
        )?;
        let frame = history
            .encode_effect_prepare(&effect_id, &command.value_ref, command.canonical.as_bytes())
            .map_err(map_local_journal_error)?;
        let next_state = FoldState::EffectPending {
            declaration_index,
            input: QualifiedValue {
                contract_ref,
                value_ref,
                canonical,
                typed: typed_input,
            },
            effect_id,
            command: Box::new(command),
        };
        Ok(PreparedAppend {
            executable,
            history,
            frame,
            next_state,
        })
    })
    .await?;
    finish_append(store, prepared).await
}

pub(crate) async fn start_pending_effect<S, C>(
    context: DriverContext<'_>,
    adapter: Arc<ErasedEffectAdapterCallback>,
) -> Result<DriverDisposition>
where
    S: EffectState<C>,
    C: EffectCapabilityContract,
{
    let DriverContext { store, accumulator } = context;
    let Accumulator {
        executable,
        history,
        state,
    } = accumulator;
    let FoldState::EffectPending {
        declaration_index,
        input,
        effect_id,
        command,
    } = state
    else {
        return Err(RuntimeError::Internal);
    };
    let command = *command;
    let outcome = adapter(&effect_id, &command)
        .await
        .map_err(map_adapter_error)?;
    let qualify_evidence = match outcome {
        EffectAdapterOutcome::Pending => {
            return Ok(DriverDisposition::Yield(Accumulator {
                executable,
                history,
                state: FoldState::EffectPending {
                    declaration_index,
                    input,
                    effect_id,
                    command: Box::new(command),
                },
            }));
        }
        EffectAdapterOutcome::Settled(qualify_evidence) => qualify_evidence,
    };
    let typed_input = input
        .typed
        .downcast::<S::Input>()
        .map_err(|_| RuntimeError::Internal)?;
    let prepared = run_blocking(move || {
        let evidence = qualify_evidence().map_err(map_hot_value_error)?;
        let typed_command = command
            .typed
            .downcast_ref::<C::Command>()
            .ok_or(RuntimeError::Internal)?;
        let typed_evidence = evidence
            .typed
            .downcast_ref::<C::Evidence>()
            .ok_or(RuntimeError::Internal)?;
        C::bind_evidence(&effect_id, typed_command, typed_evidence)
            .map_err(|_| RuntimeError::Internal)?;
        let (kind, outcome) = qualify_outcome(
            S::interpret(*typed_input, typed_evidence).map_err(|_| RuntimeError::Internal)?,
        )?;
        let frame = history
            .encode_effect_conclusion(
                &evidence.value_ref,
                evidence.canonical.as_bytes(),
                kind,
                &outcome.value_ref,
                outcome.canonical.as_bytes(),
            )
            .map_err(map_local_journal_error)?;
        let next_state = apply_outcome(&executable, declaration_index, kind, outcome)?;
        Ok(PreparedAppend {
            executable,
            history,
            frame,
            next_state,
        })
    })
    .await?;
    finish_append(store, prepared).await
}

struct PreparedAppend {
    executable: ExecutableProgram,
    history: JournalHistory,
    frame: EncodedRunFrame,
    next_state: FoldState,
}

async fn finish_append(
    store: &Arc<dyn Store>,
    prepared: PreparedAppend,
) -> Result<DriverDisposition> {
    match store
        .append_run(&prepared.frame)
        .await
        .map_err(map_append_error)?
    {
        AppendResult::NotInserted => {
            drop(prepared);
            Ok(DriverDisposition::Reload)
        }
        AppendResult::Inserted => {
            let accumulator = run_blocking(move || apply_inserted(prepared)).await?;
            Ok(DriverDisposition::Continue(accumulator))
        }
    }
}

fn apply_inserted(mut prepared: PreparedAppend) -> Result<Accumulator> {
    prepared
        .history
        .extend_inserted(prepared.frame)
        .map_err(map_local_journal_error)?;
    Ok(Accumulator {
        executable: prepared.executable,
        history: prepared.history,
        state: prepared.next_state,
    })
}

fn qualify_outcome<O: MfmValue, F: MfmValue>(
    proposed: ProposedStateOutcome<O, F>,
) -> Result<(OutcomeKind, QualifiedValue)> {
    let (kind, outcome) = match proposed {
        ProposedStateOutcome::Success { output } => (OutcomeKind::Success, qualify_hot(output)),
        ProposedStateOutcome::Failure { failure } => (OutcomeKind::Failure, qualify_hot(failure)),
    };
    outcome
        .map(|outcome| (kind, outcome))
        .map_err(map_hot_value_error)
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
            if !journal_object_matches_program(program, &executable.program) {
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
    let mut state = initial_state(&executable, c0)?;
    let run_id = history.run_id().clone();
    for record in history.records().skip(1) {
        state = apply_retained_record(&executable, &run_id, state, record)?;
    }
    Ok(Accumulator {
        executable,
        history,
        state,
    })
}

fn initial_state(executable: &ExecutableProgram, c0: QualifiedValue) -> Result<FoldState> {
    if executable.declarations.is_empty() {
        return Ok(FoldState::Succeeded(c0));
    }
    select_declaration(executable, 0, c0)
}

fn select_declaration(
    executable: &ExecutableProgram,
    mut index: u16,
    mut input: QualifiedValue,
) -> Result<FoldState> {
    loop {
        match executable
            .declarations
            .get(usize::from(index))
            .ok_or(RuntimeError::InvalidHistory)?
        {
            ExecutableDeclaration::State(_) => {
                return Ok(FoldState::Runnable {
                    declaration_index: index,
                    input,
                });
            }
            ExecutableDeclaration::Match(projection) => {
                let (next, payload) = projection.project(input)?;
                index = next;
                input = payload;
            }
        }
    }
}

fn apply_retained_record(
    executable: &ExecutableProgram,
    run_id: &RunId,
    state: FoldState,
    record: JournalRecord<'_>,
) -> Result<FoldState> {
    match (state, record) {
        (
            FoldState::Runnable {
                declaration_index,
                input: _,
            },
            JournalRecord::StateConcludedPure { kind, outcome },
        ) => {
            let selected = executable_state(executable, declaration_index)?;
            if !matches!(selected.mode, ExecutableMode::Pure { .. }) {
                return Err(RuntimeError::InvalidHistory);
            }
            let outcome = qualify_retained_outcome(selected, kind, outcome)?;
            apply_outcome(executable, declaration_index, kind, outcome)
        }
        (
            FoldState::Runnable {
                declaration_index,
                input: _,
            },
            JournalRecord::StateConcludedRead {
                intent,
                evidence,
                kind,
                outcome,
            },
        ) => {
            let selected = executable_state(executable, declaration_index)?;
            let ExecutableMode::Read {
                validate_retained,
                intent_codec,
                evidence_codec,
                ..
            } = &selected.mode
            else {
                return Err(RuntimeError::InvalidHistory);
            };
            let intent = qualify_journal_object(intent_codec, intent)?;
            let evidence = qualify_journal_object(evidence_codec, evidence)?;
            validate_retained(&intent, &evidence)?;
            let outcome = qualify_retained_outcome(selected, kind, outcome)?;
            apply_outcome(executable, declaration_index, kind, outcome)
        }
        (
            FoldState::Runnable {
                declaration_index,
                input,
            },
            JournalRecord::StateEffectPrepared { effect_id, command },
        ) => {
            let selected = executable_state(executable, declaration_index)?;
            let ExecutableMode::Effect {
                validate_prepare,
                command_codec,
                ..
            } = &selected.mode
            else {
                return Err(RuntimeError::InvalidHistory);
            };
            let command = qualify_journal_object(command_codec, command)?;
            validate_prepare(&input, &command)?;
            let expected = derive_effect_id(
                run_id,
                executable.program.content_ref(),
                declaration_index,
                &command.value_ref,
            )?;
            if &expected != effect_id {
                return Err(RuntimeError::InvalidHistory);
            }
            Ok(FoldState::EffectPending {
                declaration_index,
                input,
                effect_id: expected,
                command: Box::new(command),
            })
        }
        (
            FoldState::EffectPending {
                declaration_index,
                input: _,
                effect_id,
                command,
            },
            JournalRecord::StateEffectConcluded {
                evidence,
                kind,
                outcome,
            },
        ) => {
            let selected = executable_state(executable, declaration_index)?;
            let ExecutableMode::Effect {
                validate_evidence,
                evidence_codec,
                ..
            } = &selected.mode
            else {
                return Err(RuntimeError::InvalidHistory);
            };
            let evidence = qualify_journal_object(evidence_codec, evidence)?;
            validate_evidence(&effect_id, &command, &evidence)?;
            let outcome = qualify_retained_outcome(selected, kind, outcome)?;
            apply_outcome(executable, declaration_index, kind, outcome)
        }
        _ => Err(RuntimeError::InvalidHistory),
    }
}

fn executable_state(
    executable: &ExecutableProgram,
    declaration_index: u16,
) -> Result<&crate::assembly::ExecutableState> {
    match executable.declarations.get(usize::from(declaration_index)) {
        Some(ExecutableDeclaration::State(state)) => Ok(state),
        _ => Err(RuntimeError::InvalidHistory),
    }
}

fn qualify_retained_outcome(
    state: &crate::assembly::ExecutableState,
    kind: OutcomeKind,
    object: JournalObject<'_>,
) -> Result<QualifiedValue> {
    let codec = match kind {
        OutcomeKind::Success => &state.output_codec,
        OutcomeKind::Failure => &state.failure_codec,
    };
    qualify_journal_object(codec, object)
}

fn qualify_journal_object(codec: &ValueCodec, object: JournalObject<'_>) -> Result<QualifiedValue> {
    codec
        .qualify(object.content_ref(), object.canonical_bytes())
        .map_err(|_| RuntimeError::InvalidHistory)
}

fn apply_outcome(
    executable: &ExecutableProgram,
    declaration_index: u16,
    kind: OutcomeKind,
    outcome: QualifiedValue,
) -> Result<FoldState> {
    let declaration = match executable
        .program
        .declarations()
        .get(usize::from(declaration_index))
    {
        Some(Declaration::State(declaration)) => declaration,
        _ => return Err(RuntimeError::InvalidHistory),
    };
    match kind {
        OutcomeKind::Success => match declaration.next_index() {
            Some(next) => select_declaration(executable, next, outcome),
            None if &outcome.contract_ref == executable.program.root_success_contract_ref() => {
                Ok(FoldState::Succeeded(outcome))
            }
            None => Err(RuntimeError::InvalidHistory),
        },
        OutcomeKind::Failure => match declaration.failure_next_index() {
            Some(next) => select_declaration(executable, next, outcome),
            None if &outcome.contract_ref == executable.program.root_failure_contract_ref() => {
                Ok(FoldState::Failed(outcome))
            }
            None => Err(RuntimeError::InvalidHistory),
        },
    }
}

fn journal_object_matches_value(object: JournalObject<'_>, value: &QualifiedValue) -> bool {
    object.content_ref() == &value.value_ref
        && object.canonical_bytes() == value.canonical.as_bytes()
}

fn journal_object_matches_program(object: JournalObject<'_>, program: &Program) -> bool {
    object.content_ref() == program.content_ref()
        && object.canonical_bytes() == program.canonical_bytes()
}

fn view(accumulator: Accumulator) -> Result<RunView> {
    let state = match accumulator.state {
        FoldState::Runnable { .. } | FoldState::EffectPending { .. } => RunViewState::Runnable,
        FoldState::Succeeded(value) => RunViewState::Succeeded(retained_view(value)),
        FoldState::Failed(value) => RunViewState::Failed(retained_view(value)),
    };
    Ok(RunView {
        run_id: accumulator.history.run_id().clone(),
        head_sequence: accumulator.history.head_sequence(),
        head_digest: accumulator.history.head_digest().clone(),
        state,
    })
}

fn retained_view(value: QualifiedValue) -> RetainedValueView {
    RetainedValueView {
        contract_ref: value.contract_ref,
        value_ref: value.value_ref,
        canonical: value.canonical,
    }
}

#[derive(Serialize)]
struct EffectIdPreimage<'a> {
    command_ref: &'a ContentRef,
    declaration_index: u16,
    domain: &'static str,
    program_ref: &'a ContentRef,
    run_id: &'a RunId,
}

fn derive_effect_id(
    run_id: &RunId,
    program_ref: &ContentRef,
    declaration_index: u16,
    command_ref: &ContentRef,
) -> Result<EffectId> {
    let preimage = EffectIdPreimage {
        command_ref,
        declaration_index,
        domain: "mfm.effect-id.v1",
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

fn map_hot_value_error(error: ValueError) -> RuntimeError {
    match error {
        ValueError::Capacity => RuntimeError::Capacity,
        _ => RuntimeError::Internal,
    }
}

fn map_adapter_error(error: AdapterError) -> RuntimeError {
    match error {
        AdapterError::Unavailable => RuntimeError::Unavailable,
        AdapterError::Internal => RuntimeError::Internal,
    }
}

fn map_local_journal_error(error: JournalError) -> RuntimeError {
    match error {
        JournalError::Capacity => RuntimeError::Capacity,
        JournalError::InvalidFrame | JournalError::InvalidHistory => RuntimeError::Internal,
    }
}

fn map_load_error(error: StoreError) -> RuntimeError {
    match error {
        StoreError::Capacity => RuntimeError::Internal,
        StoreError::CorruptPhysicalState => RuntimeError::InvalidHistory,
        StoreError::Unavailable => RuntimeError::Unavailable,
        StoreError::Indeterminate => RuntimeError::Internal,
    }
}

fn map_append_error(error: StoreError) -> RuntimeError {
    match error {
        StoreError::Capacity => RuntimeError::Capacity,
        StoreError::CorruptPhysicalState => RuntimeError::InvalidHistory,
        StoreError::Unavailable => RuntimeError::Unavailable,
        StoreError::Indeterminate => RuntimeError::Indeterminate,
    }
}

#[cfg(test)]
mod tests {
    use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, SchemaId};

    use super::*;

    #[test]
    fn effect_id_preimage_and_derivation_are_exact() {
        let run_id = RunId::from_digest(DigestBytes::from_array([1; 32]));
        let program_ref = ContentRef::new(
            SchemaId::new(
                "mfm-program-document",
                "3",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([2; 32]),
            )
            .expect("program schema"),
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([3; 32])),
        )
        .expect("program ref");
        let command_ref = ContentRef::new(
            SchemaId::new(
                "mfm-test-command",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([4; 32]),
            )
            .expect("command schema"),
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([5; 32])),
        )
        .expect("command ref");
        let preimage = EffectIdPreimage {
            command_ref: &command_ref,
            declaration_index: 7,
            domain: "mfm.effect-id.v1",
            program_ref: &program_ref,
            run_id: &run_id,
        };
        let json = serde_json::to_string(&preimage).expect("json");
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical");
        assert_eq!(
            canonical.as_str(),
            "{\"command_ref\":{\"content_digest\":\"content:sha256-v1:0505050505050505050505050505050505050505050505050505050505050505\",\"schema_id\":\"schema:mfm-test-command:1:sha256-jcs-v1:0404040404040404040404040404040404040404040404040404040404040404\"},\"declaration_index\":7,\"domain\":\"mfm.effect-id.v1\",\"program_ref\":{\"content_digest\":\"content:sha256-v1:0303030303030303030303030303030303030303030303030303030303030303\",\"schema_id\":\"schema:mfm-program-document:3:sha256-jcs-v1:0202020202020202020202020202020202020202020202020202020202020202\"},\"run_id\":\"run:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101\"}"
        );
        assert_eq!(
            derive_effect_id(&run_id, &program_ref, 7, &command_ref)
                .expect("effect id")
                .as_str(),
            "effect:sha256-jcs-v1:a79a7bfef85f3e097d2d9ccf193b59da8010ab403c9980c23a3e78771d4f3e38"
        );
    }
}
