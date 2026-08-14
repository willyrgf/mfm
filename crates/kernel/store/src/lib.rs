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
    AccessPreparationOutcome, AdmissionOutcome, BackendAppendCommand, BackendAppendOutcome,
    BackendConfigurationOutcome, BackendError, BackendFuture, BackendResult,
    ConfigurationAppendCommand, ConfigurationCommitOutcome, ConfigurationStore,
    ConfigurationWriteSession, HistoryReader, MemoryStructuredBackend, OpenedStructuredStore,
    PreparedAdmission, PreparedConfigurationAppend, QualifiedHistoryPort, RawConfigurationRevision,
    RawFactPublication, RawFactSnapshot, RawFrameBytes, RawHistoryLoadLimit, RawRunPrefix,
    ResolvedConfiguration, ResolvedConfigurationHead, SelectedConclusion,
    SelectedConclusionOutcome, SelectedConclusionPreparationOutcome, StoreAuditPort,
    StoreOpenError, StoreParts, StoreWorkLimits, StructuredStore, StructuredStoreBackend,
    StructuredStoreIdentity, SuspendedConfigurationAppend,
};
pub use single_trust::{
    replay_terminality, retained_program, validate_prefix, AppendDisposition,
    ConfigurationAppendDisposition, FactContinuation, QualifiedRun, Result, RunAction, SelectedRun,
    StoreError,
};
