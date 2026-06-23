#![warn(missing_docs)]
//! Typed kernel store contracts for MFM.
//!
//! The store commit surface accepts typed event payload batches plus typed
//! preconditions. Stores own envelopes, stream sequence numbers, ordinals,
//! event ids, logical keys, commit-key idempotency entries, and projection
//! writes.
//!
//! ```compile_fail
//! use mfm_store::v1::KernelEventEnvelope;
//!
//! // Envelopes are store-owned. Callers cannot construct forged event ids,
//! // stream sequence numbers, ordinals, logical keys, or projection writes.
//! let _forged = KernelEventEnvelope {};
//! ```

/// Versioned v1 typed store contract.
pub mod v1;
