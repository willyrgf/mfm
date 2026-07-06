use std::collections::BTreeSet;

use mfm_events::v1 as events;
use mfm_ids::{AttemptId, NodeId, RunId, SideEffectPairId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::framework::public_output_is_produced;
use crate::history::RuntimeRunView;
use crate::side_effect_lifecycle::{
    open_attempt_disposition, side_effect_projection_for_attempt, validate_terminal_evidence,
};
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
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    let saga = view.projections.derive_saga_projection(
        run_id,
        &runtime_spec.spec().saga,
        &terminal_policies,
    )?;
    if saga.engagement.is_some() {
        return saga_scheduler_decision(runtime_spec, view, &saga, blocked_nodes);
    }
    if public_output_is_produced(runtime_spec, run_id, &view.projections) {
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
        if let Some(runnable) = planned_runnable_node_if_ready(runtime_spec, node, view)? {
            return Ok(Some(runnable));
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
        if node.side_effect.is_some() {
            let Some(runnable) =
                continuing_side_effect_node_if(runtime_spec, node, view, |state| {
                    state.is_forward_completion_candidate()
                })?
            else {
                continue;
            };
            return Ok(Some(runnable));
        }
        if let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework {
            let Some(state) = view
                .projections
                .side_effect_state_for_pair(&view.run_admitted.run_id, &verify.pair_id)
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?
            else {
                continue;
            };
            if !state.is_forward_completion_candidate() {
                continue;
            }
            if let Some(runnable) = planned_runnable_node_if_ready(runtime_spec, node, view)? {
                return Ok(Some(runnable));
            }
        }
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
            .side_effect_for_pair(&view.run_admitted.run_id, &obligation.forward_pair_id)
            .ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "owed obligation {} has no forward pair projection",
                    obligation.forward_pair_id
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
        if let Some(remediation) = &obligation.remediation {
            if remediation.unresolved.is_some() {
                return Ok(None);
            }
            let output_node = verify_node.unwrap_or(remediation_node);
            if remediation.closed
                && view
                    .projections
                    .cell_terminal_for_run(&view.run_admitted.run_id, &output_node.output_cell)
                    .is_some()
            {
                continue;
            }
            if let Some(verify_node) = verify_node {
                if let Some(attempt) = attempt_plan(runtime_spec, remediation_node, view)? {
                    if node_inputs_ready(runtime_spec, remediation_node, view)? {
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
                let state = view
                    .projections
                    .side_effect_state_for_pair(&view.run_admitted.run_id, &remediation.pair_id)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "remediation pair {} has no side-effect projection",
                            remediation.pair_id
                        ))
                    })?;
                if remediation_verify_completion_candidate(&state) {
                    return planned_runnable_node_if_ready(runtime_spec, verify_node, view);
                }
                return Ok(None);
            }
        }
        return planned_runnable_node_if_ready(runtime_spec, remediation_node, view);
    }
    Ok(None)
}

fn certified_side_effect_verify_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    submit_node_id: &NodeId,
) -> Result<&'a spec::NodeSpec> {
    let mut found = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
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

fn remediation_verify_completion_candidate(state: &store::SideEffectLedgerState<'_>) -> bool {
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
        let Some(projection) = side_effect_projection_for_attempt(
            runtime_spec,
            &view.run_admitted.run_id,
            &view.projections,
            node,
            attempt_id,
        )?
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
        let position = forward_confirmation_position(view, &obligation.forward_pair_id)?;
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
    pair_id: &SideEffectPairId,
) -> Result<(u64, u32)> {
    let mut found = None;
    for event in &view.stream {
        let events::KernelEventPayload::SideEffectConfirmationObserved(payload) = event.payload()
        else {
            continue;
        };
        if &payload.pair_id != pair_id {
            continue;
        }
        let position = (event.seq().as_u64(), event.ordinal().as_u32());
        if found.replace(position).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "forward pair {pair_id} has multiple confirmation events"
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "owed forward pair {pair_id} has no confirmation event"
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
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    ) {
        return Ok(None);
    }
    if node.side_effect.is_some() {
        side_effect_attempt_plan(runtime_spec, node, view)
    } else {
        non_side_effect_attempt_plan(runtime_spec, node, view)
    }
}

fn planned_runnable_node_if_ready<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    node: &'a spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<RunnableNode<'a>>> {
    let Some(attempt) = attempt_plan(runtime_spec, node, view)? else {
        return Ok(None);
    };
    if node_inputs_ready(runtime_spec, node, view)? {
        return Ok(Some(RunnableNode { node, attempt }));
    }
    Ok(None)
}

fn non_side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view
        .projections
        .cell_terminal_for_run(&view.run_admitted.run_id, &node.output_cell)
    {
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

    attempt_plan_from_started_or_new(started, &view.projections, &node.node_id)
}

fn side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view
        .projections
        .cell_terminal_for_run(&view.run_admitted.run_id, &node.output_cell)
    {
        let attempt_id = validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        validate_terminal_evidence(
            runtime_spec,
            &view.run_admitted.run_id,
            &view.projections,
            node,
            &attempt_id,
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
                        "started side-effect attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                open_attempt_disposition(
                    runtime_spec,
                    &view.run_admitted.run_id,
                    &view.projections,
                    node,
                    attempt_id,
                )?;
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal side-effect attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                if view
                    .projections
                    .cell_terminal_for_run(&view.run_admitted.run_id, output_cell_id)
                    .is_none()
                {
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

    attempt_plan_from_started_or_new(started, &view.projections, &node.node_id)
}

fn attempt_plan_from_started_or_new(
    started: Option<(AttemptId, u32)>,
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
) -> Result<Option<AttemptPlan>> {
    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew {
            attempt_no: next_attempt_no(projections, node_id)?,
        }))
    }
}

pub(crate) fn node_inputs_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<bool> {
    runtime_spec.validate_input_binding(&node.input_bindings.root)?;
    input_binding_ready(runtime_spec, node, &node.input_bindings.root, view)
}

fn input_binding_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    input: &spec::InputBindingNodeSpec,
    view: &RuntimeRunView,
) -> Result<bool> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(true),
        spec::InputBindingNodeSpec::Cell(cell) => input_cell_ready(runtime_spec, node, cell, view),
        spec::InputBindingNodeSpec::Tuple(elements) => {
            input_bindings_ready(runtime_spec, node, elements, view)
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            fields.iter().try_fold(true, |ready, field| {
                Ok(ready && input_binding_ready(runtime_spec, node, &field.node, view)?)
            })
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            input_bindings_ready(runtime_spec, node, elements, view)
        }
    }
}

fn input_bindings_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    elements: &[spec::InputBindingNodeSpec],
    view: &RuntimeRunView,
) -> Result<bool> {
    elements.iter().try_fold(true, |ready, element| {
        Ok(ready && input_binding_ready(runtime_spec, node, element, view)?)
    })
}

fn input_cell_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    input: &spec::InputBindingCellSpec,
    view: &RuntimeRunView,
) -> Result<bool> {
    let cell = runtime_spec.cell(&input.cell_id).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} input cell {} is missing",
            node.node_id, input.cell_id
        ))
    })?;
    match &cell.producer {
        spec::CellProducer::Seed(_) => Ok(view.seed_cells.contains_key(&input.cell_id)),
        spec::CellProducer::Node(_) => match view
            .projections
            .cell_terminal_for_run(&view.run_admitted.run_id, &input.cell_id)
        {
            Some(store::CellTerminalProjection::Produced { .. }) => Ok(true),
            Some(store::CellTerminalProjection::Skipped { .. }) => {
                Ok(input.required_terminal == spec::RequiredTerminal::MaybeSkipped)
            }
            None => Ok(false),
        },
    }
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
