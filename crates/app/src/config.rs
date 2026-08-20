use std::fmt;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_catalog::{CatalogEntry, ConfigDigest, ConfigName, MAX_CONFIG_DOCUMENT_BYTES};
use mfm_evm::{EvmEndpoint, EvmPhysicalTarget, EVM_BALANCE_SOURCE_LIMIT};
use mfm_ids::EntryPointId;
use mfm_portfolio::{
    plan_snapshot, PortfolioConfig, PortfolioError, PortfolioSnapshotInput,
    PortfolioSnapshotSelector, PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
};
use mfm_program::Program;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

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

    pub(crate) fn plan(&self) -> Result<(Program, PortfolioSnapshotInput), PortfolioError> {
        match &self.wire {
            ConfigDocumentWire::PortfolioSnapshot {
                targets,
                selector,
                portfolio,
            } => plan_snapshot(selector.clone(), portfolio, targets),
        }
    }

    pub(crate) fn targets(&self) -> &[EvmPhysicalTarget] {
        match &self.wire {
            ConfigDocumentWire::PortfolioSnapshot { targets, .. } => targets,
        }
    }

    pub(crate) fn entry_point(&self) -> EntryPointId {
        match &self.wire {
            ConfigDocumentWire::PortfolioSnapshot { .. } => {
                EntryPointId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID)
                    .expect("compiled entry-point id is checked by contract tests")
            }
        }
    }

    pub(crate) fn summary(&self, name: ConfigName) -> ConfigSummary {
        ConfigSummary {
            name,
            digest: self.digest.clone(),
            entry_point: self.entry_point(),
        }
    }

    pub(crate) fn catalog_entry(&self, name: ConfigName) -> Result<CatalogEntry, ()> {
        CatalogEntry::new(name, self.digest.clone(), self.canonical.to_vec()).map_err(|_| ())
    }

    pub(crate) fn stored_view(self, name: ConfigName) -> Result<StoredConfigView, ()> {
        let config = self.summary(name);
        let document = RawValue::from_string(self.canonical.as_str().to_owned()).map_err(|_| ())?;
        Ok(StoredConfigView { config, document })
    }
}

#[derive(Deserialize)]
#[serde(tag = "entry_point", content = "input", deny_unknown_fields)]
enum ConfigDocumentWireUnchecked {
    #[serde(rename = "mfm.portfolio/snapshot@1")]
    PortfolioSnapshot {
        routes: Vec<EvmRouteWire>,
        selector: PortfolioSnapshotSelector,
        portfolio: PortfolioConfig,
    },
}

enum ConfigDocumentWire {
    PortfolioSnapshot {
        targets: Vec<EvmPhysicalTarget>,
        selector: PortfolioSnapshotSelector,
        portfolio: PortfolioConfig,
    },
}

impl ConfigDocumentWireUnchecked {
    fn check(self) -> Result<ConfigDocumentWire, ConfigDocumentError> {
        match self {
            Self::PortfolioSnapshot {
                routes,
                selector,
                portfolio,
            } => {
                if routes.is_empty()
                    || routes.len() > EVM_BALANCE_SOURCE_LIMIT
                    || routes
                        .windows(2)
                        .any(|pair| pair[0].chain_id >= pair[1].chain_id)
                {
                    return Err(ConfigDocumentError::Invalid);
                }
                let targets = routes
                    .into_iter()
                    .map(EvmRouteWire::target)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ConfigDocumentWire::PortfolioSnapshot {
                    targets,
                    selector,
                    portfolio,
                })
            }
        }
    }
}

#[derive(Deserialize)]
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
        EvmPhysicalTarget::new(self.chain_id, endpoint_ref)
            .map_err(|_| ConfigDocumentError::Invalid)
    }
}

/// One compiled entry point accepted by [`ConfigDocument`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EntryPointSummary {
    entry_point: &'static str,
}

impl EntryPointSummary {
    /// Returns the checked compiled entry-point spelling.
    pub const fn entry_point(self) -> &'static str {
        self.entry_point
    }
}

pub(crate) const ENTRY_POINTS: [EntryPointSummary; 1] = [EntryPointSummary {
    entry_point: PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID,
}];

/// Public identity of one retained config revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigSummary {
    name: ConfigName,
    digest: ConfigDigest,
    entry_point: EntryPointId,
}

impl ConfigSummary {
    /// Returns the catalog name.
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

/// Result of importing an absent or identical config.
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

/// One validated stored config with its exact canonical document embedded as raw JSON.
pub struct StoredConfigView {
    config: ConfigSummary,
    document: Box<RawValue>,
}

impl StoredConfigView {
    /// Returns the retained config summary.
    pub const fn config(&self) -> &ConfigSummary {
        &self.config
    }

    /// Returns the exact canonical JSON document.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.document.get().as_bytes()
    }
}

impl Serialize for StoredConfigView {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("StoredConfigView", 2)?;
        state.serialize_field("config", &self.config)?;
        state.serialize_field("document", &self.document)?;
        state.end()
    }
}
