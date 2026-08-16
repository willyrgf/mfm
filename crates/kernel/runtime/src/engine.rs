use std::sync::Arc;

use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::RunId;
use mfm_journal::{
    EncodedRunFrame, JournalError, JournalHistory, JournalObject, JournalRecord, OutcomeKind,
    StoredRunBytes,
};
use mfm_program::{Declaration, Program, ProposedStateOutcome, PureState, ReadState};
use mfm_store::{AppendResult, Store, StoreError};
use mfm_values::{MfmValue, ValueError};

use crate::assembly::{
    qualify_hot, AssemblyInner, ErasedAdapterCallback, ExecutableDeclaration, ExecutableProgram,
    QualifiedValue, RegisteredState, RuntimeAssembly, ValueCodec,
};
use crate::{ReadAdapterError, Result, RetainedValueView, RunView, RunViewState, RuntimeError};

pub(crate) enum DriverDisposition {
    Continue,
    Reload,
}

pub(crate) struct DriverContext<'a> {
    store: &'a Arc<dyn Store>,
    accumulator: &'a mut Option<Accumulator>,
    declaration_index: usize,
}

struct Accumulator {
    executable: ExecutableProgram,
    history: JournalHistory,
    state: Option<FoldState>,
}

enum FoldState {
    Runnable {
        declaration_index: usize,
        driver: Arc<dyn RegisteredState>,
        input: QualifiedValue,
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
    let expected = mfm_program::nominal_contract_ref::<T>().map_err(|_| RuntimeError::Internal)?;
    if program.admitted_context_contract_ref() != &expected {
        return Err(RuntimeError::Internal);
    }
    let executable = RuntimeAssembly { inner: assembly }.associate(program)?;
    let c0 = qualify_hot(c0).map_err(map_hot_value_error)?;
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
    let state = Some(initial_state(&executable, c0)?);
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

async fn advance_until_stable(store: Arc<dyn Store>, accumulator: Accumulator) -> Result<RunView> {
    let mut slot = Some(accumulator);
    loop {
        let terminal = {
            let accumulator = slot.as_ref().ok_or(RuntimeError::Internal)?;
            !matches!(accumulator.state.as_ref(), Some(FoldState::Runnable { .. }))
        };
        if terminal {
            let accumulator = slot.take().ok_or(RuntimeError::Internal)?;
            return view(accumulator);
        }

        let (driver, input, declaration_index, reload_run_id, reload_assembly) = {
            let accumulator = slot.as_mut().ok_or(RuntimeError::Internal)?;
            let reload_run_id = accumulator.history.run_id().clone();
            let reload_assembly = Arc::clone(&accumulator.executable._assembly);
            let state = accumulator.state.take().ok_or(RuntimeError::Internal)?;
            let FoldState::Runnable {
                declaration_index,
                driver,
                input,
            } = state
            else {
                return Err(RuntimeError::Internal);
            };
            (
                driver,
                input,
                declaration_index,
                reload_run_id,
                reload_assembly,
            )
        };
        let context = DriverContext {
            store: &store,
            accumulator: &mut slot,
            declaration_index,
        };
        match driver.start(input, context).await? {
            DriverDisposition::Continue => {}
            DriverDisposition::Reload => {
                slot = Some(load_and_fold(reload_assembly, &store, &reload_run_id, true).await?);
            }
        }
    }
}

pub(crate) async fn start_pure<S: PureState>(
    input: QualifiedValue,
    context: DriverContext<'_>,
) -> Result<DriverDisposition> {
    let accumulator = context.accumulator.take().ok_or(RuntimeError::Internal)?;
    let typed = input
        .typed
        .downcast::<S::Input>()
        .map_err(|_| RuntimeError::Internal)?;
    let declaration_index = context.declaration_index;
    let prepared = run_blocking(move || {
        let proposed = S::evaluate(*typed);
        let (kind, outcome) = match proposed {
            ProposedStateOutcome::Success { output } => (OutcomeKind::Success, qualify_hot(output)),
            ProposedStateOutcome::Failure { failure } => {
                (OutcomeKind::Failure, qualify_hot(failure))
            }
        };
        let outcome = outcome.map_err(map_hot_value_error)?;
        let frame = accumulator
            .history
            .encode_pure_conclusion(kind, &outcome.value_ref, outcome.canonical.as_bytes())
            .map_err(map_local_journal_error)?;
        Ok(PreparedConclusion {
            accumulator,
            frame,
            kind,
            outcome,
            intent: None,
            evidence: None,
            declaration_index,
        })
    })
    .await?;
    finish_conclusion(context, prepared).await
}

pub(crate) async fn start_read<S, C>(
    input: QualifiedValue,
    context: DriverContext<'_>,
    adapter: Arc<ErasedAdapterCallback>,
) -> Result<DriverDisposition>
where
    S: ReadState<C>,
    C: ReadCapabilityContract,
{
    let accumulator = context.accumulator.take().ok_or(RuntimeError::Internal)?;
    let typed_input = input
        .typed
        .downcast::<S::Input>()
        .map_err(|_| RuntimeError::Internal)?;
    let (accumulator, typed_input, intent) = run_blocking(move || {
        let intent = S::prepare(&typed_input).map_err(|_| RuntimeError::Internal)?;
        let intent = qualify_hot(intent).map_err(map_hot_value_error)?;
        Ok((accumulator, typed_input, intent))
    })
    .await?;

    let qualify_evidence = adapter(&intent).await.map_err(|error| match error {
        ReadAdapterError::Unavailable => RuntimeError::Unavailable,
        ReadAdapterError::Internal => RuntimeError::Internal,
    })?;

    let declaration_index = context.declaration_index;
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
        C::bind_evidence(typed_intent, typed_evidence).map_err(|_| RuntimeError::Internal)?;
        let proposed = S::interpret(*typed_input, typed_evidence);
        let (kind, outcome) = match proposed {
            ProposedStateOutcome::Success { output } => (OutcomeKind::Success, qualify_hot(output)),
            ProposedStateOutcome::Failure { failure } => {
                (OutcomeKind::Failure, qualify_hot(failure))
            }
        };
        let outcome = outcome.map_err(map_hot_value_error)?;
        let frame = accumulator
            .history
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
        Ok(PreparedConclusion {
            accumulator,
            frame,
            kind,
            outcome,
            intent: Some(intent),
            evidence: Some(evidence),
            declaration_index,
        })
    })
    .await?;
    finish_conclusion(context, prepared).await
}

struct PreparedConclusion {
    accumulator: Accumulator,
    frame: EncodedRunFrame,
    kind: OutcomeKind,
    outcome: QualifiedValue,
    intent: Option<QualifiedValue>,
    evidence: Option<QualifiedValue>,
    declaration_index: usize,
}

async fn finish_conclusion(
    context: DriverContext<'_>,
    prepared: PreparedConclusion,
) -> Result<DriverDisposition> {
    match context
        .store
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
            *context.accumulator = Some(accumulator);
            Ok(DriverDisposition::Continue)
        }
    }
}

fn apply_inserted(mut prepared: PreparedConclusion) -> Result<Accumulator> {
    let matches = {
        let record = prepared
            .accumulator
            .history
            .extend_inserted(prepared.frame)
            .map_err(map_local_journal_error)?;
        match (record, prepared.intent.as_ref(), prepared.evidence.as_ref()) {
            (JournalRecord::StateConcludedPure { kind, outcome }, None, None) => {
                kind == prepared.kind && journal_object_matches_value(outcome, &prepared.outcome)
            }
            (
                JournalRecord::StateConcludedRead {
                    intent,
                    evidence,
                    kind,
                    outcome,
                },
                Some(expected_intent),
                Some(expected_evidence),
            ) => {
                kind == prepared.kind
                    && journal_object_matches_value(intent, expected_intent)
                    && journal_object_matches_value(evidence, expected_evidence)
                    && journal_object_matches_value(outcome, &prepared.outcome)
            }
            _ => false,
        }
    };
    if !matches {
        return Err(RuntimeError::Internal);
    }
    let state = apply_outcome(
        &prepared.accumulator.executable,
        prepared.declaration_index,
        prepared.kind,
        prepared.outcome,
    )?;
    prepared.accumulator.state = Some(state);
    Ok(prepared.accumulator)
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
    for record in history.records().skip(1) {
        state = apply_retained_record(&executable, state, record)?;
    }
    Ok(Accumulator {
        executable,
        history,
        state: Some(state),
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
    mut index: usize,
    mut input: QualifiedValue,
) -> Result<FoldState> {
    loop {
        match executable
            .declarations
            .get(index)
            .ok_or(RuntimeError::InvalidHistory)?
        {
            ExecutableDeclaration::State(state) => {
                return Ok(FoldState::Runnable {
                    declaration_index: index,
                    driver: Arc::clone(&state.driver),
                    input,
                });
            }
            ExecutableDeclaration::Match(projection) => {
                let (next, payload) = projection.project(input)?;
                index = usize::from(next);
                input = payload;
            }
        }
    }
}

fn apply_retained_record(
    executable: &ExecutableProgram,
    state: FoldState,
    record: JournalRecord<'_>,
) -> Result<FoldState> {
    let FoldState::Runnable {
        declaration_index,
        driver,
        input: _input,
    } = state
    else {
        return Err(RuntimeError::InvalidHistory);
    };
    let state = match executable.declarations.get(declaration_index) {
        Some(ExecutableDeclaration::State(state)) => state,
        _ => return Err(RuntimeError::InvalidHistory),
    };
    match record {
        JournalRecord::StateConcludedPure { kind, outcome } if state.read_codecs.is_none() => {
            let outcome = qualify_retained_outcome(state, kind, outcome)?;
            apply_outcome(executable, declaration_index, kind, outcome)
        }
        JournalRecord::StateConcludedRead {
            intent,
            evidence,
            kind,
            outcome,
        } if state.read_codecs.is_some() => {
            let codecs = state
                .read_codecs
                .as_ref()
                .ok_or(RuntimeError::InvalidHistory)?;
            let intent = qualify_journal_object(&codecs.intent, intent)?;
            let evidence = qualify_journal_object(&codecs.evidence, evidence)?;
            driver.validate_retained_read(&intent, &evidence)?;
            let outcome = qualify_retained_outcome(state, kind, outcome)?;
            apply_outcome(executable, declaration_index, kind, outcome)
        }
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
    declaration_index: usize,
    kind: OutcomeKind,
    outcome: QualifiedValue,
) -> Result<FoldState> {
    let declaration = match executable.program.declarations().get(declaration_index) {
        Some(Declaration::State(declaration)) => declaration,
        _ => return Err(RuntimeError::InvalidHistory),
    };
    match kind {
        OutcomeKind::Success => match declaration.next_index() {
            Some(next) => select_declaration(executable, usize::from(next), outcome),
            None if &outcome.contract_ref == executable.program.root_success_contract_ref() => {
                Ok(FoldState::Succeeded(outcome))
            }
            None => Err(RuntimeError::InvalidHistory),
        },
        OutcomeKind::Failure => match declaration.failure_next_index() {
            Some(next) => select_declaration(executable, usize::from(next), outcome),
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

fn view(mut accumulator: Accumulator) -> Result<RunView> {
    let state = match accumulator.state.take().ok_or(RuntimeError::Internal)? {
        FoldState::Runnable { .. } => RunViewState::Runnable,
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
