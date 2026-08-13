#![warn(missing_docs)]
//! Strict prior-fact values used by capability preparation and conclusion closures.

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::string_contains_secret_marker;
use serde::{Deserialize, Serialize};

/// Maximum producer/source identities in one fact request.
pub const MAX_FACT_SOURCES: usize = 64;
/// Maximum selected facts in one response.
pub const MAX_SELECTED_FACTS: usize = 256;
/// Maximum canonical bytes in one fact value.
pub const MAX_FACT_VALUE_BYTES: usize = 4 * 1024 * 1024;

fn contains_secret_marker(input: &str) -> bool {
    string_contains_secret_marker(input)
}

/// Redaction-safe fact contract error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FactError {
    /// A bounded identity or request field is invalid.
    #[error("fact contract is invalid")]
    Invalid,
    /// Canonical fact bytes are malformed, non-canonical, or oversize.
    #[error("fact value is invalid")]
    Value,
    /// The selected response is incomplete or does not bind to its request.
    #[error("fact response is not complete")]
    Incomplete,
}

/// One callback-free request for prior facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FactSelectionRequest {
    /// Stable capability-owned request identity.
    pub request_id: StableId,
    /// Ordered producer/source identities.
    pub source_refs: Vec<ContentRef>,
    /// Canonical subject identity, not a raw provider query.
    pub subject_ref: ContentRef,
    /// Optional producer frontier captured before the preparation is committed.
    pub frontier: Option<FactSelectionFrontier>,
}

/// Immutable producer frontier fixed for an interpretation-only fact selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FactSelectionFrontier {
    /// Producer stream identity.
    pub stream_ref: ContentRef,
    /// Dense producer sequence observed by the selection.
    pub through_sequence: u64,
    /// Content identity of the producer head at the captured sequence.
    pub head_ref: ContentRef,
}

impl FactSelectionFrontier {
    /// Creates one positive, bounded producer frontier.
    pub fn new(
        stream_ref: ContentRef,
        through_sequence: u64,
        head_ref: ContentRef,
    ) -> Result<Self, FactError> {
        if through_sequence == 0 {
            return Err(FactError::Invalid);
        }
        Ok(Self {
            stream_ref,
            through_sequence,
            head_ref,
        })
    }

    /// Validates a frontier decoded from retained canonical bytes.
    pub fn validate(&self) -> std::result::Result<(), FactError> {
        (self.through_sequence > 0)
            .then_some(())
            .ok_or(FactError::Invalid)
    }
}

impl FactSelectionRequest {
    /// Creates one bounded request with declaration-preserved source order.
    pub fn new(
        request_id: StableId,
        source_refs: Vec<ContentRef>,
        subject_ref: ContentRef,
    ) -> Result<Self, FactError> {
        if source_refs.is_empty() || source_refs.len() > MAX_FACT_SOURCES {
            return Err(FactError::Invalid);
        }
        if source_refs.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(FactError::Invalid);
        }
        Ok(Self {
            request_id,
            source_refs,
            subject_ref,
            frontier: None,
        })
    }

    /// Attaches the exact producer frontier that a Store fact selection must attest.
    pub fn with_frontier(mut self, frontier: FactSelectionFrontier) -> Result<Self, FactError> {
        frontier.validate()?;
        self.frontier = Some(frontier);
        Ok(self)
    }

    /// Validates a request decoded from retained canonical bytes.
    pub fn validate(&self) -> std::result::Result<(), FactError> {
        if self.source_refs.is_empty()
            || self.source_refs.len() > MAX_FACT_SOURCES
            || self.source_refs.windows(2).any(|pair| pair[0] >= pair[1])
            || self
                .frontier
                .as_ref()
                .is_some_and(|frontier| frontier.validate().is_err())
        {
            return Err(FactError::Invalid);
        }
        Ok(())
    }
}

/// One selected fact with canonical, secret-free value bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct SelectedFact {
    /// Producer/source identity selected for this position.
    pub source_ref: ContentRef,
    /// Stable logical fact identity.
    pub fact_id: StableId,
    /// Nominal fact value contract and byte identity.
    pub value_ref: ContentRef,
    /// Canonical value bytes retained for replay.
    pub canonical_value: String,
}

impl SelectedFact {
    /// Creates one selected fact and verifies its content identity.
    pub fn new(
        source_ref: ContentRef,
        fact_id: StableId,
        value_ref: ContentRef,
        canonical_value: String,
    ) -> Result<Self, FactError> {
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(canonical_value.as_bytes())
                .map_err(|_| FactError::Value)?;
        if canonical.as_str() != canonical_value
            || canonical_value.len() > MAX_FACT_VALUE_BYTES
            || value_ref.content_digest() != &raw_content_digest(canonical.as_bytes())
            || contains_secret_marker(canonical.as_str())
        {
            return Err(FactError::Value);
        }
        Ok(Self {
            source_ref,
            fact_id,
            value_ref,
            canonical_value,
        })
    }

    /// Validates a selected fact decoded from retained canonical bytes.
    pub fn validate(&self) -> std::result::Result<(), FactError> {
        Self::new(
            self.source_ref.clone(),
            self.fact_id.clone(),
            self.value_ref.clone(),
            self.canonical_value.clone(),
        )
        .map(|_| ())
    }
}

/// Dense completeness coordinate captured at preparation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FactCompleteness {
    /// Highest source sequence included by the response.
    pub through_sequence: u64,
    /// Whether all requested source identities through the coordinate were present.
    pub complete: bool,
}

/// One callback-free response selected from retained fact producers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FactSelection {
    /// Exact request identity.
    pub request_id: StableId,
    /// Selected facts in request order.
    pub facts: Vec<SelectedFact>,
    /// Dense completeness proof.
    pub completeness: FactCompleteness,
    /// The producer frontier copied from the request, when one was fixed.
    pub frontier: Option<FactSelectionFrontier>,
}

impl FactSelection {
    /// Creates one complete selection for a request.
    pub fn new(
        request: &FactSelectionRequest,
        facts: Vec<SelectedFact>,
        completeness: FactCompleteness,
    ) -> Result<Self, FactError> {
        request.validate()?;
        if facts.len() > MAX_SELECTED_FACTS
            || !completeness.complete
            || completeness.through_sequence == 0
            || request
                .frontier
                .as_ref()
                .is_some_and(|frontier| frontier.through_sequence != completeness.through_sequence)
            || facts.len() != request.source_refs.len()
            || facts.iter().any(|fact| fact.validate().is_err())
            || facts
                .iter()
                .zip(&request.source_refs)
                .any(|(fact, source)| &fact.source_ref != source)
            || facts
                .windows(2)
                .any(|pair| pair[0].fact_id >= pair[1].fact_id)
        {
            return Err(FactError::Incomplete);
        }
        Ok(Self {
            request_id: request.request_id.clone(),
            facts,
            completeness,
            frontier: request.frontier.clone(),
        })
    }

    /// Validates a decoded response against its exact request and frontier.
    pub fn validate_for(
        &self,
        request: &FactSelectionRequest,
    ) -> std::result::Result<(), FactError> {
        request.validate()?;
        if self.request_id != request.request_id
            || self.frontier != request.frontier
            || self.facts.len() > MAX_SELECTED_FACTS
            || !self.completeness.complete
            || self.completeness.through_sequence == 0
            || request.frontier.as_ref().is_some_and(|frontier| {
                frontier.through_sequence != self.completeness.through_sequence
            })
            || self.facts.len() != request.source_refs.len()
            || self.facts.iter().any(|fact| fact.validate().is_err())
            || self
                .facts
                .iter()
                .zip(&request.source_refs)
                .any(|(fact, source)| &fact.source_ref != source)
            || self
                .facts
                .windows(2)
                .any(|pair| pair[0].fact_id >= pair[1].fact_id)
        {
            return Err(FactError::Incomplete);
        }
        Ok(())
    }
}

/// One fact publication coordinate retained with a State conclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FactPublication {
    /// Tenant-local publication sequence.
    pub publication_sequence: u64,
    /// Content identity of the accepted selection.
    pub selection_ref: ContentRef,
}

/// Returns the fixed schema identity used for a fact selection object closure.
pub fn selection_schema() -> Result<SchemaId, FactError> {
    SchemaId::new(
        "mfm.fact-selection",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| FactError::Invalid)
}

/// Computes a content reference for one already validated canonical fact value.
pub fn content_ref_for_value(canonical_value: &[u8]) -> Result<ContentRef, FactError> {
    ContentRef::new(selection_schema()?, raw_content_digest(canonical_value))
        .map_err(|_| FactError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(seed: u8) -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                "mfm.fact-test-source",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([seed; 32]),
            )
            .expect("schema"),
            raw_content_digest(&[seed]),
        )
        .expect("reference")
    }

    #[test]
    fn request_rejects_empty_source_frontier() {
        assert!(FactSelectionRequest::new(
            StableId::new("mfm.fact-test-request").expect("request"),
            Vec::new(),
            reference(3),
        )
        .is_err());
    }

    #[test]
    fn selection_copies_and_checks_the_preparation_frontier() {
        let source = reference(1);
        let request = FactSelectionRequest::new(
            StableId::new("mfm.fact-test-request").expect("request"),
            vec![source.clone()],
            reference(3),
        )
        .expect("request")
        .with_frontier(FactSelectionFrontier::new(reference(4), 7, reference(5)).expect("frontier"))
        .expect("frontier");
        let value_ref = content_ref_for_value(br#"{"value":1}"#).expect("value ref");
        let fact = SelectedFact::new(
            source,
            StableId::new("mfm.fact-test-fact").expect("fact"),
            value_ref,
            r#"{"value":1}"#.to_owned(),
        )
        .expect("fact");
        let selection = FactSelection::new(
            &request,
            vec![fact],
            FactCompleteness {
                through_sequence: 7,
                complete: true,
            },
        )
        .expect("selection");
        assert_eq!(selection.frontier, request.frontier);
        assert!(FactSelection::new(
            &request,
            selection.facts.clone(),
            FactCompleteness {
                through_sequence: 8,
                complete: true,
            },
        )
        .is_err());
    }
}
