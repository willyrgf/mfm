use std::collections::BTreeSet;

use mfm_capabilities::CapabilityRole;
use mfm_ids::{AttemptId, RunId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::attempt::{
    async_error_is_stale_expected_next_seq, AttemptRunStatus, ResourceLaneBlockWitness,
};
use crate::commit::{AttemptInterruptionCommitInput, CommitPlanner};
use crate::error::async_store_error;
use crate::framework_lifecycle::FrameworkAttemptLifecycle;
use crate::frontier::node_inputs_ready;
use crate::history::RuntimeRunView;
use crate::side_effect_lifecycle::{
    open_attempt_disposition, validate_terminal_evidence, SideEffectOpenAttemptDisposition,
};
use crate::side_effects::validate_terminal_cell_has_completed_attempt;
use crate::transition::TransitionAttempt;
use crate::{CertifiedRuntimeSpec, Result, RuntimeError};

/// Recovery disposition for one projected open attempt.
pub(crate) enum OpenAttemptDisposition<'a> {
    /// Continue a started attempt that already has committed attempt-local progress.
    Continue {
        /// Certified node whose attempt remains open.
        node: &'a spec::NodeSpec,
        /// Open attempt id projected from the verified stream.
        attempt_id: AttemptId,
        /// Original attempt number.
        attempt_no: u32,
    },
    /// Retry terminalization for a framework-owned lifecycle attempt.
    RetryTerminalization {
        /// Certified framework node whose attempt remains open.
        node: &'a spec::NodeSpec,
        /// Open attempt id projected from the verified stream.
        attempt_id: AttemptId,
        /// Original attempt number.
        attempt_no: u32,
    },
    /// Interrupt a started attempt before scheduling a fresh attempt number.
    Interrupt {
        /// Certified node whose attempt remains open.
        node: &'a spec::NodeSpec,
        /// Open attempt id projected from the verified stream.
        attempt_id: AttemptId,
        /// Original attempt number.
        attempt_no: u32,
    },
    /// Delegate an open side-effect attempt to side-effect recovery.
    DelegateSideEffect {
        /// Certified side-effect node whose attempt remains open.
        node: &'a spec::NodeSpec,
        /// Open attempt id projected from the verified stream.
        attempt_id: AttemptId,
        /// Original attempt number.
        attempt_no: u32,
    },
    /// Recovery cannot safely advance the open attempt without operational intervention.
    OperationalBlock {
        /// Certified node whose attempt remains open.
        node: &'a spec::NodeSpec,
        /// Open attempt id projected from the verified stream.
        attempt_id: AttemptId,
        /// Original attempt number.
        attempt_no: u32,
    },
}

/// Recovery lifecycle for open-attempt disposition checks.
pub(crate) struct AttemptRecoveryLifecycle;

impl AttemptRecoveryLifecycle {
    /// Returns the next recoverable open attempt, if one exists.
    pub(crate) fn next_open_attempt_disposition<'a>(
        runtime_spec: &'a CertifiedRuntimeSpec,
        view: &RuntimeRunView,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<Option<OpenAttemptDisposition<'a>>> {
        for node in runtime_spec.executable_nodes() {
            if blocked_lanes
                .iter()
                .any(|witness| witness.blocks_node(&view.projections, node))
            {
                continue;
            }
            let Some((attempt_id, attempt_no)) = open_started_attempt_for_node(node, view)? else {
                continue;
            };
            return Self::classify_open_attempt(runtime_spec, view, node, attempt_id, attempt_no)
                .map(Some);
        }
        Ok(None)
    }

    /// Classifies a selected open attempt before its attempt lifecycle is resumed.
    fn open_attempt_disposition_for_attempt<'a>(
        runtime_spec: &'a CertifiedRuntimeSpec,
        view: &RuntimeRunView,
        node: &'a spec::NodeSpec,
        attempt_id: &AttemptId,
        attempt_no: u32,
    ) -> Result<OpenAttemptDisposition<'a>> {
        let Some((open_attempt_id, open_attempt_no)) = open_started_attempt_for_node(node, view)?
        else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "recovery selected node {} attempt {} but no started attempt is open",
                node.node_id, attempt_id
            )));
        };
        if open_attempt_id != *attempt_id || open_attempt_no != attempt_no {
            return Err(RuntimeError::InvalidRunStream(format!(
                "recovery selected node {} attempt {}#{} but verified open attempt is {}#{}",
                node.node_id, attempt_id, attempt_no, open_attempt_id, open_attempt_no
            )));
        }
        Self::classify_open_attempt(runtime_spec, view, node, open_attempt_id, open_attempt_no)
    }

    /// Dispatches recovery-owned async work for a selected open attempt.
    pub(crate) async fn dispatch_open_attempt_for_attempt<S: store::RunEventStore + ?Sized>(
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        attempt: &TransitionAttempt<'_>,
    ) -> Result<Option<AttemptRunStatus>> {
        let Some(attempt_id) = attempt.attempt_id.as_ref() else {
            return Ok(None);
        };
        match Self::open_attempt_disposition_for_attempt(
            runtime_spec,
            view,
            attempt.node,
            attempt_id,
            attempt.attempt_no,
        )? {
            OpenAttemptDisposition::Interrupt {
                node, attempt_id, ..
            } => Self::interrupt_attempt(store, runtime_spec, run_id, view, node, &attempt_id)
                .await
                .map(Some),
            OpenAttemptDisposition::OperationalBlock { .. } => {
                Ok(Some(AttemptRunStatus::OperationalBlock))
            }
            OpenAttemptDisposition::Continue { .. }
            | OpenAttemptDisposition::RetryTerminalization { .. }
            | OpenAttemptDisposition::DelegateSideEffect { .. } => Ok(None),
        }
    }

    /// Appends the recovery-owned interruption evidence for a resumable async store.
    async fn interrupt_attempt<S: store::RunEventStore + ?Sized>(
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
    ) -> Result<AttemptRunStatus> {
        let commit = CommitPlanner::prepare_attempt_interruption(AttemptInterruptionCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id,
            view,
        })?;
        let bundle = store::PreparedCommitBundle::without_artifacts(commit)?;
        match store.append_prepared_commit_bundle(bundle).await {
            Ok(_) => Ok(AttemptRunStatus::Advanced),
            Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                Ok(AttemptRunStatus::StaleView)
            }
            Err(error) => Err(async_store_error(error)),
        }
    }

    /// Validates that projected attempts and terminal cells form a recoverable frontier.
    pub(crate) fn validate_frontier(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        projections: &store::ProjectionSnapshot,
    ) -> Result<()> {
        for node in runtime_spec.executable_nodes() {
            if let Some(terminal) = projections.cell_terminal_for_run(run_id, &node.output_cell) {
                let attempt_id = validate_terminal_cell_has_completed_attempt(
                    runtime_spec,
                    projections,
                    node,
                    terminal,
                )?;
                if node.side_effect.is_some() {
                    validate_terminal_evidence(
                        runtime_spec,
                        run_id,
                        projections,
                        node,
                        &attempt_id,
                    )?;
                }
            }
            let mut started = None;
            for ((attempt_node_id, attempt_id), projection) in projections.attempts() {
                if attempt_node_id != &node.node_id {
                    continue;
                }
                match &projection.status {
                    store::AttemptStatus::Started { .. } => {
                        if projections
                            .cell_terminal_for_run(run_id, &node.output_cell)
                            .is_some()
                        {
                            return Err(RuntimeError::InvalidRunStream(format!(
                                "node {} has a started attempt after its output cell became terminal",
                                node.node_id
                            )));
                        }
                        if started.replace(attempt_id.clone()).is_some() {
                            return Err(RuntimeError::InvalidRunStream(format!(
                                "node {} has multiple started attempts during recovery",
                                node.node_id
                            )));
                        }
                    }
                    store::AttemptStatus::Completed { output_cell_id }
                        if projections
                            .cell_terminal_for_run(run_id, output_cell_id)
                            .is_none() =>
                    {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} attempt {} completed without terminal cell projection",
                            node.node_id, attempt_id
                        )));
                    }
                    store::AttemptStatus::Completed { .. }
                    | store::AttemptStatus::Failed { .. }
                    | store::AttemptStatus::Interrupted => {}
                }
            }
        }
        Ok(())
    }

    fn classify_open_attempt<'a>(
        runtime_spec: &'a CertifiedRuntimeSpec,
        view: &RuntimeRunView,
        node: &'a spec::NodeSpec,
        attempt_id: AttemptId,
        attempt_no: u32,
    ) -> Result<OpenAttemptDisposition<'a>> {
        if !node_inputs_ready(runtime_spec, node, view)? {
            return Err(RuntimeError::InvalidRunStream(format!(
                "node {} has a started attempt before certified inputs are terminal",
                node.node_id
            )));
        }
        if FrameworkAttemptLifecycle::owns_node(node) {
            return Ok(OpenAttemptDisposition::RetryTerminalization {
                node,
                attempt_id,
                attempt_no,
            });
        }
        if node.side_effect.is_some() {
            match open_attempt_disposition(
                runtime_spec,
                &view.run_admitted.run_id,
                &view.projections,
                node,
                &attempt_id,
            )? {
                SideEffectOpenAttemptDisposition::ContinueBeforeLedger => {
                    return Ok(OpenAttemptDisposition::Continue {
                        node,
                        attempt_id,
                        attempt_no,
                    });
                }
                SideEffectOpenAttemptDisposition::InterruptBeforeInvocationPrepared => {
                    return Ok(OpenAttemptDisposition::Interrupt {
                        node,
                        attempt_id,
                        attempt_no,
                    });
                }
                SideEffectOpenAttemptDisposition::DelegateRecovery => {
                    return Ok(OpenAttemptDisposition::DelegateSideEffect {
                        node,
                        attempt_id,
                        attempt_no,
                    });
                }
                SideEffectOpenAttemptDisposition::OperationalBlock => {
                    return Ok(OpenAttemptDisposition::OperationalBlock {
                        node,
                        attempt_id,
                        attempt_no,
                    });
                }
            }
        }
        if attempt_has_committed_progress(view, &attempt_id)
            || node_requires_same_attempt_recovery(node)
        {
            return Ok(OpenAttemptDisposition::Continue {
                node,
                attempt_id,
                attempt_no,
            });
        }
        Ok(OpenAttemptDisposition::Interrupt {
            node,
            attempt_id,
            attempt_no,
        })
    }
}

fn open_started_attempt_for_node(
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<(AttemptId, u32)>> {
    if view
        .projections
        .cell_terminal_for_run(&view.run_admitted.run_id, &node.output_cell)
        .is_some()
    {
        return Ok(None);
    }
    let mut started = None;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        let store::AttemptStatus::Started {
            attempt_no,
            state_kind,
            state_version,
        } = &projection.status
        else {
            continue;
        };
        if state_kind != &node.state_kind || state_version != &node.state_version {
            return Err(RuntimeError::InvalidRunStream(format!(
                "started attempt {} for node {} has state identity outside the certified spec",
                attempt_id, node.node_id
            )));
        }
        if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "node {} has multiple started attempts during recovery",
                node.node_id
            )));
        }
    }
    Ok(started)
}

fn attempt_has_committed_progress(view: &RuntimeRunView, attempt_id: &AttemptId) -> bool {
    view.artifact_refs
        .values()
        .any(|reference| reference.attempt_id.as_ref() == Some(attempt_id))
}

fn node_requires_same_attempt_recovery(node: &spec::NodeSpec) -> bool {
    node.capability_bindings
        .capabilities
        .iter()
        .any(|capability| capability.role == CapabilityRole::ExternalMutationAuthority)
}
