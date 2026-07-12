use super::*;

fn signature(parity: bool) -> PrimitiveSignature {
    let mut r = [0u8; 32];
    r[31] = 1;
    let mut s = [0u8; 32];
    s[31] = 2;
    PrimitiveSignature::from_scalars_and_parity(B256::from(r), B256::from(s), parity)
}

#[test]
fn signed_transaction_encodings_are_stable() {
    enum Case {
        LegacyCreate,
        LegacyCall,
        Eip1559Call,
        Eip1559Create,
    }

    for (case, expected) in [
        (Case::LegacyCreate, "0xcd80018252088080826000250102"),
        (
            Case::LegacyCall,
            "0xdf01028252089411111111111111111111111111111111111111118080260102",
        ),
        (
            Case::Eip1559Call,
            "0x02e2018001028252089400000000000000000000000000000000000000008080c0800102",
        ),
        (
            Case::Eip1559Create,
            "0x02d0018001028252088080826000c0800102",
        ),
    ] {
        let encoded = match case {
            Case::LegacyCreate => bytes_to_hex_prefixed(&encode_signed_legacy_tx(
                &LegacyTxToSign {
                    to: None,
                    value_wei: 0,
                    chain_id: 1,
                    nonce: 0,
                    gas_price_wei: 1,
                    gas_limit: 21_000,
                    data: vec![0x60, 0x00],
                },
                signature(false),
            )),
            Case::LegacyCall => bytes_to_hex_prefixed(&encode_signed_legacy_tx(
                &LegacyTxToSign {
                    to: Some(Address::from([0x11; 20])),
                    value_wei: 0,
                    chain_id: 1,
                    nonce: 1,
                    gas_price_wei: 2,
                    gas_limit: 21_000,
                    data: Vec::new(),
                },
                signature(true),
            )),
            Case::Eip1559Call => bytes_to_hex_prefixed(&encode_signed_eip1559_tx(
                &Eip1559TxToSign {
                    to: Some(Address::from([0u8; 20])),
                    value_wei: 0,
                    chain_id: 1,
                    nonce: 0,
                    max_fee_per_gas: 2,
                    max_priority_fee_per_gas: 1,
                    gas_limit: 21_000,
                    data: Vec::new(),
                },
                signature(false),
            )),
            Case::Eip1559Create => bytes_to_hex_prefixed(&encode_signed_eip1559_tx(
                &Eip1559TxToSign {
                    to: None,
                    value_wei: 0,
                    chain_id: 1,
                    nonce: 0,
                    max_fee_per_gas: 2,
                    max_priority_fee_per_gas: 1,
                    gas_limit: 21_000,
                    data: vec![0x60, 0x00],
                },
                signature(false),
            )),
        };

        assert_eq!(encoded, expected);
    }
}

#[test]
fn parse_quantity_supports_decimal_and_hex() {
    assert_eq!(parse_u128_quantity("42", "value").expect("decimal"), 42);
    assert_eq!(parse_u128_quantity("0x2a", "value").expect("hex"), 42);
}

#[test]
fn parse_data_hex_rejects_non_prefixed_input() {
    let err = parse_data_hex("1234").expect_err("missing prefix should fail");
    assert_eq!(err.code, "invalid_data");
}

#[test]
fn validate_raw_transaction_hex_rejects_non_hex() {
    let err = validate_raw_transaction_hex("0x00zz").expect_err("non-hex should fail");
    assert_eq!(err.code, "invalid_raw_transaction");
}

#[test]
fn raw_transaction_hash_rejects_empty_bytes() {
    let err = raw_transaction_hash_bytes(&[]).expect_err("empty raw tx should fail");
    assert_eq!(err.code, "invalid_raw_transaction");
}
