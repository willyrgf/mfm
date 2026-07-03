#![warn(missing_docs)]
//! Fact descriptor and query evidence contracts for the MFM typed kernel.
//!
//! This crate owns the domain-free facts kernel surface described by
//! `docs/RFC_COLLECTORS.md`: descriptor identity, field extraction contracts,
//! fact visibility, fact keys, claim identity, internal refs, and canonical
//! query evidence. The initial crate exists so later commits can add those
//! contracts behind the kernel dependency boundary without mixing them into
//! event, runtime, store, app, or collector code.
//!
//! ```
//! assert_eq!(mfm_facts::FACTS_KERNEL_CONTRACT_VERSION, "mfm.facts.v1");
//! ```

mod claim;
mod codec;
mod descriptor;
mod extraction;
mod ids;
mod query;
mod receipt;
mod scalar;
mod serde_helpers;
mod subject;
mod tags;

pub use claim::*;
pub use codec::*;
pub use descriptor::*;
pub use extraction::*;
pub use ids::*;
pub use query::*;
pub use receipt::*;
pub use scalar::*;
pub use subject::*;
pub use tags::*;

#[cfg(test)]
pub(crate) use codec::parse_canonical_scalar_value;
#[cfg(test)]
pub(crate) use extraction::json_to_typed_fact_scalar;

/// Stable facts-kernel contract version for the initial collectors RFC surface.
pub const FACTS_KERNEL_CONTRACT_VERSION: &str = "mfm.facts.v1";

/// V1 fact query compiler version recorded in canonical query plans.
pub const FACT_QUERY_COMPILER_VERSION: &str = "mfm.facts.query.v1";

/// V1 canonicalizer version recorded in canonical query plans.
pub const FACT_QUERY_CANONICALIZER_VERSION: &str = "mfm.canonical.v1";

/// Maximum UTF-8 byte length accepted for an extracted scalar in v1.
pub const MAX_FACT_SCALAR_BYTES: usize = 4096;

#[cfg(test)]
mod tests;
