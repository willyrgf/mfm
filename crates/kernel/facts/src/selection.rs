use std::cmp::Ordering;

use mfm_canonical::{CanonicalValue, ValidatedCanonicalValueV1};
use mfm_ids::{
    ContentRef, FactContentIdentityDigest, FactLogicalIdentityDigest, FactQueryDigest, SchemaId,
};

use crate::codec;
use crate::{
    CanonicalFactPredicate, FactError, FactSubject, Result, MAX_FACT_SELECTION_LIMIT,
    MAX_FACT_SELECTION_QUERIES,
};

const FACT_SELECTION_QUERY_CONTRACT: &str = "mfm.fact-selection-query.v1";
const FACT_SELECTION_REQUEST_CONTRACT: &str = "mfm.fact-selection-request.v1";
const REQUEST_VERSION: &str = "mfm.fact-selection-request.v1";
const PRODUCER_SCOPE: &str = "other_runs_in_tenant_scope";

/// Closed producer scope for recoverability-v1 fact selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    fn parse(value: &str) -> Result<Self> {
        match value {
            "ascending" => Ok(Self::Ascending),
            "descending" => Ok(Self::Descending),
            _ => Err(FactError::Canonical),
        }
    }
}

/// Deterministic tie-break over frozen fact logical identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    fn parse(value: &str) -> Result<Self> {
        match value {
            "fact_identity_ascending" => Ok(Self::FactIdentityAscending),
            "fact_identity_descending" => Ok(Self::FactIdentityDescending),
            _ => Err(FactError::Canonical),
        }
    }
}

/// Bounded result limit for one fact-selection query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    fn as_usize(self) -> usize {
        usize::from(self.0)
    }
}

impl TryFrom<u32> for FactSelectionLimit {
    type Error = FactError;

    fn try_from(value: u32) -> Result<Self> {
        Self::new(value)
    }
}

/// One annex-backed query within a reserved fact-selection request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionQuery {
    validated: ValidatedCanonicalValueV1,
    descriptor_ref: ContentRef,
    predicate: CanonicalFactPredicate,
    content_identity_filter: Option<FactContentIdentityDigest>,
    ordering: FactOrdering,
    limit: FactSelectionLimit,
    tie_break: FactTieBreak,
}

impl FactSelectionQuery {
    /// Constructs and annex-validates one query.
    pub fn new(
        descriptor_ref: ContentRef,
        predicate: CanonicalFactPredicate,
        content_identity_filter: Option<FactContentIdentityDigest>,
        ordering: FactOrdering,
        limit: FactSelectionLimit,
        tie_break: FactTieBreak,
    ) -> Result<Self> {
        let mut fields = vec![
            (
                "fact_descriptor_ref",
                codec::canonical_content_ref(&descriptor_ref)?,
            ),
            ("canonical_predicate", predicate.canonical_value()?),
            (
                "ordering",
                CanonicalValue::String(ordering.as_str().to_owned()),
            ),
            ("limit", CanonicalValue::Unsigned(u64::from(limit.get()))),
            (
                "tie_break",
                CanonicalValue::String(tie_break.as_str().to_owned()),
            ),
        ];
        if let Some(identity) = &content_identity_filter {
            fields.push((
                "content_identity_filter",
                CanonicalValue::String(identity.as_str().to_owned()),
            ));
        }
        Self::from_canonical_value(codec::object(fields)?)
    }

    /// Strictly decodes exact canonical JSON under the frozen query schema.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        Self::from_validated(codec::strict_decode(FACT_SELECTION_QUERY_CONTRACT, bytes)?)
    }

    /// Encodes a canonical value only after frozen-schema validation.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        Self::from_validated(codec::encode(FACT_SELECTION_QUERY_CONTRACT, &value)?)
    }

    fn from_validated(validated: ValidatedCanonicalValueV1) -> Result<Self> {
        let value = validated
            .canonical_value()
            .map_err(FactError::Recoverability)?;
        let object = codec::required_object(&value)?;
        let descriptor_ref =
            codec::content_ref(codec::required_field(object, "fact_descriptor_ref")?)?;
        let predicate = CanonicalFactPredicate::from_canonical_value(
            codec::required_field(object, "canonical_predicate")?.clone(),
        )?;
        let content_identity_filter = codec::optional_field(object, "content_identity_filter")
            .map(|value| {
                FactContentIdentityDigest::parse(codec::string(value)?)
                    .map_err(|_| FactError::Identity)
            })
            .transpose()?;
        let ordering =
            FactOrdering::parse(codec::string(codec::required_field(object, "ordering")?)?)?;
        let limit_value: u32 = codec::unsigned(codec::required_field(object, "limit")?)?
            .try_into()
            .map_err(|_| FactError::Canonical)?;
        let limit = FactSelectionLimit::new(limit_value)?;
        let tie_break =
            FactTieBreak::parse(codec::string(codec::required_field(object, "tie_break")?)?)?;
        Ok(Self {
            validated,
            descriptor_ref,
            predicate,
            content_identity_filter,
            ordering,
            limit,
            tie_break,
        })
    }

    /// Returns the exact canonical JSON bytes.
    pub fn canonical_json(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Returns the frozen annex-derived query schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.validated.schema_id()
    }

    /// Returns the exact query content identity.
    pub fn content_ref(&self) -> Result<ContentRef> {
        codec::contract()?
            .content_ref(&self.validated)
            .map_err(Into::into)
    }

    /// Reconstructs the checked canonical query.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated
            .canonical_value()
            .map_err(FactError::Recoverability)
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

/// One closed, state-authored request for other-run facts in the admitted tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionRequest {
    validated: ValidatedCanonicalValueV1,
    queries: Vec<FactSelectionQuery>,
}

impl FactSelectionRequest {
    /// Constructs an annex-backed request while preserving authored query order.
    pub fn new(queries: Vec<FactSelectionQuery>) -> Result<Self> {
        if queries.is_empty() || queries.len() > MAX_FACT_SELECTION_QUERIES {
            return Err(FactError::Selection(
                "fact selection request must contain 1 through 128 queries",
            ));
        }
        let query_values = queries
            .iter()
            .map(FactSelectionQuery::canonical_value)
            .collect::<Result<Vec<_>>>()?;
        Self::from_canonical_value(codec::object([
            (
                "version",
                CanonicalValue::String(REQUEST_VERSION.to_owned()),
            ),
            (
                "producer_scope",
                CanonicalValue::String(PRODUCER_SCOPE.to_owned()),
            ),
            ("queries", CanonicalValue::Array(query_values)),
        ])?)
    }

    /// Strictly decodes exact canonical JSON under the frozen request schema.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        Self::from_validated(codec::strict_decode(
            FACT_SELECTION_REQUEST_CONTRACT,
            bytes,
        )?)
    }

    /// Encodes a canonical value only after frozen-schema validation.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        Self::from_validated(codec::encode(FACT_SELECTION_REQUEST_CONTRACT, &value)?)
    }

    fn from_validated(validated: ValidatedCanonicalValueV1) -> Result<Self> {
        let value = validated
            .canonical_value()
            .map_err(FactError::Recoverability)?;
        let object = codec::required_object(&value)?;
        if codec::string(codec::required_field(object, "version")?)? != REQUEST_VERSION
            || codec::string(codec::required_field(object, "producer_scope")?)? != PRODUCER_SCOPE
        {
            return Err(FactError::Canonical);
        }
        let query_values = match codec::required_field(object, "queries")? {
            CanonicalValue::Array(values) => values,
            _ => return Err(FactError::Canonical),
        };
        if query_values.is_empty() || query_values.len() > MAX_FACT_SELECTION_QUERIES {
            return Err(FactError::Selection(
                "fact selection request must contain 1 through 128 queries",
            ));
        }
        let queries = query_values
            .iter()
            .cloned()
            .map(FactSelectionQuery::from_canonical_value)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { validated, queries })
    }

    /// Returns the exact canonical JSON bytes used as request evidence.
    pub fn canonical_json(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Returns the frozen annex-derived request schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.validated.schema_id()
    }

    /// Returns the exact request content identity.
    pub fn content_ref(&self) -> Result<ContentRef> {
        codec::contract()?
            .content_ref(&self.validated)
            .map_err(Into::into)
    }

    /// Reconstructs the checked canonical request.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated
            .canonical_value()
            .map_err(FactError::Recoverability)
    }

    /// Derives the frozen domain-separated request digest.
    pub fn request_digest(&self) -> Result<FactQueryDigest> {
        codec::contract()?
            .derive_fact_query_digest(&self.validated)
            .map_err(Into::into)
    }

    /// Returns the fixed tenant-relative producer scope.
    pub const fn producer_scope(&self) -> FactProducerScope {
        FactProducerScope::OtherRunsInTenantScope
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

    /// Considers one candidate and returns whether it matched the query.
    pub fn consider(&mut self, candidate: FactCandidate<T>) -> bool {
        if !self.query.matches(&candidate) {
            return false;
        }
        let insertion = self
            .selected
            .binary_search_by(|existing| self.query.compare(existing, &candidate))
            .unwrap_or_else(|index| index);
        if insertion < self.query.limit.as_usize() {
            self.selected.insert(insertion, candidate);
            if self.selected.len() > self.query.limit.as_usize() {
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
