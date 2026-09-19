use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_config::{ConfigDigest, ConfigRevision, MAX_CONFIG_DOCUMENT_BYTES};
use mfm_evm_live::client::portfolio::{
    self as native, EvmPortfolioClientError, EvmPortfolioConfig,
};
use mfm_ids::{ConfigName, EntryPointId};
use mfm_portfolio::{
    EnrichmentProvenance, PortfolioAdmission, PortfolioEnrichmentOutput,
    PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID, PORTFOLIO_SNAPSHOT_ENTRY_POINT_DESCRIPTION,
    PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use serde::Serialize;
use std::fmt;

/// Stable code for a checked config-document construction failure.
#[derive(Debug)]
pub enum ConfigDocumentError {
    /// JSON syntax, UTF-8, number, duplicate-key, or depth failure.
    Malformed,
    /// The typed config document is not valid.
    Invalid,
    /// The encoded document exceeds the fixed bound.
    TooLarge,
    /// The immediately awaited parsing task failed.
    Internal,
    /// Native admission preserves its complete reviewed cause.
    Native(EvmPortfolioClientError),
}

impl ConfigDocumentError {
    /// Returns the stable transport code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed_config_document",
            Self::Invalid | Self::Native(_) => "invalid_config_document",
            Self::TooLarge => "config_document_too_large",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ConfigDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "config document is malformed",
            Self::Invalid | Self::Native(_) => "config document is invalid",
            Self::TooLarge => "config document is too large",
            Self::Internal => "application internal failure",
        })
    }
}

impl std::error::Error for ConfigDocumentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Native(cause) => Some(cause),
            _ => None,
        }
    }
}

/// One opaque checked, canonical, complete execution config document.
pub struct ConfigDocument {
    canonical: PlainCanonicalJsonBytes,
    digest: ConfigDigest,
    wire: EvmPortfolioConfig,
}

impl ConfigDocument {
    /// Parses and canonicalizes one bounded config document off the async executor.
    pub async fn new(encoded: Vec<u8>) -> Result<Self, ConfigDocumentError> {
        if encoded.len() > MAX_CONFIG_DOCUMENT_BYTES {
            return Err(ConfigDocumentError::TooLarge);
        }
        tokio::task::spawn_blocking(move || Self::parse(encoded, false))
            .await
            .map_err(|_| ConfigDocumentError::Internal)?
    }

    pub(crate) fn parse_retained(encoded: Vec<u8>, expected: &ConfigDigest) -> Result<Self, ()> {
        let document = Self::parse(encoded, true).map_err(|_| ())?;
        (&document.digest == expected).then_some(document).ok_or(())
    }

    fn parse(encoded: Vec<u8>, require_canonical: bool) -> Result<Self, ConfigDocumentError> {
        if encoded.is_empty() || encoded.len() > MAX_CONFIG_DOCUMENT_BYTES {
            return Err(if encoded.len() > MAX_CONFIG_DOCUMENT_BYTES {
                ConfigDocumentError::TooLarge
            } else {
                ConfigDocumentError::Malformed
            });
        }
        let input = std::str::from_utf8(&encoded).map_err(|_| ConfigDocumentError::Malformed)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(input)
            .map_err(|_| ConfigDocumentError::Malformed)?;
        if require_canonical && canonical.as_bytes() != encoded {
            return Err(ConfigDocumentError::Malformed);
        }
        let wire: EvmPortfolioConfig = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| ConfigDocumentError::Invalid)?;
        wire.validate().map_err(ConfigDocumentError::Native)?;
        let digest = ConfigDigest::new(canonical.content_digest())
            .map_err(|_| ConfigDocumentError::Internal)?;
        Ok(Self {
            canonical,
            digest,
            wire,
        })
    }

    pub(crate) fn from_enrichment(
        output: PortfolioEnrichmentOutput,
        provenance: EnrichmentProvenance,
    ) -> Result<Self, ConfigDocumentError> {
        let wire = native::snapshot_config(&output)
            .and_then(|config| config.with_provenance(provenance))
            .map_err(ConfigDocumentError::Native)?;
        let encoded = serde_json::to_vec(&wire).map_err(|_| ConfigDocumentError::Internal)?;
        Self::parse(encoded, false)
    }

    pub(crate) fn native(&self) -> &EvmPortfolioConfig {
        &self.wire
    }

    pub(crate) fn enrichment(&self) -> Option<&EnrichmentProvenance> {
        self.wire.provenance()
    }

    pub(crate) fn matches_enrichment(
        &self,
        output: &PortfolioEnrichmentOutput,
    ) -> Result<bool, EvmPortfolioClientError> {
        native::matches_snapshot(&self.wire, output)
    }

    pub(crate) fn entry_point(&self) -> EntryPointId {
        self.wire
            .entry_point_id()
            .expect("compiled entry-point ids are checked by contract tests")
    }

    pub(crate) fn admission(&self, name: &ConfigName) -> PortfolioAdmission {
        PortfolioAdmission::new(
            name.clone(),
            self.digest.digest_bytes(),
            self.entry_point(),
            self.enrichment().cloned(),
        )
    }

    pub(crate) fn summary(&self, name: ConfigName) -> ConfigSummary {
        ConfigSummary {
            name,
            digest: self.digest.clone(),
            entry_point: self.entry_point(),
        }
    }

    pub(crate) fn revision(&self, name: ConfigName) -> Result<ConfigRevision, ()> {
        ConfigRevision::new(name, self.digest.clone(), self.canonical.to_vec()).map_err(|_| ())
    }
}

/// One compiled entry point accepted by [`ConfigDocument`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EntryPointSummary {
    entry_point: &'static str,
    #[serde(skip_serializing)]
    description: &'static str,
}

impl EntryPointSummary {
    /// Returns the checked compiled entry-point spelling.
    pub const fn entry_point(self) -> &'static str {
        self.entry_point
    }

    /// Returns the human-readable entry-point description.
    pub const fn description(self) -> &'static str {
        self.description
    }
}

pub(crate) const ENTRY_POINTS: [EntryPointSummary; 2] = [
    EntryPointSummary {
        entry_point: PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID,
        description: "Resolves configured asset candidates using anchored EVM observations.",
    },
    EntryPointSummary {
        entry_point: PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
        description: PORTFOLIO_SNAPSHOT_ENTRY_POINT_DESCRIPTION,
    },
];

/// Public identity of one retained config revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigSummary {
    name: ConfigName,
    digest: ConfigDigest,
    entry_point: EntryPointId,
}

impl ConfigSummary {
    pub(crate) fn from_admission(admission: &PortfolioAdmission) -> Self {
        Self {
            name: admission.name().clone(),
            digest: ConfigDigest::from_digest_bytes(admission.config_digest_hex()),
            entry_point: admission.entry_point().clone(),
        }
    }

    /// Returns the configuration name.
    pub const fn name(&self) -> &ConfigName {
        &self.name
    }

    /// Returns the selected canonical-document digest.
    pub const fn digest(&self) -> &ConfigDigest {
        &self.digest
    }

    /// Returns the entry point derived from the retained document.
    pub const fn entry_point(&self) -> &EntryPointId {
        &self.entry_point
    }
}

/// Result of importing a configuration revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ImportOutcome {
    /// The absent name was bound to the document.
    Created {
        /// Imported revision identity.
        config: ConfigSummary,
    },
    /// The exact revision was already retained and nothing was written.
    Unchanged {
        /// Existing revision identity.
        config: ConfigSummary,
    },
}

impl ImportOutcome {
    /// Returns the imported or retained config summary.
    pub const fn config(&self) -> &ConfigSummary {
        match self {
            Self::Created { config } | Self::Unchanged { config } => config,
        }
    }
}

#[cfg(test)]
mod tests;
