#![warn(missing_docs)]
//! Fact descriptor and query evidence contracts for the MFM typed kernel.
//!
//! This crate owns the domain-free facts kernel surface: descriptor identity,
//! field extraction contracts, fact visibility, fact keys, claim identity,
//! internal refs, and canonical query evidence. See `docs/design.md` for the
//! typed-core authority contract and the portfolio section of `docs/design.md`
//! for portfolio fact-backed reporting. The crate stays behind the kernel
//! dependency boundary without mixing domain logic into event, runtime, store,
//! app, or collector code.
//!
//! ```
//! use mfm_facts::{FACT_CONTENT_IDENTITY_DIGEST_DOMAIN, FACTS_KERNEL_CONTRACT_VERSION};
//!
//! assert_eq!(FACTS_KERNEL_CONTRACT_VERSION, "mfm.facts.v1");
//! assert_eq!(FACT_CONTENT_IDENTITY_DIGEST_DOMAIN, "mfm.fact.content-identity.v1");
//! ```

macro_rules! impl_fact_tag {
    ($ty:ty, $error_label:literal, $as_str_vis:vis, $as_str_doc:literal, {
        $($variant:path => $tag:literal),+ $(,)?
    }) => {
        impl $ty {
            #[doc = $as_str_doc]
            $as_str_vis const fn as_str(self) -> &'static str {
                match self {
                    $($variant => $tag),+
                }
            }

        }

        impl ::std::fmt::Display for $ty {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $ty {
            type Err = crate::FactError;

            fn from_str(value: &str) -> crate::Result<Self> {
                match value {
                    $($tag => Ok($variant),)+
                    _ => Err(crate::FactError::descriptor(format!(
                        "unknown {} {value:?}",
                        $error_label
                    ))),
                }
            }
        }
    };
}

mod claim;
mod codec;
mod content_identity;
mod descriptor;
mod extraction;
mod ids;
mod query;
mod receipt;
mod scalar;
mod serde_helpers;
mod subject;
mod tags;

pub use claim::{
    FactClaim, FactClaimId, FactClaimParts, FactProducerProvenance, FactRequestEvidence,
    FactResponseEvidence, FactSubjectEvidence, FactSubjectRef, InternalFactRef,
    InternalFactRefParts,
};
pub use codec::{
    canonical_fact_claim_id_bytes, canonical_fact_descriptor_bytes,
    canonical_fact_query_evidence_bytes, canonical_fact_query_plan_bytes,
    canonical_fact_subject_material_bytes, compile_fact_query_plan, derive_fact_claim_id,
    derive_fact_key, extract_subject_material, extract_terms, extract_terms_from_material,
    fact_descriptor_hash, fact_descriptor_schema_id, fact_query_evidence_hash,
    fact_query_evidence_schema_id, fact_query_plan_hash, fact_query_result_set_digest,
    fact_subject_evidence, fact_subject_evidence_from_material, fact_subject_namespace_hash,
    parse_canonical_fact_descriptor_bytes, parse_canonical_fact_query_evidence_bytes,
    parse_canonical_fact_query_shape, parse_canonical_fact_response_bytes,
    parse_canonical_fact_subject_material_bytes, selected_returned_field_summaries_digest,
    subject_material_hash, typed_fact_subject_evidence, typed_fact_subject_value,
    validate_descriptor, validate_fact_query_evidence,
};
pub use content_identity::{
    canonical_fact_content_identity_bytes, derive_fact_content_identity,
    derive_fact_content_identity_from_typed_values, fact_content_identity_digest,
    verify_fact_claim_content_identity, verify_internal_fact_ref_content_identity,
    verify_serialized_fact_content_identity_from_typed_values, FactContentIdentity,
    FactContentIdentityEvidence, FACT_CONTENT_IDENTITY_DIGEST_DOMAIN,
};
pub use descriptor::{
    FactDescriptor, FactFieldDescriptor, FactFieldPolicy, FactOrderingPolicy, FactOrderingTerm,
};
pub use extraction::{FactExtractionMetadata, FactIndexTerm};
pub use ids::{
    CanonicalValuePath, FactCanonicalizerVersion, FactError, FactFieldId, FactKind,
    FactOrderingName, FactQueryCompilerVersion, FactUnit, Result, StoreScopeRef,
};
pub use query::{
    CanonicalFactQueryPlan, CompiledFactQueryShape, FactQueryInput, FactQueryPredicate,
    FactQueryScope, ScopeDecisionEvidence,
};
pub use receipt::{
    fact_query_result_rows_from_receipt, validate_fact_query_result_rows,
    DescriptorCatalogWatermark, FactQueryEvidence, FactQueryReceipt, FactQueryResult,
    FactQueryResultMismatch, FactQueryResultRow, FactSelectionEvidence, QueryResultCardinality,
    ReturnedFactFieldSummary, ReturnedFieldSummaries, StoreCommitOrder, StoreReadFrontier,
    StoreReadFrontierType,
};
pub use scalar::FactCanonicalScalar;
pub use subject::{FactFieldValue, FactKey, FactSubjectMaterialV1};
pub use tags::{
    FactAudience, FactFieldExposure, FactFieldExtraction, FactFieldSource, FactFieldValueType,
    FactMetadataField, FactQueryOperator, FactScale, FactVisibility, FactVisibilityScope,
    NullOrdering, SortDirection,
};

#[cfg(test)]
pub(crate) use codec::parse_canonical_scalar_value;
#[cfg(test)]
pub(crate) use extraction::json_to_typed_fact_scalar;

/// Stable facts-kernel contract version for the initial collectors RFC surface.
pub const FACTS_KERNEL_CONTRACT_VERSION: &str = "mfm.facts.v1";

/// V2 fact-query evidence wire contract with deterministic unsigned receipt metadata.
pub const FACT_QUERY_EVIDENCE_CONTRACT_VERSION: &str = "mfm.fact-query-evidence.v2";

/// V1 fact query compiler version recorded in canonical query plans.
pub const FACT_QUERY_COMPILER_VERSION: &str = "mfm.facts.query.v1";

/// V1 canonicalizer version recorded in canonical query plans.
pub const FACT_QUERY_CANONICALIZER_VERSION: &str = "mfm.canonical.v1";

/// Maximum UTF-8 byte length accepted for an extracted scalar in v1.
pub const MAX_FACT_SCALAR_BYTES: usize = 4096;

#[cfg(test)]
mod tests;
