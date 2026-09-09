use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, ExecutionPosition, RunId};
use mfm_journal::StopCode;
use mfm_program::{RecoveryDenial, RecoveryLimit, RecoveryUsage, StopReason};
use mfm_values::{
    CanonicalJsonProfile, SchemaIdentity, SchemaKind, SchemaShape, MAX_RUN_OBJECT_CANONICAL_BYTES,
};
use serde::Serialize;

use crate::{Result, RunView, RuntimeError, ValueView};

/// Qualified capability cause and State-owned meaning; neither implies settlement.
pub struct AdapterIncidentView {
    /// Original operational error.
    pub error: ValueView,
    /// State-owned context for that exact error.
    pub state_context: ValueView,
}

/// Original retained cause and, for domain failures, its mapped root value.
pub enum FailureCauseView {
    /// A deterministic State domain failure.
    Domain {
        /// Unmodified State failure.
        original: ValueView,
        /// Independently mapped root failure.
        root: ValueView,
    },
    /// A contextualized Read execution failure.
    Adapter(AdapterIncidentView),
}

/// Content-addressed terminal report derived from the acknowledged history.
pub struct FailureReport {
    position: ExecutionPosition,
    reason: StopReason,
    usage: RecoveryUsage,
    cause: Box<FailureCauseView>,
    value_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
}

impl FailureReport {
    pub(crate) fn new(
        position: ExecutionPosition,
        reason: StopCode,
        usage: RecoveryUsage,
        cause: FailureCauseView,
    ) -> Result<Self> {
        #[derive(Serialize)]
        struct Object<'a> {
            contract_ref: &'a ContentRef,
            value_ref: &'a ContentRef,
            canonical: &'a serde_json::value::RawValue,
        }
        fn object(value: &ValueView) -> Result<Object<'_>> {
            Ok(Object {
                contract_ref: value.contract_ref(),
                value_ref: value.value_ref(),
                canonical: serde_json::from_slice(value.canonical_bytes())
                    .map_err(|_| RuntimeError::Internal)?,
            })
        }
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Cause<'a> {
            Domain {
                original: Object<'a>,
                root: Object<'a>,
            },
            Adapter {
                error: Object<'a>,
                state_context: Object<'a>,
            },
        }
        #[derive(Serialize)]
        struct Usage {
            state_retries: u32,
            state_restarts: u32,
            run_decisions: u32,
        }
        #[derive(Serialize)]
        struct Report<'a> {
            domain: &'static str,
            position: ExecutionPosition,
            reason: StopCode,
            usage: Usage,
            cause: Cause<'a>,
        }
        let wire = Report {
            domain: "mfm.failure-report.v1",
            position,
            reason,
            usage: Usage {
                state_retries: usage.state_retries,
                state_restarts: usage.state_restarts,
                run_decisions: usage.run_decisions,
            },
            cause: match &cause {
                FailureCauseView::Domain { original, root } => Cause::Domain {
                    original: object(original)?,
                    root: object(root)?,
                },
                FailureCauseView::Adapter(incident) => Cause::Adapter {
                    error: object(&incident.error)?,
                    state_context: object(&incident.state_context)?,
                },
            },
        };
        let json = serde_json::to_string(&wire).map_err(|_| RuntimeError::Internal)?;
        // Original and mapped failures intentionally remain inline, including identity maps.
        // Individually admissible values may exceed the report limit when combined. Reject
        // before the terminal append: the acknowledged Runnable/EffectPending head and any
        // pending Effect authority remain intact. Admission does not guarantee report fit.
        // Every embedded value is already canonical, so serialization preserves its byte size.
        crate::check_size(
            crate::SizeResource::FailureReport,
            json.len() as u64,
            MAX_RUN_OBJECT_CANONICAL_BYTES as u64,
        )?;
        let canonical =
            PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| RuntimeError::Internal)?;
        let schema = SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm-failure-report",
            mfm_ids::SchemaVersion::new("1").map_err(|_| RuntimeError::Internal)?,
            SchemaShape::CanonicalJsonTerminal {
                profile: CanonicalJsonProfile::GeneralFloatFree,
            },
        )
        .and_then(|identity| identity.schema_id())
        .map_err(|_| RuntimeError::Internal)?;
        let value_ref = ContentRef::new(schema, raw_content_digest(canonical.as_bytes()))
            .map_err(|_| RuntimeError::Internal)?;
        Ok(Self {
            position,
            reason: stop_reason(reason),
            usage,
            cause: Box::new(cause),
            value_ref,
            canonical,
        })
    }
    /// Returns the terminal execution occurrence.
    pub const fn position(&self) -> &ExecutionPosition {
        &self.position
    }
    /// Returns why automatic recovery stopped.
    pub const fn reason(&self) -> &StopReason {
        &self.reason
    }
    /// Returns counters derived from committed decisions.
    pub const fn usage(&self) -> &RecoveryUsage {
        &self.usage
    }
    /// Returns the original typed cause and any root mapping.
    pub const fn cause(&self) -> &FailureCauseView {
        &self.cause
    }
    /// Returns the canonical report instance identity.
    pub const fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }
    /// Returns the complete canonical report.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

pub(crate) fn stop_reason(reason: StopCode) -> StopReason {
    match reason {
        StopCode::Nonrecoverable => StopReason::Nonrecoverable,
        StopCode::Requested => StopReason::Requested,
        StopCode::StateRetryExhausted => StopReason::Exhausted(RecoveryLimit::StateRetry),
        StopCode::StateRestartExhausted => StopReason::Exhausted(RecoveryLimit::StateRestart),
        StopCode::RunExhausted => StopReason::Exhausted(RecoveryLimit::Run),
        StopCode::PureRetry => StopReason::Disallowed(RecoveryDenial::PureRetry),
        StopCode::CheckpointUnavailable => {
            StopReason::Disallowed(RecoveryDenial::CheckpointUnavailable)
        }
        StopCode::EffectBarrier => StopReason::Disallowed(RecoveryDenial::EffectBarrier),
        StopCode::EffectSettled => StopReason::Disallowed(RecoveryDenial::EffectSettled),
    }
}

/// Invocation failure with its last qualified observation, without claiming the current head.
pub enum InvocationFailure {
    /// Execution stopped before a successful response.
    Execution {
        /// Caller-supplied identity.
        run_id: RunId,
        /// Redaction-safe failure with its mechanical source preserved.
        error: RuntimeError,
        /// Most recent completely qualified observation, if any.
        last_observed: Option<RunView>,
    },
    /// Automatic reconciliation stopped while an acknowledged Effect remains unresolved.
    RecoveryStopped {
        /// Qualified pending Effect snapshot.
        observed: RunView,
        /// Unrecorded operational cause and its checked context.
        incident: Box<AdapterIncidentView>,
        /// Reason the invocation stopped automatic progression.
        reason: StopReason,
    },
}

impl std::fmt::Debug for InvocationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Execution { run_id, error, .. } => formatter
                .debug_struct("Execution")
                .field("run_id", run_id)
                .field("error", error)
                .finish_non_exhaustive(),
            Self::RecoveryStopped {
                observed, reason, ..
            } => formatter
                .debug_struct("RecoveryStopped")
                .field("run_id", observed.run_id())
                .field("reason", reason)
                .finish_non_exhaustive(),
        }
    }
}
impl std::fmt::Display for InvocationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Execution { error, .. } => std::fmt::Display::fmt(error, formatter),
            Self::RecoveryStopped { .. } => {
                formatter.write_str("recovery stopped; effect remains unresolved")
            }
        }
    }
}
impl std::error::Error for InvocationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Execution { error, .. } => Some(error),
            Self::RecoveryStopped { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests;
