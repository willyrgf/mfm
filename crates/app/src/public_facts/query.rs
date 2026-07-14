use std::num::NonZeroU64;

use crate::{AppError, ErrorClass};

/// Public descriptor shape selector used to disambiguate fact descriptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicFactShapeSelector(String);

impl PublicFactShapeSelector {
    /// Creates a public shape selector from transport-provided text.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the selector text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn matches_descriptor(&self, descriptor: &mfm_facts::FactDescriptor) -> bool {
        descriptor.descriptor_schema_id().as_str() == self.as_str()
    }
}

/// Public fact query predicate.
pub type PublicFactPredicate = mfm_facts::FactQueryPredicate;

/// App-level public fact query request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicFactQueryRequest {
    /// Fact kind to query.
    pub fact_kind: mfm_facts::FactKind,
    /// Optional public shape selector matching the descriptor schema id.
    pub shape: Option<PublicFactShapeSelector>,
    /// Public predicates.
    pub predicates: Vec<PublicFactPredicate>,
    /// Returnable field ids to include in each fact.
    pub return_fields: Vec<mfm_facts::FactFieldId>,
    /// Descriptor ordering policy name.
    pub ordering: mfm_facts::FactOrderingName,
    /// Optional non-zero limit.
    pub limit: Option<NonZeroU64>,
}

impl PublicFactQueryRequest {
    /// Validates transport-independent public fact query invariants.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.return_fields.is_empty() {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "FactReturnFieldMissing",
                "Fact queries must request at least one return field",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
