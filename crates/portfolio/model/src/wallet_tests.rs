use super::validate_bitcoin_address;
use bech32::{hrp, segwit};

#[test]
fn validate_bitcoin_address_accepts_supported_formats() {
    let segwit_v0 = segwit::encode_v0(hrp::BC, &[0x11; 20]).expect("valid v0 bech32 address");
    let taproot = segwit::encode_v1(hrp::BC, &[0x22; 32]).expect("valid v1 bech32m address");

    for (case, address, expected_prefix) in [
        (
            "base58",
            "1BoatSLRHtKNngkdXEeobR76b53LETtpyT".to_owned(),
            "1",
        ),
        ("segwit v0", segwit_v0, "bc1q"),
        ("taproot", taproot, "bc1p"),
    ] {
        assert!(validate_bitcoin_address(&address).is_ok(), "{case}");
        assert!(address.starts_with(expected_prefix), "{case}");
    }
}

#[test]
fn validate_bitcoin_address_rejects_checksum_mismatches() {
    let mut bad_bech32 = segwit::encode_v0(hrp::BC, &[0x33; 20]).expect("valid v0 bech32 address");
    let last = bad_bech32.pop().expect("non-empty address");
    bad_bech32.push(if last == 'q' { 'p' } else { 'q' });

    for (case, address) in [
        ("base58", "1BoatSLRHtKNngkdXEeobR76b53LETtpyY".to_owned()),
        ("bech32", bad_bech32),
    ] {
        assert!(validate_bitcoin_address(&address).is_err(), "{case}");
    }
}
