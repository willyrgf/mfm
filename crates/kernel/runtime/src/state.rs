mod decode;
pub(crate) use decode::RunCommitSeed;

use crate::assembly::ExecutableProgram;
use crate::{Result, RuntimeError};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition, RunId, StatePosition, VisitId};
use mfm_program::{
    Classification, Execution, RecoveryDenial, RecoveryLimit, RecoveryRequest, RecoveryUsage,
    StopReason,
};
use mfm_values::Object;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RunState {
    pub(crate) phase: Phase,
    pub(crate) checkpoints: Vec<Checkpoint>,
    pub(crate) usage: Vec<StateUsage>,
    pub(crate) effect_barrier: Option<StatePosition>,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Checkpoint {
    pub(crate) position: StatePosition,
    pub(crate) input: Object,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StateUsage {
    pub(crate) retries: u32,
    pub(crate) restarts: u32,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
/// An execution occurrence and its complete input.
pub struct Call {
    pub(crate) position: ExecutionPosition,
    pub(crate) input: Object,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
/// One retained Effect command and its authority.
pub struct EffectCall {
    pub(crate) call: Call,
    pub(crate) effect_id: EffectId,
    pub(crate) command: Object,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
/// Accepted evidence for one retained Effect command.
pub struct Settlement {
    pub(crate) effect: EffectCall,
    pub(crate) evidence: Object,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Runnable(Call),
    EffectPending(EffectCall),
    AwaitingInterpretation(Settlement),
    AwaitingRecovery(Failure),
    Succeeded(Object),
    Failed(TerminalFailure),
}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// The complete facts of an executed State operation.
pub enum StateCall {
    /// Deterministic execution.
    Pure(Call),
    /// An observation with accepted evidence.
    Read {
        /// Execution and full input.
        call: Call,
        /// Exact intent.
        intent: Object,
        /// Accepted observational evidence.
        evidence: Object,
    },
    /// An interpretation of committed settlement.
    Effect(Settlement),
}
#[derive(Clone, PartialEq, Eq, Serialize)]
/// A declared domain failure with its original State operation.
pub struct DomainFailure {
    pub(crate) call: StateCall,
    pub(crate) original: Object,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
/// An operational observation failure without fabricated evidence.
pub struct ReadFailure {
    pub(crate) call: Call,
    pub(crate) intent: Object,
    pub(crate) original: Object,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// The original declared failure awaiting or retained by recovery.
pub enum Failure {
    /// A State-declared domain failure.
    Domain(DomainFailure),
    /// An operational Read failure.
    Read(ReadFailure),
    /// An operational Effect failure retaining command authority.
    PendingEffect {
        /// Unchanged prepared command and execution.
        effect: EffectCall,
        /// Original operational cause.
        original: Object,
    },
}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalFailure {
    Domain {
        failure: DomainFailure,
        reason: StopReason,
        root: Object,
    },
    Read {
        failure: ReadFailure,
        reason: StopReason,
    },
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RunCommit {
    pub(crate) program_ref: ContentRef,
    pub(crate) state: RunState,
    pub(crate) facts: OperationFacts,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationFacts {
    Admitted {
        program: Object,
        initial: Object,
    },
    Succeeded {
        call: StateCall,
        output: Object,
    },
    Failed(Failure),
    EffectPrepared(EffectCall),
    EffectSettled(Settlement),
    Recovered {
        failure: Failure,
        classification: Classification,
        request: RecoveryRequest,
        decision: RecoveryDecision,
    },
}

/// The action authorized and committed by Runtime, distinct from the handler's request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryDecision {
    /// Retry the same Read or reconcile the unchanged pending Effect.
    Retry,
    /// Restore the selected active checkpoint.
    Restart {
        /// Target already checked against the admitted Program.
        checkpoint: StatePosition,
    },
    /// End automatic recovery for the reviewed reason.
    Stop {
        /// Exact reason the request did not authorize another action.
        reason: StopReason,
    },
}

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
#[error("current run state violates {self:?}")]
pub(crate) enum StateInvariant {
    Identity {
        field: &'static str,
        expected: Box<ContentRef>,
        actual: Box<ContentRef>,
    },
    Position,
    Mode,
    Contract,
    Facts,
    Checkpoint,
    Usage,
    Barrier,
    EffectIdentity,
}
fn invalid(code: StateInvariant) -> RuntimeError {
    RuntimeError::native(
        crate::Operation::Restore,
        mfm_values::NativeCause::from_error(code),
    )
}

impl StateCall {
    /// Returns the complete executed input and occurrence.
    pub fn call(&self) -> &Call {
        match self {
            Self::Pure(call) | Self::Read { call, .. } => call,
            Self::Effect(settlement) => &settlement.effect.call,
        }
    }
    pub(crate) fn phase(&self) -> mfm_program::ExecutionPhase {
        match self {
            Self::Pure(_) => mfm_program::ExecutionPhase::Pure,
            Self::Read { .. } => mfm_program::ExecutionPhase::Read,
            Self::Effect(_) => mfm_program::ExecutionPhase::EffectSettled,
        }
    }
}
impl Failure {
    /// Returns the complete executed input and occurrence.
    pub fn call(&self) -> &Call {
        match self {
            Self::Domain(failure) => failure.call.call(),
            Self::Read(failure) => &failure.call,
            Self::PendingEffect { effect, .. } => &effect.call,
        }
    }
    /// Returns the retained original without mapping or classification.
    pub fn original(&self) -> &Object {
        match self {
            Self::Domain(failure) => &failure.original,
            Self::Read(failure) => &failure.original,
            Self::PendingEffect { original, .. } => original,
        }
    }
    pub(crate) fn phase(&self) -> mfm_program::ExecutionPhase {
        match self {
            Self::Domain(failure) => failure.call.phase(),
            Self::Read(_) => mfm_program::ExecutionPhase::Read,
            Self::PendingEffect { .. } => mfm_program::ExecutionPhase::EffectPending,
        }
    }
}
impl RunState {
    pub(crate) fn initial(executable: &ExecutableProgram, input: Object) -> Result<Self> {
        let mut state = Self {
            phase: Phase::Succeeded(input.clone()),
            checkpoints: Vec::new(),
            usage: vec![
                StateUsage {
                    retries: 0,
                    restarts: 0
                };
                executable.declarations.len()
            ],
            effect_barrier: None,
        };
        if !executable.declarations.is_empty() {
            state.enter(
                executable,
                Call {
                    position: ExecutionPosition {
                        state: StatePosition::new(0)
                            .map_err(|_| invalid(StateInvariant::Position))?,
                        visit: VisitId::new(0),
                    },
                    input,
                },
            );
        }
        Ok(state)
    }
    pub(crate) fn enter(&mut self, executable: &ExecutableProgram, call: Call) {
        if executable.declarations[call.position.state.index()].is_checkpoint {
            match self
                .checkpoints
                .binary_search_by_key(&call.position.state, |checkpoint| checkpoint.position)
            {
                Ok(index) => self.checkpoints[index].input = call.input.clone(),
                Err(index) => self.checkpoints.insert(
                    index,
                    Checkpoint {
                        position: call.position.state,
                        input: call.input.clone(),
                    },
                ),
            }
        }
        self.phase = Phase::Runnable(call);
    }
    pub(crate) fn usage(&self, position: StatePosition) -> Result<RecoveryUsage> {
        let current = self
            .usage
            .get(position.index())
            .ok_or_else(|| invalid(StateInvariant::Usage))?;
        let run_decisions = self
            .usage
            .iter()
            .try_fold(0u32, |sum, used| {
                sum.checked_add(used.retries)?.checked_add(used.restarts)
            })
            .ok_or_else(|| invalid(StateInvariant::Usage))?;
        Ok(RecoveryUsage {
            state_retries: current.retries,
            state_restarts: current.restarts,
            run_decisions,
        })
    }
    pub(crate) fn eligible(
        &self,
        executable: &ExecutableProgram,
        from: StatePosition,
        target: StatePosition,
    ) -> bool {
        executable
            .program
            .declarations()
            .get(from.index())
            .is_some_and(|declaration| {
                declaration
                    .recovery_targets()
                    .iter()
                    .any(|allowed| allowed.position() == target)
            })
            && self
                .checkpoints
                .iter()
                .any(|checkpoint| checkpoint.position == target)
            && target <= from
            && self.effect_barrier.is_none_or(|barrier| target > barrier)
            && executable.program.declarations()[target.index()..=from.index()]
                .iter()
                .any(|declaration| matches!(declaration.execution(), Execution::Read { .. }))
    }
    pub(crate) fn authorize(
        &self,
        executable: &ExecutableProgram,
        failure: &Failure,
        request: RecoveryRequest,
    ) -> Result<RecoveryDecision> {
        use mfm_program::ExecutionPhase;
        let position = failure.call().position.state;
        let used = self.usage(position)?;
        let allowance = executable.program.declarations()[position.index()].allowances();
        let stop = |reason| Ok(RecoveryDecision::Stop { reason });
        match request {
            RecoveryRequest::Stop => return stop(StopReason::Requested),
            RecoveryRequest::RetryState => match failure.phase() {
                ExecutionPhase::Pure => {
                    return stop(StopReason::Disallowed(RecoveryDenial::PureRetry))
                }
                ExecutionPhase::EffectSettled => {
                    return stop(StopReason::Disallowed(RecoveryDenial::EffectSettled))
                }
                _ => {}
            },
            RecoveryRequest::Restart(target) => {
                if failure.phase() == ExecutionPhase::EffectSettled {
                    return stop(StopReason::Disallowed(RecoveryDenial::EffectSettled));
                }
                if failure.phase() == ExecutionPhase::EffectPending
                    || self
                        .effect_barrier
                        .is_some_and(|barrier| target.position() <= barrier)
                {
                    return stop(StopReason::Disallowed(RecoveryDenial::EffectBarrier));
                }
                if !self.eligible(executable, position, target.position()) {
                    return stop(StopReason::Disallowed(
                        RecoveryDenial::CheckpointUnavailable,
                    ));
                }
            }
        }
        match request {
            RecoveryRequest::RetryState if used.state_retries >= allowance.retries() => {
                stop(StopReason::Exhausted(RecoveryLimit::StateRetry))
            }
            RecoveryRequest::Restart(_) if used.state_restarts >= allowance.restarts() => {
                stop(StopReason::Exhausted(RecoveryLimit::StateRestart))
            }
            _ if used.run_decisions >= executable.program.limits().max_recovery_decisions() => {
                stop(StopReason::Exhausted(RecoveryLimit::Run))
            }
            RecoveryRequest::RetryState => Ok(RecoveryDecision::Retry),
            RecoveryRequest::Restart(target) => Ok(RecoveryDecision::Restart {
                checkpoint: target.position(),
            }),
            RecoveryRequest::Stop => stop(StopReason::Requested),
        }
    }
}

impl RunCommit {
    /// Validates this record only. No predecessor or historical state is available here.
    pub(crate) fn validate_current(
        &self,
        run_id: &RunId,
        sequence: u64,
        executable: &ExecutableProgram,
        loaded: bool,
    ) -> Result<()> {
        if &self.program_ref != executable.program.content_ref() {
            return Err(invalid(StateInvariant::Identity {
                field: "program_ref",
                expected: Box::new(executable.program.content_ref().clone()),
                actual: Box::new(self.program_ref.clone()),
            }));
        }
        let state = &self.state;
        if state.usage.len() != executable.declarations.len() {
            return Err(invalid(StateInvariant::Usage));
        }
        let mut total = 0u32;
        for (used, declaration) in state.usage.iter().zip(executable.program.declarations()) {
            if used.retries > declaration.allowances().retries()
                || used.restarts > declaration.allowances().restarts()
            {
                return Err(invalid(StateInvariant::Usage));
            }
            total = total
                .checked_add(used.retries)
                .and_then(|sum| sum.checked_add(used.restarts))
                .ok_or_else(|| invalid(StateInvariant::Usage))?;
        }
        if total > executable.program.limits().max_recovery_decisions() {
            return Err(invalid(StateInvariant::Usage));
        }
        let check = |object: &Object, expected: &ContentRef| -> Result<()> {
            if object.value_ref().schema_id() != expected.schema_id() {
                return Err(invalid(StateInvariant::Identity {
                    field: "slot_contract",
                    expected: Box::new(expected.clone()),
                    actual: Box::new(object.contract_ref().map_err(RuntimeError::from)?),
                }));
            }
            if loaded {
                executable
                    ._assembly
                    .values
                    .get(expected)
                    .ok_or_else(|| invalid(StateInvariant::Contract))?
                    .admit(object)?;
            }
            Ok(())
        };
        let call = |call: &Call| -> Result<()> {
            let declaration = executable
                .program
                .declarations()
                .get(call.position.state.index())
                .ok_or_else(|| invalid(StateInvariant::Position))?;
            check(&call.input, declaration.input_contract_ref())
        };
        let effect = |effect: &EffectCall| -> Result<()> {
            call(&effect.call)?;
            let Execution::Effect {
                command_contract_ref,
                ..
            } = executable.program.declarations()[effect.call.position.state.index()].execution()
            else {
                return Err(invalid(StateInvariant::Mode));
            };
            check(&effect.command, command_contract_ref)?;
            let expected = crate::engine::derive_effect_id(
                run_id,
                &self.program_ref,
                effect.call.position,
                effect.command.value_ref(),
            )?;
            if effect.effect_id != expected
                || state.effect_barrier != Some(effect.call.position.state)
            {
                return Err(invalid(StateInvariant::EffectIdentity));
            }
            Ok(())
        };
        let settlement = |settlement: &Settlement| -> Result<()> {
            effect(&settlement.effect)?;
            let Execution::Effect {
                evidence_contract_ref,
                ..
            } = executable.program.declarations()[settlement.effect.call.position.state.index()]
                .execution()
            else {
                return Err(invalid(StateInvariant::Mode));
            };
            check(&settlement.evidence, evidence_contract_ref)
        };
        let state_call = |value: &StateCall| -> Result<()> {
            call(value.call())?;
            let declaration =
                &executable.program.declarations()[value.call().position.state.index()];
            match (value, declaration.execution()) {
                (StateCall::Pure(_), Execution::Pure { .. }) => Ok(()),
                (
                    StateCall::Read {
                        intent, evidence, ..
                    },
                    Execution::Read {
                        intent_contract_ref,
                        evidence_contract_ref,
                        ..
                    },
                ) => {
                    check(intent, intent_contract_ref)?;
                    check(evidence, evidence_contract_ref)
                }
                (StateCall::Effect(value), Execution::Effect { .. }) => settlement(value),
                _ => Err(invalid(StateInvariant::Mode)),
            }
        };
        let read_failure = |failure: &ReadFailure| -> Result<()> {
            call(&failure.call)?;
            let Execution::Read {
                intent_contract_ref,
                error_contract_ref,
                ..
            } = executable.program.declarations()[failure.call.position.state.index()].execution()
            else {
                return Err(invalid(StateInvariant::Mode));
            };
            check(&failure.intent, intent_contract_ref)?;
            check(&failure.original, error_contract_ref)
        };
        let domain_failure = |failure: &DomainFailure| -> Result<()> {
            state_call(&failure.call)?;
            check(
                &failure.original,
                executable.program.declarations()[failure.call.call().position.state.index()]
                    .failure_contract_ref(),
            )
        };
        let failure = |failure: &Failure| -> Result<()> {
            match failure {
                Failure::Domain(value) => domain_failure(value),
                Failure::Read(value) => read_failure(value),
                Failure::PendingEffect {
                    effect: value,
                    original,
                } => {
                    effect(value)?;
                    let Execution::Effect {
                        error_contract_ref, ..
                    } = executable.program.declarations()[value.call.position.state.index()]
                        .execution()
                    else {
                        return Err(invalid(StateInvariant::Mode));
                    };
                    check(original, error_contract_ref)
                }
            }
        };
        let terminal = |failure: &TerminalFailure| -> Result<()> {
            match failure {
                TerminalFailure::Domain { failure, root, .. } => {
                    domain_failure(failure)?;
                    check(root, executable.program.root_failure_contract_ref())
                }
                TerminalFailure::Read { failure, .. } => read_failure(failure),
            }
        };
        let active = match &state.phase {
            Phase::Runnable(value) => {
                call(value)?;
                Some(value)
            }
            Phase::EffectPending(value) => {
                effect(value)?;
                Some(&value.call)
            }
            Phase::AwaitingInterpretation(value) => {
                settlement(value)?;
                Some(&value.effect.call)
            }
            Phase::AwaitingRecovery(value) => {
                failure(value)?;
                Some(value.call())
            }
            Phase::Failed(value) => {
                terminal(value)?;
                Some(match value {
                    TerminalFailure::Domain { failure, .. } => failure.call.call(),
                    TerminalFailure::Read { failure, .. } => &failure.call,
                })
            }
            Phase::Succeeded(value) => {
                check(value, executable.program.root_success_contract_ref())?;
                match &self.facts {
                    OperationFacts::Succeeded { call, .. } => Some(call.call()),
                    _ => None,
                }
            }
        };
        if state
            .checkpoints
            .windows(2)
            .any(|pair| pair[0].position >= pair[1].position)
        {
            return Err(invalid(StateInvariant::Checkpoint));
        }
        for checkpoint in &state.checkpoints {
            let declaration = executable
                .declarations
                .get(checkpoint.position.index())
                .ok_or_else(|| invalid(StateInvariant::Checkpoint))?;
            if !declaration.is_checkpoint
                || active.is_none_or(|call| checkpoint.position > call.position.state)
            {
                return Err(invalid(StateInvariant::Checkpoint));
            }
            check(
                &checkpoint.input,
                executable.program.declarations()[checkpoint.position.index()].input_contract_ref(),
            )?;
        }
        if let Some(active) = active {
            let declaration = executable
                .declarations
                .get(active.position.state.index())
                .ok_or_else(|| invalid(StateInvariant::Position))?;
            if declaration.is_checkpoint
                && !state.checkpoints.iter().any(|checkpoint| {
                    checkpoint.position == active.position.state && checkpoint.input == active.input
                })
            {
                return Err(invalid(StateInvariant::Checkpoint));
            }
        }
        if let Some(barrier) = state.effect_barrier {
            if !matches!(
                executable
                    .program
                    .declarations()
                    .get(barrier.index())
                    .map(|declaration| declaration.execution()),
                Some(Execution::Effect { .. })
            ) {
                return Err(invalid(StateInvariant::Barrier));
            }
            if active.is_none_or(|call| barrier > call.position.state)
                || matches!(&state.phase, Phase::Runnable(call) if barrier >= call.position.state)
            {
                return Err(invalid(StateInvariant::Barrier));
            }
        }
        let same_phase = |expected: Phase| {
            if state.phase == expected {
                Ok(())
            } else {
                Err(invalid(StateInvariant::Facts))
            }
        };
        match &self.facts {
            OperationFacts::Admitted { program, initial } => {
                if sequence != 1
                    || program.value_ref() != &self.program_ref
                    || program.canonical_bytes() != executable.program.canonical_bytes()
                    || initial.value_ref() != executable.program.initial_value_ref()
                {
                    return Err(invalid(StateInvariant::Facts));
                }
                check(initial, executable.program.admitted_context_contract_ref())?;
                if state != &RunState::initial(executable, initial.clone())? {
                    return Err(invalid(StateInvariant::Facts));
                }
            }
            facts => {
                if sequence <= 1 {
                    return Err(invalid(StateInvariant::Facts));
                }
                match facts {
                    OperationFacts::Succeeded {
                        call: completed,
                        output,
                    } => {
                        state_call(completed)?;
                        let position = completed.call().position;
                        check(
                            output,
                            executable.program.declarations()[position.state.index()]
                                .output_contract_ref(),
                        )?;
                        if position.state.index() + 1 == executable.declarations.len() {
                            same_phase(Phase::Succeeded(output.clone()))?;
                        } else {
                            same_phase(Phase::Runnable(Call {
                                position: ExecutionPosition {
                                    state: StatePosition::new(position.state.index() + 1)
                                        .map_err(|_| invalid(StateInvariant::Position))?,
                                    visit: position
                                        .visit
                                        .checked_next()
                                        .map_err(|_| invalid(StateInvariant::Position))?,
                                },
                                input: output.clone(),
                            }))?;
                        }
                    }
                    OperationFacts::Failed(value) => {
                        failure(value)?;
                        same_phase(Phase::AwaitingRecovery(value.clone()))?;
                    }
                    OperationFacts::EffectPrepared(value) => {
                        effect(value)?;
                        same_phase(Phase::EffectPending(value.clone()))?;
                    }
                    OperationFacts::EffectSettled(value) => {
                        settlement(value)?;
                        same_phase(Phase::AwaitingInterpretation(value.clone()))?;
                    }
                    OperationFacts::Recovered {
                        failure: value,
                        request,
                        decision,
                        ..
                    } => {
                        failure(value)?;
                        let position = value.call().position;
                        match decision {
                            RecoveryDecision::Retry => {
                                if *request != RecoveryRequest::RetryState
                                    || state.usage(position.state)?.state_retries == 0
                                {
                                    return Err(invalid(StateInvariant::Facts));
                                }
                                match value {
                                    value if value.phase() == mfm_program::ExecutionPhase::Read => {
                                        same_phase(Phase::Runnable(Call {
                                            position: ExecutionPosition {
                                                state: position.state,
                                                visit: position.visit.checked_next().map_err(
                                                    |_| invalid(StateInvariant::Position),
                                                )?,
                                            },
                                            input: value.call().input.clone(),
                                        }))?
                                    }
                                    Failure::PendingEffect { effect, .. } => {
                                        same_phase(Phase::EffectPending(effect.clone()))?
                                    }
                                    _ => return Err(invalid(StateInvariant::Mode)),
                                }
                            }
                            RecoveryDecision::Restart { checkpoint } => {
                                if !matches!(request, RecoveryRequest::Restart(target) if target.position() == *checkpoint)
                                    || matches!(
                                        value.phase(),
                                        mfm_program::ExecutionPhase::EffectPending
                                            | mfm_program::ExecutionPhase::EffectSettled
                                    )
                                    || state.usage(position.state)?.state_restarts == 0
                                    || !state.eligible(executable, position.state, *checkpoint)
                                {
                                    return Err(invalid(StateInvariant::Facts));
                                }
                                let retained = state
                                    .checkpoints
                                    .iter()
                                    .find(|entry| entry.position == *checkpoint)
                                    .ok_or_else(|| invalid(StateInvariant::Checkpoint))?;
                                same_phase(Phase::Runnable(Call {
                                    position: ExecutionPosition {
                                        state: *checkpoint,
                                        visit: position
                                            .visit
                                            .checked_next()
                                            .map_err(|_| invalid(StateInvariant::Position))?,
                                    },
                                    input: retained.input.clone(),
                                }))?;
                            }
                            RecoveryDecision::Stop { reason } => {
                                if state.authorize(executable, value, *request)? != *decision {
                                    return Err(invalid(StateInvariant::Facts));
                                }
                                match (value, &state.phase) {
                                    (
                                        Failure::Domain(value),
                                        Phase::Failed(TerminalFailure::Domain {
                                            failure,
                                            reason: retained,
                                            ..
                                        }),
                                    ) if value == failure && reason == retained => {}
                                    (
                                        Failure::Read(value),
                                        Phase::Failed(TerminalFailure::Read {
                                            failure,
                                            reason: retained,
                                        }),
                                    ) if value == failure && reason == retained => {}
                                    (
                                        Failure::PendingEffect { effect, .. },
                                        Phase::EffectPending(retained),
                                    ) if effect == retained => {}
                                    _ => return Err(invalid(StateInvariant::Facts)),
                                }
                            }
                        }
                    }
                    OperationFacts::Admitted { .. } => return Err(invalid(StateInvariant::Facts)),
                }
            }
        }
        Ok(())
    }
}

impl Call {
    /// Returns the retained execution occurrence.
    pub const fn position(&self) -> &ExecutionPosition {
        &self.position
    }
    /// Returns the complete executed input.
    pub const fn input(&self) -> &Object {
        &self.input
    }
}
impl EffectCall {
    /// Returns the original execution and input.
    pub const fn call(&self) -> &Call {
        &self.call
    }
    /// Returns the unchanged command authority.
    pub const fn effect_id(&self) -> &EffectId {
        &self.effect_id
    }
    /// Returns the complete prepared command.
    pub const fn command(&self) -> &Object {
        &self.command
    }
}
impl Settlement {
    /// Returns the settled command and authority.
    pub const fn effect(&self) -> &EffectCall {
        &self.effect
    }
    /// Returns accepted evidence, available before interpretation.
    pub const fn evidence(&self) -> &Object {
        &self.evidence
    }
}
impl DomainFailure {
    /// Returns the completed State operation and its accepted evidence when applicable.
    pub const fn call(&self) -> &StateCall {
        &self.call
    }
    /// Returns the original declared failure before any root mapping.
    pub const fn original(&self) -> &Object {
        &self.original
    }
}
impl ReadFailure {
    /// Returns the failed observation's execution and input.
    pub const fn call(&self) -> &Call {
        &self.call
    }
    /// Returns the exact prepared intent.
    pub const fn intent(&self) -> &Object {
        &self.intent
    }
    /// Returns the original operational cause.
    pub const fn original(&self) -> &Object {
        &self.original
    }
}

impl RunCommit {
    pub(crate) fn object_payload_bytes(&self) -> Result<u64> {
        fn object(value: &Object) -> u64 {
            value.canonical_bytes().len() as u64
        }
        fn call(value: &Call) -> u64 {
            object(&value.input)
        }
        fn effect(value: &EffectCall) -> u64 {
            call(&value.call) + object(&value.command)
        }
        fn settlement(value: &Settlement) -> u64 {
            effect(&value.effect) + object(&value.evidence)
        }
        fn state_call(value: &StateCall) -> u64 {
            match value {
                StateCall::Pure(value) => call(value),
                StateCall::Read {
                    call: value,
                    intent,
                    evidence,
                } => call(value) + object(intent) + object(evidence),
                StateCall::Effect(value) => settlement(value),
            }
        }
        fn domain(value: &DomainFailure) -> u64 {
            state_call(&value.call) + object(&value.original)
        }
        fn read(value: &ReadFailure) -> u64 {
            call(&value.call) + object(&value.intent) + object(&value.original)
        }
        fn failure(value: &Failure) -> u64 {
            match value {
                Failure::Domain(value) => domain(value),
                Failure::Read(value) => read(value),
                Failure::PendingEffect {
                    effect: value,
                    original,
                } => effect(value) + object(original),
            }
        }
        let phase = match &self.state.phase {
            Phase::Runnable(value) => call(value),
            Phase::EffectPending(value) => effect(value),
            Phase::AwaitingInterpretation(value) => settlement(value),
            Phase::AwaitingRecovery(value) => failure(value),
            Phase::Succeeded(value) => object(value),
            Phase::Failed(TerminalFailure::Domain { failure, root, .. }) => {
                domain(failure) + object(root)
            }
            Phase::Failed(TerminalFailure::Read { failure, .. }) => read(failure),
        };
        let facts = match &self.facts {
            OperationFacts::Admitted { program, initial } => object(program) + object(initial),
            OperationFacts::Succeeded { call, output } => state_call(call) + object(output),
            OperationFacts::Failed(value) | OperationFacts::Recovered { failure: value, .. } => {
                failure(value)
            }
            OperationFacts::EffectPrepared(value) => effect(value),
            OperationFacts::EffectSettled(value) => settlement(value),
        };
        self.state
            .checkpoints
            .iter()
            .try_fold(phase + facts, |sum, checkpoint| {
                sum.checked_add(object(&checkpoint.input))
                    .ok_or(RuntimeError::ArithmeticOverflow)
            })
    }
}
