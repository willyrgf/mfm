#![warn(missing_docs)]
//! Store-owned recoverability-v3 journal verification and atomic append contracts.
//!
//! Libraries use purpose-typed authority to load one exact run or submit one of four sealed
//! append variants. Frozen values and codecs live in `mfm-journal`; this crate owns structural
//! folding, assigned coordinates, object authority, and backend transaction seams.

/// Versioned recoverability-v3 store contract.
pub mod v2;

pub use self::v2::*;
