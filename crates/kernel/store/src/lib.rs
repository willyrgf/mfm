#![warn(missing_docs)]
//! Durable callback-free semantic evidence for one Store scope and writer epoch.
//!
//! Store is intentionally below Runtime in the dependency graph.  It validates strict journal
//! frames, reduces the sequential prefix, and returns append owners; it cannot invoke a State or
//! provider because neither authority is present in this crate.

pub mod single_trust;

pub use single_trust::{
    AppendDisposition, ConfigurationAppendDisposition, ConfigurationHistory, ConfigurationRevision,
    FactContinuation, PreparationAppend, PreparedConclusion, PreparedConfigurationAppend,
    QualifiedRun, ReducedRunState, Result, RunAction, RunReducer, RunStore, StoreError,
};
