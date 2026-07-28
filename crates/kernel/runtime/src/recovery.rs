use std::collections::BTreeSet;
use std::ops::ControlFlow;

use mfm_capabilities::CapabilityRole;
use mfm_ids::{AttemptId, RunId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::attempt::{async_error_is_stale_expected_next_seq, ResourceLaneBlockWitness};
use crate::commit::{AttemptInterruptionCommitInput, CommitPlanner};
use crate::error::async_store_error;
use crate::framework_lifecycle::FrameworkAttemptLifecycle;
use crate::frontier::node_inputs_ready;
use crate::side_effect_lifecycle::{open_attempt_disposition, SideEffectOpenAttemptDisposition};
use crate::spec_authority::CurrentSpecRead;
use crate::transition::TransitionAttempt;
use crate::{Result, RuntimeError};

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

pub(crate) enum RecoveryDispatch {
    Continue,
    OperationalBlock,
    Commit(Box<store::CommitOutcome>),
    StaleView,
}

impl AttemptRecoveryLifecycle {
    /// Returns the next recoverable open attempt, if one exists.
    pub(crate) fn next_open_attempt_disposition<'a, S>(
        runtime_spec: &'a S,
        lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<Option<OpenAttemptDisposition<'a>>>
    where
        S: CurrentSpecRead + ?Sized,
    {
        for node in runtime_spec.executable_nodes()? {
            if blocked_lanes
                .iter()
                .any(|witness| witness.blocks_node(lifecycle, node))
            {
                continue;
            }
            let Some((attempt_id, attempt_no)) = open_started_attempt_for_node(node, lifecycle)?
            else {
                continue;
            };
            return Self::classify_open_attempt(
                runtime_spec,
                lifecycle,
                node,
                attempt_id,
                attempt_no,
            )
            .map(Some);
        }
        Ok(None)
    }

    fn open_attempt_disposition_for_attempt<'a, S>(
        runtime_spec: &'a S,
        lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
        node: &'a spec::NodeSpec,
        attempt_id: &AttemptId,
        attempt_no: u32,
    ) -> Result<OpenAttemptDisposition<'a>>
    where
        S: CurrentSpecRead + ?Sized,
    {
        let Some((open_attempt_id, open_attempt_no)) =
            open_started_attempt_for_node(node, lifecycle)?
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
        Self::classify_open_attempt(
            runtime_spec,
            lifecycle,
            node,
            open_attempt_id,
            open_attempt_no,
        )
    }

    /// Dispatches recovery-owned async work for a selected open attempt.
    pub(crate) async fn dispatch_open_attempt_for_attempt<Store>(
        store: &Store,
        runtime_spec: &crate::spec_authority::CurrentRuntimeSpecRef<'_>,
        run_id: &RunId,
        lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
        attempt: &TransitionAttempt,
    ) -> Result<RecoveryDispatch>
    where
        Store: store::RunJournalStore + ?Sized,
    {
        let Some(attempt_id) = attempt.attempt_id.as_ref() else {
            return Ok(RecoveryDispatch::Continue);
        };
        let node = runtime_spec.node(&attempt.node_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "recovery selected missing certified node {}",
                attempt.node_id
            ))
        })?;
        match Self::open_attempt_disposition_for_attempt(
            runtime_spec,
            lifecycle,
            node,
            attempt_id,
            attempt.attempt_no,
        )? {
            OpenAttemptDisposition::Interrupt {
                node, attempt_id, ..
            } => {
                Self::interrupt_attempt(store, runtime_spec, run_id, lifecycle, node, &attempt_id)
                    .await
            }
            OpenAttemptDisposition::OperationalBlock { .. } => {
                Ok(RecoveryDispatch::OperationalBlock)
            }
            OpenAttemptDisposition::Continue { .. }
            | OpenAttemptDisposition::RetryTerminalization { .. }
            | OpenAttemptDisposition::DelegateSideEffect { .. } => Ok(RecoveryDispatch::Continue),
        }
    }

    async fn interrupt_attempt<Store>(
        store: &Store,
        runtime_spec: &crate::spec_authority::CurrentRuntimeSpecRef<'_>,
        run_id: &RunId,
        lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
    ) -> Result<RecoveryDispatch>
    where
        Store: store::RunJournalStore + ?Sized,
    {
        let commit = CommitPlanner::prepare_attempt_interruption(AttemptInterruptionCommitInput {
            runtime_spec: runtime_spec.current_spec_ref(),
            run_id,
            node,
            attempt_id,
            lifecycle,
        })?;
        let bundle = store::PreparedCommitBundle::without_artifacts(commit)?;
        match store.append_prepared_commit_bundle(bundle).await {
            Ok(
                outcome @ (store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_)),
            ) => Ok(RecoveryDispatch::Commit(Box::new(outcome))),
            Ok(store::CommitOutcome::AdmissionBlocked(block)) => {
                Err(RuntimeError::InvalidRunStream(format!(
                    "attempt interruption was blocked by resource lane {}:{}",
                    block.resource_lane_key.namespace, block.resource_lane_key.key
                )))
            }
            Ok(store::CommitOutcome::ExecutionClaimBusy(_)) => Err(RuntimeError::InvalidRunStream(
                "attempt interruption requested an execution claim outside admission".to_owned(),
            )),
            Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                Ok(RecoveryDispatch::StaleView)
            }
            Err(error) => Err(async_store_error(error)),
        }
    }

    fn classify_open_attempt<'a, S>(
        runtime_spec: &'a S,
        lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
        node: &'a spec::NodeSpec,
        attempt_id: AttemptId,
        attempt_no: u32,
    ) -> Result<OpenAttemptDisposition<'a>>
    where
        S: CurrentSpecRead + ?Sized,
    {
        if !node_inputs_ready(runtime_spec, node, lifecycle)? {
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
            match open_attempt_disposition(runtime_spec, lifecycle, node, &attempt_id)? {
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
        if attempt_has_committed_progress(lifecycle, &attempt_id)
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
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<Option<(AttemptId, u32)>> {
    if lifecycle.cell(&node.output_cell).is_some() {
        return Ok(None);
    }
    let mut started = None;
    let mut failure = None;
    let _ = lifecycle.visit_attempts(|attempt| {
        if attempt.node_id() != &node.node_id {
            return ControlFlow::Continue(());
        }
        let store::current_lifecycle::CurrentAttemptStatusRef::Started {
            attempt_no,
            state_kind,
            state_version,
        } = attempt.status()
        else {
            return ControlFlow::Continue(());
        };
        if state_kind != &node.state_kind || state_version != &node.state_version {
            failure = Some(RuntimeError::InvalidRunStream(format!(
                "started attempt {} for node {} has state identity outside the certified spec",
                attempt.attempt_id(),
                node.node_id
            )));
            return ControlFlow::Break(());
        }
        if started
            .replace((attempt.attempt_id().clone(), attempt_no))
            .is_some()
        {
            failure = Some(RuntimeError::InvalidRunStream(format!(
                "node {} has multiple started attempts during recovery",
                node.node_id
            )));
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(started)
}

fn attempt_has_committed_progress(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    attempt_id: &AttemptId,
) -> bool {
    matches!(
        lifecycle.visit_records(|record| {
            let store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(reference) =
                record.kind()
            else {
                return ControlFlow::Continue(());
            };
            if reference.attempt_id.as_ref() == Some(attempt_id) {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }),
        ControlFlow::Break(())
    )
}

fn node_requires_same_attempt_recovery(node: &spec::NodeSpec) -> bool {
    node.capability_bindings
        .capabilities
        .iter()
        .any(|capability| capability.role == CapabilityRole::ExternalMutationAuthority)
}
