use std::collections::BTreeSet;

use mfm_canonical::CanonicalJsonBytes;
use mfm_ids::ContentDigest;

use crate::*;

/// One descriptor-field predicate requested for a fact query.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactQueryPredicate {
    pub(crate) field_id: FactFieldId,
    pub(crate) operator: FactQueryOperator,
    pub(crate) value: FactCanonicalScalar,
}

impl FactQueryPredicate {
    /// Creates a fact query predicate.
    pub fn new(
        field_id: FactFieldId,
        operator: FactQueryOperator,
        value: FactCanonicalScalar,
    ) -> Self {
        Self {
            field_id,
            operator,
            value,
        }
    }

    /// Returns the descriptor field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns the query operator.
    pub const fn operator(&self) -> FactQueryOperator {
        self.operator
    }

    /// Returns the predicate scalar.
    pub const fn value(&self) -> &FactCanonicalScalar {
        &self.value
    }

    /// Returns whether an actual scalar satisfies this predicate.
    pub fn matches_scalar(&self, actual: &FactCanonicalScalar) -> bool {
        actual.matches_query_operator(self.operator, &self.value)
    }
}

/// Descriptor-scoped fact query request accepted by the v3 compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryInput {
    pub(crate) predicates: Vec<FactQueryPredicate>,
    pub(crate) return_fields: Vec<FactFieldId>,
    pub(crate) ordering: FactOrderingName,
    pub(crate) limit: Option<u64>,
    pub(crate) content_identity: Option<FactContentIdentityEvidence>,
}

impl FactQueryInput {
    /// Creates a descriptor-scoped query compiler input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        predicates: Vec<FactQueryPredicate>,
        return_fields: Vec<FactFieldId>,
        ordering: FactOrderingName,
        limit: Option<u64>,
    ) -> Result<Self> {
        if return_fields.is_empty() {
            return Err(FactError::descriptor(
                "fact query must request at least one return field",
            ));
        }
        if limit == Some(0) {
            return Err(FactError::descriptor(
                "fact query limit must be non-zero when present",
            ));
        }
        let mut seen_predicates = BTreeSet::new();
        for predicate in &predicates {
            if !seen_predicates.insert((
                predicate.field_id.clone(),
                predicate.operator,
                predicate.value.clone(),
            )) {
                return Err(FactError::field(
                    predicate.field_id.clone(),
                    "duplicate fact query predicate",
                ));
            }
        }
        let mut seen_return_fields = BTreeSet::new();
        for field_id in &return_fields {
            if !seen_return_fields.insert(field_id.clone()) {
                return Err(FactError::field(
                    field_id.clone(),
                    "duplicate fact query return field",
                ));
            }
        }
        Ok(Self {
            predicates,
            return_fields,
            ordering,
            limit,
            content_identity: None,
        })
    }

    /// Narrows this query to projections carrying one verified fact-content identity.
    ///
    /// The compiler binds the evidence to the resolved descriptor. Providers apply the compact
    /// filter before ordering and limiting; consumers must still hydrate and reverify the result.
    pub fn with_content_identity(mut self, content_identity: FactContentIdentityEvidence) -> Self {
        self.content_identity = Some(content_identity);
        self
    }

    /// Returns requested predicates in caller order.
    pub fn predicates(&self) -> &[FactQueryPredicate] {
        &self.predicates
    }

    /// Returns requested result fields in caller order.
    pub fn return_fields(&self) -> &[FactFieldId] {
        &self.return_fields
    }

    /// Returns the selected descriptor ordering name.
    pub const fn ordering(&self) -> &FactOrderingName {
        &self.ordering
    }

    /// Returns the optional result limit.
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Returns the optional fact-content identity narrowing evidence.
    pub const fn content_identity(&self) -> Option<&FactContentIdentityEvidence> {
        self.content_identity.as_ref()
    }
}

/// Parsed descriptor-scoped query shape from canonical query bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFactQueryShape {
    pub(crate) predicates: Vec<FactQueryPredicate>,
    pub(crate) return_fields: Vec<FactFieldId>,
    pub(crate) content_identity: Option<FactContentIdentityEvidence>,
}

impl CompiledFactQueryShape {
    pub(crate) fn new(
        predicates: Vec<FactQueryPredicate>,
        return_fields: Vec<FactFieldId>,
        content_identity: Option<FactContentIdentityEvidence>,
    ) -> Result<Self> {
        if return_fields.is_empty() {
            return Err(FactError::descriptor(
                "compiled fact query shape must contain return fields",
            ));
        }
        Ok(Self {
            predicates,
            return_fields,
            content_identity,
        })
    }

    /// Returns canonical predicates.
    pub fn predicates(&self) -> &[FactQueryPredicate] {
        &self.predicates
    }

    /// Returns requested return fields.
    pub fn return_fields(&self) -> &[FactFieldId] {
        &self.return_fields
    }

    /// Returns the optional exact fact-content identity filter.
    pub const fn content_identity(&self) -> Option<&FactContentIdentityEvidence> {
        self.content_identity.as_ref()
    }
}

/// Canonical single-descriptor v2 fact query plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFactQueryPlan {
    pub(crate) query_compiler_version: FactQueryCompilerVersion,
    pub(crate) canonicalizer_version: FactCanonicalizerVersion,
    pub(crate) resolved_descriptor: ContentDigest,
    pub(crate) canonical_query: CanonicalJsonBytes,
    pub(crate) canonical_query_hash: ContentDigest,
    pub(crate) ordering: FactOrderingPolicy,
    pub(crate) limit: Option<u64>,
}

impl CanonicalFactQueryPlan {
    pub(crate) fn new(
        query_compiler_version: FactQueryCompilerVersion,
        canonicalizer_version: FactCanonicalizerVersion,
        resolved_descriptor: ContentDigest,
        canonical_query: CanonicalJsonBytes,
        ordering: FactOrderingPolicy,
        limit: Option<u64>,
    ) -> Result<Self> {
        if limit == Some(0) {
            return Err(FactError::descriptor(
                "fact query limit must be non-zero when present",
            ));
        }
        let canonical_query_hash = canonical_query.content_digest();
        Ok(Self {
            query_compiler_version,
            canonicalizer_version,
            resolved_descriptor,
            canonical_query,
            canonical_query_hash,
            ordering,
            limit,
        })
    }

    /// Returns the query compiler version.
    pub const fn query_compiler_version(&self) -> &FactQueryCompilerVersion {
        &self.query_compiler_version
    }

    /// Returns the canonicalizer version.
    pub const fn canonicalizer_version(&self) -> &FactCanonicalizerVersion {
        &self.canonicalizer_version
    }

    /// Returns the resolved descriptor hash.
    pub const fn resolved_descriptor(&self) -> &ContentDigest {
        &self.resolved_descriptor
    }

    /// Returns canonical query bytes.
    pub const fn canonical_query(&self) -> &CanonicalJsonBytes {
        &self.canonical_query
    }

    /// Returns the canonical query hash.
    pub const fn canonical_query_hash(&self) -> &ContentDigest {
        &self.canonical_query_hash
    }

    /// Returns the selected ordering.
    pub const fn ordering(&self) -> &FactOrderingPolicy {
        &self.ordering
    }

    /// Returns the optional query limit.
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }
}
