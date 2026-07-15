use super::*;
use alloy_primitives::Address;

#[test]
fn normalize_address_lowercases() {
    let got = normalize_address("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").expect("ok");
    assert_eq!(got, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
}

#[test]
fn encode_word_has_big_endian_value() {
    let word = encode_u64_word(7);
    assert_eq!(word[31], 7u8);
}

#[test]
fn erc20_calldata_is_exact() {
    let owner = Address::from([0x11; 20]);
    assert_eq!(encode_erc20_decimals(), "0x313ce567");
    assert_eq!(
        encode_erc20_balance_of(&owner),
        "0x70a082310000000000000000000000001111111111111111111111111111111111111111"
    );
}

#[test]
fn erc20_decimals_result_accepts_u8_endpoints() {
    let zero = [0u8; 32];
    let mut max = [0u8; 32];
    max[31] = u8::MAX;

    assert_eq!(
        decode_erc20_decimals_result(&zero).expect("zero decimals"),
        0
    );
    assert_eq!(
        decode_erc20_decimals_result(&max).expect("max decimals"),
        u8::MAX
    );
}

#[test]
fn erc20_decimals_result_rejects_noncanonical_words() {
    let mut nonzero_high_padding = [0u8; 32];
    nonzero_high_padding[0] = 1;
    assert!(decode_erc20_decimals_result(&nonzero_high_padding).is_err());

    for malformed in [Vec::new(), vec![0; 31], vec![0; 33]] {
        assert!(
            decode_erc20_decimals_result(&malformed).is_err(),
            "length {} must fail",
            malformed.len()
        );
    }
}

#[test]
fn erc20_balance_result_preserves_zero_and_maximum_uint256() {
    let zero = [0u8; 32];
    let max = [u8::MAX; 32];

    assert_eq!(
        decode_erc20_balance_result(&zero)
            .expect("zero balance")
            .to_string(),
        "0"
    );
    assert_eq!(
        decode_erc20_balance_result(&max)
            .expect("max balance")
            .to_string(),
        U256::MAX.to_string()
    );
}

#[test]
fn erc20_balance_result_rejects_non_word_lengths() {
    for malformed in [Vec::new(), vec![0; 31], vec![0; 33]] {
        assert!(
            decode_erc20_balance_result(&malformed).is_err(),
            "length {} must fail",
            malformed.len()
        );
    }
}
