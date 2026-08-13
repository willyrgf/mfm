#![warn(missing_docs)]
//! Immutable live Runtime assembly and affine State execution ownership.
//!
//! Runtime depends on Store for semantic append owners.  Store and Replay cannot depend on this
//! crate and therefore cannot invoke live State implementations.

pub mod single_trust;

pub use single_trust::{
    is_direct_new, prepare_access_resolution, prepare_pure_success, qualify_success,
    AcceptedIntegrityAccess, AcceptedOutcomeAccess, AccessConclusion, AccessHandlerResolution,
    AccessImplementation, AccessResolution, AccessResolutionFuture, BoxFuture, CommittedCall,
    Effect, PendingConclusion, PendingFailure, PreparationError, PreparedExecution, Pure,
    PureImplementation, QualifiedAdapter, Read, Result, RunSession, RuntimeAssembly,
    RuntimeAssemblyBuilder, RuntimeError, State, StoreAppendResult, UnresolvedAccess,
    UnresolvedClassification,
};
