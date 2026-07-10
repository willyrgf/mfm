//! Network-coherent holding selection for fact-backed portfolio reports.
//!
//! Policy id: `mfm.portfolio.holding.latest-network-coherent.v1`
//!
//! Selection is pure: full Exact candidate sets per required holding, then
//! intersection of anchors within each network group. Independent per-holding
//! latest is forbidden (misses coherent older common anchors).

use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::sha256_digest_bytes;
use mfm_facts::FactClaimId;
use mfm_ids::{ContentDigest, DigestAlgorithm};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::portfolio::{ExecutionAnchor, NetworkPin};
use mfm_portfolio_model::symbol::{Observation, ObservationAnchor, ObservationSource};

/// Certified selection policy id for cutover portfolio holding selection.
pub const PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID: &str =
    "mfm.portfolio.holding.latest-network-coherent.v1";

/// Content digest of the selection policy id bytes (for selection evidence).
pub fn portfolio_holding_selection_policy_digest() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID.as_bytes()),
    )
}

/// Scope decision digest for Platform holding candidate fact-index queries.
pub fn portfolio_holding_select_scope_decision_hash() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.portfolio.holding.select.scope.v1"),
    )
}

/// Whether a candidate-build error should drop the row (filter-empty → missing_fact later).
pub const fn is_filter_empty_holding_error(code: PortfolioHoldingErrorCode) -> bool {
    matches!(
        code,
        PortfolioHoldingErrorCode::MissingFact | PortfolioHoldingErrorCode::UnsupportedRequirement
    )
}

/// Minimal public hard-fail codes for portfolio holding selection / assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortfolioHoldingErrorCode {
    /// No acceptable Platform fact for a required subject (includes coverage filter-empty).
    MissingFact,
    /// Network group has facts but empty intersection of anchors.
    NoCommonNetworkAnchor,
    /// Selection cardinality violated after policy / receipt.
    AmbiguousFacts,
    /// Residual same-network selected anchors disagree (guard).
    InconsistentNetworkAnchors,
    /// Selection batch receipts do not share one snapshot read frontier.
    MixedReadFrontier,
    /// Portfolio requirement has no projection rule (e.g. ERC-20 at cutover).
    UnsupportedRequirement,
}

impl PortfolioHoldingErrorCode {
    /// Returns the stable public error code string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingFact => "missing_fact",
            Self::NoCommonNetworkAnchor => "no_common_network_anchor",
            Self::AmbiguousFacts => "ambiguous_facts",
            Self::InconsistentNetworkAnchors => "inconsistent_network_anchors",
            Self::MixedReadFrontier => "mixed_read_frontier",
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
    /// Balance reader kind tag for observation source.
    pub balance_reader_kind: String,
    /// Raw amount decimal string.
    pub raw_dec: String,
    /// Token decimals.
    pub decimals: u8,
    /// Family-specific execution anchor for observation source.
    pub observation_anchor: ObservationAnchor,
    /// Coverage tag retained for honesty surfaces after selection.
    pub coverage: String,
    /// Source status tag.
    pub source_status: String,
}

/// Required holding key used as map key for selection inputs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequiredHoldingKey {
    /// Wallet id.
    pub wallet_id: String,
    /// Symbol id.
    pub symbol_id: String,
    /// Network id.
    pub network_id: String,
}

impl RequiredHoldingKey {
    /// Stable key string for diagnostics.
    pub fn as_key_str(&self) -> String {
        format!("{}/{}/{}", self.wallet_id, self.symbol_id, self.network_id)
    }
}

/// Family-normalized quantity/anchor fields used to build a [`HoldingCandidate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedHoldingFields {
    /// Balance reader kind tag for observation source.
    pub balance_reader_kind: String,
    /// Raw amount decimal string.
    pub raw_dec: String,
    /// Token decimals.
    pub decimals: u8,
    /// Family-specific execution anchor for observation source.
    pub observation_anchor: ObservationAnchor,
    /// Coverage tag checked by the portfolio selection policy.
    pub coverage: String,
    /// Source status tag.
    pub source_status: String,
}

/// Builds a holding candidate from family-normalized scalars (no BTC/EVM types).
///
/// Family crates own normalize/acceptability; this joins report keys and selection anchors.
pub fn holding_candidate_from_normalized(
    key: &RequiredHoldingKey,
    store_commit_order: u64,
    fact_claim_id: FactClaimId,
    fields: NormalizedHoldingFields,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let coverage = fields.coverage.parse::<CoverageStatus>().map_err(|_| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            "holding fact has unknown coverage status",
            Some(key.as_key_str()),
            Some(key.network_id.clone()),
        )
    })?;
    let source_status = fields
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
    if !portfolio_selection_accepts_status(coverage, source_status) {
        return Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            "holding fact is not acceptable for portfolio selection",
            Some(key.as_key_str()),
            Some(key.network_id.clone()),
        ));
    }
    let (height, hash) = match &fields.observation_anchor {
        ObservationAnchor::Bitcoin { height, block_hash } => (*height, block_hash.clone()),
        ObservationAnchor::Evm {
            block_number,
            block_hash,
            ..
        } => (*block_number, block_hash.clone()),
    };
    let anchor = HoldingAnchor::new(height, hash).map_err(|mut error| {
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
            balance_reader_kind: fields.balance_reader_kind,
            raw_dec: fields.raw_dec,
            decimals: fields.decimals,
            observation_anchor: fields.observation_anchor,
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

/// One selected holding after network-coherent selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedHolding {
    /// Required holding key.
    pub key: RequiredHoldingKey,
    /// Chosen anchor (shared across same-network group).
    pub anchor: HoldingAnchor,
    /// Store commit order of the winning candidate.
    pub store_commit_order: u64,
    /// Fact claim id of the winning candidate.
    pub fact_claim_id: FactClaimId,
    /// Material for observation construction.
    pub material: SelectedHoldingMaterial,
}

/// Pure network-coherent selection over full candidate sets.
///
/// Canonical case: A@100+A@99, B@99 same network → both @99.
pub fn select_network_coherent(
    candidates_by_holding: &BTreeMap<RequiredHoldingKey, Vec<HoldingCandidate>>,
) -> Result<Vec<SelectedHolding>, PortfolioHoldingSelectionError> {
    if candidates_by_holding.is_empty() {
        return Ok(Vec::new());
    }

    // Group required holdings by network_id.
    let mut by_network: BTreeMap<String, Vec<&RequiredHoldingKey>> = BTreeMap::new();
    for key in candidates_by_holding.keys() {
        by_network
            .entry(key.network_id.clone())
            .or_default()
            .push(key);
    }

    let mut selected = Vec::new();

    for (network_id, holdings) in by_network {
        // Empty candidate sets → missing_fact.
        for key in &holdings {
            let candidates = candidates_by_holding.get(*key).expect("key from map keys");
            if candidates.is_empty() {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    format!("no acceptable Platform fact for {}", key.as_key_str()),
                    Some(key.as_key_str()),
                    Some(network_id.clone()),
                ));
            }
            // Guard: candidates must share the holding's network_id.
            for candidate in candidates {
                if candidate.network_id != network_id {
                    return Err(PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::InconsistentNetworkAnchors,
                        "candidate network_id disagrees with holding network_id",
                        Some(key.as_key_str()),
                        Some(network_id.clone()),
                    ));
                }
            }
        }

        // common_anchors = intersection of candidate anchors per holding in group.
        let mut common: Option<BTreeSet<HoldingAnchor>> = None;
        for key in &holdings {
            let anchors: BTreeSet<HoldingAnchor> = candidates_by_holding
                .get(*key)
                .expect("key present")
                .iter()
                .map(|c| c.anchor.clone())
                .collect();
            common = Some(match common {
                None => anchors,
                Some(existing) => existing.intersection(&anchors).cloned().collect(),
            });
        }
        let common = common.unwrap_or_default();
        if common.is_empty() {
            return Err(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::NoCommonNetworkAnchor,
                format!("no common network anchor for holdings on network {network_id}"),
                None,
                Some(network_id.clone()),
            ));
        }

        // chosen_anchor = max by height desc, then hash bytes desc.
        let chosen_anchor = common
            .into_iter()
            .max_by(|left, right| {
                left.height
                    .cmp(&right.height)
                    .then_with(|| left.hash.as_bytes().cmp(right.hash.as_bytes()))
            })
            .expect("non-empty common set");

        for key in holdings {
            let candidates = candidates_by_holding.get(key).expect("key present");
            let mut at: Vec<&HoldingCandidate> = candidates
                .iter()
                .filter(|c| c.anchor == chosen_anchor)
                .collect();
            if at.is_empty() {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    format!("no candidate at chosen anchor for {}", key.as_key_str()),
                    Some(key.as_key_str()),
                    Some(network_id.clone()),
                ));
            }
            // LWW v1: store_commit_order DESC, then FactClaimId coordinate DESC (RFC secondary).
            at.sort_by(|left, right| {
                right
                    .store_commit_order
                    .cmp(&left.store_commit_order)
                    .then_with(|| right.fact_claim_id.cmp(&left.fact_claim_id))
            });
            let winner = at[0];
            selected.push(SelectedHolding {
                key: key.clone(),
                anchor: chosen_anchor.clone(),
                store_commit_order: winner.store_commit_order,
                fact_claim_id: winner.fact_claim_id.clone(),
                material: winner.response_material.clone(),
            });
        }
    }

    // Residual same-network selected anchors must agree.
    let mut network_anchors: BTreeMap<String, HoldingAnchor> = BTreeMap::new();
    for item in &selected {
        match network_anchors.get(&item.key.network_id) {
            None => {
                network_anchors.insert(item.key.network_id.clone(), item.anchor.clone());
            }
            Some(existing) if existing != &item.anchor => {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::InconsistentNetworkAnchors,
                    format!(
                        "selected anchors disagree on network {}",
                        item.key.network_id
                    ),
                    Some(item.key.as_key_str()),
                    Some(item.key.network_id.clone()),
                ));
            }
            Some(_) => {}
        }
    }

    selected.sort_by(|left, right| {
        (
            left.key.network_id.as_str(),
            left.key.wallet_id.as_str(),
            left.key.symbol_id.as_str(),
        )
            .cmp(&(
                right.key.network_id.as_str(),
                right.key.wallet_id.as_str(),
                right.key.symbol_id.as_str(),
            ))
    });
    Ok(selected)
}

/// Projects `network_pins` purely from selected observation anchors.
///
/// One pin per network; residual same-network disagreement → inconsistent_network_anchors.
pub fn project_network_pins_from_observations(
    observations: &[Observation],
) -> Result<Vec<NetworkPin>, PortfolioHoldingSelectionError> {
    let mut by_network: BTreeMap<String, ExecutionAnchor> = BTreeMap::new();
    for observation in observations {
        let execution = execution_anchor_from_observation_source(&observation.source)?;
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

fn execution_anchor_from_observation_source(
    source: &ObservationSource,
) -> Result<ExecutionAnchor, PortfolioHoldingSelectionError> {
    match &source.anchor {
        ObservationAnchor::Evm {
            chain_id,
            block_number,
            block_hash,
        } => {
            if block_hash.trim().is_empty() {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    "observation EVM block_hash is required",
                    None,
                    Some(source.network_id.clone()),
                ));
            }
            Ok(ExecutionAnchor::Evm {
                chain_id: *chain_id,
                block_number: *block_number,
                block_hash: block_hash.clone(),
            })
        }
        ObservationAnchor::Bitcoin { height, block_hash } => {
            if block_hash.trim().is_empty() {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    "observation Bitcoin block_hash is required",
                    None,
                    Some(source.network_id.clone()),
                ));
            }
            Ok(ExecutionAnchor::Bitcoin {
                height: *height,
                block_hash: block_hash.clone(),
            })
        }
    }
}

/// Subject projection kind supported at cutover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldingFactProjection {
    /// `bitcoin.address_balance_snapshot`.
    BitcoinAddressBalance,
    /// `evm.address_native_balance_snapshot`.
    EvmNativeBalance,
}

/// Classifies a configured symbol into a cutover fact projection, or unsupported.
pub fn project_holding_fact_kind(
    symbol_kind: &str,
) -> Result<HoldingFactProjection, PortfolioHoldingSelectionError> {
    match symbol_kind {
        "native_asset" | "native" | "bitcoin_native" => {
            // Disambiguated by network family at call site; this is the kind tag surface.
            // Callers should use project_holding_fact_for_network.
            Ok(HoldingFactProjection::BitcoinAddressBalance)
        }
        "evm_native" => Ok(HoldingFactProjection::EvmNativeBalance),
        "erc20" | "erc-20" | "token" => Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            format!("symbol kind {symbol_kind} is not supported at cutover"),
            None,
            None,
        )),
        other => Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            format!("symbol kind {other} has no holding fact projection"),
            None,
            None,
        )),
    }
}

/// Projects cutover fact kind from network family + symbol shape.
pub fn project_holding_fact_for_network(
    network_family: &str,
    symbol_is_native: bool,
) -> Result<HoldingFactProjection, PortfolioHoldingSelectionError> {
    match (network_family, symbol_is_native) {
        ("bitcoin", true) => Ok(HoldingFactProjection::BitcoinAddressBalance),
        ("evm", true) => Ok(HoldingFactProjection::EvmNativeBalance),
        ("bitcoin" | "evm", false) => Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            "non-native holdings are not supported at cutover",
            None,
            None,
        )),
        (family, _) => Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            format!("network family {family} has no holding fact projection"),
            None,
            None,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_ids::RunId;
    use mfm_portfolio_model::metadata::PublicMetadata;
    use mfm_portfolio_model::symbol::{ObservationQuantity, SymbolKind, SymbolRole};

    fn key(wallet: &str, symbol: &str, network: &str) -> RequiredHoldingKey {
        RequiredHoldingKey {
            wallet_id: wallet.to_owned(),
            symbol_id: symbol.to_owned(),
            network_id: network.to_owned(),
        }
    }

    fn claim(source_seq: u64) -> FactClaimId {
        FactClaimId::new(
            RunId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"portfolio-selection-test-run"),
            ),
            source_seq,
            0,
        )
        .expect("claim id")
    }

    fn material(
        wallet: &str,
        symbol: &str,
        network: &str,
        height: u64,
        hash: &str,
    ) -> SelectedHoldingMaterial {
        SelectedHoldingMaterial {
            wallet_id: wallet.to_owned(),
            symbol_id: symbol.to_owned(),
            network_id: network.to_owned(),
            balance_reader_kind: "bitcoin.address_balance".to_owned(),
            raw_dec: "1".to_owned(),
            decimals: 8,
            observation_anchor: ObservationAnchor::Bitcoin {
                height,
                block_hash: hash.to_owned(),
            },
            coverage: "configured_only".to_owned(),
            source_status: "ok".to_owned(),
        }
    }

    fn candidate(
        network: &str,
        height: u64,
        hash: &str,
        store_commit_order: u64,
        claim_sequence: u64,
        material: SelectedHoldingMaterial,
    ) -> HoldingCandidate {
        HoldingCandidate {
            network_id: network.to_owned(),
            anchor: HoldingAnchor::new(height, hash).expect("anchor"),
            store_commit_order,
            fact_claim_id: claim(claim_sequence),
            response_material: material,
        }
    }

    #[test]
    fn network_coherent_selects_common_older_anchor() {
        // A@100, A@99; B@99 → both @99 (not independent latest A@100 + B@99).
        let a = key("w1", "btc", "bitcoin-mainnet");
        let b = key("w2", "btc", "bitcoin-mainnet");
        let mut map = BTreeMap::new();
        map.insert(
            a.clone(),
            vec![
                candidate(
                    "bitcoin-mainnet",
                    100,
                    "hash100",
                    2,
                    100,
                    material("w1", "btc", "bitcoin-mainnet", 100, "hash100"),
                ),
                candidate(
                    "bitcoin-mainnet",
                    99,
                    "hash99",
                    1,
                    99,
                    material("w1", "btc", "bitcoin-mainnet", 99, "hash99"),
                ),
            ],
        );
        map.insert(
            b.clone(),
            vec![candidate(
                "bitcoin-mainnet",
                99,
                "hash99",
                3,
                99,
                material("w2", "btc", "bitcoin-mainnet", 99, "hash99"),
            )],
        );

        let selected = select_network_coherent(&map).expect("select");
        assert_eq!(selected.len(), 2);
        assert!(selected
            .iter()
            .all(|s| s.anchor.height == 99 && s.anchor.hash == "hash99"));
        assert_eq!(
            selected
                .iter()
                .find(|s| s.key == a)
                .expect("a")
                .fact_claim_id,
            claim(99)
        );
        assert_eq!(
            selected
                .iter()
                .find(|s| s.key == b)
                .expect("b")
                .fact_claim_id,
            claim(99)
        );
    }

    #[test]
    fn empty_common_anchors_hard_fails() {
        let a = key("w1", "btc", "bitcoin-mainnet");
        let b = key("w2", "btc", "bitcoin-mainnet");
        let mut map = BTreeMap::new();
        map.insert(
            a,
            vec![candidate(
                "bitcoin-mainnet",
                100,
                "hash100",
                1,
                1,
                material("w1", "btc", "bitcoin-mainnet", 100, "hash100"),
            )],
        );
        map.insert(
            b,
            vec![candidate(
                "bitcoin-mainnet",
                99,
                "hash99",
                1,
                2,
                material("w2", "btc", "bitcoin-mainnet", 99, "hash99"),
            )],
        );

        let err = select_network_coherent(&map).expect_err("no common");
        assert_eq!(err.code, PortfolioHoldingErrorCode::NoCommonNetworkAnchor);
        assert_eq!(err.code.as_str(), "no_common_network_anchor");
    }

    #[test]
    fn empty_candidates_are_missing_fact() {
        let a = key("w1", "btc", "bitcoin-mainnet");
        let mut map = BTreeMap::new();
        map.insert(a, Vec::new());
        let err = select_network_coherent(&map).expect_err("missing");
        assert_eq!(err.code, PortfolioHoldingErrorCode::MissingFact);
    }

    #[test]
    fn lww_prefers_higher_store_commit_order_then_typed_fact_claim_id() {
        let a = key("w1", "btc", "bitcoin-mainnet");
        let mut map = BTreeMap::new();
        map.insert(
            a.clone(),
            vec![
                candidate(
                    "bitcoin-mainnet",
                    50,
                    "hash50",
                    10,
                    1,
                    material("w1", "btc", "bitcoin-mainnet", 50, "hash50"),
                ),
                candidate(
                    "bitcoin-mainnet",
                    50,
                    "hash50",
                    20,
                    2,
                    material("w1", "btc", "bitcoin-mainnet", 50, "hash50"),
                ),
            ],
        );
        let selected = select_network_coherent(&map).expect("select");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].fact_claim_id, claim(2));
        assert_eq!(selected[0].store_commit_order, 20);

        // Equal store_commit_order: compare typed coordinates, not decimal text where `9`
        // sorts after `10` lexically.
        map.insert(
            a,
            vec![
                candidate(
                    "bitcoin-mainnet",
                    50,
                    "hash50",
                    20,
                    9,
                    material("w1", "btc", "bitcoin-mainnet", 50, "hash50"),
                ),
                candidate(
                    "bitcoin-mainnet",
                    50,
                    "hash50",
                    20,
                    10,
                    material("w1", "btc", "bitcoin-mainnet", 50, "hash50"),
                ),
            ],
        );
        let selected = select_network_coherent(&map).expect("select equal commit");
        assert_eq!(selected[0].fact_claim_id, claim(10));
        assert_eq!(selected[0].store_commit_order, 20);
    }

    #[test]
    fn pin_projection_matches_observation_anchors() {
        let observations = vec![
            Observation {
                wallet_id: "w1".to_owned(),
                symbol_id: "eth".to_owned(),
                display_symbol: None,
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Asset,
                network_id: "ethereum-mainnet".to_owned(),
                protocol: None,
                quantity: ObservationQuantity {
                    raw_dec: "1".to_owned(),
                    decimals: 18,
                    amount_dec: "0.000000000000000001".to_owned(),
                },
                values: Vec::new(),
                source: ObservationSource {
                    balance_reader_kind: "evm.native".to_owned(),
                    network_id: "ethereum-mainnet".to_owned(),
                    anchor: ObservationAnchor::Evm {
                        chain_id: 1,
                        block_number: 100,
                        block_hash: "0xabc".to_owned(),
                    },
                },
                coverage: "configured_only".to_owned(),
                metadata: PublicMetadata::default(),
            },
            Observation {
                wallet_id: "w2".to_owned(),
                symbol_id: "eth".to_owned(),
                display_symbol: None,
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Asset,
                network_id: "ethereum-mainnet".to_owned(),
                protocol: None,
                quantity: ObservationQuantity {
                    raw_dec: "2".to_owned(),
                    decimals: 18,
                    amount_dec: "0.000000000000000002".to_owned(),
                },
                values: Vec::new(),
                source: ObservationSource {
                    balance_reader_kind: "evm.native".to_owned(),
                    network_id: "ethereum-mainnet".to_owned(),
                    anchor: ObservationAnchor::Evm {
                        chain_id: 1,
                        block_number: 100,
                        block_hash: "0xabc".to_owned(),
                    },
                },
                coverage: "configured_only".to_owned(),
                metadata: PublicMetadata::default(),
            },
        ];
        let pins = project_network_pins_from_observations(&observations).expect("pins");
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].network_id, "ethereum-mainnet");
        assert_eq!(
            pins[0].anchor,
            ExecutionAnchor::Evm {
                chain_id: 1,
                block_number: 100,
                block_hash: "0xabc".to_owned(),
            }
        );
    }

    #[test]
    fn pin_projection_rejects_same_network_disagreement() {
        let observations = vec![
            Observation {
                wallet_id: "w1".to_owned(),
                symbol_id: "eth".to_owned(),
                display_symbol: None,
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Asset,
                network_id: "ethereum-mainnet".to_owned(),
                protocol: None,
                quantity: ObservationQuantity {
                    raw_dec: "1".to_owned(),
                    decimals: 18,
                    amount_dec: "1".to_owned(),
                },
                values: Vec::new(),
                source: ObservationSource {
                    balance_reader_kind: "evm.native".to_owned(),
                    network_id: "ethereum-mainnet".to_owned(),
                    anchor: ObservationAnchor::Evm {
                        chain_id: 1,
                        block_number: 100,
                        block_hash: "0xabc".to_owned(),
                    },
                },
                coverage: "configured_only".to_owned(),
                metadata: PublicMetadata::default(),
            },
            Observation {
                wallet_id: "w2".to_owned(),
                symbol_id: "eth".to_owned(),
                display_symbol: None,
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Asset,
                network_id: "ethereum-mainnet".to_owned(),
                protocol: None,
                quantity: ObservationQuantity {
                    raw_dec: "2".to_owned(),
                    decimals: 18,
                    amount_dec: "2".to_owned(),
                },
                values: Vec::new(),
                source: ObservationSource {
                    balance_reader_kind: "evm.native".to_owned(),
                    network_id: "ethereum-mainnet".to_owned(),
                    anchor: ObservationAnchor::Evm {
                        chain_id: 1,
                        block_number: 99,
                        block_hash: "0xdef".to_owned(),
                    },
                },
                coverage: "configured_only".to_owned(),
                metadata: PublicMetadata::default(),
            },
        ];
        let err = project_network_pins_from_observations(&observations).expect_err("disagree");
        assert_eq!(
            err.code,
            PortfolioHoldingErrorCode::InconsistentNetworkAnchors
        );
    }

    #[test]
    fn erc20_projection_is_unsupported_requirement() {
        let err = project_holding_fact_kind("erc20").expect_err("erc20");
        assert_eq!(err.code, PortfolioHoldingErrorCode::UnsupportedRequirement);
        let err = project_holding_fact_for_network("evm", false).expect_err("token");
        assert_eq!(err.code, PortfolioHoldingErrorCode::UnsupportedRequirement);
    }

    #[test]
    fn policy_digest_is_stable() {
        assert_eq!(
            portfolio_holding_selection_policy_digest(),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID.as_bytes()),
            )
        );
    }

    #[test]
    fn holding_candidate_from_normalized_builds_anchor_and_material() {
        let key = RequiredHoldingKey {
            wallet_id: "w1".to_owned(),
            symbol_id: "eth.native".to_owned(),
            network_id: "ethereum-mainnet".to_owned(),
        };
        let candidate = holding_candidate_from_normalized(
            &key,
            9,
            claim(1),
            NormalizedHoldingFields {
                balance_reader_kind: "native_balance".to_owned(),
                raw_dec: "1000".to_owned(),
                decimals: 18,
                observation_anchor: ObservationAnchor::Evm {
                    chain_id: 1,
                    block_number: 42,
                    block_hash: "0x".to_owned() + &"ab".repeat(32),
                },
                coverage: "configured_only".to_owned(),
                source_status: "ok".to_owned(),
            },
        )
        .expect("candidate");
        assert_eq!(candidate.store_commit_order, 9);
        assert_eq!(candidate.fact_claim_id, claim(1));
        assert_eq!(candidate.anchor.height, 42);
        assert_eq!(candidate.response_material.wallet_id, "w1");
        assert_eq!(candidate.response_material.coverage, "configured_only");
    }

    #[test]
    fn portfolio_selection_filters_unacceptable_holding_statuses() {
        let key = RequiredHoldingKey {
            wallet_id: "w1".to_owned(),
            symbol_id: "eth.native".to_owned(),
            network_id: "ethereum-mainnet".to_owned(),
        };
        let error = holding_candidate_from_normalized(
            &key,
            9,
            claim(1),
            NormalizedHoldingFields {
                balance_reader_kind: "native_balance".to_owned(),
                raw_dec: "1000".to_owned(),
                decimals: 18,
                observation_anchor: ObservationAnchor::Evm {
                    chain_id: 1,
                    block_number: 42,
                    block_hash: "0x".to_owned() + &"ab".repeat(32),
                },
                coverage: "truncated".to_owned(),
                source_status: "ok".to_owned(),
            },
        )
        .expect_err("portfolio policy rejects truncated coverage");

        assert_eq!(error.code, PortfolioHoldingErrorCode::MissingFact);
    }

    #[test]
    fn holding_candidate_rejects_empty_hash() {
        let key = RequiredHoldingKey {
            wallet_id: "w1".to_owned(),
            symbol_id: "btc.native".to_owned(),
            network_id: "bitcoin-mainnet".to_owned(),
        };
        let err = holding_candidate_from_normalized(
            &key,
            1,
            claim(1),
            NormalizedHoldingFields {
                balance_reader_kind: "native_balance".to_owned(),
                raw_dec: "0".to_owned(),
                decimals: 8,
                observation_anchor: ObservationAnchor::Bitcoin {
                    height: 1,
                    block_hash: "   ".to_owned(),
                },
                coverage: "configured_only".to_owned(),
                source_status: "ok".to_owned(),
            },
        )
        .expect_err("empty hash");
        assert_eq!(err.code, PortfolioHoldingErrorCode::MissingFact);
    }

    #[test]
    fn filter_empty_codes_are_missing_and_unsupported() {
        assert!(is_filter_empty_holding_error(
            PortfolioHoldingErrorCode::MissingFact
        ));
        assert!(is_filter_empty_holding_error(
            PortfolioHoldingErrorCode::UnsupportedRequirement
        ));
        assert!(!is_filter_empty_holding_error(
            PortfolioHoldingErrorCode::NoCommonNetworkAnchor
        ));
    }
}
