#[cfg(test)]
mod tests;

use crate::assembly::ExecutableProgram;
use crate::{Result, RuntimeError};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition, RunId, StatePosition, VisitId};
use mfm_program::{
    Classification, Execution, RecoveryDenial, RecoveryLimit, RecoveryRequest, RecoveryUsage,
    StopReason,
};
use mfm_values::Object;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunRecord {
    pub(crate) program_ref: ContentRef,
    pub(crate) operation: RecordedOperation,
    pub(crate) checkpoints: Vec<Checkpoint>,
    pub(crate) usage: Vec<StateUsage>,
    pub(crate) effect_barrier: Option<StatePosition>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Checkpoint {
    pub(crate) position: StatePosition,
    pub(crate) input: Object,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StateUsage {
    pub(crate) retries: u32,
    pub(crate) restarts: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// An execution occurrence and its complete input.
pub struct Call {
    pub(crate) position: ExecutionPosition,
    pub(crate) input: Object,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// One retained Effect command and its authority.
pub struct EffectCall {
    pub(crate) call: Call,
    pub(crate) effect_id: EffectId,
    pub(crate) command: Object,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Accepted evidence for one retained Effect command.
pub struct Settlement {
    pub(crate) effect: EffectCall,
    pub(crate) evidence: Object,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
/// The original declared failure awaiting or retained by recovery.
pub enum Failure {
    /// A State-declared domain failure.
    Domain {
        /// Completed State operation.
        call: StateCall,
        /// Original declared failure.
        original: Object,
    },
    /// An operational Read failure.
    Read {
        /// Failed observation occurrence.
        call: Call,
        /// Prepared intent.
        intent: Object,
        /// Original operational error.
        original: Object,
    },
    /// An operational Effect failure retaining command authority.
    PendingEffect {
        /// Unchanged prepared command and execution.
        effect: EffectCall,
        /// Original operational cause.
        original: Object,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordedOperation {
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
        outcome: RecoveryOutcome,
    },
}

/// The action authorized and committed by Runtime, distinct from the handler's request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryOutcome {
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
        /// Mapped domain root; absent for operational failures.
        root: Option<Object>,
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
        mfm_values::InvocationDiagnostic::from_fields("runtime_invariant", "invalid", &code, None),
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
            Self::Domain { call, .. } => call.call(),
            Self::Read { call, .. } => call,
            Self::PendingEffect { effect, .. } => &effect.call,
        }
    }
    /// Returns the retained original without mapping or classification.
    pub fn original(&self) -> &Object {
        match self {
            Self::Domain { original, .. } | Self::Read { original, .. } => original,
            Self::PendingEffect { original, .. } => original,
        }
    }
    pub(crate) fn phase(&self) -> mfm_program::ExecutionPhase {
        match self {
            Self::Domain { call, .. } => call.phase(),
            Self::Read { .. } => mfm_program::ExecutionPhase::Read,
            Self::PendingEffect { .. } => mfm_program::ExecutionPhase::EffectPending,
        }
    }
}
impl RunRecord {
    pub(crate) fn initial(
        executable: &ExecutableProgram,
        program: Object,
        initial: Object,
    ) -> Result<Self> {
        let mut record = Self {
            program_ref: executable.program.content_ref().clone(),
            operation: RecordedOperation::Admitted { program, initial },
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
        record.enter(executable)?;
        Ok(record)
    }
    pub(crate) fn enter(&mut self, executable: &ExecutableProgram) -> Result<()> {
        if let Continuation::Runnable {
            position, input, ..
        } = self.continuation(executable)?
        {
            if executable.declarations[position.state.index()].is_checkpoint {
                let checkpoint = Checkpoint {
                    position: position.state,
                    input: input.clone(),
                };
                match self
                    .checkpoints
                    .binary_search_by_key(&position.state, |entry| entry.position)
                {
                    Ok(index) => self.checkpoints[index] = checkpoint,
                    Err(index) => self.checkpoints.insert(index, checkpoint),
                }
            }
        }
        Ok(())
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
    ) -> Result<RecoveryOutcome> {
        use mfm_program::ExecutionPhase;
        let position = failure.call().position.state;
        let used = self.usage(position)?;
        let allowance = executable.program.declarations()[position.index()].allowances();
        let stop = |reason| Ok(RecoveryOutcome::Stop { reason, root: None });
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
            RecoveryRequest::RetryState => Ok(RecoveryOutcome::Retry),
            RecoveryRequest::Restart(target) => Ok(RecoveryOutcome::Restart {
                checkpoint: target.position(),
            }),
            RecoveryRequest::Stop => stop(StopReason::Requested),
        }
    }
}

impl RunRecord {
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
        let state = self;
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
        let failure = |value: &Failure| -> Result<()> {
            match value {
                Failure::Domain { call, original } => {
                    state_call(call)?;
                    check(
                        original,
                        executable.program.declarations()[call.call().position.state.index()]
                            .failure_contract_ref(),
                    )
                }
                Failure::Read {
                    call: value,
                    intent,
                    original,
                } => {
                    call(value)?;
                    let Execution::Read {
                        intent_contract_ref,
                        error_contract_ref,
                        ..
                    } = executable.program.declarations()[value.position.state.index()].execution()
                    else {
                        return Err(invalid(StateInvariant::Mode));
                    };
                    check(intent, intent_contract_ref)?;
                    check(original, error_contract_ref)
                }
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
        if matches!(&self.operation, RecordedOperation::Admitted { .. }) != (sequence == 1) {
            return Err(invalid(StateInvariant::Facts));
        }
        match &self.operation {
            RecordedOperation::Admitted { program, initial } => {
                if program.value_ref() != &self.program_ref
                    || program.canonical_bytes() != executable.program.canonical_bytes()
                    || initial.value_ref() != executable.program.initial_value_ref()
                    || self
                        .usage
                        .iter()
                        .any(|used| used.retries != 0 || used.restarts != 0)
                    || self.effect_barrier.is_some()
                {
                    return Err(invalid(StateInvariant::Facts));
                }
                check(initial, executable.program.admitted_context_contract_ref())?;
            }
            RecordedOperation::Succeeded { call, output } => {
                state_call(call)?;
                check(
                    output,
                    executable.program.declarations()[call.call().position.state.index()]
                        .output_contract_ref(),
                )?;
            }
            RecordedOperation::Failed(value) => failure(value)?,
            RecordedOperation::EffectPrepared(value) => effect(value)?,
            RecordedOperation::EffectSettled(value) => settlement(value)?,
            RecordedOperation::Recovered {
                failure: value,
                request,
                outcome,
                ..
            } => {
                failure(value)?;
                let position = value.call().position.state;
                match outcome {
                    RecoveryOutcome::Retry => {
                        if *request != RecoveryRequest::RetryState
                            || self.usage(position)?.state_retries == 0
                        {
                            return Err(invalid(StateInvariant::Facts));
                        }
                        if !matches!(
                            value.phase(),
                            mfm_program::ExecutionPhase::Read
                                | mfm_program::ExecutionPhase::EffectPending
                        ) {
                            return Err(invalid(StateInvariant::Mode));
                        }
                    }
                    RecoveryOutcome::Restart { checkpoint } => {
                        if !matches!(request, RecoveryRequest::Restart(target) if target.position() == *checkpoint)
                            || matches!(
                                value.phase(),
                                mfm_program::ExecutionPhase::EffectPending
                                    | mfm_program::ExecutionPhase::EffectSettled
                            )
                            || self.usage(position)?.state_restarts == 0
                            || !self.eligible(executable, position, *checkpoint)
                        {
                            return Err(invalid(StateInvariant::Facts));
                        }
                    }
                    RecoveryOutcome::Stop { reason, root } => {
                        if !matches!(self.authorize(executable, value, *request)?, RecoveryOutcome::Stop { reason: expected, .. } if expected == *reason)
                        {
                            return Err(invalid(StateInvariant::Facts));
                        }
                        match (value, root) {
                            (Failure::Domain { .. }, Some(root)) => {
                                check(root, executable.program.root_failure_contract_ref())?
                            }
                            (Failure::Read { .. } | Failure::PendingEffect { .. }, None) => {}
                            _ => return Err(invalid(StateInvariant::Facts)),
                        }
                    }
                }
            }
        }
        let continuation = self.continuation(executable)?;
        let active = continuation.active();
        if let Continuation::Runnable {
            position, input, ..
        } = &continuation
        {
            let declaration = executable
                .program
                .declarations()
                .get(position.state.index())
                .ok_or_else(|| invalid(StateInvariant::Position))?;
            check(input, declaration.input_contract_ref())?;
        }
        if let Continuation::Succeeded { output, .. } = &continuation {
            check(output, executable.program.root_success_contract_ref())?;
        }
        if self
            .checkpoints
            .windows(2)
            .any(|pair| pair[0].position >= pair[1].position)
        {
            return Err(invalid(StateInvariant::Checkpoint));
        }
        for checkpoint in &self.checkpoints {
            let declaration = executable
                .declarations
                .get(checkpoint.position.index())
                .ok_or_else(|| invalid(StateInvariant::Checkpoint))?;
            if !declaration.is_checkpoint
                || active.is_none_or(|(position, _)| checkpoint.position > position.state)
            {
                return Err(invalid(StateInvariant::Checkpoint));
            }
            check(
                &checkpoint.input,
                executable.program.declarations()[checkpoint.position.index()].input_contract_ref(),
            )?;
        }
        if let Some((position, input)) = active {
            let declaration = executable
                .declarations
                .get(position.state.index())
                .ok_or_else(|| invalid(StateInvariant::Position))?;
            if declaration.is_checkpoint
                && !self.checkpoints.iter().any(|checkpoint| {
                    checkpoint.position == position.state && &checkpoint.input == input
                })
            {
                return Err(invalid(StateInvariant::Checkpoint));
            }
        }
        if let Some(barrier) = self.effect_barrier {
            if !matches!(
                executable
                    .program
                    .declarations()
                    .get(barrier.index())
                    .map(|declaration| declaration.execution()),
                Some(Execution::Effect { .. })
            ) || active.is_none_or(|(position, _)| barrier > position.state)
                || matches!(continuation, Continuation::Runnable { position, .. } if barrier >= position.state)
            {
                return Err(invalid(StateInvariant::Barrier));
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
impl RunRecord {
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
        fn failure(value: &Failure) -> u64 {
            match value {
                Failure::Domain { call, original } => state_call(call) + object(original),
                Failure::Read {
                    call: value,
                    intent,
                    original,
                } => call(value) + object(intent) + object(original),
                Failure::PendingEffect {
                    effect: value,
                    original,
                } => effect(value) + object(original),
            }
        }
        let facts = match &self.operation {
            RecordedOperation::Admitted { program, initial } => object(program) + object(initial),
            RecordedOperation::Succeeded { call, output } => state_call(call) + object(output),
            RecordedOperation::Failed(value) => failure(value),
            RecordedOperation::Recovered {
                failure: value,
                outcome,
                ..
            } => {
                failure(value)
                    + match outcome {
                        RecoveryOutcome::Stop {
                            root: Some(root), ..
                        } => object(root),
                        _ => 0,
                    }
            }
            RecordedOperation::EffectPrepared(value) => effect(value),
            RecordedOperation::EffectSettled(value) => settlement(value),
        };
        self.checkpoints.iter().try_fold(facts, |sum, checkpoint| {
            sum.checked_add(object(&checkpoint.input))
                .ok_or(RuntimeError::ArithmeticOverflow)
        })
    }
}

// This borrowed selection is computed from the current operation, never stored beside it.
pub(crate) enum Continuation<'a> {
    Runnable {
        position: ExecutionPosition,
        input: &'a Object,
        reason: crate::RunnableReason,
    },
    EffectPending {
        effect: &'a EffectCall,
        latest_failure: Option<(&'a Object, &'a RecoveryOutcome)>,
    },
    AwaitingInterpretation(&'a Settlement),
    AwaitingRecovery(&'a Failure),
    Succeeded {
        output: &'a Object,
        completed: Option<&'a Call>,
    },
    Failed {
        failure: &'a Failure,
        reason: StopReason,
        root: Option<&'a Object>,
    },
}
impl Continuation<'_> {
    pub(crate) fn active(&self) -> Option<(ExecutionPosition, &Object)> {
        let call = match self {
            Self::Runnable {
                position, input, ..
            } => return Some((*position, input)),
            Self::EffectPending { effect, .. } => &effect.call,
            Self::AwaitingInterpretation(value) => &value.effect.call,
            Self::AwaitingRecovery(value) | Self::Failed { failure: value, .. } => value.call(),
            Self::Succeeded { completed, .. } => (*completed)?,
        };
        Some((call.position, &call.input))
    }
}
impl RunRecord {
    pub(crate) fn continuation(&self, executable: &ExecutableProgram) -> Result<Continuation<'_>> {
        let position_error = |source| {
            RuntimeError::native(
                crate::Operation::Restore,
                mfm_values::InvocationDiagnostic::from_fields(
                    "runtime_invariant",
                    "continuation",
                    &source,
                    None,
                ),
            )
        };
        let runnable = |state, visit, input, reason| Continuation::Runnable {
            position: ExecutionPosition { state, visit },
            input,
            reason,
        };
        let next_visit =
            |position: ExecutionPosition| position.visit.checked_next().map_err(position_error);
        Ok(match &self.operation {
            RecordedOperation::Admitted { initial, .. } if executable.declarations.is_empty() => {
                Continuation::Succeeded {
                    output: initial,
                    completed: None,
                }
            }
            RecordedOperation::Admitted { initial, .. } => runnable(
                StatePosition::new(0).map_err(position_error)?,
                VisitId::new(0),
                initial,
                crate::RunnableReason::Advance,
            ),
            RecordedOperation::Succeeded { call, output } => {
                let position = call.call().position;
                if position.state.index() + 1 == executable.declarations.len() {
                    Continuation::Succeeded {
                        output,
                        completed: Some(call.call()),
                    }
                } else {
                    runnable(
                        StatePosition::new(position.state.index() + 1).map_err(position_error)?,
                        next_visit(position)?,
                        output,
                        crate::RunnableReason::Advance,
                    )
                }
            }
            RecordedOperation::Failed(value) => Continuation::AwaitingRecovery(value),
            RecordedOperation::EffectPrepared(effect) => Continuation::EffectPending {
                effect,
                latest_failure: None,
            },
            RecordedOperation::EffectSettled(value) => Continuation::AwaitingInterpretation(value),
            RecordedOperation::Recovered {
                failure, outcome, ..
            } => {
                let position = failure.call().position;
                match (failure, outcome) {
                    (
                        Failure::PendingEffect { effect, original },
                        RecoveryOutcome::Retry | RecoveryOutcome::Stop { .. },
                    ) => Continuation::EffectPending {
                        effect,
                        latest_failure: Some((original, outcome)),
                    },
                    (_, RecoveryOutcome::Retry) => runnable(
                        position.state,
                        next_visit(position)?,
                        &failure.call().input,
                        crate::RunnableReason::Retry,
                    ),
                    (_, RecoveryOutcome::Restart { checkpoint }) => {
                        let input = &self
                            .checkpoints
                            .iter()
                            .find(|entry| entry.position == *checkpoint)
                            .ok_or_else(|| invalid(StateInvariant::Checkpoint))?
                            .input;
                        runnable(
                            *checkpoint,
                            next_visit(position)?,
                            input,
                            crate::RunnableReason::Restart {
                                checkpoint: *checkpoint,
                            },
                        )
                    }
                    (_, RecoveryOutcome::Stop { reason, root }) => Continuation::Failed {
                        failure,
                        reason: *reason,
                        root: root.as_ref(),
                    },
                }
            }
        })
    }
}
