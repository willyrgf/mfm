use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, ExecutionPosition, RunId};
use mfm_program::{RecoveryUsage, StopReason};
use mfm_values::{
    CanonicalJsonProfile, SchemaIdentity, SchemaKind, SchemaShape, MAX_RUN_OBJECT_CANONICAL_BYTES,
};
use serde::Serialize;

use crate::{Result, RunView, RuntimeError};

// Run-scoped so concurrent engine tests retain the production policy.
#[cfg(test)]
pub(crate) static REPORT_ENCODING_LIMIT: std::sync::Mutex<Option<(RunId, usize)>> =
    std::sync::Mutex::new(None);

/// Content-addressed terminal report derived from the acknowledged history.
pub struct FailureReport {
    declaration: mfm_program::StateDeclaration,
    failure: crate::Failure,
    reason: StopReason,
    usage: RecoveryUsage,
    value_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
}

impl FailureReport {
    pub(crate) fn new(
        run_id: &RunId,
        program_ref: &ContentRef,
        declaration: &mfm_program::StateDeclaration,
        failure: crate::Failure,
        reason: StopReason,
        usage: RecoveryUsage,
    ) -> Result<Self> {
        #[derive(Serialize)]
        struct Usage {
            state_retries: u32,
            state_restarts: u32,
            run_decisions: u32,
        }
        #[derive(Serialize)]
        struct Report<'a> {
            domain: &'static str,
            run_id: &'a RunId,
            program_ref: &'a ContentRef,
            state_implementation_ref: &'a ContentRef,
            execution: &'a mfm_program::Execution,
            failure: &'a crate::Failure,
            reason: StopReason,
            usage: Usage,
        }
        let wire = Report {
            domain: "mfm.failure-report.v6",
            run_id,
            program_ref,
            state_implementation_ref: declaration.state_implementation_ref(),
            execution: declaration.execution(),
            failure: &failure,
            reason,
            usage: Usage {
                state_retries: usage.state_retries,
                state_restarts: usage.state_restarts,
                run_decisions: usage.run_decisions,
            },
        };
        let limit = MAX_RUN_OBJECT_CANONICAL_BYTES;
        #[cfg(test)]
        let limit = REPORT_ENCODING_LIMIT
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(run, _)| run == run_id)
            .map_or(limit, |(_, limit)| *limit);
        let json = mfm_canonical::to_json_bounded(&wire, limit).map_err(|source| {
            RuntimeError::at(
                crate::Operation::Project,
                crate::Stage::Encode,
                mfm_values::InvocationDiagnostic::from_fields(
                    "canonical_error",
                    "new",
                    &source,
                    source
                        .serialization_bound()
                        .map(|(limit, observed_at_least)| {
                            mfm_values::SizeViolation::SerializationBound {
                                resource: mfm_values::SizeResource::FailureReport,
                                limit: limit as u64,
                                observed_at_least: observed_at_least as u64,
                            }
                        }),
                ),
            )
        })?;
        // Complete retained calls and originals remain inline. Their combined report can exceed
        // the limit even when each Object fits. Reject before the terminal append, leaving the
        // acknowledged original AwaitingRecovery; admission does not guarantee report fit.
        crate::check_size(
            crate::SizeResource::FailureReport,
            json.len() as u64,
            limit as u64,
        )?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json).map_err(|source| {
            RuntimeError::native(
                crate::Operation::Project,
                mfm_values::InvocationDiagnostic::from_fields(
                    "canonical_error",
                    "new",
                    &source,
                    None,
                ),
            )
        })?;
        let schema = SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm-failure-report",
            mfm_ids::SchemaVersion::new("6").map_err(|source| {
                RuntimeError::native(
                    crate::Operation::Project,
                    mfm_values::ValueError::Identity(source).into_diagnostic("new"),
                )
            })?,
            SchemaShape::CanonicalJsonTerminal {
                profile: CanonicalJsonProfile::DiagnosticFloatFree,
            },
        )
        .and_then(|identity| identity.schema_id())
        .map_err(|source| {
            RuntimeError::native(crate::Operation::Project, source.into_diagnostic("new"))
        })?;
        let value_ref = ContentRef::new(schema, raw_content_digest(canonical.as_bytes())).map_err(
            |source| {
                RuntimeError::native(
                    crate::Operation::Project,
                    mfm_values::ValueError::Identity(source).into_diagnostic("new"),
                )
            },
        )?;
        Ok(Self {
            declaration: declaration.clone(),
            failure,
            reason,
            usage,
            value_ref,
            canonical,
        })
    }
    /// Borrows the exact State contract qualified from this report's immutable Program.
    pub fn declaration(&self) -> &mfm_program::StateDeclaration {
        &self.declaration
    }
    /// Returns the terminal execution occurrence.
    pub fn position(&self) -> &ExecutionPosition {
        self.failure.call().position()
    }
    /// Returns why automatic recovery stopped.
    pub const fn reason(&self) -> &StopReason {
        &self.reason
    }
    /// Returns counters derived from committed decisions.
    pub const fn usage(&self) -> &RecoveryUsage {
        &self.usage
    }
    /// Returns the retained original operation and failure.
    pub const fn failure(&self) -> &crate::Failure {
        &self.failure
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
            Self::RecoveryStopped { observed, .. } => formatter
                .debug_struct("RecoveryStopped")
                .field("run_id", observed.run_id())
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
