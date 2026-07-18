//! Receipt-pinned holding projection helpers for fact-backed portfolio reports.
//!
//! Policy id: `mfm.portfolio.holding.collection-receipt-anchor.v1`.
//! The collection receipt fixes every source, anchor, status, coverage, and fact-content identity
//! before this module projects a hydrated, identity-matching fact into an observation.

use std::collections::BTreeMap;

use mfm_canonical::sha256_digest_bytes;
use mfm_facts::FactClaimId;
use mfm_ids::{ContentDigest, DigestAlgorithm};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::portfolio::{ExecutionAnchor, NetworkPin};
use mfm_portfolio_model::symbol::{AnchoredHoldingSource, HoldingSourceConfig, Observation};

/// Certified selection policy id for receipt-pinned portfolio holding selection.
pub const PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID: &str =
    "mfm.portfolio.holding.collection-receipt-anchor.v1";

/// Content digest of the selection policy id bytes (for selection evidence).
pub fn portfolio_holding_selection_policy_digest() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID.as_bytes()),
    )
}

/// Scope decision digest for Platform holding candidate fact-index queries.
pub fn portfolio_holding_select_scope_decision_hash() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.portfolio.holding.select.scope.v1"),
    )
}

/// Minimal public hard-fail codes for portfolio holding selection / assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortfolioHoldingErrorCode {
    /// No retained Platform fact matched an exact receipt identity.
    MissingFact,
    /// Selection cardinality violated after policy / receipt.
    AmbiguousFacts,
    /// The receipt, queried source, anchor, or fact identity did not match exactly.
    ReceiptMismatch,
    /// The fixed N + 1 candidate query saturated before exhaustion was proven.
    CandidateBoundExhausted,
    /// Residual same-network selected anchors disagree (guard).
    InconsistentNetworkAnchors,
    /// A portfolio requirement could not be projected into a report observation.
    UnsupportedRequirement,
}

impl PortfolioHoldingErrorCode {
    /// Returns the stable public error code string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingFact => "missing_fact",
            Self::AmbiguousFacts => "ambiguous_facts",
            Self::ReceiptMismatch => "receipt_mismatch",
            Self::CandidateBoundExhausted => "candidate_bound_exhausted",
            Self::InconsistentNetworkAnchors => "inconsistent_network_anchors",
            Self::UnsupportedRequirement => "unsupported_requirement",
        }
    }
}

impl std::fmt::Display for PortfolioHoldingErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Hard-fail error from pure selection or pin projection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PortfolioHoldingSelectionError {
    /// Stable public error code.
    pub code: PortfolioHoldingErrorCode,
    /// Redaction-safe message.
    pub message: String,
    /// Optional holding key that failed.
    pub holding_key: Option<String>,
    /// Optional network id.
    pub network_id: Option<String>,
}

impl PortfolioHoldingSelectionError {
    /// Builds a selection error.
    pub fn new(
        code: PortfolioHoldingErrorCode,
        message: impl Into<String>,
        holding_key: Option<String>,
        network_id: Option<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            holding_key,
            network_id,
        }
    }
}

/// Anchor identity for cutover holding kinds: mandatory height + hash.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HoldingAnchor {
    /// Block height / number.
    pub height: u64,
    /// Block hash (normalized opaque string).
    pub hash: String,
}

impl HoldingAnchor {
    /// Creates an anchor; hash must be non-empty.
    pub fn new(
        height: u64,
        hash: impl Into<String>,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        let hash = hash.into();
        if hash.trim().is_empty() {
            return Err(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::MissingFact,
                "holding anchor hash is required",
                None,
                None,
            ));
        }
        Ok(Self { height, hash })
    }
}

/// One acceptable candidate fact for a required holding (post coverage/status filter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingCandidate {
    /// Network id of the holding.
    pub network_id: String,
    /// Anchor at which the balance was proven.
    pub anchor: HoldingAnchor,
    /// Primary LWW ordering for candidates at the same subject and anchor.
    pub store_commit_order: u64,
    /// Claim identity for deterministic LWW tie-breaking and evidence binding.
    pub fact_claim_id: FactClaimId,
    /// Hydrated response payload opaque to pure selection (passed through).
    pub response_material: SelectedHoldingMaterial,
}

/// Selected holding material after pure selection (family-normalized fields + join keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedHoldingMaterial {
    /// Wallet id (joined by report, not present on facts).
    pub wallet_id: String,
    /// Symbol id (joined by report).
    pub symbol_id: String,
    /// Network id.
    pub network_id: String,
    /// Direct holding source for the observation.
    pub holding: HoldingSourceConfig,
    /// Raw amount decimal string.
    pub raw_dec: String,
    /// Token decimals.
    pub decimals: u8,
    /// Family-specific execution anchor for observation source.
    pub observation_anchor: ExecutionAnchor,
    /// Coverage tag retained for honesty surfaces after selection.
    pub coverage: String,
    /// Source status tag.
    pub source_status: String,
}

pub(crate) struct BitcoinHoldingCandidateFields {
    pub(crate) holding: HoldingSourceConfig,
    pub(crate) raw_dec: String,
    pub(crate) decimals: u8,
    pub(crate) height: u64,
    pub(crate) block_hash: String,
    pub(crate) coverage: String,
    pub(crate) source_status: String,
}

pub(crate) fn holding_candidate_from_bitcoin(
    key: &crate::HoldingRequirementKey,
    store_commit_order: u64,
    fact_claim_id: FactClaimId,
    fields: BitcoinHoldingCandidateFields,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let parsed_coverage = fields.coverage.parse::<CoverageStatus>().map_err(|_| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            "holding fact has unknown coverage status",
            Some(key.as_key_str()),
            Some(key.network_id.clone()),
        )
    })?;
    let parsed_source_status = fields
        .source_status
        .parse::<HoldingSourceStatus>()
        .map_err(|_| {
            PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::MissingFact,
                "holding fact has unknown source status",
                Some(key.as_key_str()),
                Some(key.network_id.clone()),
            )
        })?;
    if !portfolio_selection_accepts_status(parsed_coverage, parsed_source_status) {
        return Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            "holding fact is not acceptable for portfolio selection",
            Some(key.as_key_str()),
            Some(key.network_id.clone()),
        ));
    }
    let anchor = HoldingAnchor::new(fields.height, &fields.block_hash).map_err(|mut error| {
        error.holding_key = Some(key.as_key_str());
        error.network_id = Some(key.network_id.clone());
        error
    })?;
    Ok(HoldingCandidate {
        network_id: key.network_id.clone(),
        anchor,
        store_commit_order,
        fact_claim_id,
        response_material: SelectedHoldingMaterial {
            wallet_id: key.wallet_id.clone(),
            symbol_id: key.symbol_id.clone(),
            network_id: key.network_id.clone(),
            holding: fields.holding,
            raw_dec: fields.raw_dec,
            decimals: fields.decimals,
            observation_anchor: ExecutionAnchor::Bitcoin {
                height: fields.height,
                block_hash: fields.block_hash,
            },
            coverage: fields.coverage,
            source_status: fields.source_status,
        },
    })
}

fn portfolio_selection_accepts_status(
    coverage: CoverageStatus,
    source_status: HoldingSourceStatus,
) -> bool {
    matches!(
        (coverage, source_status),
        (
            CoverageStatus::CompleteAtAnchor | CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
    )
}

/// One selected holding after receipt-pinned identity filtering and ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedHolding {
    /// Required holding key.
    pub key: crate::HoldingRequirementKey,
    /// Anchor verified against the exact receipt entry.
    pub anchor: HoldingAnchor,
    /// Store commit order of the winning candidate.
    pub store_commit_order: u64,
    /// Fact claim id of the winning candidate.
    pub fact_claim_id: FactClaimId,
    /// Material for observation construction.
    pub material: SelectedHoldingMaterial,
}

/// Projects `network_pins` purely from selected observation anchors.
///
/// One pin per network; residual same-network disagreement → inconsistent_network_anchors.
pub fn project_network_pins_from_observations(
    observations: &[Observation],
) -> Result<Vec<NetworkPin>, PortfolioHoldingSelectionError> {
    let mut by_network: BTreeMap<String, ExecutionAnchor> = BTreeMap::new();
    for observation in observations {
        let execution = execution_anchor_from_anchored_holding_source(
            &observation.source,
            &observation.network_id,
        )?;
        match by_network.get(&observation.network_id) {
            None => {
                by_network.insert(observation.network_id.clone(), execution);
            }
            Some(existing) if existing != &execution => {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::InconsistentNetworkAnchors,
                    format!(
                        "observation anchors disagree on network {}",
                        observation.network_id
                    ),
                    Some(observation.wallet_id.clone()),
                    Some(observation.network_id.clone()),
                ));
            }
            Some(_) => {}
        }
    }
    let mut pins: Vec<NetworkPin> = by_network
        .into_iter()
        .map(|(network_id, anchor)| NetworkPin { network_id, anchor })
        .collect();
    pins.sort_by(|left, right| left.network_id.cmp(&right.network_id));
    Ok(pins)
}

fn execution_anchor_from_anchored_holding_source(
    source: &AnchoredHoldingSource,
    network_id: &str,
) -> Result<ExecutionAnchor, PortfolioHoldingSelectionError> {
    match &source.anchor {
        ExecutionAnchor::Evm { .. } => Ok(source.anchor.clone()),
        ExecutionAnchor::Bitcoin { block_hash, .. } => {
            if block_hash.trim().is_empty() {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    "observation Bitcoin block_hash is required",
                    None,
                    Some(network_id.to_owned()),
                ));
            }
            Ok(source.anchor.clone())
        }
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
