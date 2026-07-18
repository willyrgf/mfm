use super::*;
use mfm_evm_capabilities::{
    EvmReceipt, EvmReceiptLog, EvmTransactionPlacement as CapabilityPlacement,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};

const NETWORK: &str = "ethereum-mainnet";

fn sender() -> Address {
    Address::from([0x11; 20])
}

fn destination() -> Address {
    Address::from([0x22; 20])
}

fn binding() -> EvmNetworkBinding {
    EvmNetworkBinding::new(LocalPublicId::new(NETWORK).expect("network"), 1).expect("binding")
}

fn session(source: &str) -> EvmSessionEvidence {
    EvmSessionEvidence::new(
        &binding(),
        LocalPublicId::new(source).expect("source"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
    )
}

fn config() -> EvmTransactionConfig {
    EvmTransactionConfig::new(
        NETWORK,
        1,
        sender(),
        SignerRef::new("treasury").expect("signer"),
        vec![EvmAccessListEntry::new(
            destination(),
            vec![B256::from([0x33; 32])],
        )],
    )
    .expect("config")
}

fn fees() -> EvmFeeInputs {
    EvmFeeInputs::from_base_and_priority(U256::from(10), U256::from(2)).expect("fees")
}

fn prepared(action: EvmTransactionAction, nonce: u64) -> EvmPreparedTransaction {
    let intent = EvmTransactionIntent::from_config(&config(), &action).expect("intent");
    let estimate = intent
        .transaction_estimate(U256::from(nonce), &fees())
        .expect("estimate");
    let unsigned =
        EvmUnsignedTransaction::from_estimate(&estimate, U256::from(100_000)).expect("unsigned");
    EvmPreparedTransaction::new(
        intent,
        U256::from(nonce),
        fees(),
        U256::from(100_000),
        unsigned,
        B256::from([0x44; 32]),
        &session("primary"),
    )
    .expect("prepared")
}

fn observed_transaction(prepared: &EvmPreparedTransaction) -> CapabilityTransaction {
    let unsigned = prepared.unsigned().to_signing_envelope().expect("envelope");
    CapabilityTransaction {
        transaction_hash: prepared.expected_hash().expect("hash"),
        chain_id: unsigned.chain_id(),
        nonce: unsigned.nonce(),
        from: sender(),
        to: unsigned.to(),
        value: unsigned.value(),
        input: unsigned.input().clone(),
        gas_limit: unsigned.gas_limit(),
        max_fee_per_gas: unsigned.max_fee_per_gas(),
        max_priority_fee_per_gas: unsigned.max_priority_fee_per_gas(),
        access_list: unsigned.access_list().clone(),
        placement: Some(CapabilityPlacement {
            block: EvmBlock {
                number: U256::from(100),
                hash: B256::from([0x55; 32]),
            },
            transaction_index: U256::from(3),
        }),
    }
}

fn capability_receipt(
    prepared: &EvmPreparedTransaction,
    status: CapabilityReceiptStatus,
    contract_address: Option<Address>,
) -> EvmReceipt {
    EvmReceipt {
        transaction_hash: prepared.expected_hash().expect("hash"),
        transaction_index: U256::from(3),
        block: EvmBlock {
            number: U256::from(100),
            hash: B256::from([0x55; 32]),
        },
        from: sender(),
        to: prepared.intent().action().call_destination().expect("to"),
        contract_address,
        status,
        gas_used: U256::from(50_000),
        cumulative_gas_used: U256::from(75_000),
        logs: vec![EvmReceiptLog {
            address: destination(),
            topics: vec![B256::from([0x66; 32])],
            data: vec![0xaa, 0xbb].into(),
            block: EvmBlock {
                number: U256::from(100),
                hash: B256::from([0x55; 32]),
            },
            transaction_hash: prepared.expected_hash().expect("hash"),
            transaction_index: U256::from(3),
            log_index: U256::from(7),
            removed: false,
        }],
    }
}

fn submission(prepared: &EvmPreparedTransaction) -> EvmTransactionSubmission {
    EvmTransactionSubmission::from_observation(
        prepared,
        &observed_transaction(prepared),
        &session("secondary"),
    )
    .expect("submission")
}

#[test]
fn authored_intent_contains_only_immutable_public_authority() {
    let action = EvmTransactionAction::call(destination(), [], U256::from(9)).expect("action");
    let intent = EvmTransactionIntent::from_config(&config(), &action).expect("intent");
    let value = serde_json::to_value(&intent).expect("intent json");

    assert_eq!(value["network_id"], NETWORK);
    assert_eq!(value["signer_ref"], "treasury");
    assert_eq!(value["fee_policy"], EVM_TRANSACTION_FEE_POLICY);
    assert_eq!(value["gas_policy"], EVM_GAS_POLICY);
    let rendered = serde_json::to_string(&value).expect("rendered");
    for forbidden in [
        "signature",
        "signed_bytes",
        "raw_transaction",
        "rpc_url",
        "keystore",
        "password",
    ] {
        assert!(!rendered.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn estimate_is_the_common_authority_for_unsigned_transaction_fields() {
    let intent = EvmTransactionIntent::from_config(
        &config(),
        &EvmTransactionAction::call(destination(), [0xaa, 0xbb], U256::from(9)).expect("action"),
    )
    .expect("intent");
    let estimate = intent
        .transaction_estimate(U256::from(7), &fees())
        .expect("estimate");
    let unsigned =
        EvmUnsignedTransaction::from_estimate(&estimate, U256::from(100_000)).expect("unsigned");

    assert_eq!(unsigned.transaction_type(), estimate.transaction_type());
    assert_eq!(U256::from(unsigned.chain_id()), estimate.chain_id());
    assert_eq!(
        parse_quantity(unsigned.nonce()).expect("nonce"),
        estimate.nonce()
    );
    assert_eq!(
        parse_quantity(unsigned.max_priority_fee_per_gas()).expect("priority fee"),
        estimate.max_priority_fee_per_gas()
    );
    assert_eq!(
        parse_quantity(unsigned.max_fee_per_gas()).expect("maximum fee"),
        estimate.max_fee_per_gas()
    );
    assert_eq!(
        unsigned.to(),
        Some(canonical_address(destination()).as_str())
    );
    assert_eq!(
        parse_quantity(unsigned.value()).expect("value"),
        estimate.value()
    );
    assert_eq!(
        unsigned.access_list(),
        access_list_from_alloy(estimate.access_list())
    );
    assert_eq!(unsigned.input(), canonical_bytes(estimate.input()).as_str());
}

#[test]
fn nonce_and_fee_observations_change_estimate_and_signing_digest() {
    let intent = EvmTransactionIntent::from_config(
        &config(),
        &EvmTransactionAction::call(destination(), [0x01], U256::ZERO).expect("action"),
    )
    .expect("intent");
    let first_estimate = intent
        .transaction_estimate(U256::from(7), &fees())
        .expect("first estimate");
    let nonce_estimate = intent
        .transaction_estimate(U256::from(8), &fees())
        .expect("nonce estimate");
    let changed_fees =
        EvmFeeInputs::from_base_and_priority(U256::from(11), U256::from(2)).expect("fees");
    let fee_estimate = intent
        .transaction_estimate(U256::from(7), &changed_fees)
        .expect("fee estimate");

    assert_ne!(first_estimate, nonce_estimate);
    assert_ne!(first_estimate, fee_estimate);
    let digest = |estimate: &EvmTransactionEstimate| {
        EvmUnsignedTransaction::from_estimate(estimate, U256::from(100_000))
            .expect("unsigned")
            .to_signing_envelope()
            .expect("envelope")
            .signing_digest()
    };
    assert_ne!(digest(&first_estimate), digest(&nonce_estimate));
    assert_ne!(digest(&first_estimate), digest(&fee_estimate));
}

#[test]
fn successful_create_requires_sender_nonce_derived_address() {
    let prepared = prepared(
        EvmTransactionAction::create([0x60, 0x00], U256::ZERO).expect("create"),
        7,
    );
    let expected = sender().create(7);
    assert_eq!(
        prepared.expected_create_address(),
        Some(canonical_address(expected).as_str())
    );
    let receipt = EvmTransactionReceipt::from_observation(
        &prepared,
        &capability_receipt(&prepared, CapabilityReceiptStatus::Success, Some(expected)),
        &session("receipt"),
    )
    .expect("receipt");
    let outcome =
        EvmTransactionOutcome::from_evidence(&prepared, &submission(&prepared), &receipt, None)
            .expect("outcome");

    assert_eq!(
        receipt.contract_address(),
        prepared.expected_create_address()
    );
    assert!(matches!(
        outcome.result(),
        EvmTransactionResult::Succeeded {
            result: EvmTransactionSuccess::Created { address }
        } if address == &canonical_address(expected)
    ));
}

#[test]
fn create_receipt_rejects_missing_or_mismatched_address() {
    let prepared = prepared(
        EvmTransactionAction::create([0x60, 0x00], U256::ZERO).expect("create"),
        8,
    );
    let missing = capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &missing, &session("receipt")).is_err()
    );

    let mismatch = capability_receipt(
        &prepared,
        CapabilityReceiptStatus::Success,
        Some(Address::from([0x99; 20])),
    );
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &mismatch, &session("receipt")).is_err()
    );
}

#[test]
fn reverted_create_is_terminal_and_forbids_contract_address() {
    let prepared = prepared(
        EvmTransactionAction::create([0x60, 0x00], U256::ZERO).expect("create"),
        9,
    );
    let forbidden = capability_receipt(
        &prepared,
        CapabilityReceiptStatus::Reverted,
        Some(sender().create(9)),
    );
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &forbidden, &session("receipt"))
            .is_err()
    );

    let receipt = EvmTransactionReceipt::from_observation(
        &prepared,
        &capability_receipt(&prepared, CapabilityReceiptStatus::Reverted, None),
        &session("receipt"),
    )
    .expect("reverted receipt");
    let outcome =
        EvmTransactionOutcome::from_evidence(&prepared, &submission(&prepared), &receipt, None)
            .expect("outcome");
    assert!(matches!(
        outcome.result(),
        EvmTransactionResult::Reverted {
            action: EvmTransactionActionKind::Create
        }
    ));
}

#[test]
fn call_including_native_transfer_forbids_contract_address() {
    let prepared = prepared(
        EvmTransactionAction::call(destination(), [], U256::from(1)).expect("call"),
        10,
    );
    assert_eq!(prepared.unsigned().input(), "0x");
    let forbidden = capability_receipt(
        &prepared,
        CapabilityReceiptStatus::Success,
        Some(Address::from([0x77; 20])),
    );
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &forbidden, &session("receipt"))
            .is_err()
    );
}

#[test]
fn receipt_logs_fail_closed_on_removed_or_moved_identity() {
    let prepared = prepared(
        EvmTransactionAction::call(destination(), [0x12, 0x34], U256::ZERO).expect("call"),
        11,
    );
    let mut removed = capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    removed.logs[0].removed = true;
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &removed, &session("receipt")).is_err()
    );

    let mut moved = capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    moved.logs[0].block.hash = B256::from([0x88; 32]);
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &moved, &session("receipt")).is_err()
    );
}

#[test]
fn receipt_logs_require_canonical_indexes_and_bounded_total_data() {
    let prepared = prepared(
        EvmTransactionAction::call(destination(), [0x12, 0x35], U256::ZERO).expect("call"),
        11,
    );
    let mut duplicate = capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    duplicate.logs.push(duplicate.logs[0].clone());
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &duplicate, &session("receipt"))
            .is_err()
    );

    let template = duplicate.logs[0].clone();
    let mut out_of_order = capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    out_of_order.logs = vec![
        EvmReceiptLog {
            log_index: U256::from(8),
            ..template.clone()
        },
        EvmReceiptLog {
            log_index: U256::from(7),
            ..template.clone()
        },
    ];
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &out_of_order, &session("receipt"))
            .is_err()
    );

    let mut excessive = capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    excessive.logs = (0..33)
        .map(|index| EvmReceiptLog {
            data: vec![0u8; EVM_TRANSACTION_DATA_MAX_BYTES].into(),
            log_index: U256::from(index),
            ..template.clone()
        })
        .collect();
    assert!(
        EvmTransactionReceipt::from_observation(&prepared, &excessive, &session("receipt"))
            .is_err()
    );
}

#[test]
fn transaction_lookup_must_match_every_prepared_public_field() {
    let prepared = prepared(
        EvmTransactionAction::call(destination(), [0xab], U256::from(3)).expect("call"),
        12,
    );
    let mut observation = observed_transaction(&prepared);
    observation.nonce += U256::from(1);
    assert!(EvmTransactionSubmission::from_observation(
        &prepared,
        &observation,
        &session("lookup")
    )
    .is_err());
}

#[test]
fn receipt_gas_must_not_exceed_the_prepared_limit() {
    let prepared = prepared(
        EvmTransactionAction::call(destination(), [0xac], U256::ZERO).expect("call"),
        12,
    );
    let submission = submission(&prepared);

    let mut excessive_gas = capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    excessive_gas.gas_used = U256::from(100_001);
    excessive_gas.cumulative_gas_used = U256::from(100_001);
    let excessive_gas =
        EvmTransactionReceipt::from_observation(&prepared, &excessive_gas, &session("receipt"))
            .expect("structurally valid receipt");
    assert!(excessive_gas
        .validate_with_submission(&prepared, &submission)
        .is_err());
}

#[test]
fn confirmation_rechecks_fresh_receipt_block_and_depth() {
    let prepared = prepared(
        EvmTransactionAction::call(destination(), [0xcd], U256::ZERO).expect("call"),
        13,
    );
    let retained = EvmTransactionReceipt::from_observation(
        &prepared,
        &capability_receipt(&prepared, CapabilityReceiptStatus::Success, None),
        &session("receipt-a"),
    )
    .expect("retained");
    let fresh = EvmTransactionReceipt::from_observation(
        &prepared,
        &capability_receipt(&prepared, CapabilityReceiptStatus::Success, None),
        &session("confirmation"),
    )
    .expect("fresh");
    let canonical_block = EvmBlock {
        number: U256::from(100),
        hash: B256::from([0x55; 32]),
    };
    let head = EvmBlock {
        number: U256::from(111),
        hash: B256::from([0x99; 32]),
    };
    let confirmation = EvmTransactionConfirmation::new(
        &prepared,
        &retained,
        fresh,
        &canonical_block,
        &head,
        12,
        &session("confirmation"),
    )
    .expect("confirmation");
    assert_eq!(confirmation.required_depth(), 12);

    let shallow_head = EvmBlock {
        number: U256::from(110),
        hash: B256::from([0xaa; 32]),
    };
    let fresh = EvmTransactionReceipt::from_observation(
        &prepared,
        &capability_receipt(&prepared, CapabilityReceiptStatus::Success, None),
        &session("confirmation"),
    )
    .expect("fresh");
    assert!(EvmTransactionConfirmation::new(
        &prepared,
        &retained,
        fresh,
        &canonical_block,
        &shallow_head,
        12,
        &session("confirmation"),
    )
    .is_err());
}

#[test]
fn confirmation_rejects_moved_receipt_and_wrong_canonical_block() {
    let prepared = prepared(
        EvmTransactionAction::call(destination(), [0xef], U256::ZERO).expect("call"),
        14,
    );
    let retained = EvmTransactionReceipt::from_observation(
        &prepared,
        &capability_receipt(&prepared, CapabilityReceiptStatus::Success, None),
        &session("receipt-a"),
    )
    .expect("retained");
    let mut moved_capability =
        capability_receipt(&prepared, CapabilityReceiptStatus::Success, None);
    moved_capability.block = EvmBlock {
        number: U256::from(101),
        hash: B256::from([0x77; 32]),
    };
    moved_capability.logs[0].block = moved_capability.block.clone();
    let moved = EvmTransactionReceipt::from_observation(
        &prepared,
        &moved_capability,
        &session("confirmation"),
    )
    .expect("moved observation");
    let wrong_block = EvmBlock {
        number: U256::from(100),
        hash: B256::from([0x77; 32]),
    };
    let head = EvmBlock {
        number: U256::from(120),
        hash: B256::from([0x99; 32]),
    };
    assert!(EvmTransactionConfirmation::new(
        &prepared,
        &retained,
        moved,
        &wrong_block,
        &head,
        2,
        &session("confirmation"),
    )
    .is_err());
}

#[test]
fn prepared_authority_rejects_tampered_observation_or_digest() {
    let mut tampered_observation = prepared(
        EvmTransactionAction::call(destination(), [0xca, 0xfe], U256::ZERO).expect("call"),
        15,
    );
    tampered_observation.gas_estimate = "99999".to_owned();
    assert!(tampered_observation.validate().is_err());

    let mut tampered_digest = prepared(
        EvmTransactionAction::call(destination(), [0xca, 0xfe], U256::ZERO).expect("call"),
        15,
    );
    tampered_digest.signing_digest = canonical_hash(B256::from([0xbb; 32]));
    assert!(tampered_digest.validate().is_err());
}

#[test]
fn one_call_action_can_carry_contract_side_multicall_bytes() {
    let calldata = vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03];
    let action = EvmTransactionAction::call(destination(), &calldata, U256::ZERO).expect("call");
    let prepared = prepared(action, 16);
    assert_eq!(prepared.unsigned().input(), canonical_bytes(&calldata));
}

#[test]
fn sender_lane_is_exact_network_chain_sender_tuple() {
    let intent = EvmTransactionIntent::from_config(
        &config(),
        &EvmTransactionAction::call(destination(), [], U256::ZERO).expect("call"),
    )
    .expect("intent");
    assert_eq!(
        intent.sender_lane().resource_key(),
        format!("{NETWORK}:1:{}", canonical_address(sender()))
    );
    let claim = evm_sender_lane_resource_claim().expect("claim");
    assert!(format!("{:?}", claim.as_spec()).contains(EVM_SENDER_LANE_NAMESPACE));
}

#[test]
fn transaction_data_is_bounded_before_intent_authority() {
    let oversized = vec![0u8; EVM_TRANSACTION_DATA_MAX_BYTES + 1];
    assert!(EvmTransactionAction::call(destination(), oversized, U256::ZERO).is_err());
    assert!(EvmTransactionAction::create([], U256::ZERO).is_err());
}

#[test]
fn transaction_action_rejects_unknown_fields() {
    let mut value = serde_json::to_value(
        EvmTransactionAction::call(destination(), [], U256::ZERO).expect("action"),
    )
    .expect("serialize action");
    value
        .as_object_mut()
        .expect("object")
        .insert("legacy_policy".to_owned(), serde_json::json!(true));
    assert!(serde_json::from_value::<EvmTransactionAction>(value).is_err());
}
