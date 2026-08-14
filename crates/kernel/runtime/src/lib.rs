#![warn(missing_docs)]
//! Immutable live Runtime assembly and affine State execution ownership.
//!
//! Runtime depends on Store for semantic append owners.  Store and Replay cannot depend on this
//! crate and therefore cannot invoke live State implementations.

pub mod lifecycle;
pub mod single_trust;

pub use lifecycle::{
    AdmissionConflict, AdmissionFailure, AdmissionInput, ParkReason, ParkedRun, PendingConclusion,
    ResumeFailure, ResumeStep, RunSession, Runtime, RuntimeLimits, RuntimeStep, SpawnStep,
    SuspendedRun, TerminalRun,
};
pub use single_trust::{
    AcceptedIntegrityAccess, AcceptedOutcomeAccess, AccessImplementation, AccessIngressFuture,
    AccessResolution, BoxFuture, CommittedCall, OpenedPreparationCommit, PreparationError,
    PreparedExecution, PureImplementation, QualifiedAdapter, Result, RuntimeAssembly,
    RuntimeAssemblyBuilder, RuntimeError, UnresolvedAccess, UnresolvedClassification,
};
