#![warn(missing_docs)]
//! Opaque callback-free Program authority and exact typed value qualification.
//!
//! Program owns the normalized State/Match declaration algebra and the finalized catalog's exact
//! nominal-contract, descriptor, Rust-type, and process-local brand associations. It does not own
//! State callbacks, provider clients, Store history, or application policy.

pub mod single_trust;

pub use single_trust::{
    canonical_value, nominal_contract_ref, Declaration, ExecutionMode, MatchDeclaration,
    MatchVariant, Program, ProgramCatalog, ProgramCatalogBuilder, ProgramDocument, ProgramError,
    ProgramIngress, ProgramRef, QualifiedTypedValue, QualifiedValue, Result,
    SequentialControlAddress, StateDeclaration,
};
