use std::fmt;
use std::num::NonZeroU64;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_config::{ConfigDigest, ConfigRevision, MAX_CONFIG_DOCUMENT_BYTES};
use mfm_evm::{EvmEndpoint, EvmPhysicalTarget, EVM_BALANCE_SOURCE_LIMIT};
use mfm_ids::ConfigName;
use mfm_ids::EntryPointId;
use mfm_portfolio::{
    plan_enrichment, plan_snapshot, EnrichmentProvenance, PortfolioAdmission, PortfolioConfig,
    PortfolioEnrichmentOutput, PortfolioError, PortfolioSnapshotInput, PortfolioSnapshotSelector,
    PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID, PORTFOLIO_SNAPSHOT_ENTRY_POINT_DESCRIPTION,
    PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_program::Program;
use serde::{Deserialize, Serialize};

/// Stable code for a checked config-document construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigDocumentError {
    /// JSON syntax, UTF-8, number, duplicate-key, or depth failure.
    Malformed,
    /// The typed config document is not valid.
    Invalid,
    /// The encoded document exceeds the fixed bound.
    TooLarge,
    /// The immediately awaited parsing task failed.
    Internal,
}

impl ConfigDocumentError {
    /// Returns the stable transport code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Malformed => "malformed_config_document",
            Self::Invalid => "invalid_config_document",
            Self::TooLarge => "config_document_too_large",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ConfigDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "config document is malformed",
            Self::Invalid => "config document is invalid",
            Self::TooLarge => "config document is too large",
            Self::Internal => "application internal failure",
        })
    }
}

impl std::error::Error for ConfigDocumentError {}

/// One opaque checked, canonical, complete execution config document.
pub struct ConfigDocument {
    canonical: PlainCanonicalJsonBytes,
    digest: ConfigDigest,
    wire: ConfigDocumentWire,
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
        let raw: ConfigDocumentWireUnchecked = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| ConfigDocumentError::Invalid)?;
        let wire = raw.check()?;
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
        routes: Vec<(u64, String)>,
    ) -> Result<Self, ConfigDocumentError> {
        let routes = routes
            .into_iter()
            .map(|(chain_id, endpoint_id)| EvmRouteWire {
                chain_id,
                endpoint_id,
            })
            .collect::<Vec<_>>();
        let (portfolio, selector) = output.into_snapshot_config();
        let wire = ConfigDocumentWireUnchecked::PortfolioSnapshot(ConfigInputWire {
            routes,
            selector,
            portfolio,
            provenance: Some(provenance),
        });
        let encoded = serde_json::to_vec(&wire).map_err(|_| ConfigDocumentError::Internal)?;
        Self::parse(encoded, false)
    }

    pub(crate) fn plan(
        &self,
        admission: Option<PortfolioAdmission>,
    ) -> Result<(Program, PortfolioSnapshotInput), PortfolioError> {
        match self.wire.entry {
            ConfigEntry::Snapshot { .. } => plan_snapshot(
                self.wire.selector.clone(),
                &self.wire.portfolio,
                &self.wire.targets,
                admission,
            ),
            ConfigEntry::Enrichment => plan_enrichment(
                self.wire.selector.clone(),
                &self.wire.portfolio,
                &self.wire.targets,
                admission,
            ),
        }
    }

    pub(crate) fn targets(&self) -> &[EvmPhysicalTarget] {
        &self.wire.targets
    }

    pub(crate) fn enrichment(&self) -> Option<&EnrichmentProvenance> {
        match &self.wire.entry {
            ConfigEntry::Snapshot { provenance } => provenance.as_deref(),
            ConfigEntry::Enrichment => None,
        }
    }

    pub(crate) fn matches_enrichment(&self, output: PortfolioEnrichmentOutput) -> bool {
        let routes_match = output.bindings().all(|(chain, reference)| {
            self.wire.targets.iter().any(|target| {
                target.chain_id == chain
                    && target
                        .binding_ref()
                        .is_ok_and(|binding| &binding == reference)
            })
        });
        let (portfolio, selector) = output.into_snapshot_config();
        matches!(self.wire.entry, ConfigEntry::Snapshot { .. })
            && self.wire.portfolio == portfolio
            && self.wire.selector == selector
            && routes_match
    }

    pub(crate) fn entry_point(&self) -> EntryPointId {
        EntryPointId::new(match self.wire.entry {
            ConfigEntry::Snapshot { .. } => PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
            ConfigEntry::Enrichment => PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID,
        })
        .expect("compiled entry-point id is checked by contract tests")
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

#[derive(Deserialize, Serialize)]
#[serde(tag = "entry_point", content = "input", deny_unknown_fields)]
enum ConfigDocumentWireUnchecked {
    #[serde(rename = "mfm.portfolio/snapshot@1")]
    PortfolioSnapshot(ConfigInputWire),
    #[serde(rename = "mfm.portfolio/enrich@1")]
    PortfolioEnrichment(ConfigInputWire),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigInputWire {
    routes: Vec<EvmRouteWire>,
    selector: PortfolioSnapshotSelector,
    portfolio: PortfolioConfig,
    #[serde(default)]
    provenance: Option<EnrichmentProvenance>,
}

enum ConfigEntry {
    Snapshot {
        provenance: Option<Box<EnrichmentProvenance>>,
    },
    Enrichment,
}

struct ConfigDocumentWire {
    entry: ConfigEntry,
    targets: Vec<EvmPhysicalTarget>,
    selector: PortfolioSnapshotSelector,
    portfolio: PortfolioConfig,
}

impl ConfigDocumentWireUnchecked {
    fn check(self) -> Result<ConfigDocumentWire, ConfigDocumentError> {
        let (entry, input) = match self {
            Self::PortfolioSnapshot(mut input) => {
                let provenance = input.provenance.take().map(Box::new);
                (ConfigEntry::Snapshot { provenance }, input)
            }
            Self::PortfolioEnrichment(input) if input.provenance.is_none() => {
                (ConfigEntry::Enrichment, input)
            }
            Self::PortfolioEnrichment(_) => return Err(ConfigDocumentError::Invalid),
        };
        if input.routes.is_empty()
            || input.routes.len() > EVM_BALANCE_SOURCE_LIMIT
            || input
                .routes
                .windows(2)
                .any(|pair| pair[0].chain_id >= pair[1].chain_id)
        {
            return Err(ConfigDocumentError::Invalid);
        }
        Ok(ConfigDocumentWire {
            entry,
            targets: input
                .routes
                .into_iter()
                .map(EvmRouteWire::target)
                .collect::<Result<_, _>>()?,
            selector: input.selector,
            portfolio: input.portfolio,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EvmRouteWire {
    chain_id: u64,
    endpoint_id: String,
}

impl EvmRouteWire {
    fn target(self) -> Result<EvmPhysicalTarget, ConfigDocumentError> {
        let endpoint =
            EvmEndpoint::new(self.endpoint_id).map_err(|_| ConfigDocumentError::Invalid)?;
        let endpoint_ref = endpoint
            .endpoint_ref()
            .map_err(|_| ConfigDocumentError::Internal)?;
        let chain_id = NonZeroU64::new(self.chain_id).ok_or(ConfigDocumentError::Invalid)?;
        Ok(EvmPhysicalTarget {
            chain_id,
            endpoint_ref,
        })
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
