use super::*;

#[test]
fn normalize_hex_str_pads_odd_length() {
    assert_eq!(
        normalize_hex_str("0xabc").expect("normalize"),
        "0x0abc".to_string()
    );
}

#[test]
fn normalize_nonempty_hex_str_rejects_empty() {
    let err = normalize_nonempty_hex_str("0x").expect_err("must fail");
    assert_eq!(err.code, "hex_empty");
}

#[test]
fn hex_to_bytes_roundtrip() {
    let bytes = hex_to_bytes("0x0102ff").expect("decode");
    assert_eq!(bytes_to_hex_prefixed(&bytes), "0x0102ff");
}
