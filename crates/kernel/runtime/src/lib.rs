#![warn(missing_docs)]
//! Immutable live Runtime assembly and affine State execution ownership.
//!
//! Runtime depends on Store for semantic append owners.  Store and Replay cannot depend on this
//! crate and therefore cannot invoke live State implementations.

pub mod lifecycle;
pub mod single_trust;

pub use lifecycle::{
    AdmissionConflict, AdmissionFailure, AdmissionInput, ParkReason, ParkedRun, PendingConclusion,
    ResumeFailure, ResumeInput, ResumeStep, RunSession, Runtime, RuntimeStep, SpawnStep,
    SuspendedRun, TerminalRun,
};
pub use single_trust::{
    AcceptedIntegrityAccess, AcceptedOutcomeAccess, AccessHandlerResolution, AccessImplementation,
    AccessIngressFuture, AccessResolution, AccessResolutionFuture, BoxFuture, CommittedCall,
    Effect, FailureValue, OpenedPreparationCommit, PreparationError, PreparedExecution, Pure,
    PureImplementation, QualifiedAdapter, Read, Result, RuntimeAssembly, RuntimeAssemblyBuilder,
    RuntimeError, State, UnresolvedAccess, UnresolvedClassification,
};
