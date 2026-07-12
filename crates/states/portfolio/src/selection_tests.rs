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
