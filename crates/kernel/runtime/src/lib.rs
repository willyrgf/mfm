#![warn(missing_docs)]
//! One-action interpreter for certified structured MFM runs.
//!
//! The runtime owns qualified live-capability selection, private committed
//! request and observation proofs, affine live-access authority, and
//! deterministic one-action scheduling. Persisted structure and append
//! legality remain owned by `mfm-store`; the qualified program registry, typed
//! semantic callbacks, and value-only views remain owned by `mfm-program`.

pub mod structured;
