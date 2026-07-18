use super::*;

fn binding() -> EvmNetworkBinding {
    EvmNetworkBinding::new(LocalPublicId::new("mainnet").expect("network"), 1).expect("binding")
}

#[test]
fn network_binding_rejects_zero_chain_id() {
    let error = EvmNetworkBinding::new(LocalPublicId::new("mainnet").expect("network"), 0)
        .expect_err("zero chain id");
    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::ZeroExpectedChainId,
        }
    );
}

#[test]
fn session_evidence_is_one_redacted_checked_value() {
    let evidence = EvmSessionEvidence::new(
        &binding(),
        LocalPublicId::new("primary").expect("source"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
    );
    let value = serde_json::to_value(&evidence).expect("JSON");

    assert_eq!(value["network_id"], "mainnet");
    assert_eq!(value["chain_id"], 1);
    assert_eq!(value["source_ref"], "primary");
    assert!(value.get("policy_id").is_none());
    assert!(value.get("observed_chain_id").is_none());
}

#[test]
fn session_evidence_deserialization_rechecks_all_identifiers() {
    let value = serde_json::json!({
        "network_id": "mainnet",
        "chain_id": 0,
        "source_ref": "primary",
        "implementation_id": EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
    });
    assert!(serde_json::from_value::<EvmSessionEvidence>(value).is_err());
}

#[test]
fn block_anchor_preserves_u256_numbers_and_rejects_noncanonical_wire_values() {
    let number = U256::from(u64::MAX) + U256::from(1);
    let hash = B256::from([0xab; 32]);
    let anchor = EvmBlockAnchor::new(number, hash);
    let value = serde_json::to_value(&anchor).expect("JSON");

    assert_eq!(anchor.to_block().expect("checked anchor").number, number);
    assert_eq!(value["number"], number.to_string());
    assert!(serde_json::from_value::<EvmBlockAnchor>(serde_json::json!({
        "number": "01",
        "hash": format!("{hash:#x}"),
    }))
    .is_err());
}

#[test]
fn source_mismatch_diagnostic_is_closed_and_redacted() {
    let error = source_mismatch_error(
        &binding(),
        U256::from(2),
        &LocalPublicId::new("primary").expect("source"),
    );
    let diagnostic = error.redacted_diagnostic().expect("diagnostic");
    let rendered = format!("{diagnostic:?} {diagnostic}");

    assert_eq!(diagnostic.stable_error_code(), "evm_source_mismatch");
    assert!(diagnostic.summary().contains("observed_chain_id=2"));
    for forbidden in ["http://", "Bearer", "password"] {
        assert!(!rendered.contains(forbidden));
    }
}

#[test]
fn fee_policy_uses_checked_u256_arithmetic() {
    let fees = EvmFeeInputs::from_base_and_priority(U256::from(10), U256::from(3)).expect("fees");
    assert_eq!(fees.max_fee_per_gas, U256::from(23));
    assert_eq!(
        EvmFeeInputs::from_base_and_priority(U256::MAX, U256::ZERO),
        Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::FeeOverflow,
        })
    );
}

#[test]
fn call_request_requires_an_explicit_nonzero_gas_bound() {
    let selector = EvmBlockSelector::ExactHash(B256::from([3; 32]));
    let error = EvmCall::new(
        Address::ZERO,
        Address::from([4; 20]),
        U256::ZERO,
        Bytes::new(),
        U256::ZERO,
        AccessList::default(),
        selector,
    )
    .expect_err("zero gas bound");
    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::ZeroCallGasLimit,
        }
    );
}

#[test]
fn receipt_rejects_removed_or_incoherent_logs() {
    let transaction_hash = B256::from([1; 32]);
    let block_hash = B256::from([2; 32]);
    let mut receipt = EvmReceipt {
        transaction_hash,
        transaction_index: U256::from(3),
        block_number: U256::from(4),
        block_hash,
        from: Address::ZERO,
        to: None,
        contract_address: None,
        status: EvmReceiptStatus::Success,
        gas_used: U256::from(5),
        cumulative_gas_used: U256::from(6),
        logs: vec![EvmReceiptLog {
            address: Address::ZERO,
            topics: vec![],
            data: Bytes::new(),
            block_number: U256::from(4),
            block_hash,
            transaction_hash,
            transaction_index: U256::from(3),
            log_index: U256::ZERO,
            removed: false,
        }],
    };
    receipt.validate().expect("coherent receipt");
    receipt.logs[0].removed = true;
    assert_eq!(
        receipt.validate(),
        Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::IncoherentReceipt,
        })
    );
}
