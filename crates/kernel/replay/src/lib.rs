#![warn(missing_docs)]
//! Callback-free recorded-history verification and inspection for MFM.
//!
//! This crate consumes purpose-authorized, store-verified journal views. It
//! traverses only committed canonical records and retained objects: it owns no
//! runtime catalog, live capability, executor, transport, signer, append path,
//! or replay broker. Recorded verification returns an affine session whose
//! authoritative view remains private; non-verification modes consume that
//! session while binding an explicit caller-held semantic portable export.
//!
//! ```
//! use mfm_program::QualifiedProgramRegistry;
//! use mfm_replay::trace_export::VerifiedExportStream;
//! use mfm_replay::{
//!     compare_current, verify_recorded_history, CanonicalReplayResult, Result,
//!     VerifiedHistoryResult,
//! };
//! use mfm_store::{FactScanBackend, Replay, RunAccessAuthority, RunHistoryReader};
//!
//! async fn verify<B: FactScanBackend>(
//!     reader: &RunHistoryReader<B>,
//!     authority: &RunAccessAuthority<Replay>,
//! ) -> Result<VerifiedHistoryResult> {
//!     verify_recorded_history(reader, authority).await
//! }
//!
//! fn compare(
//!     historical: &VerifiedExportStream,
//!     registry: &QualifiedProgramRegistry,
//! ) -> Result<CanonicalReplayResult> {
//!     compare_current(historical, registry)
//! }
//! ```

/// Deterministic trace and portable-export canonicalization.
pub mod trace_export;

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
pub use mfm_store::{TransitionTracePageRequest, TransitionTraceSourceRequirements};
