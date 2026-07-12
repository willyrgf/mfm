use super::*;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use alloy_primitives::{Address, B256};
use mfm_core::keystore::Keystore;
use mfm_evm_signing::{
    evm_signing_algorithm_id, evm_transaction_domain_id, legacy_transaction_purpose_id,
    primitive_signature_from_bytes, recover_signing_address,
};
use mfm_signing::{
    manual_resolution_signing_domain_id, manual_resolution_signing_purpose_id,
    ExpectedSignerIdentity, SigningDomainId, SigningPurposeId, SigningRequest,
};
use tempfile::TempDir;

const TEST_KEY: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";
const PASSWORD: &str = "strong_password_123";

struct TestKeystore {
    _dir: TempDir,
    keystore_path: PathBuf,
    password_file: PathBuf,
    entry_id: Uuid,
    address: Address,
}

fn signer_ref() -> SignerRef {
    SignerRef::new("deployer").expect("signer ref")
}

fn test_keystore() -> TestKeystore {
    let dir = tempfile::tempdir().expect("tempdir");
    let keystore_path = dir.path().join("wallet.keystore");
    let password_file = dir.path().join("wallet.password");
    fs::write(&password_file, format!("{PASSWORD}\n")).expect("password file");

    let mut keystore =
        Keystore::new_with_config(&keystore_path, KeystoreConfig::insecure_integration_test())
            .expect("keystore");
    keystore.unlock(PASSWORD).expect("unlock");
    let entry_id = keystore
        .import_private_key(Some("deployer".to_owned()), TEST_KEY)
        .expect("import key");
    let secure_key = keystore.get_private_key(entry_id).expect("secure key");
    let address = secure_key.ethereum_address().expect("address");

    TestKeystore {
        _dir: dir,
        keystore_path,
        password_file,
        entry_id,
        address,
    }
}

fn registry_entry(keystore: &TestKeystore) -> KeystoreSignerRegistryEntry {
    KeystoreSignerRegistryEntry::new(
        signer_ref(),
        keystore.entry_id,
        &keystore.keystore_path,
        &keystore.password_file,
    )
}

fn provider(entry: KeystoreSignerRegistryEntry) -> KeystoreSignerProvider {
    KeystoreSignerProvider::new_with_config([entry], KeystoreConfig::insecure_integration_test())
}

fn signing_request(address: Address) -> SigningRequest {
    signing_request_with(
        address,
        evm_transaction_domain_id().expect("domain"),
        legacy_transaction_purpose_id().expect("purpose"),
        true,
    )
}

fn signing_request_with(
    address: Address,
    domain: SigningDomainId,
    purpose: SigningPurposeId,
    require_expected_identity: bool,
) -> SigningRequest {
    let request = SigningRequest::from_digest(
        signer_ref(),
        evm_signing_algorithm_id().expect("algorithm"),
        domain,
        purpose,
        mfm_ids::DigestBytes::from_array([0x42; 32]),
    );
    if require_expected_identity {
        request.require_public_identity(
            ExpectedSignerIdentity::account_id(format!("{address:?}")).expect("identity"),
        )
    } else {
        request
    }
}

#[test]
fn provider_signs_through_generic_signing_request_without_returning_private_key() {
    let keystore = test_keystore();
    let provider = provider(registry_entry(&keystore));
    let request = signing_request(keystore.address);

    let result = poll_ready(SigningProvider::sign(&provider, &request)).expect("sign");

    assert_eq!(result.signer_ref(), request.signer_ref());
    let account_id = format!("{:?}", keystore.address);
    assert_eq!(
        result.public_identity().account_id(),
        Some(account_id.as_str())
    );
    assert_eq!(result.signature().as_bytes().len(), 65);
    let signature = primitive_signature_from_bytes(result.signature()).expect("signature");
    let recovered = recover_signing_address(B256::from(*request.digest().as_bytes()), signature)
        .expect("recover");
    assert_eq!(recovered, keystore.address);
}

#[test]
fn provider_signs_manual_resolution_digest_with_expected_identity() {
    let keystore = test_keystore();
    let provider = provider(registry_entry(&keystore));
    let request = signing_request_with(
        keystore.address,
        manual_resolution_signing_domain_id().expect("domain"),
        manual_resolution_signing_purpose_id().expect("purpose"),
        true,
    );

    let result = provider.sign_request(&request).expect("sign");

    assert_eq!(result.signature().as_bytes().len(), 65);
    let signature = primitive_signature_from_bytes(result.signature()).expect("signature");
    let recovered = recover_signing_address(B256::from(*request.digest().as_bytes()), signature)
        .expect("recover");
    assert_eq!(recovered, keystore.address);
}

#[test]
fn password_file_trims_line_endings_and_returns_zeroizing_password() {
    let keystore = test_keystore();
    let resolved = password_from_file(&keystore.password_file).expect("password");
    assert_eq!(resolved.as_str(), PASSWORD);
    assert_zeroizing_string(&resolved.0);

    let provider = provider(KeystoreSignerRegistryEntry::new(
        signer_ref(),
        keystore.entry_id,
        &keystore.keystore_path,
        &keystore.password_file,
    ));
    provider
        .sign_request(&signing_request(keystore.address))
        .expect("sign");
}

#[test]
fn rejects_invalid_request_contracts_before_unlocking_keystore() {
    enum Case {
        UnsupportedDomain,
        UnsupportedPurpose,
        MissingExpectedIdentity,
    }

    let keystore = test_keystore();
    let provider = provider(registry_entry(&keystore));

    for (case, expected) in [
        (
            Case::UnsupportedDomain,
            KeystoreSignerError::UnsupportedDomain,
        ),
        (
            Case::UnsupportedPurpose,
            KeystoreSignerError::UnsupportedPurpose,
        ),
        (
            Case::MissingExpectedIdentity,
            KeystoreSignerError::MissingExpectedIdentity,
        ),
    ] {
        let request = match case {
            Case::UnsupportedDomain => signing_request_with(
                keystore.address,
                SigningDomainId::new("evm.other").expect("domain"),
                legacy_transaction_purpose_id().expect("purpose"),
                true,
            ),
            Case::UnsupportedPurpose => signing_request_with(
                keystore.address,
                evm_transaction_domain_id().expect("domain"),
                SigningPurposeId::new("evm.transaction.other").expect("purpose"),
                true,
            ),
            Case::MissingExpectedIdentity => signing_request_with(
                keystore.address,
                evm_transaction_domain_id().expect("domain"),
                legacy_transaction_purpose_id().expect("purpose"),
                false,
            ),
        };

        assert_eq!(provider.sign_request(&request), Err(expected));
    }
}

#[test]
fn redacted_errors_do_not_leak_paths_passwords_or_key_material() {
    let dir = tempfile::tempdir().expect("tempdir");
    let secret_path = dir.path().join("secret-wallet-file-name.keystore");
    let password_file = dir.path().join("secret-password-file-name.txt");
    fs::write(&password_file, "very_secret_password\n").expect("password file");

    let entry = KeystoreSignerRegistryEntry::new(
        signer_ref(),
        Uuid::new_v4(),
        &secret_path,
        &password_file,
    );
    let provider = provider(entry);
    let error = provider
        .sign_request(&signing_request(Address::from([0x11; 20])))
        .expect_err("missing keystore");
    let rendered = format!("{error:?} {error}");
    let secret_path = secret_path.to_string_lossy();
    let password_file = password_file.to_string_lossy();

    assert!(!rendered.contains(secret_path.as_ref()));
    assert!(!rendered.contains(password_file.as_ref()));
    assert!(!rendered.contains("very_secret_password"));
    assert!(!rendered.contains(TEST_KEY.trim_start_matches("0x")));
}

#[test]
fn tampered_keystore_failure_stays_redacted() {
    let keystore = test_keystore();
    fs::write(&keystore.keystore_path, b"{\"tampered\":true}").expect("tamper");
    let provider = provider(registry_entry(&keystore));

    let error = provider
        .sign_request(&signing_request(keystore.address))
        .expect_err("tampered keystore");

    assert_eq!(error, KeystoreSignerError::KeystoreUnavailable);
    assert!(!format!("{error:?} {error}").contains("tampered"));
}

fn assert_zeroizing_string(_: &Zeroizing<String>) {}

fn poll_ready<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("provider future should be ready"),
    }
}
