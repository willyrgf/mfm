use super::*;

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
