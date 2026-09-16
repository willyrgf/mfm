use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, ExecutionPosition, RunId};
use mfm_program::{RecoveryUsage, StopReason};
use mfm_values::{
    CanonicalJsonProfile, SchemaIdentity, SchemaKind, SchemaShape, MAX_RUN_OBJECT_CANONICAL_BYTES,
};
use serde::Serialize;

use crate::{Object, Result, RunView, RuntimeError};

/// Content-addressed terminal report derived from the acknowledged history.
pub struct FailureReport {
    failure: crate::Failure,
    reason: StopReason,
    usage: RecoveryUsage,
    root: Option<Object>,
    value_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
}

impl FailureReport {
    pub(crate) fn new(
        failure: crate::Failure,
        reason: StopReason,
        usage: RecoveryUsage,
        root: Option<Object>,
    ) -> Result<Self> {
        let position = *failure.call().position();
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
                        mfm_values::InvocationDiagnostic::from_fields(
                            "json_error",
                            "object",
                            &mfm_canonical::JsonError::new(source),
                            None,
                        ),
                    )
                })?,
            })
        }
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        // This temporary borrowing serializer needs no heap-owned cause tree.
        #[allow(clippy::large_enum_variant)]
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
            domain: "mfm.failure-report.v5",
            position,
            reason,
            usage: Usage {
                state_retries: usage.state_retries,
                state_restarts: usage.state_restarts,
                run_decisions: usage.run_decisions,
            },
            cause: match (&failure, &root) {
                (crate::Failure::Domain { original, .. }, Some(root)) => Cause::Domain {
                    original: object(original)?,
                    root: object(root)?,
                },
                (
                    crate::Failure::Read {
                        call,
                        intent,
                        original,
                    },
                    None,
                ) => Cause::Read {
                    mode: "read",
                    error: object(original)?,
                    input: object(call.input())?,
                    intent: object(intent)?,
                },
                _ => return Err(RuntimeError::InvalidHistory),
            },
        };
        let json = mfm_canonical::to_json_bounded(&wire, MAX_RUN_OBJECT_CANONICAL_BYTES).map_err(
            |source| {
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
            mfm_ids::SchemaVersion::new("5").map_err(|source| {
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
            failure,
            reason,
            usage,
            root,
            value_ref,
            canonical,
        })
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
    /// Returns the mapped domain root, absent for Read failures.
    pub const fn root(&self) -> Option<&Object> {
        self.root.as_ref()
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
