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
        })
    }

    /// Validates a request decoded from retained canonical bytes.
    pub fn validate(&self) -> std::result::Result<(), FactError> {
        if self.source_refs.is_empty()
            || self.source_refs.len() > MAX_FACT_SOURCES
            || self.source_refs.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(FactError::Invalid);
        }
        Ok(())
    }
}

/// One canonical, secret-free fact value selected or proposed by Store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FactValue {
    /// Producer/source identity selected for this position.
    pub source_ref: ContentRef,
    /// Canonical subject identity bound to this fact.
    pub subject_ref: ContentRef,
    /// Stable logical fact identity.
    pub fact_id: StableId,
    /// Nominal fact value contract and byte identity.
    pub value_ref: ContentRef,
    /// Canonical value bytes retained for replay.
    pub canonical_value: String,
}

impl FactValue {
    /// Creates one fact value and verifies its content identity.
    pub fn new(
        source_ref: ContentRef,
        subject_ref: ContentRef,
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
            subject_ref,
            fact_id,
            value_ref,
            canonical_value,
        })
    }

    /// Validates a fact decoded from retained canonical bytes.
    pub fn validate(&self) -> std::result::Result<(), FactError> {
        Self::new(
            self.source_ref.clone(),
            self.subject_ref.clone(),
            self.fact_id.clone(),
            self.value_ref.clone(),
            self.canonical_value.clone(),
        )
        .map(|_| ())
    }
}

/// One bounded set of new fact values emitted by a State conclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct FactProposalSet {
    /// New fact values in deterministic source/subject/id order.
    pub facts: Vec<FactValue>,
}

impl FactProposalSet {
    /// Creates one canonical proposal set without a publication coordinate.
    pub fn new(facts: Vec<FactValue>) -> Result<Self, FactError> {
        if facts.len() > MAX_SELECTED_FACTS
            || facts.iter().any(|fact| fact.validate().is_err())
            || facts.windows(2).any(|pair| {
                (
                    pair[0].source_ref.clone(),
                    pair[0].subject_ref.clone(),
                    pair[0].fact_id.clone(),
                ) >= (
                    pair[1].source_ref.clone(),
                    pair[1].subject_ref.clone(),
                    pair[1].fact_id.clone(),
                )
            })
        {
            return Err(FactError::Invalid);
        }
        Ok(Self { facts })
    }

    /// Validates a proposal set decoded from retained canonical bytes.
    pub fn validate(&self) -> std::result::Result<(), FactError> {
        Self::new(self.facts.clone()).map(|_| ())
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
    pub facts: Vec<FactValue>,
    /// Dense completeness proof.
    pub completeness: FactCompleteness,
    /// The Store-captured producer frontier.
    pub frontier: FactSelectionFrontier,
}

impl FactSelection {
    /// Creates one complete selection for a request.
    pub fn new(
        request: &FactSelectionRequest,
        facts: Vec<FactValue>,
        completeness: FactCompleteness,
        frontier: FactSelectionFrontier,
    ) -> Result<Self, FactError> {
        request.validate()?;
        frontier.validate()?;
        if facts.len() > MAX_SELECTED_FACTS
            || !completeness.complete
            || completeness.through_sequence == 0
            || frontier.through_sequence != completeness.through_sequence
            || facts.len() != request.source_refs.len()
            || facts.iter().any(|fact| fact.validate().is_err())
            || facts
                .iter()
                .zip(&request.source_refs)
                .any(|(fact, source)| &fact.source_ref != source)
            || facts
                .iter()
                .any(|fact| fact.subject_ref != request.subject_ref)
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
            frontier,
        })
    }

    /// Validates a decoded response against its exact request and frontier.
    pub fn validate_for(
        &self,
        request: &FactSelectionRequest,
    ) -> std::result::Result<(), FactError> {
        request.validate()?;
        if self.request_id != request.request_id
            || self.facts.len() > MAX_SELECTED_FACTS
            || !self.completeness.complete
            || self.completeness.through_sequence == 0
            || self.frontier.validate().is_err()
            || self.frontier.through_sequence != self.completeness.through_sequence
            || self.facts.len() != request.source_refs.len()
            || self.facts.iter().any(|fact| fact.validate().is_err())
            || self
                .facts
                .iter()
                .zip(&request.source_refs)
                .any(|(fact, source)| &fact.source_ref != source)
            || self
                .facts
                .iter()
                .any(|fact| fact.subject_ref != request.subject_ref)
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
        .expect("request");
        let value_ref = content_ref_for_value(br#"{"value":1}"#).expect("value ref");
        let fact = FactValue::new(
            source,
            request.subject_ref.clone(),
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
            FactSelectionFrontier::new(reference(4), 7, reference(5)).expect("frontier"),
        )
        .expect("selection");
        assert_eq!(selection.frontier.through_sequence, 7);
        assert!(FactSelection::new(
            &request,
            selection.facts.clone(),
            FactCompleteness {
                through_sequence: 8,
                complete: true,
            },
            FactSelectionFrontier::new(reference(4), 7, reference(5)).expect("frontier"),
        )
        .is_err());
    }

    #[test]
    fn proposal_sets_reject_duplicate_positions_and_secrets() {
        let source = reference(6);
        let subject = reference(7);
        let value_ref = content_ref_for_value(br#"{"value":1}"#).expect("value ref");
        let first = FactValue::new(
            source.clone(),
            subject.clone(),
            StableId::new("mfm.fact-test-proposal").expect("fact"),
            value_ref.clone(),
            r#"{"value":1}"#.to_owned(),
        )
        .expect("fact");
        assert!(FactProposalSet::new(vec![first.clone(), first]).is_err());

        let secret_ref = content_ref_for_value(br#"{"secret":"x"}"#).expect("value ref");
        assert!(FactValue::new(
            source,
            subject,
            StableId::new("mfm.fact-test-secret").expect("fact"),
            secret_ref,
            r#"{"secret":"x"}"#.to_owned(),
        )
        .is_err());
    }
}
