//! Bitcoin receipt types shared by collection composition and portfolio report selection.
//!
//! Bitcoin collectors verify concrete fact material and emit checked
//! [`mfm_facts::FactContentIdentity`] values. A portfolio receipt carries the fact layer's opaque
//! [`mfm_facts::FactContentIdentityEvidence`]: it is not a usable identity after deserialization.
//! Report selection re-derives every queried identity from hydrated material and obtains a checked
//! identity only when that evidence matches, before any claim ordering occurs. EVM holdings bypass
//! this receipt path and reach assembly through direct network snapshots.

use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts::FactContentIdentityEvidence;
use mfm_portfolio_model::portfolio::{ExecutionAnchor, NetworkConfig, NetworkPin, PortfolioConfig};
use mfm_portfolio_model::symbol::HoldingSourceConfig;
use mfm_program_derive::{MfmValue, StateInput};
use serde::{Deserialize, Serialize};

use crate::{PortfolioHoldingErrorCode, PortfolioHoldingSelectionError};

/// Stable logical portfolio holding requirement.
///
/// This key carries presentation identifiers only at the portfolio boundary; source-near facts
/// never include these fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "holding-requirement-key",
    schema = "mfm.portfolio.holding_requirement_key"
)]
pub struct HoldingRequirementKey {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Stable symbol identifier.
    pub symbol_id: String,
    /// Stable semantic network identifier.
    pub network_id: String,
}

impl HoldingRequirementKey {
    /// Returns the canonical diagnostic key.
    pub fn as_key_str(&self) -> String {
        format!("{}/{}/{}", self.wallet_id, self.symbol_id, self.network_id)
    }
}

/// Bitcoin source key used by the remaining fact-backed portfolio selection path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(tag = "family", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "holding-source-key",
    schema = "mfm.portfolio.holding_source_key"
)]
pub enum HoldingSourceKey {
    /// Bitcoin native-address balance source.
    BitcoinNative {
        /// Semantic network id.
        network_id: String,
        /// Bitcoin Core network tag.
        bitcoin_network: String,
        /// Non-secret semantic source identity.
        semantic_source_identity: String,
        /// Canonical Bitcoin address.
        address: String,
    },
}

impl HoldingSourceKey {
    /// Returns the semantic network id bound to this source.
    pub fn network_id(&self) -> &str {
        let Self::BitcoinNative { network_id, .. } = self;
        network_id
    }

    /// Returns the exact successful coverage required by this source family.
    pub const fn required_coverage(&self) -> &'static str {
        "configured_only"
    }

    /// Returns the exact successful source status required by this source family.
    pub const fn required_source_status(&self) -> &'static str {
        "ok"
    }

    /// Returns Bitcoin-native source parts when this is a Bitcoin native source.
    pub fn bitcoin_native_parts(&self) -> (&str, &str, &str, &str) {
        let Self::BitcoinNative {
            network_id,
            bitcoin_network,
            semantic_source_identity,
            address,
        } = self;
        (
            network_id,
            bitcoin_network,
            semantic_source_identity,
            address,
        )
    }
}

/// One exact logical holding completed by a family collection receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "collected-holding-receipt",
    schema = "mfm.portfolio.collected_holding_receipt"
)]
pub struct CollectedHoldingReceipt {
    requirement: HoldingRequirementKey,
    source: HoldingSourceKey,
    anchor: ExecutionAnchor,
    coverage: String,
    source_status: String,
    fact_content_identity: FactContentIdentityEvidence,
}

impl CollectedHoldingReceipt {
    /// Creates an exact receipt entry after the family source receipt has been verified.
    pub fn new(
        requirement: HoldingRequirementKey,
        source: HoldingSourceKey,
        anchor: ExecutionAnchor,
        coverage: String,
        source_status: String,
        fact_content_identity: FactContentIdentityEvidence,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        if requirement.network_id != source.network_id() {
            return Err(receipt_error(
                "receipt requirement network did not match source network",
                Some(requirement),
            ));
        }
        if coverage != source.required_coverage()
            || source_status != source.required_source_status()
        {
            return Err(receipt_error(
                "receipt source coverage/status was not admissible",
                Some(requirement),
            ));
        }
        if !anchor_matches_source(&anchor, &source) {
            return Err(receipt_error(
                "receipt anchor family did not match source family",
                Some(requirement),
            ));
        }
        Ok(Self {
            requirement,
            source,
            anchor,
            coverage,
            source_status,
            fact_content_identity,
        })
    }

    /// Returns the exact logical requirement.
    pub const fn requirement(&self) -> &HoldingRequirementKey {
        &self.requirement
    }

    /// Returns the exact family/source binding.
    pub const fn source(&self) -> &HoldingSourceKey {
        &self.source
    }

    /// Returns the exact collection anchor.
    pub const fn anchor(&self) -> &ExecutionAnchor {
        &self.anchor
    }

    /// Returns the fixed successful coverage tag.
    pub fn coverage(&self) -> &str {
        &self.coverage
    }

    /// Returns the fixed successful source-status tag.
    pub fn source_status(&self) -> &str {
        &self.source_status
    }

    /// Returns opaque fact-layer evidence copied from the verified Bitcoin receipt.
    ///
    /// Consumers must rederive against hydrated material before using it as an identity.
    pub const fn fact_content_identity_evidence(&self) -> &FactContentIdentityEvidence {
        &self.fact_content_identity
    }
}

/// Exact logical manifest entry compiled from normalized portfolio configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "holding-manifest-entry",
    schema = "mfm.portfolio.holding_manifest_entry"
)]
pub struct HoldingManifestEntry {
    requirement: HoldingRequirementKey,
    source: HoldingSourceKey,
}

impl HoldingManifestEntry {
    /// Creates a logical-to-physical source mapping.
    pub fn new(
        requirement: HoldingRequirementKey,
        source: HoldingSourceKey,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        if requirement.network_id != source.network_id() {
            return Err(receipt_error(
                "manifest requirement network did not match source network",
                Some(requirement),
            ));
        }
        Ok(Self {
            requirement,
            source,
        })
    }

    /// Returns the logical requirement.
    pub const fn requirement(&self) -> &HoldingRequirementKey {
        &self.requirement
    }

    /// Returns its exact source mapping.
    pub const fn source(&self) -> &HoldingSourceKey {
        &self.source
    }
}

/// Exact receipt for the Bitcoin-backed portion of a portfolio snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-collection-receipt",
    schema = "mfm.portfolio.collection_receipt"
)]
pub struct PortfolioCollectionReceipt {
    manifest_identity: String,
    holdings: Vec<CollectedHoldingReceipt>,
    network_anchors: Vec<NetworkPin>,
}

impl PortfolioCollectionReceipt {
    /// Creates a receipt only when every logical manifest entry completed exactly once.
    pub fn new(
        manifest: &[HoldingManifestEntry],
        holdings: Vec<CollectedHoldingReceipt>,
        network_anchors: Vec<NetworkPin>,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        validate_manifest(manifest)?;
        let manifest_identity = manifest_identity(manifest)?;
        let expected = manifest
            .iter()
            .map(|entry| (entry.requirement.clone(), entry.source.clone()))
            .collect::<BTreeMap<_, _>>();
        if expected.len() != manifest.len() {
            return Err(receipt_error(
                "portfolio collection manifest had duplicate requirements",
                None,
            ));
        }

        let mut actual = BTreeMap::new();
        let mut actual_sources = BTreeSet::new();
        for holding in &holdings {
            let rebuilt = CollectedHoldingReceipt::new(
                holding.requirement.clone(),
                holding.source.clone(),
                holding.anchor.clone(),
                holding.coverage.clone(),
                holding.source_status.clone(),
                holding.fact_content_identity.clone(),
            )?;
            if rebuilt != *holding {
                return Err(receipt_error(
                    "portfolio receipt entry failed reconstruction",
                    Some(holding.requirement.clone()),
                ));
            }
            if !actual_sources.insert(holding.source.clone()) {
                return Err(receipt_error(
                    "portfolio receipt had duplicate source",
                    Some(holding.requirement.clone()),
                ));
            }
            if actual
                .insert(holding.requirement.clone(), holding.source.clone())
                .is_some()
            {
                return Err(receipt_error(
                    "portfolio receipt had duplicate requirement",
                    Some(holding.requirement.clone()),
                ));
            }
        }
        if actual != expected {
            return Err(receipt_error(
                "portfolio receipt did not exactly match the logical manifest",
                None,
            ));
        }

        let mut expected_anchors = BTreeMap::new();
        for holding in &holdings {
            match expected_anchors.get(&holding.requirement.network_id) {
                None => {
                    expected_anchors.insert(
                        holding.requirement.network_id.clone(),
                        holding.anchor.clone(),
                    );
                }
                Some(anchor) if anchor == &holding.anchor => {}
                Some(_) => {
                    return Err(receipt_error(
                        "portfolio receipt entries disagreed on a network anchor",
                        Some(holding.requirement.clone()),
                    ));
                }
            }
        }
        let actual_anchors = network_anchor_map(&network_anchors)?;
        if actual_anchors != expected_anchors {
            return Err(receipt_error(
                "portfolio receipt network anchors did not match holding anchors",
                None,
            ));
        }

        let mut holdings = holdings;
        holdings.sort_by(|left, right| left.requirement.cmp(&right.requirement));
        let mut network_anchors = network_anchors;
        network_anchors.sort_by(|left, right| left.network_id.cmp(&right.network_id));
        Ok(Self {
            manifest_identity,
            holdings,
            network_anchors,
        })
    }

    /// Returns the canonical identity of the exact logical manifest.
    pub fn manifest_identity(&self) -> &str {
        &self.manifest_identity
    }

    /// Returns sorted exact logical holding receipts.
    pub fn holdings(&self) -> &[CollectedHoldingReceipt] {
        &self.holdings
    }

    /// Returns sorted exact network anchors.
    pub fn network_anchors(&self) -> &[NetworkPin] {
        &self.network_anchors
    }
}

impl<'de> Deserialize<'de> for PortfolioCollectionReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            manifest_identity: String,
            holdings: Vec<CollectedHoldingReceipt>,
            network_anchors: Vec<NetworkPin>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let manifest = wire
            .holdings
            .iter()
            .map(|holding| {
                HoldingManifestEntry::new(holding.requirement.clone(), holding.source.clone())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(serde::de::Error::custom)?;
        let rebuilt = Self::new(&manifest, wire.holdings, wire.network_anchors)
            .map_err(serde::de::Error::custom)?;
        if rebuilt.manifest_identity != wire.manifest_identity {
            return Err(serde::de::Error::custom(
                "portfolio receipt manifest identity did not match its exact source mappings",
            ));
        }
        Ok(rebuilt)
    }
}

/// Typed receipt input consumed by receipt-pinned holding selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[mfm(schema = "mfm.portfolio.input.select_holdings")]
pub struct SelectHoldingsInput {
    /// Exact collection receipt produced by the same graph before fact selection.
    pub receipt: PortfolioCollectionReceipt,
}

/// Returns the canonical manifest identity for sorted logical source mappings.
pub fn manifest_identity(
    manifest: &[HoldingManifestEntry],
) -> Result<String, PortfolioHoldingSelectionError> {
    validate_manifest(manifest)?;
    let json = serde_json::to_string(manifest).map_err(|error| {
        receipt_error(
            format!("could not canonicalize portfolio collection manifest: {error}"),
            None,
        )
    })?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).map_err(|error| {
        receipt_error(
            format!("could not canonicalize portfolio collection manifest: {error}"),
            None,
        )
    })?;
    Ok(canonical.content_digest().as_str().to_owned())
}

/// Verifies that an exact receipt is the complete Bitcoin demand of one normalized portfolio.
///
/// This is defense in depth for report selection: collection assembly proves Bitcoin receipts
/// against its certified manifest, and selection independently proves that the receipt still
/// belongs to the certified aggregate portfolio config it was given.
pub fn validate_receipt_against_portfolio(
    receipt: &PortfolioCollectionReceipt,
    portfolio: &PortfolioConfig,
) -> Result<(), PortfolioHoldingSelectionError> {
    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str(), network))
        .collect::<BTreeMap<_, _>>();
    let symbols = portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let mut expected = Vec::new();
    for wallet in &portfolio.wallets {
        let network = networks
            .get(wallet.network_id.as_str())
            .copied()
            .ok_or_else(|| receipt_error("portfolio wallet referenced an unknown network", None))?;
        if matches!(network, NetworkConfig::Evm { .. }) {
            continue;
        }
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols.get(symbol_id.as_str()).copied().ok_or_else(|| {
                receipt_error("portfolio wallet referenced an unknown symbol", None)
            })?;
            let requirement = HoldingRequirementKey {
                wallet_id: wallet.wallet_id.to_string(),
                symbol_id: symbol.symbol_id.to_string(),
                network_id: wallet.network_id.to_string(),
            };
            let source = match (network, &symbol.source) {
                (
                    NetworkConfig::Bitcoin {
                        network_id,
                        bitcoin_network,
                        source_identity,
                        ..
                    },
                    HoldingSourceConfig::Native,
                ) => HoldingSourceKey::BitcoinNative {
                    network_id: network_id.to_string(),
                    bitcoin_network: bitcoin_network.clone(),
                    semantic_source_identity: source_identity.to_string(),
                    address: wallet.subject.address_str().to_owned(),
                },
                _ => {
                    return Err(receipt_error(
                        "portfolio holding source did not match network family",
                        Some(requirement),
                    ));
                }
            };
            expected.push(HoldingManifestEntry::new(requirement, source)?);
        }
    }
    expected.sort_by(|left, right| left.requirement.cmp(&right.requirement));
    let identity = manifest_identity(&expected)?;
    if receipt.manifest_identity != identity {
        return Err(receipt_error(
            "collection receipt manifest identity did not match portfolio demand",
            None,
        ));
    }
    let actual = receipt
        .holdings
        .iter()
        .map(|holding| (holding.requirement.clone(), holding.source.clone()))
        .collect::<BTreeMap<_, _>>();
    let expected = expected
        .into_iter()
        .map(|entry| (entry.requirement, entry.source))
        .collect::<BTreeMap<_, _>>();
    if actual != expected {
        return Err(receipt_error(
            "collection receipt logical mappings did not exactly match portfolio demand",
            None,
        ));
    }
    Ok(())
}

fn validate_manifest(
    manifest: &[HoldingManifestEntry],
) -> Result<(), PortfolioHoldingSelectionError> {
    let mut requirements = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut previous = None;
    for entry in manifest {
        HoldingManifestEntry::new(entry.requirement.clone(), entry.source.clone())?;
        if !requirements.insert(entry.requirement.clone()) {
            return Err(receipt_error(
                "portfolio collection manifest had duplicate requirements",
                Some(entry.requirement.clone()),
            ));
        }
        if !sources.insert(entry.source.clone()) {
            return Err(receipt_error(
                "portfolio collection manifest had aliased sources",
                Some(entry.requirement.clone()),
            ));
        }
        if previous
            .as_ref()
            .is_some_and(|key: &HoldingRequirementKey| key >= &entry.requirement)
        {
            return Err(receipt_error(
                "portfolio collection manifest was not strictly sorted",
                Some(entry.requirement.clone()),
            ));
        }
        previous = Some(entry.requirement.clone());
    }
    Ok(())
}

fn network_anchor_map(
    network_anchors: &[NetworkPin],
) -> Result<BTreeMap<String, ExecutionAnchor>, PortfolioHoldingSelectionError> {
    let mut result = BTreeMap::new();
    for network_anchor in network_anchors {
        if result
            .insert(
                network_anchor.network_id.clone(),
                network_anchor.anchor.clone(),
            )
            .is_some()
        {
            return Err(receipt_error(
                "portfolio receipt had duplicate network anchors",
                None,
            ));
        }
    }
    Ok(result)
}

fn anchor_matches_source(anchor: &ExecutionAnchor, source: &HoldingSourceKey) -> bool {
    match (anchor, source) {
        (ExecutionAnchor::Bitcoin { block_hash, .. }, HoldingSourceKey::BitcoinNative { .. }) => {
            is_lower_hex(block_hash, 64)
        }
        _ => false,
    }
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn receipt_error(
    message: impl Into<String>,
    requirement: Option<HoldingRequirementKey>,
) -> PortfolioHoldingSelectionError {
    let (holding_key, network_id) = requirement
        .as_ref()
        .map(|key| (Some(key.as_key_str()), Some(key.network_id.clone())))
        .unwrap_or((None, None));
    PortfolioHoldingSelectionError::new(
        PortfolioHoldingErrorCode::ReceiptMismatch,
        message,
        holding_key,
        network_id,
    )
}
