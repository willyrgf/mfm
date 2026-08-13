#![warn(missing_docs)]
//! Durable callback-free semantic evidence for one Store scope and writer epoch.
//!
//! Store is intentionally below Runtime in the dependency graph.  It validates strict journal
//! frames, reduces the sequential prefix, and returns append owners; it cannot invoke a State or
//! provider because neither authority is present in this crate.

pub mod backend;
pub mod single_trust;

#[cfg(feature = "backend-conformance")]
pub mod backend_conformance;

pub use backend::{
    BackendAppendCommand, BackendAppendOutcome, BackendConfigurationOutcome, BackendError,
    BackendFuture, BackendResult, ConclusionCommitOutcome, ConfigurationAppendCommand,
    ConfigurationCommitOutcome, ConfigurationStore, HistoryReader, MemoryStructuredBackend,
    OpenedStructuredStore, PreparedConfigurationWrite, QualifiedHistoryPort,
    RawConfigurationRevision, RawFactPublication, RawFactSnapshot, RawFrameBytes,
    RawHistoryLoadLimit, RawRunPrefix, StoreAuditPort, StoreOpenError, StoreParts, StoreWorkLimits,
    StructuredStore, StructuredStoreBackend, StructuredStoreIdentity,
};
pub use single_trust::{
    validate_prefix, AppendDisposition, ConfigurationAppendDisposition, ConfigurationRevision,
    ConfigurationSnapshot, FactContinuation, PreparationAppend, PreparedConclusion, QualifiedRun,
    ReducedRunState, Result, RunAction, RunReducer, StoreError,
};
