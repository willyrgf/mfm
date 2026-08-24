use mfm_ids::StableId;
use mfm_keystore::{KeystoreError, KeystoreOwner, SecretSecp256k1Scalar, MAX_KEY_INSTANCES};
use mfm_signing::{recover_public_key, Secp256k1Signer, SigningDigest, SigningError};

#[tokio::test]
async fn duplicate_import_and_signing_are_key_and_purpose_bound_deterministic_and_recoverable() {
    let owner = KeystoreOwner::start().expect("owner");
    let mut scalar = [0_u8; 32];
    scalar[31] = 1;
    let purpose = StableId::new("mfm.test/sign@1").expect("purpose");
    let first = owner
        .import_secp256k1(
            SecretSecp256k1Scalar::new(scalar).expect("scalar"),
            purpose.clone(),
        )
        .await
        .expect("first import");
    let duplicate = owner
        .import_secp256k1(
            SecretSecp256k1Scalar::new(scalar).expect("scalar"),
            purpose.clone(),
        )
        .await
        .expect("duplicate import");
    let other_purpose = owner
        .import_secp256k1(
            SecretSecp256k1Scalar::new(scalar).expect("scalar"),
            StableId::new("mfm.test/other-sign@1").expect("other purpose"),
        )
        .await
        .expect("same key under another purpose");
    assert!(first.public_key() == duplicate.public_key());
    assert!(first.public_key() == other_purpose.public_key());
    assert_eq!(first.purpose(), &purpose);
    assert_eq!(duplicate.purpose(), &purpose);
    assert_ne!(first.purpose(), other_purpose.purpose());
    let public_key_hex = first
        .public_key()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        public_key_hex,
        "0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"
    );

    let digest = SigningDigest::from_bytes([0x2a; 32]);
    let first_signature = first.sign(digest).await.expect("first signature");
    let second_signature = duplicate.sign(digest).await.expect("second signature");
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
    let expected_key = first.public_key();
    assert!(recover_public_key(digest, first_signature).expect("recovered key") == *expected_key);

    owner.shutdown().await.expect("shutdown");
    assert!(matches!(
        first.sign(SigningDigest::from_bytes([1; 32])).await,
        Err(SigningError::Failed)
    ));
}

#[tokio::test]
async fn scalar_and_distinct_key_capacity_bounds_are_exact() {
    assert!(SecretSecp256k1Scalar::new([0_u8; 32]).is_err());
    assert!(SecretSecp256k1Scalar::new([0xff_u8; 32]).is_err());

    let owner = KeystoreOwner::start().expect("owner");
    let purpose = StableId::new("mfm.test/capacity-sign@1").expect("purpose");
    for value in 1..=MAX_KEY_INSTANCES {
        let mut scalar = [0_u8; 32];
        scalar[24..].copy_from_slice(&u64::try_from(value).expect("scalar value").to_be_bytes());
        owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new(scalar).expect("scalar"),
                purpose.clone(),
            )
            .await
            .expect("within capacity");
    }
    let mut first_scalar = [0_u8; 32];
    first_scalar[31] = 1;
    owner
        .import_secp256k1(
            SecretSecp256k1Scalar::new(first_scalar).expect("first scalar"),
            StableId::new("mfm.test/capacity-other-sign@1").expect("other purpose"),
        )
        .await
        .expect("same key under another purpose does not consume capacity");
    let mut overflow = [0_u8; 32];
    overflow[24..].copy_from_slice(
        &u64::try_from(MAX_KEY_INSTANCES + 1)
            .expect("overflow scalar")
            .to_be_bytes(),
    );
    assert!(matches!(
        owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new(overflow).expect("scalar"),
                purpose,
            )
            .await,
        Err(KeystoreError::Capacity)
    ));
    owner.shutdown().await.expect("shutdown");
}
