use super::*;

fn evidence(
    network_id: &str,
    expected_chain_id: u64,
    observed_chain_id: u64,
) -> RedactedEvmSourceEvidence {
    RedactedEvmSourceEvidence {
        network_id: EvmNetworkId::new(network_id).expect("network"),
        expected_chain_id,
        observed_chain_id,
        source_ref: EvmSourceRef::new("primary").expect("source"),
        policy_id: EvmSourcePolicyId::new("policy").expect("policy"),
    }
}

#[test]
fn fee_request_is_operation_only() {
    let request = EvmFeeReadRequest::new();

    assert_eq!(request, EvmFeeReadRequest);
}

#[test]
fn network_binding_rejects_zero_expected_chain_id() {
    let error = EvmNetworkBinding::new(EvmNetworkId::new("mainnet").expect("network"), 0)
        .expect_err("zero chain id");

    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: EvmInvalidRequest::ZeroExpectedChainId,
        }
    );
}

#[test]
fn source_evidence_from_binding_fails_closed_on_chain_mismatch() {
    let binding =
        EvmNetworkBinding::new(EvmNetworkId::new("mainnet").expect("network"), 1).expect("binding");
    let error = RedactedEvmSourceEvidence::from_binding(
        &binding,
        2,
        EvmSourceRef::new("primary").expect("source"),
        EvmSourcePolicyId::new("policy").expect("policy"),
    )
    .expect_err("chain mismatch must fail closed");

    let EvmCapabilityError::SourceMismatch { diagnostic } = error else {
        panic!("expected source mismatch");
    };
    assert_eq!(diagnostic.stable_error_code(), "evm_source_mismatch");
    assert!(diagnostic.summary().contains("observed_chain_id=2"));
}

#[test]
fn source_mismatch_diagnostic_omits_secret_surfaces() {
    let diagnostic = evidence("mainnet", 1, 2).source_mismatch_diagnostic();
    let rendered = format!("{diagnostic:?} {diagnostic}");

    assert_eq!(diagnostic.stable_error_code(), "evm_source_mismatch");
    assert!(diagnostic.summary().contains("observed_chain_id=2"));
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains("Bearer"));
}
