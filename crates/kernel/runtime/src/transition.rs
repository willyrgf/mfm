use std::collections::BTreeSet;

use mfm_ids::{AttemptId, NodeId, RunId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::attempt::ResourceLaneBlockWitness;
use crate::frontier::{
    scheduler_decision_with_blocked_nodes, AttemptPlan, RunnableNode, SchedulerDecision,
};
use crate::history::RuntimeRunView;
use crate::recovery::{AttemptRecoveryLifecycle, OpenAttemptDisposition};
use crate::{CertifiedRuntimeSpec, Result};

/// Explicit pure transition decision for one scheduler step.
pub(crate) enum TransitionDecision<'a> {
    /// Start a new ordinary certified node attempt.
    StartNode(TransitionAttempt<'a>),
    /// Continue or recover an open attempt for a certified node.
    ContinueAttempt(TransitionAttempt<'a>),
    /// Start a remediation attempt selected by saga obligation authority.
    StartRemediation(TransitionAttempt<'a>),
    /// Wait for certified operator/manual resolution evidence.
    AwaitManualResolution,
    /// Resolve a terminal saga outcome through the certified framework node.
    ResolveSagaTerminal(TransitionAttempt<'a>),
    /// No certified transition is currently runnable.
    Blocked,
}

/// Node attempt selected by the pure transition lifecycle.
pub(crate) struct TransitionAttempt<'a> {
    pub(crate) node: &'a spec::NodeSpec,
    pub(crate) attempt_id: Option<AttemptId>,
    pub(crate) attempt_no: u32,
}

/// Pure transition lifecycle over certified runtime authority and verified history.
pub(crate) struct TransitionLifecycle;

impl TransitionLifecycle {
    pub(crate) fn decide<'a>(
        runtime_spec: &'a CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<TransitionDecision<'a>> {
        if let Some(disposition) = AttemptRecoveryLifecycle::next_open_attempt_disposition(
            runtime_spec,
            view,
            blocked_lanes,
        )? {
            match disposition {
                OpenAttemptDisposition::Continue {
                    node,
                    attempt_id,
                    attempt_no,
                }
                | OpenAttemptDisposition::RetryTerminalization {
                    node,
                    attempt_id,
                    attempt_no,
                } => {
                    return Ok(TransitionDecision::ContinueAttempt(TransitionAttempt {
                        node,
                        attempt_id: Some(attempt_id),
                        attempt_no,
                    }));
                }
                OpenAttemptDisposition::Interrupt {
                    node,
                    attempt_id,
                    attempt_no,
                } => {
                    return Ok(TransitionDecision::ContinueAttempt(TransitionAttempt {
                        node,
                        attempt_id: Some(attempt_id),
                        attempt_no,
                    }));
                }
                OpenAttemptDisposition::DelegateSideEffect {
                    node,
                    attempt_id,
                    attempt_no,
                }
                | OpenAttemptDisposition::OperationalBlock {
                    node,
                    attempt_id,
                    attempt_no,
                    ..
                } => {
                    return Ok(TransitionDecision::ContinueAttempt(TransitionAttempt {
                        node,
                        attempt_id: Some(attempt_id),
                        attempt_no,
                    }));
                }
            }
        }
        let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
        let saga = view.projections.derive_saga_projection(
            run_id,
            &runtime_spec.spec().saga,
            &terminal_policies,
        )?;
        let blocked_nodes = blocked_node_ids(runtime_spec, view, blocked_lanes);
        match scheduler_decision_with_blocked_nodes(runtime_spec, run_id, view, &blocked_nodes)? {
            SchedulerDecision::Run(runnable) => classify_runnable(runtime_spec, runnable),
            SchedulerDecision::Blocked if saga.run_mode == store::RunMode::ManualBlocked => {
                Ok(TransitionDecision::AwaitManualResolution)
            }
            SchedulerDecision::Blocked | SchedulerDecision::Completed => {
                Ok(TransitionDecision::Blocked)
            }
        }
    }

    pub(crate) fn public_output_projected(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<bool> {
        let blocked_nodes = blocked_node_ids(runtime_spec, view, blocked_lanes);
        Ok(matches!(
            scheduler_decision_with_blocked_nodes(runtime_spec, run_id, view, &blocked_nodes)?,
            SchedulerDecision::Completed
        ))
    }
}

fn blocked_node_ids(
    runtime_spec: &CertifiedRuntimeSpec,
    view: &RuntimeRunView,
    blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
) -> BTreeSet<NodeId> {
    let mut blocked = BTreeSet::new();
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if blocked_lanes
            .iter()
            .any(|witness| witness.blocks_node(&view.projections, node))
        {
            blocked.insert(node_id.clone());
        }
    }
    for (_, node) in runtime_spec.remediations() {
        if blocked_lanes
            .iter()
            .any(|witness| witness.blocks_node(&view.projections, node))
        {
            blocked.insert(node.node_id.clone());
        }
    }
    blocked
}

fn classify_runnable<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    runnable: RunnableNode<'a>,
) -> Result<TransitionDecision<'a>> {
    let attempt = transition_attempt(runnable);
    if attempt.attempt_id.is_some() {
        return Ok(TransitionDecision::ContinueAttempt(attempt));
    }
    if is_resolve_saga_terminal(attempt.node) {
        return Ok(TransitionDecision::ResolveSagaTerminal(attempt));
    }
    if is_remediation_node(runtime_spec, attempt.node) {
        return Ok(TransitionDecision::StartRemediation(attempt));
    }
    Ok(TransitionDecision::StartNode(attempt))
}

fn transition_attempt(runnable: RunnableNode<'_>) -> TransitionAttempt<'_> {
    match runnable.attempt {
        AttemptPlan::StartNew { attempt_no } => TransitionAttempt {
            node: runnable.node,
            attempt_id: None,
            attempt_no,
        },
        AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        } => TransitionAttempt {
            node: runnable.node,
            attempt_id: Some(attempt_id),
            attempt_no,
        },
    }
}

fn is_resolve_saga_terminal(node: &spec::NodeSpec) -> bool {
    matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    )
}

fn is_remediation_node(runtime_spec: &CertifiedRuntimeSpec, node: &spec::NodeSpec) -> bool {
    runtime_spec
        .remediations()
        .any(|(_, remediation)| remediation.node_id == node.node_id)
}
