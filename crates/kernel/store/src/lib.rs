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
//!
//! ```compile_fail
//! use mfm_store::v1::{AdmissionLane, AdmissionLaneClass, WaitFifo};
//!
//! // Admission lane class/mode pairs are not caller-assembled. Resource admission
//! // lanes are built only through the resource FIFO constructor, and execution
//! // claim lanes are built only through the nowait constructor.
//! let _forged = AdmissionLane::<WaitFifo> {
//!     class: AdmissionLaneClass::ExecutionClaim,
//! };
//! ```

/// Versioned v1 typed store contract.
pub mod v1;
