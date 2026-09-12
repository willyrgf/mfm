use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition, RunId};
use mfm_program::{RecoveryUsage, StopReason};
use mfm_values::{
    CanonicalJsonProfile, SchemaIdentity, SchemaKind, SchemaShape, MAX_RUN_OBJECT_CANONICAL_BYTES,
};
use serde::Serialize;

use crate::{Object, Result, RunView, RuntimeError};

/// Qualified operational cause and the complete facts of its adapter invocation.
pub enum AdapterIncidentView {
    /// A duplicate-safe observation that failed operationally.
    Read {
        /// Original operational error.
        error: Object,
        /// Complete executed State input.
        input: Object,
        /// Exact prepared observational intent.
        intent: Object,
    },
    /// An unresolved Effect invocation; these facts do not imply settlement.
    Effect {
        /// Original operational error.
        error: Object,
        /// Complete executed State input.
        input: Object,
        /// Retained command, unchanged by this failure.
        command: Object,
        /// Existing command authority.
        effect_id: EffectId,
    },
}

impl AdapterIncidentView {
    /// Returns the original operational error.
    pub const fn error(&self) -> &Object {
        match self {
            Self::Read { error, .. } | Self::Effect { error, .. } => error,
        }
    }
    /// Returns the complete executed State input.
    pub const fn input(&self) -> &Object {
        match self {
            Self::Read { input, .. } | Self::Effect { input, .. } => input,
        }
    }
}

/// Original retained cause and, for domain failures, its mapped root value.
pub enum FailureCauseView {
    /// A deterministic State domain failure.
    Domain {
        /// Unmodified State failure.
        original: Object,
        /// Independently mapped root failure.
        root: Object,
    },
    /// A Read execution failure with its original invocation facts.
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
        reason: StopReason,
        usage: RecoveryUsage,
        cause: FailureCauseView,
    ) -> Result<Self> {
        #[derive(Serialize)]
        struct WireObject<'a> {
            contract_ref: ContentRef,
            value_ref: &'a ContentRef,
            canonical: &'a serde_json::value::RawValue,
        }
        fn object(value: &Object) -> Result<WireObject<'_>> {
            Ok(WireObject {
                contract_ref: value.contract_ref().map_err(RuntimeError::from)?,
                value_ref: value.value_ref(),
                canonical: serde_json::from_slice(value.canonical_bytes()).map_err(|source| {
                    RuntimeError::native(
                        crate::Operation::Project,
                        mfm_values::NativeCause::from_error(mfm_canonical::JsonError::new(source)),
                    )
                })?,
            })
        }
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Cause<'a> {
            Domain {
                original: WireObject<'a>,
                root: WireObject<'a>,
            },
            #[serde(rename = "adapter")]
            Read {
                mode: &'static str,
                error: WireObject<'a>,
                input: WireObject<'a>,
                intent: WireObject<'a>,
            },
            #[serde(rename = "adapter")]
            Effect {
                mode: &'static str,
                error: WireObject<'a>,
                input: WireObject<'a>,
                command: WireObject<'a>,
                effect_id: &'a EffectId,
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
            reason: StopReason,
            usage: Usage,
            cause: Cause<'a>,
        }
        let wire = Report {
            domain: "mfm.failure-report.v4",
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
                FailureCauseView::Adapter(AdapterIncidentView::Read {
                    error,
                    input,
                    intent,
                }) => Cause::Read {
                    mode: "read",
                    error: object(error)?,
                    input: object(input)?,
                    intent: object(intent)?,
                },
                FailureCauseView::Adapter(AdapterIncidentView::Effect {
                    error,
                    input,
                    command,
                    effect_id,
                }) => Cause::Effect {
                    mode: "effect",
                    error: object(error)?,
                    input: object(input)?,
                    command: object(command)?,
                    effect_id,
                },
            },
        };
        let json = mfm_canonical::to_json_bounded(&wire, MAX_RUN_OBJECT_CANONICAL_BYTES).map_err(
            |source| {
                RuntimeError::at(
                    crate::Operation::Project,
                    crate::Stage::Encode,
                    mfm_values::NativeCause::from_error(source),
                )
            },
        )?;
        // Original and mapped failures intentionally remain inline, including identity maps.
        // Individually admissible values may exceed the report limit when combined. Reject
        // before the terminal append: the original remains AwaitingRecovery at its acknowledged
        // head. Admission does not guarantee report fit.
        // Every embedded value is already canonical, so serialization preserves its byte size.
        crate::check_size(
            crate::SizeResource::FailureReport,
            json.len() as u64,
            MAX_RUN_OBJECT_CANONICAL_BYTES as u64,
        )?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json).map_err(|source| {
            RuntimeError::native(
                crate::Operation::Project,
                mfm_values::NativeCause::from_error(source),
            )
        })?;
        let schema = SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm-failure-report",
            mfm_ids::SchemaVersion::new("4").map_err(|source| {
                RuntimeError::native(
                    crate::Operation::Project,
                    mfm_values::NativeCause::from_error(source),
                )
            })?,
            SchemaShape::CanonicalJsonTerminal {
                profile: CanonicalJsonProfile::GeneralFloatFree,
            },
        )
        .and_then(|identity| identity.schema_id())
        .map_err(|source| {
            RuntimeError::native(
                crate::Operation::Project,
                mfm_values::NativeCause::from_error(source),
            )
        })?;
        let value_ref = ContentRef::new(schema, raw_content_digest(canonical.as_bytes())).map_err(
            |source| {
                RuntimeError::native(
                    crate::Operation::Project,
                    mfm_values::NativeCause::from_error(source),
                )
            },
        )?;
        Ok(Self {
            position,
            reason,
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
