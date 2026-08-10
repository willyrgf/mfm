#![warn(missing_docs)]
//! One-action interpreter for certified structured MFM runs.
//!
//! The runtime owns qualified live-capability selection, private committed
//! request and observation proofs, affine live-access authority, and
//! deterministic one-action scheduling. History mutation is requested only
//! through [`history::RuntimeHistoryPort`]; production adapters and the sole
//! reducer remain owned by `mfm-store`.

pub mod history;
pub mod structured;
