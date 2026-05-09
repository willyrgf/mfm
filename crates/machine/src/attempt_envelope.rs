//! Internal helpers for validating state-attempt envelopes in the event stream.
//!
//! Not part of the stable API contract (Appendix C.1).

#![allow(dead_code)]

use crate::events::{Event, EventEnvelope, KernelEvent};
use crate::ids::{ArtifactId, RunId, StateId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrphanAttempt {
    pub(crate) run_id: RunId,
    pub(crate) state_id: StateId,
    pub(crate) attempt: u32,
    pub(crate) base_snapshot_id: ArtifactId,
    pub(crate) entered_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KernelAnalysis {
    pub(crate) last_checkpoint_snapshot: Option<ArtifactId>,
    pub(crate) orphan_attempt: Option<OrphanAttempt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttemptEnvelopeError {
    RunIdMismatch,
    NonMonotonicSeq,
    NestedStateEntered,
    TerminalWithoutEntered,
    TerminalStateIdMismatch,
    UnexpectedKernelInsideAttempt,
}

pub(crate) fn analyze_kernel_events(
    stream: &[EventEnvelope],
) -> Result<KernelAnalysis, AttemptEnvelopeError> {
    let mut last_checkpoint_snapshot = None;

    let mut last_seq = None;
    let mut run_id = None;
    let mut open_attempt: Option<OrphanAttempt> = None;

    for e in stream {
        match run_id {
            Some(rid) if rid != e.run_id => return Err(AttemptEnvelopeError::RunIdMismatch),
            None => run_id = Some(e.run_id),
            _ => {}
        }

        match last_seq {
            Some(prev) if e.seq <= prev => return Err(AttemptEnvelopeError::NonMonotonicSeq),
            _ => {}
        }
        last_seq = Some(e.seq);

        match &e.event {
            Event::Kernel(ke) => match ke {
                KernelEvent::StateEntered {
                    state_id,
                    attempt,
                    base_snapshot_id,
                } => {
                    if open_attempt.is_some() {
                        return Err(AttemptEnvelopeError::NestedStateEntered);
                    }
                    open_attempt = Some(OrphanAttempt {
                        run_id: e.run_id,
                        state_id: state_id.clone(),
                        attempt: *attempt,
                        base_snapshot_id: base_snapshot_id.clone(),
                        entered_seq: e.seq,
                    });
                }
                KernelEvent::StateCompleted {
                    state_id,
                    context_snapshot_id,
                } => {
                    let Some(open) = open_attempt.take() else {
                        return Err(AttemptEnvelopeError::TerminalWithoutEntered);
                    };
                    if open.state_id != *state_id {
                        return Err(AttemptEnvelopeError::TerminalStateIdMismatch);
                    }
                    last_checkpoint_snapshot = Some(context_snapshot_id.clone());
                }
                KernelEvent::StateFailed { state_id, .. } => {
                    let Some(open) = open_attempt.take() else {
                        return Err(AttemptEnvelopeError::TerminalWithoutEntered);
                    };
                    if open.state_id != *state_id {
                        return Err(AttemptEnvelopeError::TerminalStateIdMismatch);
                    }
                }
                KernelEvent::RunStarted { .. } | KernelEvent::RunCompleted { .. } => {
                    if open_attempt.is_some() {
                        return Err(AttemptEnvelopeError::UnexpectedKernelInsideAttempt);
                    }
                }
            },
            Event::Domain(_) => {}
        }
    }

    Ok(KernelAnalysis {
        last_checkpoint_snapshot,
        orphan_attempt: open_attempt,
    })
}

pub(crate) fn last_checkpoint_snapshot_id(
    stream: &[EventEnvelope],
) -> Result<Option<ArtifactId>, AttemptEnvelopeError> {
    analyze_kernel_events(stream).map(|a| a.last_checkpoint_snapshot)
}

pub(crate) fn orphan_attempt(
    stream: &[EventEnvelope],
) -> Result<Option<OrphanAttempt>, AttemptEnvelopeError> {
    analyze_kernel_events(stream).map(|a| a.orphan_attempt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{DomainEvent, RunStatus};
    use crate::ids::{ArtifactId, OpId};

    fn env(run_id: RunId, seq: u64, event: Event) -> EventEnvelope {
        EventEnvelope {
            run_id,
            seq,
            ts_millis: None,
            event,
        }
    }

    #[test]
    fn validates_and_ignores_domain_events_for_checkpoint() {
        let run_id = RunId(uuid::Uuid::new_v4());

        let started = env(
            run_id,
            1,
            Event::Kernel(KernelEvent::RunStarted {
                op_id: OpId::must_new("op".to_string()),
                manifest_id: ArtifactId::must_new("0".repeat(64)),
                initial_snapshot_id: ArtifactId::must_new("1".repeat(64)),
            }),
        );

        let entered = env(
            run_id,
            2,
            Event::Kernel(KernelEvent::StateEntered {
                state_id: StateId::must_new("machine.main.s1".to_string()),
                attempt: 0,
                base_snapshot_id: ArtifactId::must_new("2".repeat(64)),
            }),
        );

        let dom = env(
            run_id,
            3,
            Event::Domain(DomainEvent {
                name: "custom".to_string(),
                payload: serde_json::json!({"k":"v"}),
                payload_ref: None,
            }),
        );

        let completed = env(
            run_id,
            4,
            Event::Kernel(KernelEvent::StateCompleted {
                state_id: StateId::must_new("machine.main.s1".to_string()),
                context_snapshot_id: ArtifactId::must_new("3".repeat(64)),
            }),
        );

        let finished = env(
            run_id,
            5,
            Event::Kernel(KernelEvent::RunCompleted {
                status: RunStatus::Completed,
                final_snapshot_id: None,
            }),
        );

        let with_domain = vec![
            started.clone(),
            entered.clone(),
            dom.clone(),
            completed.clone(),
            finished.clone(),
        ];

        let without_domain = vec![
            started,
            entered,
            env(
                run_id,
                3,
                Event::Kernel(KernelEvent::StateCompleted {
                    state_id: StateId::must_new("machine.main.s1".to_string()),
                    context_snapshot_id: ArtifactId::must_new("3".repeat(64)),
                }),
            ),
            env(
                run_id,
                4,
                Event::Kernel(KernelEvent::RunCompleted {
                    status: RunStatus::Completed,
                    final_snapshot_id: None,
                }),
            ),
        ];

        assert_eq!(
            last_checkpoint_snapshot_id(&with_domain).expect("analysis"),
            Some(ArtifactId::must_new("3".repeat(64)))
        );
        assert_eq!(
            last_checkpoint_snapshot_id(&without_domain).expect("analysis"),
            Some(ArtifactId::must_new("3".repeat(64)))
        );
    }

    #[test]
    fn detects_orphan_attempt_at_end_of_stream() {
        let run_id = RunId(uuid::Uuid::new_v4());

        let stream = vec![env(
            run_id,
            1,
            Event::Kernel(KernelEvent::StateEntered {
                state_id: StateId::must_new("machine.main.s1".to_string()),
                attempt: 7,
                base_snapshot_id: ArtifactId::must_new("2".repeat(64)),
            }),
        )];

        let orphan = orphan_attempt(&stream).expect("analysis").expect("orphan");
        assert_eq!(orphan.run_id, run_id);
        assert_eq!(orphan.attempt, 7);
        assert_eq!(orphan.entered_seq, 1);
    }

    #[test]
    fn recovered_orphan_attempt_sequence_is_valid() {
        let run_id = RunId(uuid::Uuid::new_v4());
        let state_id = StateId::must_new("machine.main.s1".to_string());

        let stream = vec![
            env(
                run_id,
                1,
                Event::Kernel(KernelEvent::StateEntered {
                    state_id: state_id.clone(),
                    attempt: 0,
                    base_snapshot_id: ArtifactId::must_new("2".repeat(64)),
                }),
            ),
            env(
                run_id,
                2,
                Event::Kernel(KernelEvent::StateFailed {
                    state_id: state_id.clone(),
                    error: crate::errors::StateError {
                        state_id: Some(state_id.clone()),
                        info: crate::errors::ErrorInfo {
                            code: crate::ids::ErrorCode("orphan_attempt_recovered".to_string()),
                            category: crate::errors::ErrorCategory::Unknown,
                            retryable: true,
                            message: "orphaned state attempt recovered during resume".to_string(),
                            details: None,
                        },
                    },
                    failure_snapshot_id: None,
                }),
            ),
            env(
                run_id,
                3,
                Event::Kernel(KernelEvent::StateEntered {
                    state_id: state_id.clone(),
                    attempt: 1,
                    base_snapshot_id: ArtifactId::must_new("2".repeat(64)),
                }),
            ),
            env(
                run_id,
                4,
                Event::Kernel(KernelEvent::StateCompleted {
                    state_id,
                    context_snapshot_id: ArtifactId::must_new("3".repeat(64)),
                }),
            ),
        ];

        let analysis = analyze_kernel_events(&stream).expect("recovered sequence");
        assert_eq!(analysis.orphan_attempt, None);
        assert_eq!(
            analysis.last_checkpoint_snapshot,
            Some(ArtifactId::must_new("3".repeat(64)))
        );
    }

    #[test]
    fn rejects_terminal_without_entered() {
        let run_id = RunId(uuid::Uuid::new_v4());

        let stream = vec![env(
            run_id,
            1,
            Event::Kernel(KernelEvent::StateCompleted {
                state_id: StateId::must_new("machine.main.s1".to_string()),
                context_snapshot_id: ArtifactId::must_new("2".repeat(64)),
            }),
        )];

        let err = analyze_kernel_events(&stream).expect_err("expected error");
        assert_eq!(err, AttemptEnvelopeError::TerminalWithoutEntered);
    }
}
