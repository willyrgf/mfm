use std::num::NonZeroU64;

use mfm_canonical::raw_content_digest;
use mfm_evm::{
    EvmAddress, EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance, EvmHash,
    EvmTransactionConfirmation, EvmTransactionSettlement, EvmU256,
};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, EffectId, SchemaId};

use super::*;

fn reference(name: &str, byte: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
        .expect("schema"),
        raw_content_digest(&[byte]),
    )
    .expect("content ref")
}

fn domain() -> NonceDomain {
    NonceDomain::new(
        EvmAuthorityEpoch::new([1; 32]),
        EvmChainInstance::new(
            NonZeroU64::new(1).expect("nonzero chain"),
            EvmHash::new(format!("0x{}", "02".repeat(32))).expect("genesis"),
        ),
        EvmAddress::new("0x0303030303030303030303030303030303030303").expect("sender"),
    )
}

fn settlement(effect_id: EffectId, nonce: u64, hash: EvmHash) -> EvmTransactionSettlement {
    EvmTransactionSettlement::confirmed(
        effect_id,
        nonce,
        EvmTransactionConfirmation::Called {
            block_anchor: EvmBlockAnchor::new(
                EvmU256::from_u64(8),
                EvmHash::new(format!("0x{}", "09".repeat(32))).expect("block hash"),
            ),
            transaction_hash: hash,
        },
    )
}

#[test]
fn exact_raw_transaction_enforces_both_bounds() {
    assert_eq!(
        ExactRawTransaction::new(Vec::new()).err(),
        Some(AuthorityError::Internal)
    );
    assert!(ExactRawTransaction::new(vec![1]).is_ok());
    assert!(ExactRawTransaction::new(vec![1; MAX_EXACT_RAW_TRANSACTION_BYTES]).is_ok());
    assert_eq!(
        ExactRawTransaction::new(vec![1; MAX_EXACT_RAW_TRANSACTION_BYTES + 1]).err(),
        Some(AuthorityError::Internal)
    );
}

#[test]
fn settlement_requires_every_predecessor_identity() {
    let effect_id = EffectId::from_digest(DigestBytes::from_array([5; 32]));
    let transaction_hash =
        EvmHash::new(format!("0x{}", "06".repeat(32))).expect("transaction hash");
    let reservation = Reservation::new(
        effect_id.clone(),
        reference("mfm.test.command", 7),
        domain(),
        11,
    );
    let prepared = PreparedRecord::new(
        reservation,
        transaction_hash.clone(),
        ExactRawTransaction::new(vec![2, 3]).expect("raw"),
    );
    assert!(SettledRecord::new(
        prepared.clone(),
        settlement(effect_id.clone(), 11, transaction_hash.clone()),
    )
    .is_ok());
    assert!(SettledRecord::new(
        prepared.clone(),
        settlement(
            EffectId::from_digest(DigestBytes::from_array([8; 32])),
            11,
            transaction_hash.clone(),
        ),
    )
    .is_err());
    assert!(SettledRecord::new(
        prepared.clone(),
        settlement(effect_id.clone(), 12, transaction_hash.clone()),
    )
    .is_err());
    assert!(SettledRecord::new(
        prepared,
        settlement(
            effect_id,
            11,
            EvmHash::new(format!("0x{}", "0a".repeat(32))).expect("wrong hash"),
        ),
    )
    .is_err());
}
