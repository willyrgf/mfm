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
    ProposedObservationOutcome, ProposedTransitionValue, QualifiedRuntimeIntent,
    StateTransitionProposal, StructuredAdmissionCommand, StructuredAdmissionMaterial,
};
pub use cursor::{
    ActionableState, EffectEntryAttentionResolution, EffectEntrySubject, LaneCursor, ProgramCursor,
    StateLeaf, StructuredFrontier,
};
pub use error::HistoryError;
pub use identity::{PhysicalTargetIdentity, StructuredStoreIdentity};
pub use port::{HistoryFuture, RuntimeHistoryPort, VerifiedRunView};
pub use proofs::{
    CertifiedAccessAuthorization, CommittedAccessAuthorization, HistoryAppendOutcome,
    PriorRunFactScanCompletion, StructuredAppendAttempt,
};

/// Result type for history-port operations.
pub type Result<T> = std::result::Result<T, HistoryError>;
