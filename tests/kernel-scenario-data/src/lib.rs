#![warn(missing_docs)]
//! Shared data descriptors for kernel scenario and golden tests.
//!
//! The crate is intentionally data-only. It names scenario families, public JSON
//! fragments and golden labels, while execution remains in the integration test
//! crate.
//!
//! ```
//! let descriptor = mfm_kernel_scenario_data::proof_transport::SCENARIO;
//! assert_eq!(descriptor.label, "proof_transport.conformance.deterministic");
//! assert_eq!(descriptor.public_json_fragments.len(), 5);
//! ```

use std::fmt;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::ContentDigest;
use mfm_spec::v1::MediaType;
use serde::Serialize;

/// Proof-transport scenario descriptors.
pub mod proof_transport;
/// Replay artifact negative-case descriptors.
pub mod replay_artifacts;

/// Result type for scenario data helper functions.
pub type Result<T> = std::result::Result<T, ScenarioDataError>;

/// Error returned while serializing or canonicalizing scenario descriptors.
#[derive(Debug)]
pub enum ScenarioDataError {
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Canonical JSON validation failed.
    Canonical(mfm_canonical::CanonicalError),
}

impl fmt::Display for ScenarioDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "scenario data JSON error: {error}"),
            Self::Canonical(error) => write!(f, "scenario data canonical JSON error: {error}"),
        }
    }
}

impl std::error::Error for ScenarioDataError {}

impl From<serde_json::Error> for ScenarioDataError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<mfm_canonical::CanonicalError> for ScenarioDataError {
    fn from(error: mfm_canonical::CanonicalError) -> Self {
        Self::Canonical(error)
    }
}

/// Descriptor for one reusable kernel scenario family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ScenarioDescriptor {
    /// Stable scenario label.
    pub label: &'static str,
    /// Human-readable scenario purpose.
    pub purpose: &'static str,
    /// Media type expected for public scenario summaries.
    pub public_summary_media_type: &'static str,
    /// Public JSON fragments expected from the scenario.
    pub public_json_fragments: &'static [PublicJsonFragment],
    /// Golden labels associated with this scenario family.
    pub golden_labels: &'static [GoldenLabel],
}

impl ScenarioDescriptor {
    /// Parses the public summary media type.
    pub fn parse_public_summary_media_type(
        &self,
    ) -> std::result::Result<MediaType, mfm_spec::SpecError> {
        MediaType::new(self.public_summary_media_type)
    }

    /// Returns a canonical digest for this descriptor's serialized data.
    pub fn canonical_digest(&self) -> Result<ContentDigest> {
        let json = serde_json::to_string(self)?;
        Ok(PlainCanonicalJsonBytes::from_json_str(&json)?.content_digest())
    }
}

/// One expected public JSON fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PublicJsonFragment {
    /// JSON Pointer path.
    pub pointer: &'static str,
    /// Expected scalar value at the pointer.
    pub expected: ExpectedJsonValue,
}

/// Scalar values supported by public JSON fragment descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ExpectedJsonValue {
    /// String value.
    String(&'static str),
    /// Boolean value.
    Bool(bool),
    /// Unsigned integer value.
    Unsigned(u64),
    /// JSON null.
    Null,
}

impl ExpectedJsonValue {
    /// Converts the expected scalar into a JSON value.
    pub fn to_json_value(self) -> serde_json::Value {
        match self {
            Self::String(value) => serde_json::Value::String(value.to_owned()),
            Self::Bool(value) => serde_json::Value::Bool(value),
            Self::Unsigned(value) => serde_json::Value::Number(value.into()),
            Self::Null => serde_json::Value::Null,
        }
    }
}

/// Stable golden label associated with a scenario family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GoldenLabel {
    /// Golden label.
    pub label: &'static str,
    /// File or assertion surface that owns the golden.
    pub surface: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_descriptors_have_valid_media_types_and_digests() {
        for descriptor in [
            proof_transport::SCENARIO,
            replay_artifacts::NEGATIVE_CASE_SCENARIO,
        ] {
            descriptor
                .parse_public_summary_media_type()
                .expect("public summary media type parses");
            let digest = descriptor
                .canonical_digest()
                .expect("scenario descriptor has canonical digest");
            assert_eq!(digest.algorithm(), mfm_ids::DigestAlgorithm::Sha256JcsV1);
        }
    }
}
