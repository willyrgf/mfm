use alloy_eips::eip2930::AccessList;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
use mfm_values::MfmValue;

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

fn chain_binding(chain_id: u64) -> Result<EvmChainInstanceBinding, WalletAuthorityContractError> {
    let registry = EvmWalletReference::from_content_ref(
        generation(250)
            .to_content_ref()
            .expect("registry reference"),
    );
    let declaration = ChainInstanceDeclaration::new(
        registry.clone(),
        StableId::new("mfm.test/chain-instance").expect("chain namespace"),
        chain_id,
        B256::repeat_byte(0x11),
        U256::from(1_u64),
        B256::repeat_byte(0x12),
    )?;
    ChainInstanceRegistryAttestation::new(declaration, registry.clone(), registry)?.binding()
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
    let binding = chain_binding(1).expect("qualified chain");
    assert!(EvmNetworkBinding::new("ethereum-mainnet", binding.clone(), generation(1)).is_ok());
    assert!(chain_binding(0).is_err());
    assert!(EvmNetworkBinding::new("contains secret", binding, generation(1)).is_err());
}

#[test]
fn safe_diagnostic_wire_contains_exactly_three_variants_and_four_invalid_kinds() {
    let cases = [
        EvmSafeDiagnostic::HttpStatus { status: 503 },
        EvmSafeDiagnostic::JsonRpcError { code: -32005 },
        EvmSafeDiagnostic::ResponseInvalid {
            kind: EvmResponseInvalidKind::MalformedEnvelope,
        },
        EvmSafeDiagnostic::ResponseInvalid {
            kind: EvmResponseInvalidKind::MissingResult,
        },
        EvmSafeDiagnostic::ResponseInvalid {
            kind: EvmResponseInvalidKind::InvalidResult,
        },
        EvmSafeDiagnostic::ResponseInvalid {
            kind: EvmResponseInvalidKind::TooLarge,
        },
    ];
    let identity = EvmSafeDiagnostic::schema_descriptor()
        .expect("diagnostic descriptor")
        .identity()
        .clone();
    for case in &cases {
        let wire = serde_json::to_string(case).expect("diagnostic JSON");
        let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&wire)
            .expect("canonical diagnostic");
        identity
            .validate_canonical_value(canonical.as_bytes())
            .expect("diagnostic matches its schema identity");
    }
    let rendered = serde_json::to_string(&cases).expect("safe diagnostics");
    assert!(rendered.contains("\"status\":503"));
    assert!(rendered.contains("\"code\":-32005"));
    assert_eq!(
        rendered.matches("\"diagnostic\":\"http_status\"").count(),
        1
    );
    assert_eq!(
        rendered
            .matches("\"diagnostic\":\"json_rpc_error\"")
            .count(),
        1
    );
    assert_eq!(
        rendered
            .matches("\"diagnostic\":\"response_invalid\"")
            .count(),
        4
    );
    for kind in [
        "malformed_envelope",
        "missing_result",
        "invalid_result",
        "too_large",
    ] {
        assert!(rendered.contains(&format!("\"kind\":\"{kind}\"")));
    }
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
