use super::*;
use alloy_primitives::{B256, U256};
use mfm_evm::EvmBlockAnchor;
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::ExecutionAnchor;
use mfm_portfolio_model::symbol::{
    AnchoredHoldingSource, HoldingSourceConfig, Observation, ObservationQuantity,
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
        observation("w1", 100, [0xab; 32]),
        observation("w2", 100, [0xab; 32]),
    ];
    let pins = project_network_pins_from_observations(&observations).expect("pins");
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].network_id, "ethereum-mainnet");
    assert_eq!(
        pins[0].anchor,
        ExecutionAnchor::Evm {
            chain_id: std::num::NonZeroU64::new(1).expect("chain id"),
            block: EvmBlockAnchor::new(U256::from(100), B256::from([0xab; 32])),
        }
    );

    let disagreement = vec![
        observation("w1", 100, [0xab; 32]),
        observation("w2", 99, [0xde; 32]),
    ];
    assert_eq!(
        project_network_pins_from_observations(&disagreement)
            .expect_err("different anchor")
            .code,
        PortfolioHoldingErrorCode::InconsistentNetworkAnchors
    );
}

fn observation(wallet_id: &str, block_number: u64, block_hash: [u8; 32]) -> Observation {
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
            anchor: ExecutionAnchor::Evm {
                chain_id: std::num::NonZeroU64::new(1).expect("chain id"),
                block: EvmBlockAnchor::new(U256::from(block_number), B256::from(block_hash)),
            },
        },
        metadata: PublicMetadata::default(),
    }
}
