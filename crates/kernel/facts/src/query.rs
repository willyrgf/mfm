use std::collections::BTreeSet;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::ContentDigest;

use crate::*;

/// Query-time visibility scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryScope {
    pub(crate) audience: FactAudience,
    pub(crate) scope: FactVisibilityScope,
}

impl FactQueryScope {
    /// Creates a fact query scope.
    pub const fn new(audience: FactAudience, scope: FactVisibilityScope) -> Self {
        Self { audience, scope }
    }

    /// Returns the query audience.
    pub const fn audience(&self) -> FactAudience {
        self.audience
    }

    /// Returns the query visibility scope.
    pub const fn scope(&self) -> FactVisibilityScope {
        self.scope
    }
}

/// Scope decision evidence bound into a canonical fact query plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDecisionEvidence {
    pub(crate) decision_hash: ContentDigest,
}

impl ScopeDecisionEvidence {
    /// Creates scope decision evidence from a policy or authorization digest.
    pub const fn new(decision_hash: ContentDigest) -> Self {
        Self { decision_hash }
    }

    /// Returns the decision digest.
    pub const fn decision_hash(&self) -> &ContentDigest {
        &self.decision_hash
    }
}

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

/// One descriptor field requested in fact query results.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactQueryReturnField {
    pub(crate) field_id: FactFieldId,
}

impl FactQueryReturnField {
    /// Creates a requested return field.
    pub fn new(field_id: FactFieldId) -> Self {
        Self { field_id }
    }

    /// Returns the descriptor field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }
}

/// Descriptor-scoped fact query request accepted by the v1 compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryInput {
    pub(crate) store_scope: StoreScopeRef,
    pub(crate) query_scope: FactQueryScope,
    pub(crate) scope_decision_evidence: ScopeDecisionEvidence,
    pub(crate) predicates: Vec<FactQueryPredicate>,
    pub(crate) return_fields: Vec<FactQueryReturnField>,
    pub(crate) ordering: FactOrderingName,
    pub(crate) limit: Option<u64>,
}

impl FactQueryInput {
    /// Creates a descriptor-scoped query compiler input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope: StoreScopeRef,
        query_scope: FactQueryScope,
        scope_decision_evidence: ScopeDecisionEvidence,
        predicates: Vec<FactQueryPredicate>,
        return_fields: Vec<FactQueryReturnField>,
        ordering: FactOrderingName,
        limit: Option<u64>,
    ) -> Result<Self> {
        if return_fields.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "fact query must request at least one return field",
            ));
        }
        if limit == Some(0) {
            return Err(FactDescriptorError::descriptor(
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
                return Err(FactDescriptorError::field(
                    predicate.field_id.clone(),
                    "duplicate fact query predicate",
                ));
            }
        }
        let mut seen_return_fields = BTreeSet::new();
        for field in &return_fields {
            if !seen_return_fields.insert(field.field_id.clone()) {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "duplicate fact query return field",
                ));
            }
        }
        Ok(Self {
            store_scope,
            query_scope,
            scope_decision_evidence,
            predicates,
            return_fields,
            ordering,
            limit,
        })
    }

    /// Returns the store scope.
    pub const fn store_scope(&self) -> &StoreScopeRef {
        &self.store_scope
    }

    /// Returns the query scope.
    pub const fn query_scope(&self) -> &FactQueryScope {
        &self.query_scope
    }

    /// Returns scope decision evidence.
    pub const fn scope_decision_evidence(&self) -> &ScopeDecisionEvidence {
        &self.scope_decision_evidence
    }

    /// Returns requested predicates in caller order.
    pub fn predicates(&self) -> &[FactQueryPredicate] {
        &self.predicates
    }

    /// Returns requested result fields in caller order.
    pub fn return_fields(&self) -> &[FactQueryReturnField] {
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
}

/// Parsed descriptor-scoped query shape from canonical query bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFactQueryShape {
    pub(crate) predicates: Vec<FactQueryPredicate>,
    pub(crate) return_fields: Vec<FactQueryReturnField>,
}

impl CompiledFactQueryShape {
    /// Creates a parsed query shape.
    pub(crate) fn new(
        predicates: Vec<FactQueryPredicate>,
        return_fields: Vec<FactQueryReturnField>,
    ) -> Result<Self> {
        if return_fields.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "compiled fact query shape must contain return fields",
            ));
        }
        Ok(Self {
            predicates,
            return_fields,
        })
    }

    /// Returns canonical predicates.
    pub fn predicates(&self) -> &[FactQueryPredicate] {
        &self.predicates
    }

    /// Returns requested return fields.
    pub fn return_fields(&self) -> &[FactQueryReturnField] {
        &self.return_fields
    }
}

/// Canonical single-descriptor v1 fact query plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFactQueryPlan {
    pub(crate) store_scope: StoreScopeRef,
    pub(crate) query_scope: FactQueryScope,
    pub(crate) query_compiler_version: FactQueryCompilerVersion,
    pub(crate) canonicalizer_version: FactCanonicalizerVersion,
    pub(crate) resolved_descriptor: ContentDigest,
    pub(crate) scope_decision_evidence: ScopeDecisionEvidence,
    pub(crate) canonical_query: PlainCanonicalJsonBytes,
    pub(crate) canonical_query_hash: ContentDigest,
    pub(crate) ordering: FactOrderingPolicy,
    pub(crate) limit: Option<u64>,
}

impl CanonicalFactQueryPlan {
    /// Creates a canonical fact query plan and computes its query hash.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        store_scope: StoreScopeRef,
        query_scope: FactQueryScope,
        query_compiler_version: FactQueryCompilerVersion,
        canonicalizer_version: FactCanonicalizerVersion,
        resolved_descriptor: ContentDigest,
        scope_decision_evidence: ScopeDecisionEvidence,
        canonical_query: PlainCanonicalJsonBytes,
        ordering: FactOrderingPolicy,
        limit: Option<u64>,
    ) -> Result<Self> {
        if limit == Some(0) {
            return Err(FactDescriptorError::descriptor(
                "fact query limit must be non-zero when present",
            ));
        }
        let canonical_query_hash = canonical_query.content_digest();
        Ok(Self {
            store_scope,
            query_scope,
            query_compiler_version,
            canonicalizer_version,
            resolved_descriptor,
            scope_decision_evidence,
            canonical_query,
            canonical_query_hash,
            ordering,
            limit,
        })
    }

    /// Returns the store scope.
    pub const fn store_scope(&self) -> &StoreScopeRef {
        &self.store_scope
    }

    /// Returns the query scope.
    pub const fn query_scope(&self) -> &FactQueryScope {
        &self.query_scope
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

    /// Returns scope decision evidence.
    pub const fn scope_decision_evidence(&self) -> &ScopeDecisionEvidence {
        &self.scope_decision_evidence
    }

    /// Returns canonical query bytes.
    pub const fn canonical_query(&self) -> &PlainCanonicalJsonBytes {
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
