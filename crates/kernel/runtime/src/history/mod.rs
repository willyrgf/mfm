//! Consumer-side history port and sealed semantic command types.
//!
//! Production mutation authority is reachable only through [`RuntimeHistoryPort`]
//! implementations owned by store assembly. Callers may implement the port for
//! isolated tests, but cannot attach a test port to MFM production backends.

mod commands;
mod cursor;
mod error;
mod identity;
mod port;
mod proofs;

pub use commands::{
    AccessAuthorizationProposal, AccessObservationProposal, ProposedCanonicalValue,
    ProposedObservationOutcome, ProposedTransitionValue, StateTransitionProposal,
    StructuredAdmissionCommand, StructuredAdmissionMaterial,
};
pub use cursor::{
    ActionableState, LaneCursor, ObservationQualification, ProgramCursor, StateLeaf,
    StructuredFrontier,
};
pub use error::HistoryError;
pub use identity::StructuredStoreIdentity;
pub use port::{
    AppendAttemptApi, AuthorizationApi, HistoryFuture, ObservationCommitApi, RuntimeHistoryPort,
    VerifiedRunView,
};
pub use proofs::{
    HistoryAppendOutcome, NewlyAppendedAuthorization, ObservationCommit,
    PriorRunFactScanCompletion, StructuredAppendAttempt,
};

/// Result type for history-port operations.
pub type Result<T> = std::result::Result<T, HistoryError>;
