use std::cmp::Ordering;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, FactContentIdentityDigest, FactLogicalIdentityDigest, FactQueryDigest};
use mfm_program_derive::{MfmValue, PersistedSchema};
use mfm_values::{
    CanonicalJsonPersistedSchema, FieldDescriptor, LiteralValue, MfmValue, SchemaIdentity,
    SchemaKind, SchemaShape, ValueError,
};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    CanonicalFactPredicate, FactError, FactSelectionCompletenessMode, FactSelectionScanBounds,
    FactSubject, Result, MAX_FACT_SELECTION_LIMIT, MAX_FACT_SELECTION_QUERIES,
};

const FACT_QUERY_DOMAIN: &str = "mfm.fact-query.v1";
const PRODUCER_SCOPE: &str = "other_runs_in_tenant_scope";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.prior-run-fact-selector-contract", version = "1")]
struct PriorRunFactSelectorDefinition {
    #[mfm(literal = "mfm.prior-run-fact-selector.v1")]
    contract: String,
    #[mfm(literal = "fact_top_k_v1")]
    evaluator: String,
    #[mfm(literal = "tenant_publication_then_fact_identity")]
    ordering: String,
    #[mfm(literal = "other_runs_in_tenant_scope")]
    producer_scope: String,
}

impl PriorRunFactSelectorDefinition {
    fn current() -> Self {
        Self {
            contract: "mfm.prior-run-fact-selector.v1".to_owned(),
            evaluator: "fact_top_k_v1".to_owned(),
            ordering: "tenant_publication_then_fact_identity".to_owned(),
            producer_scope: PRODUCER_SCOPE.to_owned(),
        }
    }
}

/// Returns the one fixed selector contract accepted by prior-run fact reads.
pub fn prior_run_fact_selector_contract_ref() -> Result<ContentRef> {
    PriorRunFactSelectorDefinition::current()
        .content_ref()
        .map_err(|_| FactError::Identity)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct PriorRunFactSelectorRef(ContentRef);

impl PriorRunFactSelectorRef {
    fn current() -> Result<Self> {
        prior_run_fact_selector_contract_ref().map(Self)
    }

    const fn as_ref(&self) -> &ContentRef {
        &self.0
    }
}

impl<'de> Deserialize<'de> for PriorRunFactSelectorRef {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let reference = ContentRef::deserialize(deserializer)?;
        let expected = prior_run_fact_selector_contract_ref().map_err(serde::de::Error::custom)?;
        if reference != expected {
            return Err(serde::de::Error::custom(
                "unsupported prior-run fact selector reference",
            ));
        }
        Ok(Self(reference))
    }
}

impl mfm_values::PersistedSchema for PriorRunFactSelectorRef {
    fn schema_identity() -> mfm_values::Result<SchemaIdentity> {
        let reference =
            prior_run_fact_selector_contract_ref().map_err(|_| ValueError::SchemaShapeMismatch)?;
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.prior-run-fact-selector-reference",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| ValueError::Identity(error.to_string()))?,
            SchemaShape::named_struct(vec![
                FieldDescriptor::required(
                    "content_digest",
                    SchemaShape::Literal(LiteralValue::String(
                        reference.content_digest().as_str().to_owned(),
                    )),
                ),
                FieldDescriptor::required(
                    "schema_id",
                    SchemaShape::Literal(LiteralValue::String(
                        reference.schema_id().as_str().to_owned(),
                    )),
                ),
            ])?,
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        if prior_run_fact_selector_contract_ref().as_ref() == Ok(&self.0) {
            Ok(())
        } else {
            Err(ValueError::SchemaShapeMismatch)
        }
    }
}

/// Closed producer scope for prior-run fact selection.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    PersistedSchema,
)]
#[serde(rename_all = "snake_case")]
#[mfm(schema = "mfm.fact-producer-scope", version = "1")]
pub enum FactProducerScope {
    /// Facts emitted by other runs admitted in the same tenant scope.
    OtherRunsInTenantScope,
}

impl FactProducerScope {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OtherRunsInTenantScope => PRODUCER_SCOPE,
        }
    }
}

/// Primary ordering over dense tenant fact publication order.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    PersistedSchema,
)]
#[serde(rename_all = "snake_case")]
#[mfm(schema = "mfm.fact-ordering", version = "1")]
pub enum FactOrdering {
    /// Oldest publication first.
    Ascending,
    /// Newest publication first.
    Descending,
}

impl FactOrdering {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ascending => "ascending",
            Self::Descending => "descending",
        }
    }
}

/// Deterministic tie-break over frozen fact logical identity.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    PersistedSchema,
)]
#[serde(rename_all = "snake_case")]
#[mfm(schema = "mfm.fact-tie-break", version = "1")]
pub enum FactTieBreak {
    /// Lowest fact logical identity first.
    FactIdentityAscending,
    /// Highest fact logical identity first.
    FactIdentityDescending,
}

impl FactTieBreak {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FactIdentityAscending => "fact_identity_ascending",
            Self::FactIdentityDescending => "fact_identity_descending",
        }
    }
}

/// Bounded result limit for one fact-selection query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, PersistedSchema)]
#[serde(transparent)]
#[mfm(
    schema = "mfm.fact-selection-limit",
    version = "1",
    unsigned_minimum = 1,
    unsigned_maximum = 128
)]
pub struct FactSelectionLimit(u8);

impl FactSelectionLimit {
    /// Creates a limit in the frozen inclusive range `1..=128`.
    pub fn new(value: u32) -> Result<Self> {
        if !(1..=MAX_FACT_SELECTION_LIMIT).contains(&value) {
            return Err(FactError::Selection(
                "fact selection limit must be 1 through 128",
            ));
        }
        Ok(Self(value as u8))
    }

    /// Returns the selected limit.
    pub const fn get(self) -> u32 {
        self.0 as u32
    }
}

impl TryFrom<u32> for FactSelectionLimit {
    type Error = FactError;

    fn try_from(value: u32) -> Result<Self> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for FactSelectionLimit {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u32::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// One owner-validated query within a reserved fact-selection request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.fact-selection-query", version = "1")]
pub struct FactSelectionQuery {
    #[serde(rename = "fact_descriptor_ref")]
    descriptor_ref: ContentRef,
    #[serde(rename = "canonical_predicate")]
    predicate: CanonicalFactPredicate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_identity_filter: Option<FactContentIdentityDigest>,
    ordering: FactOrdering,
    limit: FactSelectionLimit,
    tie_break: FactTieBreak,
}

impl FactSelectionQuery {
    /// Constructs and validates one query.
    pub fn new(
        descriptor_ref: ContentRef,
        predicate: CanonicalFactPredicate,
        content_identity_filter: Option<FactContentIdentityDigest>,
        ordering: FactOrdering,
        limit: FactSelectionLimit,
        tie_break: FactTieBreak,
    ) -> Result<Self> {
        let query = Self {
            descriptor_ref,
            predicate,
            content_identity_filter,
            ordering,
            limit,
            tie_break,
        };
        mfm_values::PersistedSchema::validate(&query).map_err(|_| FactError::Canonical)?;
        Ok(query)
    }

    /// Returns the exact retained fact descriptor.
    pub const fn descriptor_ref(&self) -> &ContentRef {
        &self.descriptor_ref
    }

    /// Returns the exact canonical subject predicate.
    pub const fn predicate(&self) -> &CanonicalFactPredicate {
        &self.predicate
    }

    /// Returns the optional exact content-identity filter.
    pub const fn content_identity_filter(&self) -> Option<&FactContentIdentityDigest> {
        self.content_identity_filter.as_ref()
    }

    /// Returns primary fact-publication ordering.
    pub const fn ordering(&self) -> FactOrdering {
        self.ordering
    }

    /// Returns the bounded result limit.
    pub const fn limit(&self) -> FactSelectionLimit {
        self.limit
    }

    /// Returns deterministic logical-identity tie-break ordering.
    pub const fn tie_break(&self) -> FactTieBreak {
        self.tie_break
    }

    /// Returns whether one verified candidate qualifies for this query.
    pub fn matches<T>(&self, candidate: &FactCandidate<T>) -> bool {
        &candidate.descriptor_ref == self.descriptor_ref()
            && self.predicate.matches(&candidate.subject)
            && self
                .content_identity_filter()
                .is_none_or(|expected| expected == &candidate.content_identity)
    }

    fn compare<T>(&self, left: &FactCandidate<T>, right: &FactCandidate<T>) -> Ordering {
        let publication = left.publication_order.cmp(&right.publication_order);
        let publication = match self.ordering {
            FactOrdering::Ascending => publication,
            FactOrdering::Descending => publication.reverse(),
        };
        publication.then_with(|| {
            let identity = left.fact_identity.cmp(&right.fact_identity);
            match self.tie_break {
                FactTieBreak::FactIdentityAscending => identity,
                FactTieBreak::FactIdentityDescending => identity.reverse(),
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(schema = "mfm.fact-selection-request-version", version = "1")]
enum FactSelectionRequestVersion {
    #[serde(rename = "mfm.fact-selection-request.v1")]
    V1,
}

/// One closed, state-authored request for other-run facts in the admitted tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.fact",
    name = "selection-request",
    version = "1",
    schema = "mfm.fact.selection_request"
)]
pub struct FactSelectionRequest {
    admitted_source_manifest_ref: ContentRef,
    #[mfm(persisted)]
    producer_scope: FactProducerScope,
    #[mfm(persisted)]
    completeness_mode: FactSelectionCompletenessMode,
    #[mfm(persisted)]
    scan_bounds: FactSelectionScanBounds,
    #[mfm(persisted)]
    selector_contract_ref: PriorRunFactSelectorRef,
    #[mfm(persisted, minimum_items = 1, maximum_items = 128)]
    queries: Vec<FactSelectionQuery>,
    #[mfm(persisted)]
    version: FactSelectionRequestVersion,
}

#[derive(Serialize)]
struct FactQueryPreimage<'a> {
    domain: &'static str,
    value: &'a FactSelectionRequest,
}

impl FactSelectionRequest {
    /// Strictly decodes exact canonical bytes through this concrete owner's shape.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| FactError::Canonical)?;
        <Self as MfmValue>::schema_descriptor()
            .and_then(|schema| {
                schema
                    .identity
                    .validate_canonical_value(canonical.as_bytes())
            })
            .map_err(|_| FactError::Canonical)?;
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| FactError::Canonical)
    }

    /// Constructs an owner-validated request while preserving authored query order.
    pub fn new(
        admitted_source_manifest_ref: ContentRef,
        scan_bounds: FactSelectionScanBounds,
        queries: Vec<FactSelectionQuery>,
    ) -> Result<Self> {
        scan_bounds.validate()?;
        if queries.is_empty() || queries.len() > MAX_FACT_SELECTION_QUERIES {
            return Err(FactError::Selection(
                "fact selection request must contain 1 through 128 queries",
            ));
        }
        Ok(Self {
            admitted_source_manifest_ref,
            producer_scope: FactProducerScope::OtherRunsInTenantScope,
            completeness_mode: FactSelectionCompletenessMode::CompleteThroughAuthorizationFrontier,
            scan_bounds,
            selector_contract_ref: PriorRunFactSelectorRef::current()?,
            queries,
            version: FactSelectionRequestVersion::V1,
        })
    }

    /// Derives the frozen domain-separated request digest from this typed owner.
    pub fn request_digest(&self) -> Result<FactQueryDigest> {
        let json = serde_json::to_string(&FactQueryPreimage {
            domain: FACT_QUERY_DOMAIN,
            value: self,
        })
        .map_err(|_| FactError::Canonical)?;
        let canonical =
            PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| FactError::Canonical)?;
        Ok(FactQueryDigest::from_digest(sha256_digest_bytes(
            canonical.as_bytes(),
        )))
    }

    /// Returns the fixed tenant-relative producer scope.
    pub const fn producer_scope(&self) -> FactProducerScope {
        self.producer_scope
    }

    /// Returns the sole completeness mode admitted by the scanner.
    pub const fn completeness_mode(&self) -> FactSelectionCompletenessMode {
        self.completeness_mode
    }

    /// Returns the exact source manifest admitted with this run.
    pub const fn admitted_source_manifest_ref(&self) -> &ContentRef {
        &self.admitted_source_manifest_ref
    }

    /// Returns the fixed selector contract frozen into this request.
    pub const fn selector_contract_ref(&self) -> &ContentRef {
        self.selector_contract_ref.as_ref()
    }

    /// Returns the explicit total scan bounds.
    pub const fn scan_bounds(&self) -> &FactSelectionScanBounds {
        &self.scan_bounds
    }

    /// Returns queries in state-authored order.
    pub fn queries(&self) -> &[FactSelectionQuery] {
        &self.queries
    }
}

/// Pure evaluator input for one structurally verified fact.
///
/// Plain publication order is comparison data only; this type carries no
/// store, object, journal, or completeness authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactCandidate<T> {
    descriptor_ref: ContentRef,
    subject: FactSubject,
    content_identity: FactContentIdentityDigest,
    fact_identity: FactLogicalIdentityDigest,
    publication_order: u64,
    value: T,
}

impl<T> FactCandidate<T> {
    /// Constructs one verified pure evaluator input.
    pub fn new(
        descriptor_ref: ContentRef,
        subject: FactSubject,
        content_identity: FactContentIdentityDigest,
        fact_identity: FactLogicalIdentityDigest,
        publication_order: u64,
        value: T,
    ) -> Self {
        Self {
            descriptor_ref,
            subject,
            content_identity,
            fact_identity,
            publication_order,
            value,
        }
    }

    /// Returns the exact fact descriptor.
    pub const fn descriptor_ref(&self) -> &ContentRef {
        &self.descriptor_ref
    }

    /// Returns exact canonical subject material.
    pub const fn subject(&self) -> &FactSubject {
        &self.subject
    }

    /// Returns the verified content identity.
    pub const fn content_identity(&self) -> &FactContentIdentityDigest {
        &self.content_identity
    }

    /// Returns the fact logical identity used for deterministic tie-break.
    pub const fn fact_identity(&self) -> &FactLogicalIdentityDigest {
        &self.fact_identity
    }

    /// Returns the dense publication order supplied by the store fold.
    pub const fn publication_order(&self) -> u64 {
        self.publication_order
    }

    /// Returns the caller-owned selected value.
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Consumes this candidate into the caller-owned selected value.
    pub fn into_value(self) -> T {
        self.value
    }
}

/// Bounded incremental top-K evaluator for one exact query.
///
/// It stores at most the query limit and grants no positive completeness
/// meaning; only the store-owned scan session may construct a response after
/// reaching its exact frontier.
#[derive(Debug, Clone)]
pub struct FactTopK<T> {
    query: FactSelectionQuery,
    selected: Vec<FactCandidate<T>>,
}

impl<T> FactTopK<T> {
    /// Starts an empty bounded evaluator.
    pub fn new(query: FactSelectionQuery) -> Self {
        Self {
            query,
            selected: Vec::new(),
        }
    }

    /// Returns whether one already verified candidate matches this accumulator's query.
    pub fn matches<U>(&self, candidate: &FactCandidate<U>) -> bool {
        self.query.matches(candidate)
    }

    /// Considers one candidate and returns whether it matched the query.
    pub fn consider(&mut self, candidate: FactCandidate<T>) -> bool {
        if !self.query.matches(&candidate) {
            return false;
        }
        let insertion = self
            .selected
            .binary_search_by(|existing| self.query.compare(existing, &candidate))
            .unwrap_or_else(|index| index);
        if insertion < usize::from(self.query.limit.0) {
            self.selected.insert(insertion, candidate);
            if self.selected.len() > usize::from(self.query.limit.0) {
                self.selected.pop();
            }
        }
        true
    }

    /// Returns the number of currently retained best candidates.
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    /// Returns whether no matching candidate has been retained.
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// Borrows current best candidates in final deterministic order.
    pub fn as_slice(&self) -> &[FactCandidate<T>] {
        &self.selected
    }

    /// Finishes the pure accumulator in deterministic query order.
    ///
    /// This operation does not assert that an authoritative scan reached its
    /// frontier and therefore cannot construct a journal response.
    pub fn finish(self) -> Vec<FactCandidate<T>> {
        self.selected
    }
}
