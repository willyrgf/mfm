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
fn source_ref_is_audit_provenance_not_semantic_binding() {
    let binding = binding();
    let first = EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new("primary").expect("source"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
    );
    let resumed = EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new("replacement").expect("source"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
    );

    assert!(first.matches_binding(&binding));
    assert!(resumed.matches_binding(&binding));
    assert_ne!(first.source_ref(), resumed.source_ref());
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
        block: EvmBlock {
            number: U256::from(4),
            hash: block_hash,
        },
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
            block: EvmBlock {
                number: U256::from(4),
                hash: block_hash,
            },
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
