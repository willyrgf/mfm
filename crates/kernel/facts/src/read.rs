use mfm_canonical::CanonicalBytes;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{FactError, Result};

/// Maximum tenant publications one authorized fact read may examine.
pub const MAX_FACT_SCAN_PUBLICATIONS: u64 = 1_000_000;
/// Maximum individual facts one authorized fact read may examine.
pub const MAX_FACT_SCAN_FACTS: u64 = 16_000_000;
/// Maximum eligible canonical source bytes one authorized fact read may examine while scanning.
pub const MAX_FACT_SCAN_RETAINED_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum selected facts across all authored queries.
pub const MAX_FACT_SCAN_SELECTED_RESULTS: u64 = 16_384;
/// Maximum canonical bytes in one returned fact-selection value.
pub const MAX_FACT_SCAN_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// Sole completeness mode supported by the prior-run fact scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fact",
    name = "selection-completeness-mode",
    version = "1",
    schema = "mfm.fact-selection-completeness-mode"
)]
pub enum FactSelectionCompletenessMode {
    /// Scan every dense tenant publication through the authorization-captured frontier.
    CompleteThroughAuthorizationFrontier,
}

/// Explicit total bounds frozen into one prior-run fact request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.fact",
    name = "selection-scan-bounds",
    version = "1",
    schema = "mfm.fact-selection-scan-bounds"
)]
pub struct FactSelectionScanBounds {
    maximum_publications: u64,
    maximum_facts: u64,
    maximum_retained_source_bytes: u64,
    maximum_selected_results: u64,
    maximum_response_bytes: u64,
}

impl<'de> Deserialize<'de> for FactSelectionScanBounds {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            maximum_publications: u64,
            maximum_facts: u64,
            maximum_retained_source_bytes: u64,
            maximum_selected_results: u64,
            maximum_response_bytes: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.maximum_publications,
            wire.maximum_facts,
            wire.maximum_retained_source_bytes,
            wire.maximum_selected_results,
            wire.maximum_response_bytes,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl FactSelectionScanBounds {
    /// Constructs bounded scan limits under the frozen absolute maxima.
    pub fn new(
        maximum_publications: u64,
        maximum_facts: u64,
        maximum_retained_source_bytes: u64,
        maximum_selected_results: u64,
        maximum_response_bytes: u64,
    ) -> Result<Self> {
        let bounds = Self {
            maximum_publications,
            maximum_facts,
            maximum_retained_source_bytes,
            maximum_selected_results,
            maximum_response_bytes,
        };
        bounds.validate()?;
        Ok(bounds)
    }

    /// Returns the maximum dense publications scanned.
    pub const fn maximum_publications(&self) -> u64 {
        self.maximum_publications
    }

    /// Returns the maximum individual facts scanned.
    pub const fn maximum_facts(&self) -> u64 {
        self.maximum_facts
    }

    /// Returns the maximum eligible canonical source bytes examined.
    pub const fn maximum_retained_source_bytes(&self) -> u64 {
        self.maximum_retained_source_bytes
    }

    /// Returns the maximum selected facts across every query.
    pub const fn maximum_selected_results(&self) -> u64 {
        self.maximum_selected_results
    }

    /// Returns the maximum canonical response bytes.
    pub const fn maximum_response_bytes(&self) -> u64 {
        self.maximum_response_bytes
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.maximum_publications == 0
            || self.maximum_publications > MAX_FACT_SCAN_PUBLICATIONS
            || self.maximum_facts == 0
            || self.maximum_facts > MAX_FACT_SCAN_FACTS
            || self.maximum_retained_source_bytes == 0
            || self.maximum_retained_source_bytes > MAX_FACT_SCAN_RETAINED_SOURCE_BYTES
            || self.maximum_selected_results == 0
            || self.maximum_selected_results > MAX_FACT_SCAN_SELECTED_RESULTS
            || self.maximum_response_bytes == 0
            || self.maximum_response_bytes > MAX_FACT_SCAN_RESPONSE_BYTES
        {
            return Err(FactError::Selection(
                "fact selection scan bounds are outside the frozen limits",
            ));
        }
        Ok(())
    }
}

/// One self-contained typed response returned by the sealed fact scanner.
#[derive(Debug, Clone, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.fact",
    name = "selection-read-response",
    version = "1",
    schema = "mfm.fact-selection-read-response"
)]
pub struct FactSelectionReadResponse {
    #[serde(rename = "canonical_response_base64url")]
    canonical_response_json: String,
}

impl<'de> Deserialize<'de> for FactSelectionReadResponse {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            canonical_response_base64url: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let canonical = CanonicalBytes::from_base64url_no_pad(wire.canonical_response_base64url)
            .map_err(serde::de::Error::custom)?;
        let canonical =
            String::from_utf8(canonical.into_bytes()).map_err(serde::de::Error::custom)?;
        Self::from_canonical_json(canonical).map_err(serde::de::Error::custom)
    }
}

impl Serialize for FactSelectionReadResponse {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            canonical_response_base64url: &'a str,
        }

        let encoded = CanonicalBytes::new(self.canonical_response_json.as_bytes().to_vec());
        Wire {
            canonical_response_base64url: encoded.encoded(),
        }
        .serialize(serializer)
    }
}

impl FactSelectionReadResponse {
    /// Wraps one exact canonical scanner response under the absolute byte bound.
    pub fn from_canonical_json(canonical_response_json: impl Into<String>) -> Result<Self> {
        let canonical_response_json = canonical_response_json.into();
        if canonical_response_json.is_empty()
            || u64::try_from(canonical_response_json.len()).ok()
                > Some(MAX_FACT_SCAN_RESPONSE_BYTES)
            || mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(
                canonical_response_json.as_bytes(),
            )
            .is_err()
        {
            return Err(FactError::Selection(
                "fact selection response is not bounded canonical JSON",
            ));
        }
        Ok(Self {
            canonical_response_json,
        })
    }

    /// Returns the exact embedded canonical response.
    pub fn canonical_response_json(&self) -> &str {
        &self.canonical_response_json
    }
}

/// Reviewed definite failure codes for a bounded prior-run fact read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fact",
    name = "selection-read-failure-code",
    version = "1",
    schema = "mfm.fact-selection-read-failure-code"
)]
pub enum FactSelectionReadFailureCode {
    /// The authoritative store was unavailable for this non-mutating read.
    StoreUnavailable,
    /// The authorization frontier exceeds the request's publication bound.
    PublicationBoundExceeded,
    /// Complete traversal exceeds the request's fact bound.
    FactBoundExceeded,
    /// Verified eligible source material exceeds the request's retained-byte bound.
    RetainedSourceBoundExceeded,
    /// Authored query results exceed the request's total-selection bound.
    SelectedResultBoundExceeded,
    /// The complete typed result exceeds the request's response-byte bound.
    ResponseBoundExceeded,
}

/// Reviewed redaction-safe definite failure returned by the fact scanner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.fact",
    name = "selection-read-failure",
    version = "1",
    schema = "mfm.fact-selection-read-failure"
)]
pub struct FactSelectionReadFailure {
    code: FactSelectionReadFailureCode,
}

impl FactSelectionReadFailure {
    /// Constructs one reviewed scanner failure.
    pub const fn new(code: FactSelectionReadFailureCode) -> Self {
        Self { code }
    }

    /// Returns the stable definite-failure code.
    pub const fn code(&self) -> FactSelectionReadFailureCode {
        self.code
    }
}
