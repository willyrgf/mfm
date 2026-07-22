//! Receipt-pinned holding projection helpers for fact-backed portfolio reports.
//!
//! Policy id: `mfm.portfolio.holding.collection-receipt-anchor.v1`.
//! The typed family receipts fix every source, anchor, and fact-content identity before this module
//! projects hydrated, identity-matching facts into observations. Receipt and fact eligibility are
//! normalized only after exact identity has been established.

use std::collections::BTreeMap;

use crate::{AnchoredHoldingSource, ExecutionAnchor, HoldingSourceConfig, NetworkPin, Observation};
use mfm_canonical::sha256_digest_bytes;
use mfm_facts::FactClaimId;
use mfm_ids::{ContentDigest, DigestAlgorithm};

/// Certified selection policy id for receipt-pinned portfolio holding selection.
pub const PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID: &str =
    "mfm.portfolio.holding.collection-receipt-anchor.v1";

/// Content digest of the selection policy id bytes (for selection evidence).
pub(super) fn portfolio_holding_selection_policy_digest() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID.as_bytes()),
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

/// One acceptable candidate fact for a required holding after receipt and identity validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HoldingCandidate {
    /// Network id of the holding.
    pub network_id: String,
    /// Anchor at which the balance was proven.
    pub anchor: ExecutionAnchor,
    /// Primary LWW ordering for candidates at the same subject and anchor.
    pub store_commit_order: u64,
    /// Claim identity for deterministic LWW tie-breaking and evidence binding.
    pub fact_claim_id: FactClaimId,
    /// Hydrated response payload opaque to pure selection (passed through).
    pub response_material: SelectedHoldingMaterial,
}

/// Selected holding material after pure selection (family-normalized fields + join keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SelectedHoldingMaterial {
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
}

pub(crate) struct BitcoinHoldingCandidateFields {
    pub(crate) holding: HoldingSourceConfig,
    pub(crate) raw_dec: String,
    pub(crate) decimals: u8,
    pub(crate) height: u64,
    pub(crate) block_hash: String,
}

pub(crate) fn holding_candidate_from_bitcoin(
    key: &super::HoldingRequirementKey,
    store_commit_order: u64,
    fact_claim_id: FactClaimId,
    fields: BitcoinHoldingCandidateFields,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let anchor = ExecutionAnchor::Bitcoin {
        height: fields.height,
        block_hash: fields.block_hash.clone(),
    };
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
        },
    })
}

/// One selected holding after receipt-pinned identity filtering and ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SelectedHolding {
    /// Required holding key.
    pub key: super::HoldingRequirementKey,
    /// Anchor verified against the exact receipt entry.
    pub anchor: ExecutionAnchor,
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
pub(super) fn project_network_pins_from_observations(
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
mod tests;
