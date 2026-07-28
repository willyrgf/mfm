use std::collections::BTreeSet;

use mfm_ids::{AttemptId, NodeId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::attempt::ResourceLaneBlockWitness;
use crate::frontier::{
    scheduler_decision_with_blocked_nodes, AttemptPlan, RunnableNode, SchedulerDecision,
};
use crate::recovery::{AttemptRecoveryLifecycle, OpenAttemptDisposition};
use crate::spec_authority::CurrentSpecRead;
use crate::{Result, RuntimeError};

/// Explicit pure transition decision for one scheduler step.
pub(crate) enum TransitionDecision {
    /// Start a new ordinary certified node attempt.
    StartNode(TransitionAttempt),
    /// Continue or recover an open attempt for a certified node.
    ContinueAttempt(TransitionAttempt),
    /// Start a remediation attempt selected by saga obligation authority.
    StartRemediation(TransitionAttempt),
    /// Wait for certified operator/manual resolution evidence.
    AwaitManualResolution,
    /// Resolve a terminal saga outcome through the certified framework node.
    ResolveSagaTerminal(TransitionAttempt),
    /// No certified transition is currently runnable.
    Blocked,
}

/// Node attempt selected by the pure transition lifecycle.
pub(crate) struct TransitionAttempt {
    pub(crate) node_id: NodeId,
    pub(crate) attempt_id: Option<AttemptId>,
    pub(crate) attempt_no: u32,
}

/// Pure transition lifecycle over certified runtime authority and verified history.
pub(crate) struct TransitionLifecycle;

impl TransitionLifecycle {
    pub(crate) fn decide<S>(
        runtime_spec: &S,
        lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<TransitionDecision>
    where
        S: CurrentSpecRead + ?Sized,
    {
        if let Some(disposition) = AttemptRecoveryLifecycle::next_open_attempt_disposition(
            runtime_spec,
            lifecycle,
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
                }
                | OpenAttemptDisposition::Interrupt {
                    node,
                    attempt_id,
                    attempt_no,
                }
                | OpenAttemptDisposition::DelegateSideEffect {
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
                        node_id: node.node_id.clone(),
                        attempt_id: Some(attempt_id),
                        attempt_no,
                    }));
                }
            }
        }
        let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
        let run_mode = lifecycle
            .with_saga(&runtime_spec.spec().saga, &terminal_policies, |saga| {
                saga.run_mode()
            })
            .map_err(RuntimeError::from)?;
        let blocked_nodes = blocked_node_ids(runtime_spec, lifecycle, blocked_lanes)?;
        match scheduler_decision_with_blocked_nodes(runtime_spec, lifecycle, &blocked_nodes)? {
            SchedulerDecision::Run(runnable) => classify_runnable(runtime_spec, runnable),
            SchedulerDecision::Blocked if run_mode == store::RunMode::ManualBlocked => {
                Ok(TransitionDecision::AwaitManualResolution)
            }
            SchedulerDecision::Blocked | SchedulerDecision::Completed => {
                Ok(TransitionDecision::Blocked)
            }
        }
    }

    pub(crate) fn public_output_projected<S>(
        runtime_spec: &S,
        lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<bool>
    where
        S: CurrentSpecRead + ?Sized,
    {
        let blocked_nodes = blocked_node_ids(runtime_spec, lifecycle, blocked_lanes)?;
        Ok(matches!(
            scheduler_decision_with_blocked_nodes(runtime_spec, lifecycle, &blocked_nodes)?,
            SchedulerDecision::Completed
        ))
    }
}

fn blocked_node_ids<S>(
    runtime_spec: &S,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
) -> Result<BTreeSet<NodeId>>
where
    S: CurrentSpecRead + ?Sized,
{
    let mut blocked = BTreeSet::new();
    for node in runtime_spec.executable_nodes()? {
        if blocked_lanes
            .iter()
            .any(|witness| witness.blocks_node(lifecycle, node))
        {
            blocked.insert(node.node_id.clone());
        }
    }
    Ok(blocked)
}

fn classify_runnable<'a, S>(
    runtime_spec: &'a S,
    runnable: RunnableNode<'a>,
) -> Result<TransitionDecision>
where
    S: CurrentSpecRead + ?Sized,
{
    let attempt = match runnable.attempt {
        AttemptPlan::StartNew { attempt_no } => TransitionAttempt {
            node_id: runnable.node.node_id.clone(),
            attempt_id: None,
            attempt_no,
        },
        AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        } => TransitionAttempt {
            node_id: runnable.node.node_id.clone(),
            attempt_id: Some(attempt_id),
            attempt_no,
        },
    };
    if attempt.attempt_id.is_some() {
        return Ok(TransitionDecision::ContinueAttempt(attempt));
    }
    if matches!(
        &runnable.node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    ) {
        return Ok(TransitionDecision::ResolveSagaTerminal(attempt));
    }
    if runtime_spec
        .forward_node_for_remediation(&attempt.node_id)
        .is_some()
    {
        return Ok(TransitionDecision::StartRemediation(attempt));
    }
    Ok(TransitionDecision::StartNode(attempt))
}
