#![warn(missing_docs)]
//! Opaque callback-free Program authority and exact typed value qualification.
//!
//! Program owns the normalized State/Match declaration algebra and the catalog brand.  It does
//! not own State callbacks, provider clients, Store history, or application policy.

pub mod single_trust;

pub use single_trust::{
    canonical_value, Declaration, ExecutionMode, MatchDeclaration, MatchVariant, Program,
    ProgramCatalog, ProgramCatalogBuilder, ProgramDocument, ProgramError, ProgramIngress,
    ProgramRef, QualifiedTypedValue, QualifiedValue, Result, SequentialControlAddress,
    StateDeclaration,
};
