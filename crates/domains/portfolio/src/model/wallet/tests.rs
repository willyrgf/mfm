use super::BitcoinAddress;

#[test]
fn bitcoin_address_authority_accepts_supported_formats() {
    for (case, address, expected_prefix) in [
        ("base58", "1BoatSLRHtKNngkdXEeobR76b53LETtpyT", "1"),
        (
            "segwit v0",
            "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw",
            "bc1q",
        ),
        (
            "taproot",
            "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr",
            "bc1p",
        ),
    ] {
        assert!(BitcoinAddress::new(address).is_ok(), "{case}");
        assert!(address.starts_with(expected_prefix), "{case}");
    }
}

#[test]
fn bitcoin_address_authority_rejects_malformed_and_noncanonical_values() {
    for (case, address) in [
        ("base58 checksum", "1BoatSLRHtKNngkdXEeobR76b53LETtpyY"),
        (
            "bech32 checksum",
            "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuq",
        ),
        (
            "uppercase bech32",
            "BC1QVZVKJN4Q3NSZQXRV3NRAGA2R822XJTY3YKVKUW",
        ),
        ("whitespace", " bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw"),
    ] {
        assert!(BitcoinAddress::new(address).is_err(), "{case}");
    }
}
