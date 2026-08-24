use mfm_signing::{
    recover_public_key, CompactRecoverableSignature, Secp256k1PublicKey, SigningDigest,
};

const GENERATOR_PUBLIC_KEY: [u8; 65] = [
    4, 121, 190, 102, 126, 249, 220, 187, 172, 85, 160, 98, 149, 206, 135, 11, 7, 2, 155, 252, 219,
    45, 206, 40, 217, 89, 242, 129, 91, 22, 248, 23, 152, 72, 58, 218, 119, 38, 163, 196, 101, 93,
    164, 251, 252, 14, 17, 8, 168, 253, 23, 180, 72, 166, 133, 84, 25, 156, 71, 208, 143, 251, 16,
    212, 184,
];

static_assertions::assert_not_impl_any!(
    SigningDigest: std::fmt::Debug, std::fmt::Display, serde::Serialize
);
static_assertions::assert_not_impl_any!(
    CompactRecoverableSignature: std::fmt::Debug, std::fmt::Display, serde::Serialize
);

fn frozen_signature() -> CompactRecoverableSignature {
    CompactRecoverableSignature::new(
        [
            0x74, 0xa6, 0xb2, 0x03, 0xfe, 0xee, 0x50, 0x6a, 0xb5, 0xc3, 0x9e, 0xcb, 0x33, 0xa3,
            0x27, 0x69, 0xf7, 0x9c, 0xbf, 0x76, 0x5d, 0xb4, 0x57, 0x8d, 0x15, 0xf7, 0xe1, 0x96,
            0xfb, 0x68, 0x63, 0xa9, 0x6e, 0x4b, 0x06, 0x79, 0x55, 0x96, 0x55, 0x53, 0x4b, 0x1c,
            0x57, 0x5b, 0x98, 0x57, 0xf1, 0xf2, 0x60, 0x4e, 0xaf, 0x21, 0xed, 0xd0, 0xe7, 0x03,
            0xcf, 0x72, 0x30, 0x42, 0x99, 0x2c, 0x2c, 0xb4,
        ],
        1,
    )
    .expect("checked signature")
}

#[test]
fn checked_public_key_accepts_only_valid_uncompressed_points() {
    assert!(Secp256k1PublicKey::new(GENERATOR_PUBLIC_KEY).is_ok());
    let mut compressed_prefix = GENERATOR_PUBLIC_KEY;
    compressed_prefix[0] = 2;
    assert!(Secp256k1PublicKey::new(compressed_prefix).is_err());
    let mut off_curve = GENERATOR_PUBLIC_KEY;
    off_curve[64] ^= 1;
    assert!(Secp256k1PublicKey::new(off_curve).is_err());
}

#[test]
fn public_recovery_matches_the_frozen_key_and_digest() {
    let signature = frozen_signature();
    let expected = Secp256k1PublicKey::new(GENERATOR_PUBLIC_KEY).expect("generator key");
    assert!(
        recover_public_key(SigningDigest::from_bytes([0x2a; 32]), signature).expect("matching key")
            == expected
    );

    let mut other_digest = [0x2a; 32];
    other_digest[31] ^= 1;
    assert!(
        recover_public_key(SigningDigest::from_bytes(other_digest), signature)
            .expect("another recoverable key")
            != expected
    );
}

#[test]
fn compact_signature_rejects_invalid_scalars_high_s_and_recovery_ids() {
    assert!(CompactRecoverableSignature::new([0; 64], 0).is_err());
    assert!(CompactRecoverableSignature::new(*frozen_signature().as_bytes(), 4).is_err());

    let mut high_s = *frozen_signature().as_bytes();
    high_s[32..].copy_from_slice(&[
        0x91, 0xb4, 0xf9, 0x86, 0xaa, 0x69, 0xaa, 0xac, 0xb4, 0xe3, 0xa8, 0xa4, 0x67, 0xa8, 0x0e,
        0x0c, 0x5f, 0x60, 0x2d, 0xc8, 0xd1, 0x77, 0xb0, 0x37, 0xcc, 0x60, 0x5e, 0x63, 0x3d, 0x74,
        0x14, 0x8d,
    ]);
    assert!(CompactRecoverableSignature::new(high_s, 1).is_err());
}
