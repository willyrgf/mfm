use std::collections::BTreeSet;
use std::ops::ControlFlow;

use mfm_events::v1 as events;
use mfm_ids::{AttemptId, NodeId, SideEffectPairId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::framework::public_output_is_produced;
use crate::side_effect_lifecycle::{
    open_attempt_disposition, side_effect_for_attempt, validate_terminal_evidence,
};
use crate::side_effects::validate_terminal_cell_has_completed_attempt;
use crate::spec_authority::CurrentSpecRead;
use crate::{Result, RuntimeError};

pub(crate) struct RunnableNode<'a> {
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) attempt: AttemptPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttemptPlan {
    StartNew {
        attempt_no: u32,
    },
    Continue {
        attempt_id: AttemptId,
        attempt_no: u32,
    },
}

/// Pure scheduler frontier decision for the current verified run view.
pub(crate) enum SchedulerDecision<'a> {
    /// A certified node attempt can run.
    Run(RunnableNode<'a>),
    /// No node can run yet.
    Blocked,
    /// The public-output frontier has completed.
    Completed,
}

pub(crate) fn scheduler_decision_with_blocked_nodes<'a, S>(
    runtime_spec: &'a S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<SchedulerDecision<'a>>
where
    S: CurrentSpecRead + ?Sized,
{
    if lifecycle.run_state() == store::RunState::Completed {
        return Ok(SchedulerDecision::Completed);
    }
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    lifecycle
        .with_saga(&runtime_spec.spec().saga, &terminal_policies, |saga| {
            if saga.engagement().is_some() {
                return saga_scheduler_decision(runtime_spec, lifecycle, &saga, blocked_nodes);
            }
            if public_output_is_produced(runtime_spec, lifecycle) {
                return match next_runnable_node(runtime_spec, lifecycle, blocked_nodes)? {
                    Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                    None => Ok(SchedulerDecision::Completed),
                };
            }
            match next_runnable_node(runtime_spec, lifecycle, blocked_nodes)? {
                Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                None => Ok(SchedulerDecision::Blocked),
            }
        })
        .map_err(RuntimeError::from)?
}

fn saga_scheduler_decision<'a, S>(
    runtime_spec: &'a S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    saga: &store::current_lifecycle::CurrentSagaRef<'_>,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<SchedulerDecision<'a>>
where
    S: CurrentSpecRead + ?Sized,
{
    if let Some(runnable) = next_forward_completion_node(runtime_spec, lifecycle, blocked_nodes)? {
        return Ok(SchedulerDecision::Run(runnable));
    }
    match saga.run_mode() {
        store::RunMode::Forward => Ok(SchedulerDecision::Blocked),
        store::RunMode::Remediating | store::RunMode::Compensated => {
            match next_remediation_node(runtime_spec, lifecycle, saga, blocked_nodes)? {
                Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                None if saga.run_mode() == store::RunMode::Compensated => {
                    match next_saga_terminal_node(runtime_spec, lifecycle, saga)? {
                        Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                        None => Ok(SchedulerDecision::Blocked),
                    }
                }
                None => Ok(SchedulerDecision::Blocked),
            }
        }
        store::RunMode::Completed => Ok(SchedulerDecision::Completed),
        store::RunMode::ManuallyResolved | store::RunMode::FailedWithoutAcdcClaim => {
            match next_saga_terminal_node(runtime_spec, lifecycle, saga)? {
                Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                None => Ok(SchedulerDecision::Blocked),
            }
        }
        store::RunMode::ManualBlocked => Ok(SchedulerDecision::Blocked),
    }
}

fn next_runnable_node<'a, S>(
    runtime_spec: &'a S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<Option<RunnableNode<'a>>>
where
    S: CurrentSpecRead + ?Sized,
{
    for node_id in runtime_spec.topological_order() {
        if blocked_nodes.contains(node_id) {
            continue;
        }
        let node = runtime_spec.node(node_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "certified topological order references missing node {node_id}"
            ))
        })?;
        if let Some(runnable) = planned_runnable_node_if_ready(runtime_spec, node, lifecycle)? {
            return Ok(Some(runnable));
        }
    }
    Ok(None)
}

fn next_forward_completion_node<'a, S>(
    runtime_spec: &'a S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<Option<RunnableNode<'a>>>
where
    S: CurrentSpecRead + ?Sized,
{
    for node_id in runtime_spec.topological_order() {
        if blocked_nodes.contains(node_id) {
            continue;
        }
        let node = runtime_spec.node(node_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "certified topological order references missing node {node_id}"
            ))
        })?;
        if node.side_effect.is_some() {
            let Some(runnable) =
                continuing_side_effect_node_if(runtime_spec, node, lifecycle, |state| {
                    state.is_forward_completion_candidate()
                })?
            else {
                continue;
            };
            return Ok(Some(runnable));
        }
        if let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework {
            let Some(ledger) = lifecycle.side_effect(&verify.pair_id) else {
                continue;
            };
            let state = ledger
                .ledger_state()
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            if !state.is_forward_completion_candidate() {
                continue;
            }
            if let Some(runnable) = planned_runnable_node_if_ready(runtime_spec, node, lifecycle)? {
                return Ok(Some(runnable));
            }
        }
    }
    Ok(None)
}

fn next_remediation_node<'a, S>(
    runtime_spec: &'a S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    saga: &store::current_lifecycle::CurrentSagaRef<'_>,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<Option<RunnableNode<'a>>>
where
    S: CurrentSpecRead + ?Sized,
{
    for forward_pair_id in owed_obligations_reverse_confirmation_order(lifecycle, saga)? {
        let obligation = saga.obligation(&forward_pair_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "owed obligation {forward_pair_id} disappeared from current saga"
            ))
        })?;
        let forward = lifecycle.side_effect(&forward_pair_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "owed obligation {forward_pair_id} has no forward ledger"
            ))
        })?;
        let remediation_node = runtime_spec
            .remediation_for_forward_node(forward.intent().node_id())
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "forward node {} has no certified remediation node",
                    forward.intent().node_id()
                ))
            })?;
        let verify_node = if remediation_node.side_effect.is_some() {
            Some(certified_side_effect_verify_node(
                runtime_spec,
                &remediation_node.node_id,
            )?)
        } else {
            None
        };
        if blocked_nodes.contains(&remediation_node.node_id) {
            continue;
        }
        if let Some(remediation) = obligation.remediation() {
            if remediation.unresolved().is_some() {
                return Ok(None);
            }
            let output_node = verify_node.unwrap_or(remediation_node);
            if remediation.closed() && lifecycle.cell(&output_node.output_cell).is_some() {
                continue;
            }
            if let Some(verify_node) = verify_node {
                if let Some(attempt) = attempt_plan(runtime_spec, remediation_node, lifecycle)? {
                    if node_inputs_ready(runtime_spec, remediation_node, lifecycle)? {
                        return Ok(Some(RunnableNode {
                            node: remediation_node,
                            attempt,
                        }));
                    }
                    return Ok(None);
                }
                if blocked_nodes.contains(&verify_node.node_id) {
                    return Ok(None);
                }
                let ledger = lifecycle
                    .side_effect(remediation.pair_id())
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "remediation pair {} has no side-effect ledger",
                            remediation.pair_id()
                        ))
                    })?;
                let state = ledger
                    .ledger_state()
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                if remediation_verify_completion_candidate(&state) {
                    return planned_runnable_node_if_ready(runtime_spec, verify_node, lifecycle);
                }
                return Ok(None);
            }
        }
        return planned_runnable_node_if_ready(runtime_spec, remediation_node, lifecycle);
    }
    Ok(None)
}

fn certified_side_effect_verify_node<'a, S>(
    runtime_spec: &'a S,
    submit_node_id: &NodeId,
) -> Result<&'a spec::NodeSpec>
where
    S: CurrentSpecRead + ?Sized,
{
    let mut found = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "certified topological order references missing node {node_id}"
            ))
        })?;
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::SideEffectVerify(verify))
                if verify.submit_node_id == *submit_node_id
        ) && found.replace(node).is_some()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "submit node {submit_node_id} has multiple certified side-effect verify nodes"
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "submit node {submit_node_id} is missing certified side-effect verify node"
        ))
    })
}

fn remediation_verify_completion_candidate(
    state: &store::current_lifecycle::CurrentSideEffectLedgerState<'_>,
) -> bool {
    matches!(
        state.ledger_purpose(),
        events::SideEffectLedgerPurpose::Remediation { .. }
    ) && matches!(
        state.phase(),
        store::SideEffectLedgerPhase::SubmissionKnown { .. }
            | store::SideEffectLedgerPhase::ReceiptObserved { .. }
            | store::SideEffectLedgerPhase::Confirmed { .. }
    )
}

fn next_saga_terminal_node<'a, S>(
    runtime_spec: &'a S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    saga: &store::current_lifecycle::CurrentSagaRef<'_>,
) -> Result<Option<RunnableNode<'a>>>
where
    S: CurrentSpecRead + ?Sized,
{
    if !saga.forward_quiescent() {
        return Ok(None);
    }
    if !matches!(
        saga.run_mode(),
        store::RunMode::Compensated
            | store::RunMode::ManuallyResolved
            | store::RunMode::FailedWithoutAcdcClaim
    ) || !lifecycle.no_open_attempts()
    {
        return Ok(None);
    }
    let node = certified_resolve_saga_terminal_node(runtime_spec)?;
    let Some(attempt) = non_side_effect_attempt_plan(runtime_spec, node, lifecycle)? else {
        return Ok(None);
    };
    if node_inputs_ready(runtime_spec, node, lifecycle)? {
        return Ok(Some(RunnableNode { node, attempt }));
    }
    Ok(None)
}

fn certified_resolve_saga_terminal_node<S>(runtime_spec: &S) -> Result<&spec::NodeSpec>
where
    S: CurrentSpecRead + ?Sized,
{
    let mut resolve_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "certified topological order references missing node {node_id}"
            ))
        })?;
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) && resolve_node.replace(node).is_some()
        {
            return Err(RuntimeError::InvalidSpec(
                "multiple certified resolve-saga-terminal framework nodes".to_owned(),
            ));
        }
    }
    resolve_node.ok_or_else(|| {
        RuntimeError::InvalidSpec(
            "saga terminal resolution lacks a certified ResolveSagaTerminal framework node"
                .to_owned(),
        )
    })
}

fn continuing_side_effect_node_if<'a, S>(
    runtime_spec: &'a S,
    node: &'a spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    mut predicate: impl FnMut(&store::current_lifecycle::CurrentSideEffectLedgerState<'_>) -> bool,
) -> Result<Option<RunnableNode<'a>>>
where
    S: CurrentSpecRead + ?Sized,
{
    let mut selected = None;
    let mut failure = None;
    let _ = lifecycle.visit_attempts(|attempt| {
        if attempt.node_id() != &node.node_id
            || !matches!(
                attempt.status(),
                store::current_lifecycle::CurrentAttemptStatusRef::Started { .. }
            )
        {
            return ControlFlow::Continue(());
        }
        match side_effect_for_attempt(runtime_spec, lifecycle, node, attempt.attempt_id()) {
            Ok(Some(ledger)) => match ledger
                .ledger_state()
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))
            {
                Ok(state) if predicate(&state) => {
                    if selected.replace(attempt.attempt_id().clone()).is_some() {
                        failure = Some(RuntimeError::InvalidRunStream(format!(
                            "node {} has multiple side-effect completion candidates",
                            node.node_id
                        )));
                        return ControlFlow::Break(());
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    failure = Some(error);
                    return ControlFlow::Break(());
                }
            },
            Ok(None) => {}
            Err(error) => {
                failure = Some(error);
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    });
    if let Some(error) = failure {
        return Err(error);
    }
    let Some(_) = selected else {
        return Ok(None);
    };
    match side_effect_attempt_plan(runtime_spec, node, lifecycle)? {
        Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }) if node_inputs_ready(runtime_spec, node, lifecycle)? => Ok(Some(RunnableNode {
            node,
            attempt: AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            },
        })),
        Some(AttemptPlan::Continue { .. }) | None => Ok(None),
        Some(AttemptPlan::StartNew { .. }) => Err(RuntimeError::InvalidRunStream(format!(
            "engaged saga attempted to start forward side-effect node {}",
            node.node_id
        ))),
    }
}

fn owed_obligations_reverse_confirmation_order(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    saga: &store::current_lifecycle::CurrentSagaRef<'_>,
) -> Result<Vec<SideEffectPairId>> {
    let mut pair_ids = Vec::new();
    let _ = saga.visit_obligations(|obligation| {
        if obligation.classification() == store::ForwardLedgerClassification::Owed {
            pair_ids.push(obligation.forward_pair_id().clone());
        }
        ControlFlow::<()>::Continue(())
    });
    let mut positioned = pair_ids
        .into_iter()
        .map(|pair_id| {
            forward_confirmation_position(lifecycle, &pair_id).map(|position| (position, pair_id))
        })
        .collect::<Result<Vec<_>>>()?;
    positioned.sort_by_key(|(position, _)| *position);
    positioned.reverse();
    Ok(positioned.into_iter().map(|(_, pair_id)| pair_id).collect())
}

fn forward_confirmation_position(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    pair_id: &SideEffectPairId,
) -> Result<(u64, u32)> {
    let mut found = None;
    let mut duplicate = false;
    let _ =
        lifecycle.visit_records(|record| {
            let store::current_lifecycle::CurrentRecordKindRef::SideEffectConfirmationObserved(
                payload,
            ) = record.kind()
            else {
                return ControlFlow::Continue(());
            };
            if &payload.pair_id != pair_id {
                return ControlFlow::Continue(());
            }
            let position = (record.sequence().as_u64(), record.ordinal().as_u32());
            if found.replace(position).is_some() {
                duplicate = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
    if duplicate {
        return Err(RuntimeError::InvalidRunStream(format!(
            "forward pair {pair_id} has multiple confirmation events"
        )));
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "owed forward pair {pair_id} has no confirmation event"
        ))
    })
}

fn attempt_plan<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<Option<AttemptPlan>>
where
    S: CurrentSpecRead + ?Sized,
{
    if matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    ) {
        return Ok(None);
    }
    if node.side_effect.is_some() {
        side_effect_attempt_plan(runtime_spec, node, lifecycle)
    } else {
        non_side_effect_attempt_plan(runtime_spec, node, lifecycle)
    }
}

fn planned_runnable_node_if_ready<'a, S>(
    runtime_spec: &'a S,
    node: &'a spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<Option<RunnableNode<'a>>>
where
    S: CurrentSpecRead + ?Sized,
{
    let Some(attempt) = attempt_plan(runtime_spec, node, lifecycle)? else {
        return Ok(None);
    };
    if node_inputs_ready(runtime_spec, node, lifecycle)? {
        return Ok(Some(RunnableNode { node, attempt }));
    }
    Ok(None)
}

fn non_side_effect_attempt_plan<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<Option<AttemptPlan>>
where
    S: CurrentSpecRead + ?Sized,
{
    if let Some(cell_terminal) = lifecycle.cell(&node.output_cell) {
        validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            lifecycle,
            node,
            &cell_terminal,
        )?;
        return Ok(None);
    }
    let mut started = None::<(AttemptId, u32)>;
    let mut non_retryable = false;
    let mut failure = None;
    let _ = lifecycle.visit_attempts(|attempt| {
        if attempt.node_id() != &node.node_id {
            return ControlFlow::Continue(());
        }
        match attempt.status() {
            store::current_lifecycle::CurrentAttemptStatusRef::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    failure = Some(RuntimeError::InvalidRunStream(format!(
                        "started attempt {} for node {} has state identity outside the certified spec",
                        attempt.attempt_id(), node.node_id
                    )));
                    return ControlFlow::Break(());
                }
                if started
                    .replace((attempt.attempt_id().clone(), attempt_no))
                    .is_some()
                {
                    failure = Some(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal attempts",
                        node.node_id
                    )));
                    return ControlFlow::Break(());
                }
            }
            store::current_lifecycle::CurrentAttemptStatusRef::Completed { output_cell_id } => {
                failure = Some(RuntimeError::InvalidRunStream(format!(
                    "node {} attempt {} completed output cell {} without terminal cell authority",
                    node.node_id,
                    attempt.attempt_id(),
                    output_cell_id
                )));
                return ControlFlow::Break(());
            }
            store::current_lifecycle::CurrentAttemptStatusRef::Failed {
                retryable: false, ..
            } => non_retryable = true,
            store::current_lifecycle::CurrentAttemptStatusRef::Failed { .. }
            | store::current_lifecycle::CurrentAttemptStatusRef::Interrupted => {}
        }
        ControlFlow::Continue(())
    });
    if let Some(error) = failure {
        return Err(error);
    }
    if non_retryable {
        return Ok(None);
    }
    attempt_plan_from_started_or_new(started, lifecycle, &node.node_id)
}

fn side_effect_attempt_plan<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<Option<AttemptPlan>>
where
    S: CurrentSpecRead + ?Sized,
{
    if let Some(cell_terminal) = lifecycle.cell(&node.output_cell) {
        let attempt_id = validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            lifecycle,
            node,
            &cell_terminal,
        )?;
        validate_terminal_evidence(runtime_spec, lifecycle, node, &attempt_id)?;
        return Ok(None);
    }
    let mut started = None::<(AttemptId, u32)>;
    let mut non_retryable = false;
    let mut failure = None;
    let _ = lifecycle.visit_attempts(|attempt| {
        if attempt.node_id() != &node.node_id {
            return ControlFlow::Continue(());
        }
        match attempt.status() {
            store::current_lifecycle::CurrentAttemptStatusRef::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    failure = Some(RuntimeError::InvalidRunStream(format!(
                        "started side-effect attempt {} for node {} has state identity outside the certified spec",
                        attempt.attempt_id(), node.node_id
                    )));
                    return ControlFlow::Break(());
                }
                if let Err(error) = open_attempt_disposition(
                    runtime_spec,
                    lifecycle,
                    node,
                    attempt.attempt_id(),
                ) {
                    failure = Some(error);
                    return ControlFlow::Break(());
                }
                if started
                    .replace((attempt.attempt_id().clone(), attempt_no))
                    .is_some()
                {
                    failure = Some(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal side-effect attempts",
                        node.node_id
                    )));
                    return ControlFlow::Break(());
                }
            }
            store::current_lifecycle::CurrentAttemptStatusRef::Completed { output_cell_id } => {
                if lifecycle.cell(output_cell_id).is_none() {
                    failure = Some(RuntimeError::InvalidRunStream(format!(
                        "side-effect node {} attempt {} completed without terminal output cell {}",
                        node.node_id,
                        attempt.attempt_id(),
                        output_cell_id
                    )));
                    return ControlFlow::Break(());
                }
            }
            store::current_lifecycle::CurrentAttemptStatusRef::Failed {
                retryable: false, ..
            } => non_retryable = true,
            store::current_lifecycle::CurrentAttemptStatusRef::Failed { .. }
            | store::current_lifecycle::CurrentAttemptStatusRef::Interrupted => {}
        }
        ControlFlow::Continue(())
    });
    if let Some(error) = failure {
        return Err(error);
    }
    if non_retryable {
        return Ok(None);
    }
    attempt_plan_from_started_or_new(started, lifecycle, &node.node_id)
}

fn attempt_plan_from_started_or_new(
    started: Option<(AttemptId, u32)>,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    node_id: &NodeId,
) -> Result<Option<AttemptPlan>> {
    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew {
            attempt_no: next_attempt_no(lifecycle, node_id)?,
        }))
    }
}

pub(crate) fn node_inputs_ready<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<bool>
where
    S: CurrentSpecRead + ?Sized,
{
    runtime_spec.validate_input_binding(&node.input_bindings.root)?;
    input_binding_ready(runtime_spec, node, &node.input_bindings.root, lifecycle)
}

fn input_binding_ready<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    input: &spec::InputBindingNodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<bool>
where
    S: CurrentSpecRead + ?Sized,
{
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(true),
        spec::InputBindingNodeSpec::Cell(cell) => {
            input_cell_ready(runtime_spec, node, cell, lifecycle)
        }
        spec::InputBindingNodeSpec::Tuple(elements) => {
            input_bindings_ready(runtime_spec, node, elements, lifecycle)
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            fields.iter().try_fold(true, |ready, field| {
                Ok(ready && input_binding_ready(runtime_spec, node, &field.node, lifecycle)?)
            })
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            input_bindings_ready(runtime_spec, node, elements, lifecycle)
        }
    }
}

fn input_bindings_ready<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    elements: &[spec::InputBindingNodeSpec],
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<bool>
where
    S: CurrentSpecRead + ?Sized,
{
    elements.iter().try_fold(true, |ready, element| {
        Ok(ready && input_binding_ready(runtime_spec, node, element, lifecycle)?)
    })
}

fn input_cell_ready<S>(
    runtime_spec: &S,
    node: &spec::NodeSpec,
    input: &spec::InputBindingCellSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<bool>
where
    S: CurrentSpecRead + ?Sized,
{
    let cell = runtime_spec.cell(&input.cell_id).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} input cell {} is missing",
            node.node_id, input.cell_id
        ))
    })?;
    match &cell.producer {
        spec::CellProducer::Seed(_) => Ok(lifecycle.seed(&input.cell_id)?.is_some()),
        spec::CellProducer::Node(_) => match lifecycle.cell(&input.cell_id) {
            Some(terminal) if terminal.produced().is_some() => Ok(true),
            Some(terminal) if terminal.skipped().is_some() => {
                Ok(input.required_terminal == spec::RequiredTerminal::MaybeSkipped)
            }
            Some(_) | None => Ok(false),
        },
    }
}

fn next_attempt_no(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    node_id: &NodeId,
) -> Result<u32> {
    let mut count = 0_usize;
    let _ = lifecycle.visit_attempts(|attempt| {
        if attempt.node_id() == node_id {
            count += 1;
        }
        ControlFlow::<()>::Continue(())
    });
    u32::try_from(count + 1).map_err(|_| {
        RuntimeError::InvalidRunStream(format!("attempt count overflow for {node_id}"))
    })
}
