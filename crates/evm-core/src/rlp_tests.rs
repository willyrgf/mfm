use super::*;

#[test]
fn rlp_zero_length_list_is_c0() {
    assert_eq!(rlp_encode_list_preencoded(&[]), vec![0xc0]);
}

#[test]
fn usize_zero_is_single_zero_byte() {
    assert_eq!(usize_to_min_be(0), vec![0]);
}

#[test]
fn u64_zero_is_empty() {
    assert_eq!(u64_to_min_be(0), Vec::<u8>::new());
}
