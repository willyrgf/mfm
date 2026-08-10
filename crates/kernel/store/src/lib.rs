#![warn(missing_docs)]
//! Store-owned structured-history verification and atomic append contracts.
//!
//! Libraries use purpose-typed authority to load one exact run or submit one of four sealed
//! append variants. Frozen values and codecs live in `mfm-journal`; this crate owns structural
//! reduction, assigned coordinates, object authority, and backend transaction seams.

pub mod structured;
