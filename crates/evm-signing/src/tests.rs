use super::*;
use alloy_primitives::address;
use mfm_signing::PublicSigningIdentity;

fn signer_ref() -> SignerRef {
    SignerRef::new("deployer").expect("signer ref")
}

fn legacy_tx() -> LegacyTxToSign {
    LegacyTxToSign {
        to: None,
        value_wei: 0,
        chain_id: 1,
        nonce: 7,
        gas_price_wei: 1,
        gas_limit: 21_000,
        data: vec![0xde, 0xad, 0xbe, 0xef],
    }
}

fn eip1559_tx() -> Eip1559TxToSign {
    Eip1559TxToSign {
        to: Some(Address::from([0x11; 20])),
        value_wei: 0,
        chain_id: 1,
        nonce: 7,
        max_fee_per_gas: 2,
        max_priority_fee_per_gas: 1,
        gas_limit: 21_000,
        data: vec![0xca, 0xfe],
    }
}

fn expected_sender() -> Address {
    address!("0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e")
}

fn recovered_signature_bytes() -> SignatureBytes {
    SignatureBytes::new(
        hex_literal(
            "48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353\
             efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b",
        )
        .to_vec(),
    )
    .expect("signature bytes")
}

fn provider_identity(account: Address) -> PublicSigningIdentity {
    PublicSigningIdentity::new(
        evm_signing_algorithm_id().expect("algorithm"),
        None,
        Some(format!("{account:?}")),
    )
    .expect("identity")
}

fn legacy_request_for_signed_payload(
    signing_hash: B256,
    expected_from: Address,
) -> EvmSigningRequest {
    EvmSigningRequest {
        transaction: EvmSigningTransaction::Legacy(legacy_tx()),
        signing_request: SigningRequest::from_digest(
            signer_ref(),
            evm_signing_algorithm_id().expect("algorithm"),
            evm_transaction_domain_id().expect("domain"),
            legacy_transaction_purpose_id().expect("purpose"),
            digest_from_hash(signing_hash),
        )
        .require_public_identity(
            ExpectedSignerIdentity::account_id(format!("{expected_from:?}")).expect("expected"),
        ),
        signing_hash,
        expected_from,
    }
}

#[test]
fn default_transaction_style_is_eip1559() {
    assert_eq!(EvmTransactionStyle::default(), EvmTransactionStyle::Eip1559);
}

#[test]
fn transaction_signing_hashes_are_stable() {
    enum Case {
        Legacy,
        Eip1559,
    }

    for (case, expected_hash, expected_purpose) in [
        (
            Case::Legacy,
            "0x7b5763c12ba4587de9d52aac395936808bf4d52eb0d6cc0a8803011825d8aa55",
            EVM_LEGACY_TRANSACTION_PURPOSE_ID,
        ),
        (
            Case::Eip1559,
            "0x57806671c35732b1b46a1f8c8a7d63844b94639d997b46c482ea0062d86a8185",
            EVM_EIP1559_TRANSACTION_PURPOSE_ID,
        ),
    ] {
        let request = match case {
            Case::Legacy => EvmSigningRequest::legacy(signer_ref(), legacy_tx(), expected_sender()),
            Case::Eip1559 => {
                EvmSigningRequest::eip1559(signer_ref(), eip1559_tx(), expected_sender())
            }
        }
        .expect("request");

        assert_eq!(format!("{:?}", request.signing_hash()), expected_hash);
        assert_eq!(
            request.signing_request().purpose().as_str(),
            expected_purpose
        );
        assert_eq!(request.expected_from(), expected_sender());
        assert!(request.signing_request().expected_identity().is_some());
    }
}

#[test]
fn recovered_address_matches_expected_signer_address() {
    let signing_hash = "0x5eb4f5a33c621f32a8622d5f943b6b102994dfe4e5aebbefe69bb1b2aa0fc93e"
        .parse::<B256>()
        .expect("hash");
    let signature =
        primitive_signature_from_bytes(&recovered_signature_bytes()).expect("signature");

    assert_eq!(
        recover_signing_address(signing_hash, signature).expect("recover"),
        address!("0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e")
    );
}

#[test]
fn materializes_transient_legacy_raw_transaction() {
    let expected = expected_sender();
    let signing_hash = "0x5eb4f5a33c621f32a8622d5f943b6b102994dfe4e5aebbefe69bb1b2aa0fc93e"
        .parse::<B256>()
        .expect("hash");
    let request = legacy_request_for_signed_payload(signing_hash, expected);
    let result = SigningResult::for_request(
        request.signing_request(),
        provider_identity(expected),
        recovered_signature_bytes(),
    )
    .expect("signing result");

    let raw = request
        .materialize_signed_payload(&result)
        .expect("raw transaction");

    assert!(raw.hex().starts_with("0x"));
    assert!(raw.transaction_hash().starts_with("0x"));
    assert!(!format!("{raw:?}").contains(raw.hex().as_str()));
}

#[test]
fn materialize_rejects_recovered_address_mismatch() {
    let expected = address!("0x1111111111111111111111111111111111111111");
    let signing_hash = "0x5eb4f5a33c621f32a8622d5f943b6b102994dfe4e5aebbefe69bb1b2aa0fc93e"
        .parse::<B256>()
        .expect("hash");
    let request = legacy_request_for_signed_payload(signing_hash, expected);
    let result = SigningResult::for_request(
        request.signing_request(),
        provider_identity(expected),
        recovered_signature_bytes(),
    )
    .expect("signing result");

    assert_eq!(
        request.materialize_signed_payload(&result),
        Err(EvmSigningError::RecoveredAddressMismatch)
    );
}

fn hex_literal(raw: &str) -> Vec<u8> {
    let compact = raw.split_whitespace().collect::<String>();
    hex::decode(compact).expect("hex")
}
