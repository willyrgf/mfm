use alloy_primitives::B256;
use mfm_capabilities::CapabilitySpec;
use mfm_evm_capabilities::*;

#[test]
fn capability_names_are_authority_names_not_workflow_names() {
    let names = capability_names();

    assert_eq!(
        names,
        [
            "mfm.evm.chain_identity.read",
            "mfm.evm.block.read",
            "mfm.evm.balance.read",
            "mfm.evm.call.read",
            "mfm.evm.logs.read",
            "mfm.evm.nonce.read",
            "mfm.evm.fee.read",
            "mfm.evm.gas_estimate.read",
            "mfm.evm.transaction.submit",
            "mfm.evm.receipt.read",
            "mfm.evm.nonce_occupancy.read",
        ]
    );
    for name in names {
        assert!(!name.contains(concat!("d", "cv")));
        assert!(!name.contains("deploy"));
        assert!(!name.contains("configure"));
        assert!(!name.contains("validate"));
    }
}

#[test]
fn fee_market_and_gas_estimate_capabilities_are_separate() {
    assert_ne!(
        EvmFeeReadCapability::name(),
        EvmGasEstimateCapability::name()
    );
    assert_ne!(
        EvmFeeReadCapability::kind().expect("fee kind"),
        EvmGasEstimateCapability::kind().expect("gas kind")
    );
}

#[test]
fn concrete_source_details_are_absent_from_contracts() {
    let source = include_str!("../src/lib.rs");
    let forbidden = [
        concat!("rpc", "_", "url"),
        concat!("end", "point"),
        concat!("author", "ization"),
        concat!("api", "_", "key"),
        concat!("bear", "er"),
        concat!("pass", "word"),
        concat!("private", "_", "key"),
        concat!("mn", "emonic"),
        concat!("key", "store", "_", "path"),
    ];

    for term in forbidden {
        assert!(
            !source.contains(term),
            "forbidden concrete source detail: {term}"
        );
    }
}

#[test]
fn signed_payload_debug_redacts_bytes() {
    let payload =
        SignedEvmPayload::from_verified_bytes(vec![1, 2, 3], B256::from([4; 32])).expect("payload");

    let rendered = format!("{payload:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("1, 2, 3"));
}

fn capability_names() -> [&'static str; 11] {
    [
        EvmChainIdentityCapability::name(),
        EvmBlockReadCapability::name(),
        EvmBalanceReadCapability::name(),
        EvmCallReadCapability::name(),
        EvmLogsReadCapability::name(),
        EvmNonceReadCapability::name(),
        EvmFeeReadCapability::name(),
        EvmGasEstimateCapability::name(),
        EvmTransactionSubmitCapability::name(),
        EvmReceiptReadCapability::name(),
        EvmNonceOccupancyReadCapability::name(),
    ]
}
