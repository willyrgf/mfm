#![warn(missing_docs)]
//! Strict append-only run frames and the three-family semantic journal.
//!
//! Journal owns bytes, identities, and hashable record shape.  It never owns callbacks, Runtime
//! sessions, provider authority, or ambient I/O.  Store is the only semantic reducer.

pub mod single_trust;

pub use single_trust::{
    ImmutableObject, PreparationRef, RecordLogicalKey, RunAdmitted, RunFrame, RunRecord,
    SequentialControlAddress, StateConcluded, StateOutcome, StatePrepared, ValueRef,
};
