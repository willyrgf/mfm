use super::*;

use mfm_ids::RunId;
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::symbol::{
    AnchoredHoldingSource, HoldingSourceConfig, Observation, ObservationAnchor, ObservationQuantity,
};

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
        holding: HoldingSourceConfig::Native,
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
        .all(|selected| selected.anchor.height == 99 && selected.anchor.hash == "hash99"));
    assert_eq!(
        selected
            .iter()
            .find(|selected| selected.key == a)
            .expect("a")
            .fact_claim_id,
        claim(99)
    );
    assert_eq!(
        selected
            .iter()
            .find(|selected| selected.key == b)
            .expect("b")
            .fact_claim_id,
        claim(99)
    );
}

#[test]
fn selection_requires_candidates_and_one_shared_anchor() {
    let a = key("w1", "btc", "bitcoin-mainnet");
    let b = key("w2", "btc", "bitcoin-mainnet");
    let mut map = BTreeMap::new();
    map.insert(a.clone(), Vec::new());
    assert_eq!(
        select_network_coherent(&map).expect_err("missing").code,
        PortfolioHoldingErrorCode::MissingFact
    );

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
    assert_eq!(
        select_network_coherent(&map)
            .expect_err("no common anchor")
            .code,
        PortfolioHoldingErrorCode::NoCommonNetworkAnchor
    );
}

#[test]
fn selection_prefers_newer_store_order_then_typed_claim_coordinate() {
    let required = key("w1", "btc", "bitcoin-mainnet");
    let mut map = BTreeMap::new();
    map.insert(
        required.clone(),
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

    let selected = select_network_coherent(&map).expect("select");
    assert_eq!(selected[0].fact_claim_id, claim(10));
    assert_eq!(selected[0].store_commit_order, 20);
}

#[test]
fn pin_projection_uses_direct_anchored_holding_source() {
    let observations = vec![
        observation("w1", 100, "0xabc"),
        observation("w2", 100, "0xabc"),
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

    let disagreement = vec![
        observation("w1", 100, "0xabc"),
        observation("w2", 99, "0xdef"),
    ];
    assert_eq!(
        project_network_pins_from_observations(&disagreement)
            .expect_err("different anchor")
            .code,
        PortfolioHoldingErrorCode::InconsistentNetworkAnchors
    );
}

fn observation(wallet_id: &str, block_number: u64, block_hash: &str) -> Observation {
    Observation {
        wallet_id: wallet_id.to_owned(),
        symbol_id: "eth".to_owned(),
        display_symbol: None,
        network_id: "ethereum-mainnet".to_owned(),
        quantity: ObservationQuantity {
            raw_dec: "1".to_owned(),
            decimals: 18,
            amount_dec: "0.000000000000000001".to_owned(),
        },
        values: Vec::new(),
        source: AnchoredHoldingSource {
            holding: HoldingSourceConfig::Native,
            anchor: ObservationAnchor::Evm {
                chain_id: 1,
                block_number,
                block_hash: block_hash.to_owned(),
            },
        },
        coverage: "configured_only".to_owned(),
        metadata: PublicMetadata::default(),
    }
}

#[test]
fn direct_source_projection_and_candidate_material_are_checked() {
    let erc20 = HoldingSourceConfig::Erc20 {
        contract_address: "0x0000000000000000000000000000000000000001"
            .parse()
            .expect("token"),
    };
    assert_eq!(
        project_holding_fact_for_network(NetworkFamilyConfig::Evm, &erc20)
            .expect_err("internal ERC-20 collection not installed yet")
            .code,
        PortfolioHoldingErrorCode::UnsupportedRequirement
    );

    let required = key("w1", "eth.native", "ethereum-mainnet");
    let candidate = holding_candidate_from_normalized(
        &required,
        9,
        claim(1),
        NormalizedHoldingFields {
            holding: HoldingSourceConfig::Native,
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
    assert_eq!(
        candidate.response_material.holding,
        HoldingSourceConfig::Native
    );
    assert_eq!(candidate.anchor.height, 42);

    let error = holding_candidate_from_normalized(
        &required,
        9,
        claim(1),
        NormalizedHoldingFields {
            holding: HoldingSourceConfig::Native,
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
    .expect_err("truncated holding");
    assert_eq!(error.code, PortfolioHoldingErrorCode::MissingFact);
}

#[test]
fn policy_digest_and_filter_empty_codes_are_stable() {
    assert_eq!(
        portfolio_holding_selection_policy_digest(),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID.as_bytes()),
        )
    );
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
