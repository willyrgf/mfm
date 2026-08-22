use mfm_ids::StableId;
use mfm_signing::{
    verify_recoverable_signature, CompactRecoverableSignature, PublicSignerIdentity,
    PublicSigningKey, SigningDigest, SigningError, UncompressedSec1PublicKey,
    IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID,
};
use mfm_values::{canonicalize_mfm_value, MfmValue};

const GENERATOR_PUBLIC_KEY: [u8; 65] = [
    4, 121, 190, 102, 126, 249, 220, 187, 172, 85, 160, 98, 149, 206, 135, 11, 7, 2, 155, 252, 219,
    45, 206, 40, 217, 89, 242, 129, 91, 22, 248, 23, 152, 72, 58, 218, 119, 38, 163, 196, 101, 93,
    164, 251, 252, 14, 17, 8, 168, 253, 23, 180, 72, 166, 133, 84, 25, 156, 71, 208, 143, 251, 16,
    212, 184,
];

#[test]
fn public_signing_values_have_exact_identity_wire_and_key_instance_ref() {
    let key = PublicSigningKey::new(
        UncompressedSec1PublicKey::new(GENERATOR_PUBLIC_KEY).expect("generator public key"),
    )
    .expect("public signing key");
    let identity = PublicSignerIdentity::new(
        StableId::new(IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID).expect("signer route"),
        key.clone(),
    );
    let (key_bytes, key_ref) = canonicalize_mfm_value(&key).expect("canonical key");
    assert_eq!(
        key_bytes.as_str(),
        "{\"algorithm\":\"mfm.signing.secp256k1-ecdsa-recoverable@1\",\"public_key\":\"BHm-Zn753LusVaBilc6HCwcCm_zbLc4o2VnygVsW-BeYSDradyajxGVdpPv8DhEIqP0XtEimhVQZnEfQj_sQ1Lg\"}"
    );
    assert_eq!(
        key_ref.content_digest().as_str(),
        "content:sha256-v1:8eb349762d5f6a6bb8048037a6c7f03b7ab61972179b028c473718b6a64f271d"
    );
    assert_eq!(identity.key_instance_ref().expect("key ref"), key_ref);
    assert_eq!(
        PublicSigningKey::semantic_id()
            .expect("key semantic")
            .as_str(),
        "semantic:mfm.signing:public-key:1:sha256-jcs-v1:49f95411e9a3a6f8f25d021a17853dcd3002bd5655712635f74367716eae077a"
    );
    assert_eq!(
        PublicSignerIdentity::semantic_id()
            .expect("identity semantic")
            .as_str(),
        "semantic:mfm.signing:public-signer-identity:1:sha256-jcs-v1:d25d361c1e6c8e4ac93da4a67b084fca8d85fba2c27f1ee9f386e43245f65555"
    );
    assert_eq!(
        PublicSigningKey::schema_descriptor()
            .expect("key descriptor")
            .schema_id()
            .expect("key schema")
            .as_str(),
        "schema:mfm.signing-public-key:1:sha256-jcs-v1:829dff094375b8139d6b883b0ba5f808015a57f391986457cf2f9da2a56a71d7"
    );
    assert_eq!(
        PublicSignerIdentity::schema_descriptor()
            .expect("identity descriptor")
            .schema_id()
            .expect("identity schema")
            .as_str(),
        "schema:mfm.signing-public-signer-identity:1:sha256-jcs-v1:5adff596d867c5219f4400b07749c2c1937bd8b2aface7686e55204a985b3bd2"
    );

    let (identity_bytes, _) = canonicalize_mfm_value(&identity).expect("canonical identity");
    assert_eq!(
        identity_bytes.as_str(),
        "{\"public_key\":{\"algorithm\":\"mfm.signing.secp256k1-ecdsa-recoverable@1\",\"public_key\":\"BHm-Zn753LusVaBilc6HCwcCm_zbLc4o2VnygVsW-BeYSDradyajxGVdpPv8DhEIqP0XtEimhVQZnEfQj_sQ1Lg\"},\"signer_route\":\"mfm.signer.in-process-keystore@1\"}"
    );
}

#[test]
fn public_signing_key_decode_rejects_every_noncanonical_or_unsupported_shape() {
    let valid = "{\"algorithm\":\"mfm.signing.secp256k1-ecdsa-recoverable@1\",\"public_key\":\"BHm-Zn753LusVaBilc6HCwcCm_zbLc4o2VnygVsW-BeYSDradyajxGVdpPv8DhEIqP0XtEimhVQZnEfQj_sQ1Lg\"}";
    assert!(serde_json::from_str::<PublicSigningKey>(valid).is_ok());
    for invalid in [
        valid.replace("@1", "@2"),
        valid.replace(
            "BHm-Zn753LusVaBilc6HCwcCm_zbLc4o2VnygVsW-BeYSDradyajxGVdpPv8DhEIqP0XtEimhVQZnEfQj_sQ1Lg",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ),
        valid.replace("Lg\"}", "Lg=\"}"),
        valid.replace("}", ",\"extra\":true}"),
    ] {
        assert!(serde_json::from_str::<PublicSigningKey>(&invalid).is_err());
    }
}

#[test]
fn public_recovery_verifies_the_frozen_recoverable_signature() {
    let signature = CompactRecoverableSignature::new(
        [
            0x74, 0xa6, 0xb2, 0x03, 0xfe, 0xee, 0x50, 0x6a, 0xb5, 0xc3, 0x9e, 0xcb, 0x33, 0xa3,
            0x27, 0x69, 0xf7, 0x9c, 0xbf, 0x76, 0x5d, 0xb4, 0x57, 0x8d, 0x15, 0xf7, 0xe1, 0x96,
            0xfb, 0x68, 0x63, 0xa9, 0x6e, 0x4b, 0x06, 0x79, 0x55, 0x96, 0x55, 0x53, 0x4b, 0x1c,
            0x57, 0x5b, 0x98, 0x57, 0xf1, 0xf2, 0x60, 0x4e, 0xaf, 0x21, 0xed, 0xd0, 0xe7, 0x03,
            0xcf, 0x72, 0x30, 0x42, 0x99, 0x2c, 0x2c, 0xb4,
        ],
        1,
    )
    .expect("checked signature");
    let expected = UncompressedSec1PublicKey::new(GENERATOR_PUBLIC_KEY).expect("generator key");
    verify_recoverable_signature(SigningDigest::from_bytes([0x2a; 32]), signature, &expected)
        .expect("matching key");

    let mut other_digest = [0x2a; 32];
    other_digest[31] ^= 1;
    assert!(matches!(
        verify_recoverable_signature(
            SigningDigest::from_bytes(other_digest),
            signature,
            &expected
        ),
        Err(SigningError::Failed)
    ));
    assert!(CompactRecoverableSignature::new([0; 64], 0).is_err());
    assert!(CompactRecoverableSignature::new(*signature.as_bytes(), 4).is_err());
}
