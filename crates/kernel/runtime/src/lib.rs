#![warn(missing_docs)]
//! Stateless audited interpreter for certified MFM runs.
//!
//! The runtime owns qualified live-capability selection, private committed
//! request and observation proofs, affine live-access authority, and
//! deterministic one-action scheduling. Persisted structure and append
//! legality remain owned by `mfm-store`; the qualified program registry, typed
//! semantic callbacks, and value-only views remain owned by `mfm-program`.

mod access;
mod append_id;
mod callback_material;
mod capability_registry;
mod decision;
mod drive;
mod materialization;
mod observation_material;
mod outcome;
mod runtime_error;

pub use access::{
    AuditedReadCapability, AuthorizedEnsureAccess, AuthorizedReadAccess, ReadCapabilityFuture,
    ReadCapabilityOutcome, RecoverableEffectExecutor,
};
pub use capability_registry::{qualify_effect_executor, qualify_read_capability};
pub use drive::Runtime;
pub use outcome::{DriveOutcome, DriveWaitReason};
pub use runtime_error::{Result, RuntimeError};
