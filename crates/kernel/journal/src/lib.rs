#![warn(missing_docs)]
//! Frozen recoverability-v3 journal values.
//!
//! Every public persisted value in this crate is backed by
//! [`mfm_canonical::ValidatedCanonicalValueV3`]. Construction and decoding
//! always use the embedded recoverability annex; Rust serialization is not a
//! second wire authority.
//!
//! Store atomicity, structural folding, scheduling, replay, callbacks, and
//! ambient access are deliberately outside this crate.
//!
//! The superseded lifecycle event algebra has no compatibility surface:
//!
//! ```compile_fail
//! use mfm_journal::v2::KernelEventPayload;
//! ```
//!
//! ```compile_fail
//! use mfm_journal::v2::StateAttemptStarted;
//! ```

/// Frozen recoverability-v3 journal values and codecs.
pub mod v2;
