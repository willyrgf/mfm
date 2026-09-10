use mfm_ids::StatePosition;
use serde::{Deserialize, Serialize};

/// Closed structural reason automatic recovery stopped. Runtime owns its interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopCode {
    /// Handler selected Stop.
    Requested,
    /// Per-State retry budget exhausted.
    StateRetryExhausted,
    /// Per-State restart budget exhausted.
    StateRestartExhausted,
    /// Global recovery budget exhausted.
    RunExhausted,
    /// Identical Pure input cannot be retried.
    PureRetry,
    /// Requested checkpoint was unavailable.
    CheckpointUnavailable,
    /// A retained Effect prevented restart.
    EffectBarrier,
    /// A settled Effect cannot be repeated.
    EffectSettled,
}

/// Recorded recovery for an operational Read error, with no fabricated domain result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryDecision {
    /// Retry the same State with a fresh visit.
    Retry,
    /// Restore the permitted checkpoint's active input with a fresh visit.
    Restart {
        /// Selected declaration boundary.
        checkpoint: StatePosition,
    },
    /// Terminal Read execution failure.
    Stop {
        /// Reviewed stop code.
        reason: StopCode,
    },
}

/// Audited pending-Effect decision; command authority is retained and restart is impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PendingDecision {
    /// Spend recovery allowance and yield with the unchanged command.
    Retry,
    /// End this invocation while retaining pending command authority.
    Stop {
        /// Reviewed stop reason.
        reason: StopCode,
    },
}

/// Domain recovery decision; only a terminal stop retains the mapped root failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DomainDecision<T> {
    /// Retry the same State.
    Retry,
    /// Restore an eligible checkpoint.
    Restart {
        /// Selected declaration boundary.
        checkpoint: StatePosition,
    },
    /// Terminal domain failure.
    Stop {
        /// Reviewed stop reason.
        reason: StopCode,
        /// Mapped root failure object.
        root: T,
    },
}

/// One domain result, including the unmodified original cause of failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DomainConclusion<T> {
    /// Successful State output.
    Success {
        /// Complete output object.
        output: T,
    },
    /// Failed State and its atomic recovery decision.
    Failure {
        /// Original domain failure object.
        original: T,
        /// Recorded recovery, with a root object only on Stop.
        decision: DomainDecision<T>,
    },
}

/// Fused Read conclusion: accepted evidence or a contextualized operational error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadConclusion<T> {
    /// Accepted evidence interpreted by the State.
    Observed {
        /// Exact accepted evidence object.
        evidence: T,
        /// Deterministic domain result and any recovery decision.
        outcome: DomainConclusion<T>,
    },
    /// Operational error, without fabricated evidence or domain failure.
    AdapterFailed {
        /// Unmodified capability error object.
        error: T,
        /// State-owned contextualization object.
        state_context: T,
        /// Recorded recovery or terminal execution-failure decision.
        decision: RecoveryDecision,
    },
}

/// Settlement of a prepared Effect; this type cannot encode retry or restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectConclusion<T> {
    /// Successful settlement output.
    Success {
        /// Complete State output.
        output: T,
    },
    /// Terminal domain failure after accepted settlement evidence.
    Failure {
        /// Original domain cause.
        original: T,
        /// Mapped root failure.
        root: T,
        /// Reviewed stop code.
        reason: StopCode,
    },
}

impl<T> DomainConclusion<T> {
    pub(crate) fn map_ref<'a, U>(&'a self, f: &mut impl FnMut(&'a T) -> U) -> DomainConclusion<U> {
        match self {
            Self::Success { output } => DomainConclusion::Success { output: f(output) },
            Self::Failure { original, decision } => DomainConclusion::Failure {
                original: f(original),
                decision: match decision {
                    DomainDecision::Retry => DomainDecision::Retry,
                    DomainDecision::Restart { checkpoint } => DomainDecision::Restart {
                        checkpoint: *checkpoint,
                    },
                    DomainDecision::Stop { reason, root } => DomainDecision::Stop {
                        reason: *reason,
                        root: f(root),
                    },
                },
            },
        }
    }
}
impl<T> ReadConclusion<T> {
    pub(crate) fn map_ref<'a, U>(&'a self, f: &mut impl FnMut(&'a T) -> U) -> ReadConclusion<U> {
        match self {
            Self::Observed { evidence, outcome } => ReadConclusion::Observed {
                evidence: f(evidence),
                outcome: outcome.map_ref(f),
            },
            Self::AdapterFailed {
                error,
                state_context,
                decision,
            } => ReadConclusion::AdapterFailed {
                error: f(error),
                state_context: f(state_context),
                decision: *decision,
            },
        }
    }
}
impl<T> EffectConclusion<T> {
    pub(crate) fn map_ref<'a, U>(&'a self, f: &mut impl FnMut(&'a T) -> U) -> EffectConclusion<U> {
        match self {
            Self::Success { output } => EffectConclusion::Success { output: f(output) },
            Self::Failure {
                original,
                root,
                reason,
            } => EffectConclusion::Failure {
                original: f(original),
                root: f(root),
                reason: *reason,
            },
        }
    }
}
