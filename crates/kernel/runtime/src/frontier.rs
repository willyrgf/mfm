use std::collections::BTreeSet;

use mfm_events::v1 as events;
use mfm_ids::{AttemptId, NodeId, RunId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::framework::public_output_is_produced;
use crate::history::RuntimeRunView;
use crate::side_effect_lifecycle::SideEffectLifecycle;
use crate::side_effects::validate_terminal_cell_has_completed_attempt;
use crate::{CertifiedRuntimeSpec, Result, RuntimeError};

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

pub(crate) fn scheduler_decision_with_blocked_nodes<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &RunId,
    view: &RuntimeRunView,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<SchedulerDecision<'a>> {
    if view.projections.run_state(run_id) == store::RunState::Completed {
        return Ok(SchedulerDecision::Completed);
    }
    let saga = view
        .projections
        .derive_saga_projection(run_id, &runtime_spec.spec().saga);
    if saga.engagement.is_some() {
        return saga_scheduler_decision(runtime_spec, view, &saga, blocked_nodes);
    }
    if public_output_is_produced(runtime_spec, &view.projections) {
        return match next_runnable_node(runtime_spec, view, blocked_nodes)? {
            Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
            None => Ok(SchedulerDecision::Completed),
        };
    }
    match next_runnable_node(runtime_spec, view, blocked_nodes)? {
        Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
        None => Ok(SchedulerDecision::Blocked),
    }
}

fn saga_scheduler_decision<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
    saga: &store::SagaProjection,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<SchedulerDecision<'a>> {
    if let Some(runnable) = next_forward_completion_node(runtime_spec, view, blocked_nodes)? {
        return Ok(SchedulerDecision::Run(runnable));
    }
    match saga.run_mode {
        store::RunMode::Forward => Ok(SchedulerDecision::Blocked),
        store::RunMode::Remediating | store::RunMode::Compensated => {
            match next_remediation_node(runtime_spec, view, saga, blocked_nodes)? {
                Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                None if saga.run_mode == store::RunMode::Compensated => {
                    match next_saga_terminal_node(runtime_spec, view, saga)? {
                        Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                        None => Ok(SchedulerDecision::Blocked),
                    }
                }
                None => Ok(SchedulerDecision::Blocked),
            }
        }
        store::RunMode::Completed => Ok(SchedulerDecision::Completed),
        store::RunMode::ManuallyResolved | store::RunMode::FailedWithoutAcdcClaim => {
            match next_saga_terminal_node(runtime_spec, view, saga)? {
                Some(runnable) => Ok(SchedulerDecision::Run(runnable)),
                None => Ok(SchedulerDecision::Blocked),
            }
        }
        store::RunMode::ManualBlocked => Ok(SchedulerDecision::Blocked),
    }
}

fn next_runnable_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<Option<RunnableNode<'a>>> {
    for node_id in runtime_spec.topological_order() {
        if blocked_nodes.contains(node_id) {
            continue;
        }
        let node = runtime_spec.node(node_id).expect("topological node exists");
        let Some(attempt) = attempt_plan(runtime_spec, node, view)? else {
            continue;
        };
        if node_inputs_ready(runtime_spec, node, view)? {
            return Ok(Some(RunnableNode { node, attempt }));
        }
    }
    Ok(None)
}

fn next_forward_completion_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<Option<RunnableNode<'a>>> {
    for node_id in runtime_spec.topological_order() {
        if blocked_nodes.contains(node_id) {
            continue;
        }
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if node.side_effect.is_none() {
            continue;
        }
        let Some(runnable) = continuing_side_effect_node_if(runtime_spec, node, view, |state| {
            state.is_forward_completion_candidate()
        })?
        else {
            continue;
        };
        return Ok(Some(runnable));
    }
    Ok(None)
}

fn next_remediation_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
    saga: &store::SagaProjection,
    blocked_nodes: &BTreeSet<NodeId>,
) -> Result<Option<RunnableNode<'a>>> {
    for obligation in owed_obligations_reverse_confirmation_order(view, saga)? {
        let forward = view
            .projections
            .side_effect(&obligation.forward_ledger_key)
            .ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "owed obligation {} has no forward ledger projection",
                    obligation.forward_ledger_key
                ))
            })?;
        let remediation_node = runtime_spec
            .remediation_for_forward_node(&forward.intent.node_id)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "forward node {} has no certified remediation node",
                    forward.intent.node_id
                ))
            })?;
        if blocked_nodes.contains(&remediation_node.node_id) {
            continue;
        }
        if let Some(remediation) = &obligation.remediation {
            if remediation.unresolved.is_some() {
                return Ok(None);
            }
            if remediation.closed
                && view
                    .projections
                    .cell_terminal(&remediation_node.output_cell)
                    .is_some()
            {
                continue;
            }
        }
        let Some(attempt) = attempt_plan(runtime_spec, remediation_node, view)? else {
            return Ok(None);
        };
        if node_inputs_ready(runtime_spec, remediation_node, view)? {
            return Ok(Some(RunnableNode {
                node: remediation_node,
                attempt,
            }));
        }
        return Ok(None);
    }
    Ok(None)
}

fn next_saga_terminal_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
    saga: &store::SagaProjection,
) -> Result<Option<RunnableNode<'a>>> {
    if !saga.forward_quiescent {
        return Ok(None);
    }
    if !matches!(
        saga.run_mode,
        store::RunMode::Compensated
            | store::RunMode::ManuallyResolved
            | store::RunMode::FailedWithoutAcdcClaim
    ) {
        return Ok(None);
    }
    if view
        .projections
        .attempts()
        .any(|(_, attempt)| matches!(attempt.status, store::AttemptStatus::Started { .. }))
    {
        return Ok(None);
    }
    let node = certified_resolve_saga_terminal_node(runtime_spec)?;
    let Some(attempt) = non_side_effect_attempt_plan(runtime_spec, node, view)? else {
        return Ok(None);
    };
    if node_inputs_ready(runtime_spec, node, view)? {
        return Ok(Some(RunnableNode { node, attempt }));
    }
    Ok(None)
}

fn certified_resolve_saga_terminal_node(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<&spec::NodeSpec> {
    let mut resolve_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
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

fn continuing_side_effect_node_if<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    node: &'a spec::NodeSpec,
    view: &RuntimeRunView,
    mut predicate: impl FnMut(&store::SideEffectLedgerState<'_>) -> bool,
) -> Result<Option<RunnableNode<'a>>> {
    let mut selected = None;
    for ((attempt_node_id, attempt_id), attempt) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        if !matches!(attempt.status, store::AttemptStatus::Started { .. }) {
            continue;
        }
        let Some(projection) =
            SideEffectLifecycle::projection_for_attempt(&view.projections, node, attempt_id)?
        else {
            continue;
        };
        let state = projection
            .ledger_state()
            .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
        if predicate(&state) && selected.replace(attempt_id.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "node {} has multiple side-effect completion candidates",
                node.node_id
            )));
        }
    }
    let Some(_) = selected else {
        return Ok(None);
    };
    match side_effect_attempt_plan(runtime_spec, node, view)? {
        Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }) if node_inputs_ready(runtime_spec, node, view)? => Ok(Some(RunnableNode {
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

fn owed_obligations_reverse_confirmation_order<'a>(
    view: &RuntimeRunView,
    saga: &'a store::SagaProjection,
) -> Result<Vec<&'a store::SagaObligationProjection>> {
    let mut positioned = Vec::new();
    for obligation in saga.obligations.values() {
        if obligation.classification != store::ForwardLedgerClassification::Owed {
            continue;
        }
        let position = forward_confirmation_position(view, &obligation.forward_ledger_key)?;
        positioned.push((position, obligation));
    }
    positioned.sort_by_key(|(position, _)| *position);
    positioned.reverse();
    Ok(positioned
        .into_iter()
        .map(|(_, obligation)| obligation)
        .collect())
}

fn forward_confirmation_position(
    view: &RuntimeRunView,
    ledger_key: &events::SideEffectLedgerKey,
) -> Result<(u64, u32)> {
    let mut found = None;
    for event in &view.stream {
        let events::KernelEventPayload::SideEffectConfirmationObserved(payload) = event.payload()
        else {
            continue;
        };
        if &payload.ledger_key != ledger_key {
            continue;
        }
        let position = (event.seq().as_u64(), event.ordinal().as_u32());
        if found.replace(position).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "forward ledger {ledger_key} has multiple confirmation events"
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "owed forward ledger {ledger_key} has no confirmation event"
        ))
    })
}

fn attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if matches!(
        &node.framework,
        Some(
            spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
                | spec::FrameworkNodeSpec::SideEffectVerify(_)
        )
    ) {
        return Ok(None);
    }
    if node.side_effect.is_some() {
        side_effect_attempt_plan(runtime_spec, node, view)
    } else {
        non_side_effect_attempt_plan(runtime_spec, node, view)
    }
}

fn non_side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "node {} attempt {} completed output cell {} without terminal cell authority",
                    node.node_id, attempt_id, output_cell_id
                )));
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
            store::AttemptStatus::Interrupted => {}
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew {
            attempt_no: next_attempt_no(&view.projections, &node.node_id)?,
        }))
    }
}

fn side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        let attempt_id = validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        SideEffectLifecycle::validate_terminal_evidence(&view.projections, node, &attempt_id)?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started side-effect attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                SideEffectLifecycle::open_attempt_disposition(&view.projections, node, attempt_id)?;
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal side-effect attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                if view.projections.cell_terminal(output_cell_id).is_none() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "side-effect node {} attempt {} completed without terminal output cell {}",
                        node.node_id, attempt_id, output_cell_id
                    )));
                }
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
            store::AttemptStatus::Interrupted => {}
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew {
            attempt_no: next_attempt_no(&view.projections, &node.node_id)?,
        }))
    }
}

pub(crate) fn node_inputs_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<bool> {
    let input_cells = runtime_spec.validate_input_binding(&node.input_bindings.root)?;
    for cell_id in input_cells {
        let cell = runtime_spec.cell(&cell_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} input cell {} is missing",
                node.node_id, cell_id
            ))
        })?;
        match &cell.producer {
            spec::CellProducer::Seed(_) => {
                if !view.seed_cells.contains_key(&cell_id) {
                    return Ok(false);
                }
            }
            spec::CellProducer::Node(_) => {
                if view.projections.cell_terminal(&cell_id).is_none() {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn next_attempt_no(projections: &store::ProjectionSnapshot, node_id: &NodeId) -> Result<u32> {
    let count = projections
        .attempts()
        .filter(|((attempt_node_id, _), _)| attempt_node_id == node_id)
        .count();
    u32::try_from(count + 1).map_err(|_| {
        RuntimeError::InvalidRunStream(format!("attempt count overflow for {node_id}"))
    })
}
