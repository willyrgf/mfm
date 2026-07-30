//! Recoverability-v3 replay and inspection contracts.

mod error;
mod inspection;
mod reproduction;
mod service;

pub(crate) use self::error::store_error;
pub use self::error::{ReplayError, ReplayErrorKind, Result};
pub use self::inspection::{
    AccessAuditEntry, AccessAuditPage, CanonicalTransitionTrace, TransitionTracePage,
};
pub use self::reproduction::{
    compare_current, reproduce_exact, CanonicalReplayResult, ExactReproduction,
    ExactReproductionPlan, ReproductionFuture, ReproductionResolver, VerifiedHistoryResult,
};
pub use self::service::{
    discover_transition_trace_sources, inspect_access_audit, inspect_transition_trace,
    required_export_source_run_ids, verify_recorded_history,
};
pub use mfm_store::v2::{TransitionTracePageRequest, TransitionTraceSourceRequirements};
