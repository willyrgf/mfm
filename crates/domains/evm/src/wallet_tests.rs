use alloy_primitives::{address, b256, TxKind, B256, U256};

use super::*;

const SENDER: Address = address!("1111111111111111111111111111111111111111");
const RECIPIENT: Address = address!("2222222222222222222222222222222222222222");

#[test]
fn wallet_policy_descriptors_are_distinct_and_current() {
    let refs = [
        evm_wallet_nonce_policy_ref().expect("nonce policy"),
        evm_wallet_finality_policy_ref().expect("finality policy"),
        evm_wallet_assurance_policy_ref().expect("assurance policy"),
    ];
    assert_eq!(
        refs.iter().collect::<std::collections::BTreeSet<_>>().len(),
        refs.len()
    );
    let assurance = evm_wallet_assurance_policy_canonical().expect("assurance policy bytes");
    assert!(assurance
        .as_str()
        .contains("\"wallet_authority_lineage_and_fence\":\"exact\""));
}

#[test]
fn selector_and_fee_values_reject_noncanonical_inputs() {
    assert!(EvmTransactionTarget::new("primary").is_ok());
    assert!(EvmTransactionTarget::new("not valid").is_err());
    assert!(EvmWalletFeeCandidate::new(U256::from(1), U256::from(2)).is_err());
    assert!(
        serde_json::from_value::<EvmWalletFeeCandidate>(serde_json::json!({
            "max_fee_per_gas": "01",
            "max_priority_fee_per_gas": "1"
        }))
        .is_err()
    );
}

#[test]
fn transaction_template_preserves_order_and_rejects_duplicate_access_addresses() {
    let entry = EvmWalletAccessListEntry::new(
        RECIPIENT,
        vec![b256!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )],
    )
    .expect("access-list entry");
    let template = EvmWalletTransactionTemplate::new(
        EvmTransactionTarget::new("primary").expect("target"),
        EvmWalletTransactionAction::call(RECIPIENT),
        U256::from(5),
        [0x12, 0x34],
        vec![entry.clone()],
        U256::from(75_000),
    )
    .expect("template");
    assert_eq!(template.input(), "0x1234");
    assert_eq!(
        template.action().to_alloy().expect("target"),
        TxKind::Call(RECIPIENT)
    );

    assert!(EvmWalletTransactionTemplate::new(
        EvmTransactionTarget::new("primary").expect("target"),
        EvmWalletTransactionAction::call(RECIPIENT),
        U256::ZERO,
        [],
        vec![entry.clone(), entry],
        U256::from(21_000),
    )
    .is_err());
}

#[test]
fn transport_observations_reject_incoherent_receipt_evidence() {
    let block = EvmBlockAnchor::new(U256::from(9), B256::repeat_byte(0x44));
    let transaction_hash = B256::repeat_byte(0x55);
    let placement =
        EvmWalletTransactionPlacement::new(block.clone(), U256::from(1)).expect("placement");
    let transaction = EvmWalletObservedTransaction::new(
        transaction_hash,
        U256::from(1),
        U256::from(7),
        SENDER,
        TxKind::Call(RECIPIENT),
        U256::ZERO,
        [],
        U256::from(21_000),
        U256::from(20),
        U256::from(2),
        Vec::new(),
        Some(placement),
    )
    .expect("observed transaction");
    assert_eq!(
        transaction.transaction_hash(),
        format!("{transaction_hash:#x}")
    );

    let log = EvmWalletReceiptLog::new(
        RECIPIENT,
        Vec::new(),
        [],
        block.clone(),
        transaction_hash,
        U256::from(1),
        U256::ZERO,
        false,
    )
    .expect("receipt log");
    assert!(EvmWalletReceipt::new(
        transaction_hash,
        U256::from(1),
        block,
        SENDER,
        Some(RECIPIENT),
        None,
        EvmWalletReceiptStatus::Reverted,
        U256::from(21_000),
        U256::from(21_000),
        vec![log],
    )
    .is_err());
}

#[test]
fn submission_failure_is_closed_and_payload_free() {
    assert_eq!(
        serde_json::to_value(EvmSubmissionFailure::NonceDomainBusy).expect("failure JSON"),
        serde_json::json!({"kind": "nonce_domain_busy"})
    );
    assert!(
        serde_json::from_value::<EvmSubmissionFailure>(serde_json::json!({
            "kind": "nonce_domain_busy",
            "diagnostic": "provider text"
        }))
        .is_err()
    );
}
