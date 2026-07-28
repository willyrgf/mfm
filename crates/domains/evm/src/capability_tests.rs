use alloy_eips::eip2930::AccessList;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId};

use super::*;

fn generation(byte: u8) -> EvmRoutingGenerationRef {
    let schema = SchemaId::new(
        "mfm.test.evm-routing",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(b"test routing schema"),
    )
    .expect("schema");
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        mfm_canonical::sha256_digest_bytes(&[byte]),
    );
    EvmRoutingGenerationRef::from_content_ref(ContentRef::new(schema, digest).expect("content ref"))
        .expect("generation")
}

#[test]
fn routing_generation_round_trips_as_one_exact_content_reference() {
    let generation = generation(7);
    let content_ref = generation.to_content_ref().expect("content ref");
    assert_eq!(
        EvmRoutingGenerationRef::from_content_ref(content_ref).expect("generation"),
        generation
    );

    let mut wire = serde_json::to_value(generation).expect("generation JSON");
    wire["content_digest"] = serde_json::json!("not-a-content-digest");
    assert!(serde_json::from_value::<EvmRoutingGenerationRef>(wire).is_err());
}

#[test]
fn network_binding_rejects_zero_chain_and_invalid_network() {
    assert!(EvmNetworkBinding::new("ethereum-mainnet", 1, generation(1)).is_ok());
    assert!(EvmNetworkBinding::new("ethereum-mainnet", 0, generation(1)).is_err());
    assert!(EvmNetworkBinding::new("contains secret", 1, generation(1)).is_err());
}

#[test]
fn safe_failure_wire_contains_only_closed_reviewed_diagnostics() {
    let cases = [
        EvmSafeFailure::HttpStatus { status: 503 },
        EvmSafeFailure::JsonRpcError {
            json_rpc_code: -32005,
        },
        EvmSafeFailure::ResponseInvalid {
            response_kind: EvmResponseInvalidKind::MalformedEnvelope,
            size_class: EvmCoarseSizeClass::UpTo16Kib,
        },
    ];
    let rendered = serde_json::to_string(&cases).expect("safe failures");
    assert!(rendered.contains("\"status\":503"));
    assert!(rendered.contains("\"json_rpc_code\":-32005"));
    for forbidden in [
        "provider_message",
        "response_body",
        "authorization",
        "endpoint",
        "path",
    ] {
        assert!(!rendered.contains(forbidden));
    }
}

#[test]
fn quantity_response_rejects_noncanonical_decimal_forms() {
    let maximum = EvmQuantityResponse::new(U256::MAX);
    assert_eq!(maximum.quantity().expect("quantity"), U256::MAX);
    for invalid in [
        serde_json::json!({"quantity_dec": ""}),
        serde_json::json!({"quantity_dec": "01"}),
        serde_json::json!({"quantity_dec": "0x1"}),
        serde_json::json!({"quantity_dec": "-1"}),
    ] {
        assert!(serde_json::from_value::<EvmQuantityResponse>(invalid).is_err());
    }
}

#[test]
fn transaction_protocol_primitives_remain_checked_but_grant_no_lifecycle() {
    let fees = EvmFeeInputs::from_base_and_priority(U256::from(10), U256::from(2)).expect("fees");
    assert_eq!(fees.max_fee_per_gas, U256::from(22));
    let estimate = EvmTransactionEstimate::new(
        U256::from(1),
        U256::from(9),
        Address::from([1; 20]),
        TxKind::Call(Address::from([2; 20])),
        U256::ZERO,
        Bytes::new(),
        AccessList::default(),
        fees.max_fee_per_gas,
        fees.max_priority_fee_per_gas,
    )
    .expect("estimate");
    assert_eq!(estimate.transaction_type(), EVM_EIP1559_TRANSACTION_TYPE);
    assert_eq!(estimate.nonce(), U256::from(9));

    assert!(EvmTransactionEstimate::new(
        U256::ZERO,
        U256::ZERO,
        Address::ZERO,
        TxKind::Create,
        U256::ZERO,
        Bytes::new(),
        AccessList::default(),
        U256::ONE,
        U256::ZERO,
    )
    .is_err());
}
