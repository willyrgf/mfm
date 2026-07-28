#![warn(missing_docs)]
//! Pure fact authoring and selection semantics for the MFM typed kernel.
//!
//! Facts are transition outputs. Same-run consumers use certified graph edges;
//! deliberate cross-run selection uses one annex-backed
//! [`FactSelectionRequest`]. This crate owns only journal-independent value
//! semantics. Journal references, tenant coordinates, scan authority,
//! responses, completeness proofs, and retained-object authority live in
//! `mfm-journal` and the store.
//!
//! ```
//! use mfm_canonical::CanonicalValue;
//! use mfm_facts::{CanonicalFactPredicate, FactSelectionLimit, FactSet};
//!
//! let predicate = CanonicalFactPredicate::from_canonical_value(
//!     CanonicalValue::object([("wallet", CanonicalValue::String("alice".into()))])
//!         .expect("canonical predicate"),
//! )?;
//! assert_eq!(predicate.canonical_json(), br#"{"wallet":"alice"}"#);
//! assert_eq!(FactSelectionLimit::new(128)?.get(), 128);
//! assert!(FactSet::empty().as_slice().is_empty());
//! # Ok::<(), mfm_facts::FactError>(())
//! ```

mod codec;
mod descriptor;
mod emission;
mod selection;
mod value;

pub use descriptor::{FactDescriptor, FactKind};
pub use emission::{FactProposal, FactSet, ProposedFactValue};
pub use selection::{
    FactCandidate, FactOrdering, FactProducerScope, FactSelectionLimit, FactSelectionQuery,
    FactSelectionRequest, FactTieBreak, FactTopK,
};
pub use value::{CanonicalFactPredicate, FactScalar, FactSubject};

/// Result type for pure fact construction and validation.
pub type Result<T> = std::result::Result<T, FactError>;

/// Redaction-safe fact contract failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactError {
    /// The frozen recoverability codec rejected the value.
    #[error(transparent)]
    Recoverability(#[from] mfm_canonical::RecoverabilityError),
    /// A canonical value could not be built or projected.
    #[error("canonical fact value is invalid")]
    Canonical,
    /// A checked identity could not be projected from validated material.
    #[error("fact identity is invalid")]
    Identity,
    /// A fact descriptor violates the closed authoring contract.
    #[error("fact descriptor is invalid: {0}")]
    Descriptor(&'static str),
    /// A fact selection violates an owner-local invariant.
    #[error("fact selection is invalid: {0}")]
    Selection(&'static str),
    /// A same-run fact emission violates its bounded deterministic contract.
    #[error("fact emission is invalid: {0}")]
    Emission(&'static str),
}

/// Maximum number of queries in one fact-selection request.
pub const MAX_FACT_SELECTION_QUERIES: usize = 128;

/// Maximum selected facts returned for one query.
pub const MAX_FACT_SELECTION_LIMIT: u32 = 128;

/// Maximum facts emitted by one successful transition settlement.
pub const MAX_FACT_EMISSIONS: usize = 4096;

#[cfg(test)]
mod tests;
