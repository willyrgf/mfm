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
//! use mfm_replay::v1::{
//!     compare_current, verify_recorded_history, CanonicalReplayResult, Result,
//!     VerifiedHistoryResult,
//! };
//! use mfm_store::v1::{FactScanBackend, Replay, RunAccessAuthority, RunHistoryReader};
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

/// Recoverability-v1 replay and inspection contracts.
pub mod v1;
