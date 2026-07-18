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
fn capability_failure_classification_is_closed_and_phase_aware() {
    use EvmCapabilityFailureDisposition::{OperationalBlock, TerminalValidation};
    use EvmCapabilityPhase::{AfterSubmission, BeforeSubmission, ReadOnly};

    let invalid = EvmCapabilityError::InvalidRequest {
        reason: EvmInvalidRequest::SessionAuthorityMismatch,
    };
    for phase in [ReadOnly, BeforeSubmission, AfterSubmission] {
        assert_eq!(invalid.failure_disposition(phase), TerminalValidation);
    }

    for code in [
        ProviderDiagnosticCode::ProviderConfigurationMissing,
        ProviderDiagnosticCode::ProviderConfigurationInvalid,
        ProviderDiagnosticCode::RouteUnavailable,
        ProviderDiagnosticCode::SourceUnavailable,
        ProviderDiagnosticCode::TransportFailed,
        ProviderDiagnosticCode::RpcHttpStatus,
        ProviderDiagnosticCode::OperationIncomplete,
    ] {
        let error = EvmCapabilityError::provider_failure(evm_diagnostic(code));
        assert_eq!(error.failure_disposition(ReadOnly), OperationalBlock);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            OperationalBlock
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }

    for code in [
        ProviderDiagnosticCode::SourceNotAllowed,
        ProviderDiagnosticCode::RpcJsonError,
        ProviderDiagnosticCode::ResponseInvalid,
        ProviderDiagnosticCode::ResponseMissingResult,
        ProviderDiagnosticCode::UnsupportedOperation,
    ] {
        let error = EvmCapabilityError::provider_failure(evm_diagnostic(code));
        assert_eq!(error.failure_disposition(ReadOnly), TerminalValidation);
        assert_eq!(
            error.failure_disposition(BeforeSubmission),
            TerminalValidation
        );
        assert_eq!(error.failure_disposition(AfterSubmission), OperationalBlock);
    }

    let mismatch = source_mismatch_error(
        &binding(),
        U256::from(2),
        &LocalPublicId::new("replacement").expect("source"),
    );
    for phase in [ReadOnly, BeforeSubmission, AfterSubmission] {
        assert_eq!(mismatch.failure_disposition(phase), OperationalBlock);
    }
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
fn estimate_admission_rejects_alloy_width_overflow_before_io() {
    let overflow_u64 = U256::from(u64::MAX) + U256::from(1);
    let overflow_u128 = U256::from(u128::MAX) + U256::from(1);

    for (chain_id, nonce, maximum, priority, reason) in [
        (
            overflow_u64,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            EvmInvalidRequest::ChainIdOutOfRange,
        ),
        (
            U256::from(1),
            overflow_u64,
            U256::ZERO,
            U256::ZERO,
            EvmInvalidRequest::NonceOutOfRange,
        ),
        (
            U256::from(1),
            U256::ZERO,
            overflow_u128,
            U256::ZERO,
            EvmInvalidRequest::MaxFeePerGasOutOfRange,
        ),
        (
            U256::from(1),
            U256::ZERO,
            U256::from(u128::MAX),
            overflow_u128,
            EvmInvalidRequest::MaxPriorityFeePerGasOutOfRange,
        ),
        (
            U256::from(1),
            U256::ZERO,
            U256::from(1),
            U256::from(2),
            EvmInvalidRequest::PriorityFeeExceedsMaxFee,
        ),
    ] {
        let error = EvmTransactionEstimate::new(
            chain_id,
            nonce,
            Address::ZERO,
            TxKind::Create,
            U256::MAX,
            Bytes::new(),
            AccessList::default(),
            maximum,
            priority,
        )
        .expect_err("invalid estimate width");
        assert_eq!(error, EvmCapabilityError::InvalidRequest { reason });
    }

    let estimate = EvmTransactionEstimate::new(
        U256::from(u64::MAX),
        U256::from(u64::MAX),
        Address::ZERO,
        TxKind::Create,
        U256::MAX,
        Bytes::new(),
        AccessList::default(),
        U256::from(u128::MAX),
        U256::from(u128::MAX),
    )
    .expect("exact boundaries");
    assert_eq!(estimate.transaction_type(), EVM_EIP1559_TRANSACTION_TYPE);
    assert_eq!(estimate.nonce(), U256::from(u64::MAX));
    assert_eq!(estimate.max_fee_per_gas(), U256::from(u128::MAX));
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
        0,
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
fn call_request_requires_a_bounded_result() {
    let selector = EvmBlockSelector::ExactHash(B256::from([3; 32]));
    let empty = EvmCall::new(
        Address::ZERO,
        Address::from([4; 20]),
        U256::ZERO,
        Bytes::new(),
        U256::from(1),
        AccessList::default(),
        selector.clone(),
        0,
    )
    .expect("empty result bound");
    assert_eq!(empty.max_response_bytes(), 0);

    let error = EvmCall::new(
        Address::ZERO,
        Address::from([4; 20]),
        U256::ZERO,
        Bytes::new(),
        U256::from(1),
        AccessList::default(),
        selector,
        EVM_CALL_MAX_RESPONSE_BYTES + 1,
    )
    .expect_err("oversized result bound");
    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::CallResponseLimitExceeded,
        }
    );
}

#[test]
fn receipt_rejects_removed_incoherent_or_reverted_logs() {
    let transaction_hash = B256::from([1; 32]);
    let block_hash = B256::from([2; 32]);
    let mut receipt = EvmReceipt {
        transaction_hash,
        transaction_index: U256::from(3),
        block: EvmBlockAnchor::new(U256::from(4), block_hash),
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
            block: EvmBlockAnchor::new(U256::from(4), block_hash),
            transaction_hash,
            transaction_index: U256::from(3),
            log_index: U256::ZERO,
            removed: false,
        }],
    };
    receipt.validate().expect("coherent receipt");
    receipt.status = EvmReceiptStatus::Reverted;
    assert_eq!(
        receipt.validate(),
        Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::IncoherentReceipt,
        })
    );
    receipt.logs.clear();
    receipt.validate().expect("empty reverted receipt");

    receipt.status = EvmReceiptStatus::Success;
    receipt.logs.push(EvmReceiptLog {
        address: Address::ZERO,
        topics: vec![],
        data: Bytes::new(),
        block: EvmBlockAnchor::new(U256::from(4), block_hash),
        transaction_hash,
        transaction_index: U256::from(3),
        log_index: U256::ZERO,
        removed: false,
    });
    receipt.logs[0].removed = true;
    assert_eq!(
        receipt.validate(),
        Err(EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::IncoherentReceipt,
        })
    );
}
