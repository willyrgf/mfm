use super::*;

const TEST_KEY: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";

#[test]
fn private_key_derives_expected_ethereum_address() {
    let key = EthereumPrivateKey::from_hex_secret(TEST_KEY).expect("valid key");
    assert_eq!(
        format!("{:?}", key.address().expect("address")),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
}

#[test]
fn private_key_rejects_invalid_inputs_without_echoing_secrets() {
    for (case, raw, expected) in [
        (
            "invalid hex",
            "not-a-valid-private-key-secret",
            EthereumKeyError::InvalidHex,
        ),
        ("short key", "0x1234", EthereumKeyError::InvalidLength),
        (
            "invalid curve key",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            EthereumKeyError::InvalidPrivateKey,
        ),
    ] {
        let err = EthereumPrivateKey::from_hex_secret(raw).expect_err(case);

        assert_eq!(err, expected, "{case}");
        assert!(!err.to_string().contains(raw), "{case}");
    }
}

#[test]
fn recoverable_signature_recovers_expected_address() {
    let key = EthereumPrivateKey::from_hex_secret(TEST_KEY).expect("valid key");
    let hash_bytes = [0x42; 32];
    let hash = B256::from(hash_bytes);
    let signature = key
        .sign_hash_recoverable(&hash_bytes)
        .expect("recoverable signature");

    assert_eq!(
        signature
            .recover_address_from_prehash(&hash)
            .expect("recover address"),
        key.address().expect("address")
    );
}
