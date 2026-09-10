use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_ids::{EffectId, ExecutionPosition, RunId, StatePosition, VisitId};
use mfm_journal::{
    DomainConclusion, DomainDecision, EffectConclusion, JournalRecord, PendingDecision,
    ReadConclusion, RecoveryDecision, StopCode,
};
use mfm_program::{Execution, ExecutionPhase, RecoveryUsage};

use super::{derive_effect_id, qualify_journal_object};
use crate::assembly::{ExecutableMode, ExecutableProgram, QualifiedValue};
use crate::{Result, RunnableReason, RuntimeError};

pub(super) struct FoldState {
    pub(super) cursor: Cursor,
    checkpoints: BTreeMap<StatePosition, Arc<QualifiedValue>>,
    usage: Vec<(u32, u32)>,
    decisions: u32,
    barrier: Option<StatePosition>,
}

pub(super) enum Cursor {
    Runnable {
        position: ExecutionPosition,
        input: Arc<QualifiedValue>,
        reason: RunnableReason,
    },
    EffectPending {
        position: ExecutionPosition,
        input: Arc<QualifiedValue>,
        effect_id: EffectId,
        command: Arc<QualifiedValue>,
        failures: u32,
        latest_failure: Option<PendingFailure>,
    },
    Succeeded(Arc<QualifiedValue>),
    Failed(Failure),
}

pub(super) struct PendingFailure {
    pub(super) original: Arc<QualifiedValue>,
    pub(super) context: Arc<QualifiedValue>,
    pub(super) decision: PendingDecision,
}

pub(super) struct Failure {
    pub(super) position: ExecutionPosition,
    pub(super) reason: StopCode,
    pub(super) cause: FailureCause,
}

pub(super) enum FailureCause {
    Domain {
        original: Arc<QualifiedValue>,
        root: Arc<QualifiedValue>,
    },
    Adapter {
        error: Arc<QualifiedValue>,
        context: Arc<QualifiedValue>,
    },
}

impl FoldState {
    pub(super) fn initial(executable: &ExecutableProgram, input: QualifiedValue) -> Result<Self> {
        let input = Arc::new(input);
        let mut state = Self {
            cursor: Cursor::Succeeded(Arc::clone(&input)),
            checkpoints: BTreeMap::new(),
            usage: vec![(0, 0); executable.declarations.len()],
            decisions: 0,
            barrier: None,
        };
        if !executable.declarations.is_empty() {
            state.enter(
                executable,
                ExecutionPosition {
                    state: StatePosition::new(0).map_err(|_| RuntimeError::InvalidHistory)?,
                    visit: VisitId::new(0),
                },
                input,
                RunnableReason::Advance,
            )?;
        }
        Ok(state)
    }

    fn enter(
        &mut self,
        executable: &ExecutableProgram,
        position: ExecutionPosition,
        input: Arc<QualifiedValue>,
        reason: RunnableReason,
    ) -> Result<()> {
        let declaration = executable
            .program
            .declarations()
            .get(position.state.index())
            .ok_or(RuntimeError::InvalidHistory)?;
        if declaration.input_contract_ref() != &input.contract_ref {
            return Err(RuntimeError::InvalidHistory);
        }
        if executable.declarations[position.state.index()].is_checkpoint {
            self.checkpoints.insert(position.state, Arc::clone(&input));
        }
        self.cursor = Cursor::Runnable {
            position,
            input,
            reason,
        };
        Ok(())
    }

    pub(super) fn usage(&self, position: StatePosition) -> Result<RecoveryUsage> {
        let &(state_retries, state_restarts) = self
            .usage
            .get(position.index())
            .ok_or(RuntimeError::InvalidHistory)?;
        Ok(RecoveryUsage {
            state_retries,
            state_restarts,
            run_decisions: self.decisions,
        })
    }

    pub(super) fn eligible(
        &self,
        executable: &ExecutableProgram,
        from: StatePosition,
        target: StatePosition,
    ) -> bool {
        executable
            .program
            .declarations()
            .get(from.index())
            .is_some_and(|state| {
                state
                    .recovery_targets()
                    .iter()
                    .any(|permitted| permitted.position() == target)
            })
            && self.checkpoints.contains_key(&target)
            && self.barrier.is_none_or(|barrier| target > barrier)
            && target <= from
            && executable.program.declarations()[target.index()..=from.index()]
                .iter()
                .any(|state| matches!(state.execution(), Execution::Read { .. }))
    }

    pub(super) fn recovery_denial(
        &self,
        executable: &ExecutableProgram,
        from: StatePosition,
        decision: RecoveryDecision,
        phase: mfm_program::ExecutionPhase,
    ) -> Result<Option<StopCode>> {
        let declaration = executable
            .program
            .declarations()
            .get(from.index())
            .ok_or(RuntimeError::InvalidHistory)?;
        let usage = self.usage(from)?;
        let denial = match decision {
            RecoveryDecision::Stop { .. } => None,
            RecoveryDecision::Retry => {
                if matches!(declaration.execution(), Execution::Pure { .. }) {
                    Some(StopCode::PureRetry)
                } else if phase == mfm_program::ExecutionPhase::EffectSettled {
                    Some(StopCode::EffectSettled)
                } else if usage.state_retries >= declaration.allowances().retries() {
                    Some(StopCode::StateRetryExhausted)
                } else if usage.run_decisions
                    >= executable.program.limits().max_recovery_decisions()
                {
                    Some(StopCode::RunExhausted)
                } else {
                    None
                }
            }
            RecoveryDecision::Restart { checkpoint } => {
                if phase == mfm_program::ExecutionPhase::EffectSettled {
                    Some(StopCode::EffectSettled)
                } else if phase == mfm_program::ExecutionPhase::EffectPending
                    || self.barrier.is_some_and(|barrier| checkpoint <= barrier)
                {
                    Some(StopCode::EffectBarrier)
                } else if !self.eligible(executable, from, checkpoint) {
                    Some(StopCode::CheckpointUnavailable)
                } else if usage.state_restarts >= declaration.allowances().restarts() {
                    Some(StopCode::StateRestartExhausted)
                } else if usage.run_decisions
                    >= executable.program.limits().max_recovery_decisions()
                {
                    Some(StopCode::RunExhausted)
                } else {
                    None
                }
            }
        };
        Ok(denial)
    }

    fn recover(
        &mut self,
        executable: &ExecutableProgram,
        position: ExecutionPosition,
        input: Arc<QualifiedValue>,
        decision: RecoveryDecision,
    ) -> Result<()> {
        if self
            .recovery_denial(
                executable,
                position.state,
                decision,
                match executable.program.declarations()[position.state.index()].execution() {
                    Execution::Pure { .. } => mfm_program::ExecutionPhase::Pure,
                    Execution::Read { .. } => mfm_program::ExecutionPhase::Read,
                    Execution::Effect { .. } => mfm_program::ExecutionPhase::EffectSettled,
                },
            )?
            .is_some()
        {
            return Err(RuntimeError::InvalidHistory);
        }
        let visit = position
            .visit
            .checked_next()
            .map_err(|_| RuntimeError::InvalidHistory)?;
        let usage = self
            .usage
            .get_mut(position.state.index())
            .ok_or(RuntimeError::InvalidHistory)?;
        let (target, restored, reason) = match decision {
            RecoveryDecision::Retry => {
                usage.0 = usage.0.checked_add(1).ok_or(RuntimeError::InvalidHistory)?;
                (position.state, input, RunnableReason::Retry)
            }
            RecoveryDecision::Restart { checkpoint } => {
                usage.1 = usage.1.checked_add(1).ok_or(RuntimeError::InvalidHistory)?;
                let restored = Arc::clone(
                    self.checkpoints
                        .get(&checkpoint)
                        .ok_or(RuntimeError::InvalidHistory)?,
                );
                self.checkpoints
                    .retain(|position, _| *position <= checkpoint);
                (checkpoint, restored, RunnableReason::Restart { checkpoint })
            }
            RecoveryDecision::Stop { .. } => return Err(RuntimeError::InvalidHistory),
        };
        self.decisions = self
            .decisions
            .checked_add(1)
            .ok_or(RuntimeError::InvalidHistory)?;
        self.enter(
            executable,
            ExecutionPosition {
                state: target,
                visit,
            },
            restored,
            reason,
        )
    }

    fn succeed(
        &mut self,
        executable: &ExecutableProgram,
        position: ExecutionPosition,
        output: QualifiedValue,
    ) -> Result<()> {
        let output = Arc::new(output);
        let next = position
            .state
            .index()
            .checked_add(1)
            .ok_or(RuntimeError::InvalidHistory)?;
        if next == executable.declarations.len() {
            if &output.contract_ref != executable.program.root_success_contract_ref() {
                return Err(RuntimeError::InvalidHistory);
            }
            self.cursor = Cursor::Succeeded(output);
            Ok(())
        } else {
            self.enter(
                executable,
                ExecutionPosition {
                    state: StatePosition::new(next).map_err(|_| RuntimeError::InvalidHistory)?,
                    visit: position
                        .visit
                        .checked_next()
                        .map_err(|_| RuntimeError::InvalidHistory)?,
                },
                output,
                RunnableReason::Advance,
            )
        }
    }

    fn domain(
        &mut self,
        executable: &ExecutableProgram,
        position: ExecutionPosition,
        input: Arc<QualifiedValue>,
        outcome: DomainConclusion<mfm_journal::JournalObject<'_>>,
    ) -> Result<()> {
        let selected = executable
            .declarations
            .get(position.state.index())
            .ok_or(RuntimeError::InvalidHistory)?;
        match outcome {
            DomainConclusion::Success { output } => self.succeed(
                executable,
                position,
                qualify_journal_object(&selected.output_codec, output)?,
            ),
            DomainConclusion::Failure { original, decision } => {
                let original = Arc::new(qualify_journal_object(&selected.failure_codec, original)?);
                match decision {
                    DomainDecision::Retry => {
                        self.recover(executable, position, input, RecoveryDecision::Retry)
                    }
                    DomainDecision::Restart { checkpoint } => self.recover(
                        executable,
                        position,
                        input,
                        RecoveryDecision::Restart { checkpoint },
                    ),
                    DomainDecision::Stop { reason, root } => {
                        let root = Arc::new(qualify_journal_object(
                            &executable.root_failure_codec,
                            root,
                        )?);
                        self.stop(
                            executable,
                            position,
                            reason,
                            FailureCause::Domain { original, root },
                        )
                    }
                }
            }
        }
    }

    fn stop(
        &mut self,
        executable: &ExecutableProgram,
        position: ExecutionPosition,
        reason: StopCode,
        cause: FailureCause,
    ) -> Result<()> {
        let declaration = executable
            .program
            .declarations()
            .get(position.state.index())
            .ok_or(RuntimeError::InvalidHistory)?;
        let phase = match declaration.execution() {
            Execution::Pure { .. } => ExecutionPhase::Pure,
            Execution::Read { .. } => ExecutionPhase::Read,
            Execution::Effect { .. } => ExecutionPhase::EffectSettled,
        };
        self.validate_stop(executable, position.state, reason, phase)?;
        self.cursor = Cursor::Failed(Failure {
            position,
            reason,
            cause,
        });
        Ok(())
    }

    fn validate_stop(
        &self,
        executable: &ExecutableProgram,
        position: StatePosition,
        reason: StopCode,
        phase: ExecutionPhase,
    ) -> Result<()> {
        let declaration = executable
            .program
            .declarations()
            .get(position.index())
            .ok_or(RuntimeError::InvalidHistory)?;
        let usage = self.usage(position)?;
        let pending = phase == ExecutionPhase::EffectPending;
        let valid = match reason {
            StopCode::Requested => true,
            StopCode::CheckpointUnavailable => !pending,
            StopCode::StateRetryExhausted => {
                usage.state_retries >= declaration.allowances().retries()
            }
            StopCode::StateRestartExhausted => {
                !pending && usage.state_restarts >= declaration.allowances().restarts()
            }
            StopCode::RunExhausted => {
                usage.run_decisions >= executable.program.limits().max_recovery_decisions()
            }
            StopCode::PureRetry => phase == ExecutionPhase::Pure,
            StopCode::EffectBarrier => pending || self.barrier.is_some(),
            StopCode::EffectSettled => phase == ExecutionPhase::EffectSettled,
        };
        if valid {
            Ok(())
        } else {
            Err(RuntimeError::InvalidHistory)
        }
    }

    pub(super) fn apply(
        &mut self,
        executable: &ExecutableProgram,
        run_id: &RunId,
        record: JournalRecord<'_>,
    ) -> Result<()> {
        match (&self.cursor, record) {
            (
                Cursor::Runnable {
                    position, input, ..
                },
                JournalRecord::PureConcluded {
                    position: recorded,
                    outcome,
                },
            ) if *position == recorded => {
                if !matches!(
                    executable.declarations[position.state.index()].mode,
                    ExecutableMode::Pure { .. }
                ) {
                    return Err(RuntimeError::InvalidHistory);
                }
                self.domain(executable, recorded, Arc::clone(input), outcome)
            }
            (
                Cursor::Runnable {
                    position, input, ..
                },
                JournalRecord::ReadConcluded {
                    position: recorded,
                    intent,
                    outcome,
                },
            ) if *position == recorded => {
                let ExecutableMode::Read {
                    intent_codec,
                    evidence_codec,
                    validate_retained,
                    incident,
                    ..
                } = &executable.declarations[position.state.index()].mode
                else {
                    return Err(RuntimeError::InvalidHistory);
                };
                let intent = qualify_journal_object(intent_codec, intent)?;
                let input = Arc::clone(input);
                match outcome {
                    ReadConclusion::Observed { evidence, outcome } => {
                        let evidence = qualify_journal_object(evidence_codec, evidence)?;
                        validate_retained(&intent, &evidence)?;
                        self.domain(executable, recorded, input, outcome)
                    }
                    ReadConclusion::AdapterFailed {
                        error,
                        state_context,
                        decision,
                    } => {
                        let error = Arc::new(qualify_journal_object(&incident.error_codec, error)?);
                        let context = Arc::new(qualify_journal_object(
                            &incident.context_codec,
                            state_context,
                        )?);
                        match decision {
                            RecoveryDecision::Stop { reason } => self.stop(
                                executable,
                                recorded,
                                reason,
                                FailureCause::Adapter { error, context },
                            ),
                            decision => self.recover(executable, recorded, input, decision),
                        }
                    }
                }
            }
            (
                Cursor::Runnable {
                    position, input, ..
                },
                JournalRecord::EffectPrepared {
                    position: recorded,
                    effect_id,
                    command,
                },
            ) if *position == recorded => {
                let ExecutableMode::Effect { command_codec, .. } =
                    &executable.declarations[position.state.index()].mode
                else {
                    return Err(RuntimeError::InvalidHistory);
                };
                let command = Arc::new(qualify_journal_object(command_codec, command)?);
                let expected = derive_effect_id(
                    run_id,
                    executable.program.content_ref(),
                    recorded,
                    &command.value_ref,
                )?;
                if &expected != effect_id {
                    return Err(RuntimeError::InvalidHistory);
                }
                self.barrier = Some(position.state);
                self.cursor = Cursor::EffectPending {
                    position: recorded,
                    input: Arc::clone(input),
                    effect_id: expected,
                    command,
                    failures: 0,
                    latest_failure: None,
                };
                Ok(())
            }
            (
                Cursor::EffectPending {
                    position, failures, ..
                },
                JournalRecord::EffectAdapterFailed {
                    position: recorded,
                    original,
                    state_context,
                    decision,
                },
            ) if *position == recorded => {
                let Execution::Effect { bounds, .. } =
                    executable.program.declarations()[position.state.index()].execution()
                else {
                    return Err(RuntimeError::InvalidHistory);
                };
                if *failures >= bounds.max_pending_failures() {
                    return Err(RuntimeError::InvalidHistory);
                }
                let ExecutableMode::Effect { incident, .. } =
                    &executable.declarations[position.state.index()].mode
                else {
                    return Err(RuntimeError::InvalidHistory);
                };
                let original = Arc::new(qualify_journal_object(&incident.error_codec, original)?);
                let context = Arc::new(qualify_journal_object(
                    &incident.context_codec,
                    state_context,
                )?);
                match decision {
                    PendingDecision::Retry => {
                        if self
                            .recovery_denial(
                                executable,
                                recorded.state,
                                RecoveryDecision::Retry,
                                mfm_program::ExecutionPhase::EffectPending,
                            )?
                            .is_some()
                        {
                            return Err(RuntimeError::InvalidHistory);
                        }
                        let usage = &mut self.usage[recorded.state.index()];
                        usage.0 = usage.0.checked_add(1).ok_or(RuntimeError::InvalidHistory)?;
                        self.decisions = self
                            .decisions
                            .checked_add(1)
                            .ok_or(RuntimeError::InvalidHistory)?;
                    }
                    PendingDecision::Stop { reason } => self.validate_stop(
                        executable,
                        recorded.state,
                        reason,
                        ExecutionPhase::EffectPending,
                    )?,
                }
                let Cursor::EffectPending {
                    failures,
                    latest_failure,
                    ..
                } = &mut self.cursor
                else {
                    return Err(RuntimeError::InvalidHistory);
                };
                *failures = failures
                    .checked_add(1)
                    .ok_or(RuntimeError::InvalidHistory)?;
                *latest_failure = Some(PendingFailure {
                    original,
                    context,
                    decision,
                });
                Ok(())
            }
            (
                Cursor::EffectPending {
                    position,
                    input,
                    effect_id,
                    command,
                    ..
                },
                JournalRecord::EffectConcluded { evidence, outcome },
            ) => {
                let ExecutableMode::Effect {
                    evidence_codec,
                    validate_evidence,
                    ..
                } = &executable.declarations[position.state.index()].mode
                else {
                    return Err(RuntimeError::InvalidHistory);
                };
                let evidence = qualify_journal_object(evidence_codec, evidence)?;
                validate_evidence(effect_id, command, &evidence)?;
                let outcome = match outcome {
                    EffectConclusion::Success { output } => DomainConclusion::Success { output },
                    EffectConclusion::Failure {
                        original,
                        root,
                        reason,
                    } => DomainConclusion::Failure {
                        original,
                        decision: DomainDecision::Stop { reason, root },
                    },
                };
                self.domain(executable, *position, Arc::clone(input), outcome)
            }
            _ => Err(RuntimeError::InvalidHistory),
        }
    }

    pub(super) fn validate_pending(&self, executable: &ExecutableProgram) -> Result<()> {
        if let Cursor::EffectPending {
            position,
            input,
            command,
            ..
        } = &self.cursor
        {
            let ExecutableMode::Effect {
                validate_prepare, ..
            } = &executable.declarations[position.state.index()].mode
            else {
                return Err(RuntimeError::InvalidHistory);
            };
            validate_prepare(input, command)?;
        }
        Ok(())
    }
}
