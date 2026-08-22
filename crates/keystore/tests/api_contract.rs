use mfm_ids::StableId;
use mfm_keystore::{KeystoreError, KeystoreOwner, SecretSecp256k1Scalar, MAX_KEY_INSTANCES};
use mfm_signing::{
    recover_public_key, Signer, SigningDigest, SigningError, IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID,
};

#[test]
fn keystore_remains_thread_affine() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}

#[tokio::test]
async fn duplicate_import_and_signing_are_key_bound_deterministic_and_recoverable() {
    let owner = KeystoreOwner::start().expect("owner");
    let mut scalar = [0_u8; 32];
    scalar[31] = 1;
    let first = owner
        .import_secp256k1(SecretSecp256k1Scalar::new(scalar).expect("scalar"))
        .await
        .expect("first import");
    let duplicate = owner
        .import_secp256k1(SecretSecp256k1Scalar::new(scalar).expect("scalar"))
        .await
        .expect("duplicate import");
    assert_eq!(first.public_identity(), duplicate.public_identity());
    assert_eq!(
        first.public_identity().signer_route().as_str(),
        IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID
    );
    assert_eq!(
        first
            .public_identity()
            .key_instance_ref()
            .expect("first key ref"),
        duplicate
            .public_identity()
            .key_instance_ref()
            .expect("duplicate key ref")
    );
    let public_key_hex = first
        .public_identity()
        .public_key()
        .public_key()
        .expect("public key")
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        public_key_hex,
        "0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"
    );

    let digest = SigningDigest::from_bytes([0x2a; 32]);
    let purpose = StableId::new("mfm.test/sign@1").expect("purpose");
    let first_signature = first
        .sign(digest, purpose.clone())
        .await
        .expect("first signature");
    let second_signature = duplicate
        .sign(digest, purpose)
        .await
        .expect("second signature");
    assert!(first_signature == second_signature);
    let actual = first_signature
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        actual,
        "74a6b203feee506ab5c39ecb33a32769f79cbf765db4578d15f7e196fb6863a96e4b0679559655534b1c575b9857f1f2604eaf21edd0e703cf723042992c2cb4"
    );
    assert_eq!(first_signature.recovery_id(), 1);
    let parsed =
        k256::ecdsa::Signature::from_slice(first_signature.as_bytes()).expect("compact signature");
    assert!(parsed.normalize_s().is_none());
    let expected_key = first
        .public_identity()
        .public_key()
        .public_key()
        .expect("public key");
    assert!(recover_public_key(digest, first_signature).expect("recovered key") == expected_key);

    owner.shutdown().await.expect("shutdown");
    assert!(matches!(
        first
            .sign(
                SigningDigest::from_bytes([1; 32]),
                StableId::new("mfm.test/after-shutdown@1").expect("purpose"),
            )
            .await,
        Err(SigningError::Failed)
    ));
}

#[tokio::test]
async fn scalar_and_distinct_key_capacity_bounds_are_exact() {
    assert!(SecretSecp256k1Scalar::new([0_u8; 32]).is_err());
    assert!(SecretSecp256k1Scalar::new([0xff_u8; 32]).is_err());

    let owner = KeystoreOwner::start().expect("owner");
    for value in 1..=MAX_KEY_INSTANCES {
        let mut scalar = [0_u8; 32];
        scalar[24..].copy_from_slice(&u64::try_from(value).expect("scalar value").to_be_bytes());
        owner
            .import_secp256k1(SecretSecp256k1Scalar::new(scalar).expect("scalar"))
            .await
            .expect("within capacity");
    }
    let mut overflow = [0_u8; 32];
    overflow[24..].copy_from_slice(
        &u64::try_from(MAX_KEY_INSTANCES + 1)
            .expect("overflow scalar")
            .to_be_bytes(),
    );
    assert!(matches!(
        owner
            .import_secp256k1(SecretSecp256k1Scalar::new(overflow).expect("scalar"))
            .await,
        Err(KeystoreError::Capacity)
    ));
    owner.shutdown().await.expect("shutdown");
}
