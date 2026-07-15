use super::*;
use crate::HoldingRequirementKey;

use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::symbol::{
    AnchoredHoldingSource, HoldingSourceConfig, Observation, ObservationAnchor, ObservationQuantity,
};

#[test]
fn receipt_anchor_policy_digest_is_stable() {
    assert_eq!(
        portfolio_holding_selection_policy_digest(),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID.as_bytes()),
        )
    );
}

#[test]
fn pin_projection_requires_one_exact_anchor_per_network() {
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

#[test]
fn normalized_candidate_requires_admissible_fact_status() {
    let key = HoldingRequirementKey {
        wallet_id: "wallet".to_owned(),
        symbol_id: "eth.native.ethereum-mainnet".to_owned(),
        network_id: "ethereum-mainnet".to_owned(),
    };
    let claim = mfm_facts::FactClaimId::new(
        mfm_ids::RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"portfolio-selection-test-run"),
        ),
        1,
        0,
    )
    .expect("claim id");

    let candidate = holding_candidate_from_normalized(
        &key,
        9,
        claim.clone(),
        NormalizedHoldingFields {
            holding: HoldingSourceConfig::Native,
            raw_dec: "0".to_owned(),
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
    assert_eq!(candidate.fact_claim_id, claim);
    assert_eq!(candidate.response_material.raw_dec, "0");

    let error = holding_candidate_from_normalized(
        &key,
        9,
        mfm_facts::FactClaimId::new(
            mfm_ids::RunId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"portfolio-selection-test-run-2"),
            ),
            1,
            0,
        )
        .expect("claim id"),
        NormalizedHoldingFields {
            holding: HoldingSourceConfig::Native,
            raw_dec: "1".to_owned(),
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
    .expect_err("inadmissible coverage");
    assert_eq!(error.code, PortfolioHoldingErrorCode::MissingFact);
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
