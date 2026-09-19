use mfm_ids::ContentRef;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use super::{check_public_text, BalanceTextError};

/// Checked caller correlation and public route for one collection occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct BalanceCollectionMetadata {
    collection_ordinal: u32,
    correlation: String,
    route_ref: ContentRef,
}
impl BalanceCollectionMetadata {
    /// Checks public correlation text; its caller owns ordinal agreement with the retained request.
    pub fn new(
        collection_ordinal: u32,
        correlation: String,
        route_ref: ContentRef,
    ) -> Result<Self, BalanceTextError> {
        check_public_text(&correlation)?;
        Ok(Self {
            collection_ordinal,
            correlation,
            route_ref,
        })
    }
    /// Exact occurrence ordinal supplied by the caller.
    pub fn collection_ordinal(&self) -> u32 {
        self.collection_ordinal
    }
    /// Checked public correlation identifier.
    pub fn correlation(&self) -> &str {
        &self.correlation
    }
    /// Exact public native route identity.
    pub fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }
}
impl<'de> Deserialize<'de> for BalanceCollectionMetadata {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            collection_ordinal: u32,
            correlation: String,
            route_ref: ContentRef,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.collection_ordinal, wire.correlation, wire.route_ref)
            .map_err(serde::de::Error::custom)
    }
}
