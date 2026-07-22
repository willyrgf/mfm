use super::*;

use std::io::{self, Cursor, Read};

use alloy_primitives::{Address, PrimitiveSignature, B256};
use mfm_signing::{
    ExpectedSignerIdentity, SigningAlgorithmId, SigningDomainId, SigningProfileId,
    SigningProviderError, SigningPurposeId,
};
use tempfile::TempDir;

const TEST_KEY: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";
const OTHER_KEY: &str = "0x0000000000000000000000000000000000000000000000000000000000000002";
const PASSWORD: &str = "strong_password_123";

struct TestKeystore {
    _dir: TempDir,
    keystore_path: PathBuf,
    unlock_file: PathBuf,
    entry_id: Uuid,
    other_entry_id: Uuid,
    address: Address,
}

fn signer_ref() -> SignerRef {
    SignerRef::new("deployer").expect("signer ref")
}

fn algorithm() -> SigningAlgorithmId {
    SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID).expect("algorithm")
}

fn profile() -> SigningProfileId {
    SigningProfileId::new(SECP256K1_RFC6979_LOW_S_PROFILE_ID).expect("profile")
}

fn test_keystore() -> TestKeystore {
    let dir = tempfile::tempdir().expect("tempdir");
    let keystore_path = dir.path().join("wallet.keystore");
    let unlock_file = dir.path().join("wallet.password");
    fs::write(&unlock_file, format!("{PASSWORD}\r\n")).expect("unlock file");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::insecure_integration_test())
            .expect("keystore");
    keystore.unlock(PASSWORD).expect("unlock");
    let entry_id = keystore
        .import_private_key(Some("deployer".to_owned()), TEST_KEY)
        .expect("import key");
    let other_entry_id = keystore
        .import_private_key(Some("other".to_owned()), OTHER_KEY)
        .expect("import key");
    let address = keystore
        .get_private_key(entry_id)
        .expect("secure key")
        .ethereum_address()
        .expect("address");

    TestKeystore {
        _dir: dir,
        keystore_path,
        unlock_file,
        entry_id,
        other_entry_id,
        address,
    }
}

fn provider(keystore: &TestKeystore, entry_id: Uuid) -> KeystoreSignerProvider {
    KeystoreSignerProvider::new_with_config(
        signer_ref(),
        entry_id,
        &keystore.keystore_path,
        &keystore.unlock_file,
        KeystoreConfig::insecure_integration_test(),
    )
    .expect("checked provider paths")
}

fn request(
    address: Address,
    algorithm: SigningAlgorithmId,
    profile: SigningProfileId,
    domain: &str,
    purpose: &str,
) -> SigningRequest {
    SigningRequest::from_digest(
        signer_ref(),
        algorithm,
        profile,
        SigningDomainId::new(domain).expect("domain"),
        SigningPurposeId::new(purpose).expect("purpose"),
        mfm_ids::DigestBytes::from_array([0x42; 32]),
    )
    .require_public_identity(
        ExpectedSignerIdentity::account_id(format!("{address:?}")).expect("identity"),
    )
}

fn evm_request(address: Address) -> SigningRequest {
    request(
        address,
        algorithm(),
        profile(),
        "evm.transaction",
        "evm.transaction.eip1559",
    )
}

#[tokio::test]
async fn signatures_are_byte_identical_across_provider_and_keystore_reopen() {
    let keystore = test_keystore();
    let request = evm_request(keystore.address);
    let first_provider = provider(&keystore, keystore.entry_id);
    let second_provider = provider(&keystore, keystore.entry_id);

    assert_eq!(
        DeterministicSigningProvider::deterministic_profile_id(&first_provider),
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    );

    let first = SigningProvider::sign(&first_provider, &request)
        .await
        .expect("first sign");
    let second = SigningProvider::sign(&second_provider, &request)
        .await
        .expect("second sign");

    assert_eq!(first.signature(), second.signature());
    assert_eq!(first.profile(), request.profile());
    let signature =
        PrimitiveSignature::from_raw(first.signature().as_bytes()).expect("recoverable signature");
    assert!(signature.normalize_s().is_none());
    assert_eq!(
        signature
            .recover_address_from_prehash(&B256::from(*request.digest().as_bytes()))
            .expect("recover"),
        keystore.address
    );
}

#[tokio::test]
async fn provider_accepts_caller_owned_domain_and_purpose() {
    let keystore = test_keystore();
    let request = request(
        keystore.address,
        algorithm(),
        profile(),
        "protocol.other",
        "operation.authorize.v9",
    );

    let result = provider(&keystore, keystore.entry_id)
        .sign(&request)
        .await
        .expect("sign arbitrary caller-owned purpose");

    assert_eq!(result.signer_ref(), request.signer_ref());
    assert_eq!(
        result.public_identity().account_id(),
        Some(format!("{:?}", keystore.address).as_str())
    );
}

#[tokio::test]
async fn wrong_key_identity_is_rejected_by_the_generic_contract() {
    let keystore = test_keystore();
    let request = evm_request(keystore.address);

    let error = provider(&keystore, keystore.other_entry_id)
        .sign(&request)
        .await
        .expect_err("wrong key");

    assert!(matches!(
        error,
        SigningError::PublicIdentityMismatch {
            field: "account_id",
            ..
        }
    ));
}

#[tokio::test]
async fn unsupported_binding_algorithm_and_profile_fail_before_file_access() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = KeystoreSignerProvider::new(
        signer_ref(),
        Uuid::new_v4(),
        dir.path().join("missing.keystore"),
        dir.path().join("missing.password"),
    )
    .expect("checked provider paths");
    let expected = Address::from([0x11; 20]);

    let cases = [
        request(
            expected,
            SigningAlgorithmId::new("ed25519.sha512").expect("algorithm"),
            profile(),
            "caller.domain",
            "caller.purpose",
        ),
        request(
            expected,
            algorithm(),
            SigningProfileId::new("secp256k1.nondeterministic.v1").expect("profile"),
            "caller.domain",
            "caller.purpose",
        ),
        SigningRequest::from_digest(
            SignerRef::new("other").expect("signer"),
            algorithm(),
            profile(),
            SigningDomainId::new("caller.domain").expect("domain"),
            SigningPurposeId::new("caller.purpose").expect("purpose"),
            mfm_ids::DigestBytes::from_array([0x42; 32]),
        )
        .require_public_identity(
            ExpectedSignerIdentity::account_id(format!("{expected:?}")).expect("identity"),
        ),
    ];

    for request in cases {
        assert_eq!(
            provider.sign(&request).await,
            Err(SigningError::Provider {
                reason: SigningProviderError::Failed
            })
        );
    }
}

#[tokio::test]
async fn missing_expected_identity_is_rejected_before_file_access() {
    let keystore = test_keystore();
    let request = SigningRequest::from_digest(
        signer_ref(),
        algorithm(),
        profile(),
        SigningDomainId::new("caller.domain").expect("domain"),
        SigningPurposeId::new("caller.purpose").expect("purpose"),
        mfm_ids::DigestBytes::from_array([0x42; 32]),
    );

    assert!(matches!(
        provider(&keystore, keystore.entry_id).sign(&request).await,
        Err(SigningError::Provider { .. })
    ));
}

#[tokio::test]
async fn every_failure_and_debug_surface_redacts_runtime_secrets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let secret_path = dir.path().join("secret-wallet-file-name.keystore");
    let unlock_file = dir.path().join("secret-unlock-file-name.txt");
    fs::write(&unlock_file, "very_secret_password\n").expect("unlock file");
    let provider =
        KeystoreSignerProvider::new(signer_ref(), Uuid::new_v4(), &secret_path, &unlock_file)
            .expect("checked provider paths");

    let error = provider
        .sign(&evm_request(Address::from([0x11; 20])))
        .await
        .expect_err("missing keystore");
    let rendered = format!("{provider:?} {error:?} {error}");

    for secret in [
        secret_path.to_string_lossy().as_ref(),
        unlock_file.to_string_lossy().as_ref(),
        "very_secret_password",
        TEST_KEY.trim_start_matches("0x"),
    ] {
        assert!(!rendered.contains(secret));
    }
}

#[tokio::test]
async fn tampered_keystore_failure_stays_redacted() {
    let keystore = test_keystore();
    fs::write(&keystore.keystore_path, b"{\"tampered\":true}").expect("tamper");

    let error = provider(&keystore, keystore.entry_id)
        .sign(&evm_request(keystore.address))
        .await
        .expect_err("tampered keystore");

    assert!(matches!(error, SigningError::Provider { .. }));
    assert!(!format!("{error:?} {error}").contains("tampered"));
}

#[test]
fn unlock_file_strips_at_most_one_line_ending_in_zeroizing_storage() {
    let keystore = test_keystore();
    let checked =
        CheckedRuntimePath::new(keystore.unlock_file.clone(), RuntimeSourceKind::UnlockFile)
            .expect("checked unlock path");
    let resolved = unlock_secret_from_file(&checked).expect("unlock secret");
    assert_eq!(resolved.as_str(), PASSWORD);
    assert_zeroizing_string(&resolved.secret);

    for (input, expected) in [
        (&b"secret\n\n"[..], "secret\n"),
        (&b"secret\r\n\r\n"[..], "secret\r\n"),
        (&b"secret\r"[..], "secret\r"),
    ] {
        let resolved = unlock_secret_from_reader(Cursor::new(input)).expect("unlock secret");
        assert_eq!(resolved.as_str(), expected);
    }
}

#[test]
fn checked_provider_paths_reject_invalid_values_without_exposing_them() {
    for path in [
        PathBuf::new(),
        PathBuf::from("x".repeat(MAX_RUNTIME_PATH_BYTES + 1)),
    ] {
        let error =
            KeystoreSignerProvider::new(signer_ref(), Uuid::new_v4(), path, "/run/mfm/unlock")
                .expect_err("invalid path");
        assert_eq!(
            error,
            SigningError::Provider {
                reason: SigningProviderError::Failed
            }
        );
    }
}

#[test]
fn unlock_reader_zeroizes_success_and_every_error_path() {
    let success_witness = ZeroizeWitness::default();
    let string_witness = ZeroizeWitness::default();
    let resolved = unlock_secret_from_reader_with_witness(
        Cursor::new(b"protected-unlock\r\n"),
        success_witness.clone(),
    )
    .expect("bounded UTF-8 unlock secret")
    .with_drop_witness(string_witness.clone());
    assert_eq!(resolved.as_str(), "protected-unlock");
    assert!(success_witness.observed_zeroized_drop());
    assert_zeroizing_string(&resolved.secret);
    drop(resolved);
    assert!(string_witness.observed_zeroized_drop());

    let boundary_witness = ZeroizeWitness::default();
    let boundary = unlock_secret_from_reader_with_witness(
        Cursor::new(vec![b'x'; MAX_UNLOCK_FILE_BYTES]),
        boundary_witness.clone(),
    )
    .expect("exact unlock-file limit");
    assert_eq!(boundary.as_str().len(), MAX_UNLOCK_FILE_BYTES);
    assert!(boundary_witness.observed_zeroized_drop());
    drop(boundary);

    let cases = [
        (Vec::new(), "empty"),
        (vec![0xff, 0xfe], "invalid UTF-8"),
        (vec![b'x'; MAX_UNLOCK_FILE_BYTES + 1], "oversize"),
    ];
    for (bytes, case) in cases {
        let witness = ZeroizeWitness::default();
        let error =
            match unlock_secret_from_reader_with_witness(Cursor::new(bytes), witness.clone()) {
                Ok(_) => panic!("{case} must reject"),
                Err(error) => error,
            };
        assert!(matches!(
            error,
            KeystoreSignerError::InvalidRuntimeSource {
                kind: RuntimeSourceKind::UnlockFile
            }
        ));
        assert!(witness.observed_zeroized_drop(), "{case}");
    }

    let partial_witness = ZeroizeWitness::default();
    let error = match unlock_secret_from_reader_with_witness(
        PartialReadFailure::default(),
        partial_witness.clone(),
    ) {
        Ok(_) => panic!("partial read failure must reject"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        KeystoreSignerError::UnreadableRuntimeSource {
            kind: RuntimeSourceKind::UnlockFile
        }
    ));
    assert!(partial_witness.observed_zeroized_drop());
}

fn assert_zeroizing_string(_: &Zeroizing<String>) {}

#[derive(Default)]
struct PartialReadFailure {
    emitted: bool,
}

impl Read for PartialReadFailure {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.emitted {
            return Err(io::Error::other("injected partial read failure"));
        }
        self.emitted = true;
        let secret = b"partially-read-secret";
        let count = secret.len().min(buffer.len());
        buffer[..count].copy_from_slice(&secret[..count]);
        Ok(count)
    }
}
