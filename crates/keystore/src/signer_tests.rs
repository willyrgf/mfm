use super::*;

use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use alloy_primitives::{Address, PrimitiveSignature, B256};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId};
use mfm_signing::{
    DeterministicSigningProvider, ExpectedSignerIdentity,
    GenerationGuardedDeterministicSigningProvider, PublicKeyBytes, PublicSigningIdentity,
    QualifiedReadSigningProvider, SignatureBytes, SignerRef, SigningAlgorithmId, SigningDomainId,
    SigningGenerationGuard, SigningGenerationGuardError, SigningGenerationGuardFuture,
    SigningProfileId, SigningProvider, SigningProviderError, SigningPurposeId, SigningResult,
    VerifiedGenerationGuardedSignerBinding,
};
use static_assertions::assert_not_impl_any;
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

#[cfg(unix)]
struct ReadOnlyDirectory {
    path: PathBuf,
    original: fs::Permissions,
}

#[cfg(unix)]
impl ReadOnlyDirectory {
    fn new(path: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let original = fs::metadata(path)
            .expect("directory metadata")
            .permissions();
        let read_only = fs::Permissions::from_mode(original.mode() & !0o222);
        fs::set_permissions(path, read_only).expect("disable directory writes");
        Self {
            path: path.to_path_buf(),
            original,
        }
    }
}

#[cfg(unix)]
impl Drop for ReadOnlyDirectory {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.original.clone());
    }
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

fn semantic_signer_id() -> StableId {
    StableId::new("mfm.test.signer/key-identity").expect("semantic signer id")
}

fn content_ref(seed: u8) -> ContentRef {
    let schema_id = SchemaId::new(
        "mfm.test.signer-binding",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([seed; 32]),
    )
    .expect("schema id");
    let content_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        DigestBytes::from_array([seed; 32]),
    );
    ContentRef::new(schema_id, content_digest).expect("content ref")
}

fn binding(address: Address, generation_seed: u8) -> VerifiedGenerationGuardedSignerBinding {
    VerifiedGenerationGuardedSignerBinding::verify(
        signer_ref(),
        KEYSTORE_SIGNING_IMPLEMENTATION_ID,
        algorithm(),
        profile(),
        PublicSigningIdentity::new(algorithm(), None, Some(format!("{address:?}")))
            .expect("public identity"),
        content_ref(generation_seed),
        content_ref(generation_seed.wrapping_add(1)),
        content_ref(generation_seed.wrapping_add(2)),
    )
    .expect("verified guarded binding")
}

fn qualified_binding(
    address: Address,
    generation_seed: u8,
) -> VerifiedGenerationGuardedSignerBinding {
    qualified_binding_for_implementation(
        address,
        generation_seed,
        KEYSTORE_SIGNING_IMPLEMENTATION_ID,
    )
}

fn qualified_binding_for_implementation(
    address: Address,
    generation_seed: u8,
    implementation_id: &str,
) -> VerifiedGenerationGuardedSignerBinding {
    VerifiedGenerationGuardedSignerBinding::verify(
        signer_ref(),
        implementation_id,
        algorithm(),
        profile(),
        PublicSigningIdentity::new(
            algorithm(),
            Some(test_public_key()),
            Some(format!("{address:?}")),
        )
        .expect("complete public identity"),
        content_ref(generation_seed),
        content_ref(generation_seed.wrapping_add(1)),
        content_ref(generation_seed.wrapping_add(2)),
    )
    .expect("qualified guarded binding evidence")
}

fn persisted_audit_count(bytes: &[u8]) -> usize {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .expect("persisted keystore JSON")
        .get("audit_log")
        .and_then(serde_json::Value::as_array)
        .expect("persisted audit log")
        .len()
}

fn test_public_key() -> PublicKeyBytes {
    let mut key_bytes = [0_u8; 32];
    key_bytes[31] = 1;
    let public_key = k256::SecretKey::from_slice(&key_bytes)
        .expect("test key")
        .public_key()
        .to_encoded_point(true);
    PublicKeyBytes::new(public_key.as_bytes().to_vec()).expect("compressed public key")
}

fn test_address() -> Address {
    let mut key_bytes = [0_u8; 32];
    key_bytes[31] = 1;
    crate::crypto::EthereumPrivateKey::from_secret_bytes(&key_bytes)
        .expect("test private key")
        .address()
        .expect("test address")
}

#[derive(Clone, Copy)]
enum GuardVerdict {
    Current,
    Unavailable,
    Fenced,
    DirectSigningOverlap,
}

struct TestGenerationGuard {
    verdict: GuardVerdict,
    eligible: AtomicUsize,
    checks: AtomicUsize,
}

impl TestGenerationGuard {
    fn new(verdict: GuardVerdict) -> Arc<Self> {
        Arc::new(Self {
            verdict,
            eligible: AtomicUsize::new(1),
            checks: AtomicUsize::new(0),
        })
    }

    fn ineligible(verdict: GuardVerdict) -> Arc<Self> {
        Arc::new(Self {
            verdict,
            eligible: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
        })
    }

    fn set_eligible(&self, eligible: bool) {
        self.eligible.store(usize::from(eligible), Ordering::SeqCst);
    }

    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }
}

impl SigningGenerationGuard for TestGenerationGuard {
    fn is_read_attestation_eligible(&self) -> bool {
        self.eligible.load(Ordering::SeqCst) == 1
    }

    fn verify_current_and_exclusive<'a>(
        &'a self,
        _binding: &'a VerifiedGenerationGuardedSignerBinding,
    ) -> SigningGenerationGuardFuture<'a> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let result = match self.verdict {
            GuardVerdict::Current => Ok(()),
            GuardVerdict::Unavailable => Err(SigningGenerationGuardError::Unavailable),
            GuardVerdict::Fenced => Err(SigningGenerationGuardError::Fenced),
            GuardVerdict::DirectSigningOverlap => {
                Err(SigningGenerationGuardError::DirectSigningOverlap)
            }
        };
        Box::pin(async move { result })
    }
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
    let address = test_address();

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
    provider_with_guard(
        keystore,
        entry_id,
        TestGenerationGuard::new(GuardVerdict::Current),
    )
}

fn provider_with_guard(
    keystore: &TestKeystore,
    entry_id: Uuid,
    generation_guard: Arc<dyn SigningGenerationGuard>,
) -> KeystoreSignerProvider {
    KeystoreSignerProvider::new_with_config(
        binding(keystore.address, 0x10),
        entry_id,
        &keystore.keystore_path,
        &keystore.unlock_file,
        generation_guard,
        KeystoreConfig::insecure_integration_test(),
    )
    .expect("checked provider paths")
}

async fn sign(
    provider: &KeystoreSignerProvider,
    request: &SigningRequest,
) -> mfm_signing::Result<SigningResult> {
    let generation = provider.binding().durable_generation_ref().clone();
    provider.sign_guarded(&generation, request).await
}

#[tokio::test]
async fn qualification_checks_key_identity_and_guard_before_consuming_handoff() {
    let keystore = test_keystore();
    let persisted_before = fs::read(&keystore.keystore_path).expect("persisted keystore");
    let audit_count_before = persisted_audit_count(&persisted_before);
    #[cfg(unix)]
    let _read_only_directory = ReadOnlyDirectory::new(
        keystore
            .keystore_path
            .parent()
            .expect("keystore parent directory"),
    );
    let guard = TestGenerationGuard::new(GuardVerdict::Current);
    let candidate = KeystoreSignerProvider::new_with_config(
        qualified_binding(keystore.address, 0x08),
        keystore.entry_id,
        &keystore.keystore_path,
        &keystore.unlock_file,
        guard.clone(),
        KeystoreConfig::insecure_integration_test(),
    )
    .expect("candidate");
    let semantic_contract_ref = content_ref(0x0b);

    let qualified = candidate
        .qualify(semantic_signer_id(), semantic_contract_ref.clone())
        .await
        .expect("qualified signer");
    assert_eq!(guard.checks(), 1);
    assert_eq!(
        qualified.semantic_signer_contract_ref(),
        &semantic_contract_ref
    );
    assert_eq!(qualified.semantic_signer_id(), &semantic_signer_id());
    assert_eq!(
        qualified.binding().expected_public_identity().public_key(),
        Some(&test_public_key())
    );

    let (handed_off_signer_id, handed_off_contract, provider) = qualified
        .into_read_signing_provider()
        .await
        .expect("current affine handoff");
    assert_eq!(handed_off_contract, semantic_contract_ref);
    assert_eq!(handed_off_signer_id, semantic_signer_id());
    assert_eq!(guard.checks(), 2);

    let request = SigningRequest::from_digest(
        signer_ref(),
        algorithm(),
        profile(),
        SigningDomainId::new("evm.transaction").expect("domain"),
        SigningPurposeId::new("evm.transaction.eip1559").expect("purpose"),
        mfm_ids::DigestBytes::from_array([0x42; 32]),
    )
    .require_public_identity(
        ExpectedSignerIdentity::public_key_and_account_id(
            test_public_key(),
            format!("{:?}", keystore.address),
        )
        .expect("identity"),
    );
    let generation = provider.binding().durable_generation_ref().clone();
    let result = provider
        .sign_guarded(&generation, &request)
        .await
        .expect("guarded sign");
    let repeated = provider
        .sign_guarded(&generation, &request)
        .await
        .expect("repeated guarded sign");
    assert_eq!(
        result.public_identity(),
        provider.binding().expected_public_identity()
    );
    assert_eq!(result.signature(), repeated.signature());
    assert_eq!(guard.checks(), 4);
    let persisted_after = fs::read(&keystore.keystore_path).expect("persisted keystore");
    assert_eq!(persisted_audit_count(&persisted_after), audit_count_before);
    assert!(
        persisted_after == persisted_before,
        "Read qualification or signing changed persisted keystore bytes"
    );
}

#[tokio::test]
async fn read_qualification_rejects_every_semantic_guard_before_guard_or_key_access() {
    let directory = tempfile::tempdir().expect("tempdir");
    for (index, mutation) in [
        "quota",
        "approval",
        "anti-replay",
        "billing",
        "rate-limit",
        "other-semantic-state",
    ]
    .into_iter()
    .enumerate()
    {
        let guard = TestGenerationGuard::ineligible(GuardVerdict::Current);
        let candidate = KeystoreSignerProvider::new(
            qualified_binding(Address::from([0x11; 20]), 0x40 + index as u8),
            Uuid::new_v4(),
            directory.path().join("missing.keystore"),
            directory.path().join("missing.unlock"),
            guard.clone(),
        )
        .expect("candidate");
        assert!(
            matches!(
                candidate
                    .qualify(semantic_signer_id(), content_ref(0x70 + index as u8))
                    .await,
                Err(SigningError::Provider {
                    reason: SigningProviderError::ReadAttestationIneligible
                })
            ),
            "accepted {mutation} mutation"
        );
        assert_eq!(guard.checks(), 0);
    }
}

#[tokio::test]
async fn handoff_and_signing_recheck_revoked_read_eligibility() {
    let keystore = test_keystore();
    let guard = TestGenerationGuard::new(GuardVerdict::Current);
    let candidate = KeystoreSignerProvider::new_with_config(
        qualified_binding(keystore.address, 0x48),
        keystore.entry_id,
        &keystore.keystore_path,
        &keystore.unlock_file,
        guard.clone(),
        KeystoreConfig::insecure_integration_test(),
    )
    .expect("candidate");
    let qualified = candidate
        .qualify(semantic_signer_id(), content_ref(0x4b))
        .await
        .expect("initial qualification");
    assert_eq!(guard.checks(), 1);
    guard.set_eligible(false);
    assert!(matches!(
        qualified.into_read_signing_provider().await,
        Err(SigningError::Provider {
            reason: SigningProviderError::ReadAttestationIneligible
        })
    ));
    assert_eq!(guard.checks(), 1);

    guard.set_eligible(true);
    let candidate = KeystoreSignerProvider::new_with_config(
        qualified_binding(keystore.address, 0x4c),
        keystore.entry_id,
        &keystore.keystore_path,
        &keystore.unlock_file,
        guard.clone(),
        KeystoreConfig::insecure_integration_test(),
    )
    .expect("candidate");
    let (_, _, provider) = candidate
        .qualify(semantic_signer_id(), content_ref(0x4f))
        .await
        .expect("initial qualification")
        .into_read_signing_provider()
        .await
        .expect("handoff qualification");
    let checks_before_revocation = guard.checks();
    guard.set_eligible(false);
    assert!(matches!(
        provider
            .sign_guarded(
                provider.binding().durable_generation_ref(),
                &evm_request(keystore.address),
            )
            .await,
        Err(SigningError::Provider {
            reason: SigningProviderError::ReadAttestationIneligible
        })
    ));
    assert_eq!(guard.checks(), checks_before_revocation);
}

#[tokio::test]
async fn qualification_rejects_wrong_key_and_incomplete_public_evidence() {
    let keystore = test_keystore();
    let wrong_key_guard = TestGenerationGuard::new(GuardVerdict::Current);
    let wrong_key = KeystoreSignerProvider::new_with_config(
        qualified_binding(keystore.address, 0x0c),
        keystore.other_entry_id,
        &keystore.keystore_path,
        &keystore.unlock_file,
        wrong_key_guard.clone(),
        KeystoreConfig::insecure_integration_test(),
    )
    .expect("candidate");
    assert!(matches!(
        wrong_key
            .qualify(semantic_signer_id(), content_ref(0x0f))
            .await,
        Err(SigningError::Provider {
            reason: SigningProviderError::BindingMismatch
        })
    ));
    assert_eq!(wrong_key_guard.checks(), 1);

    let incomplete_guard = TestGenerationGuard::new(GuardVerdict::Current);
    let incomplete = KeystoreSignerProvider::new_with_config(
        binding(keystore.address, 0x10),
        keystore.entry_id,
        &keystore.keystore_path,
        &keystore.unlock_file,
        incomplete_guard.clone(),
        KeystoreConfig::insecure_integration_test(),
    )
    .expect("candidate");
    assert!(matches!(
        incomplete
            .qualify(semantic_signer_id(), content_ref(0x13))
            .await,
        Err(SigningError::InvalidRequest { .. })
    ));
    assert_eq!(incomplete_guard.checks(), 0);
}

#[tokio::test]
async fn qualification_guard_failure_precedes_every_runtime_source_access() {
    let dir = tempfile::tempdir().expect("tempdir");
    let guard = TestGenerationGuard::new(GuardVerdict::DirectSigningOverlap);
    let candidate = KeystoreSignerProvider::new(
        qualified_binding(Address::from([0x11; 20]), 0x14),
        Uuid::new_v4(),
        dir.path().join("missing.keystore"),
        dir.path().join("missing.unlock"),
        guard.clone(),
    )
    .expect("candidate");
    assert!(matches!(
        candidate
            .qualify(semantic_signer_id(), content_ref(0x17))
            .await,
        Err(SigningError::Provider {
            reason: SigningProviderError::DirectSigningOverlap
        })
    ));
    assert_eq!(guard.checks(), 1);
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
        first_provider.binding().profile().as_str(),
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    );

    let first = sign(&first_provider, &request).await.expect("first sign");
    let second = sign(&second_provider, &request).await.expect("second sign");

    assert_eq!(first.signature(), second.signature());
    assert_eq!(first.profile(), request.profile());
    let signature =
        PrimitiveSignature::from_raw(first.signature().as_bytes()).expect("recoverable signature");
    assert!(signature.normalize_s().is_none());
    assert_eq!(
        signature
            .recover_address_from_prehash(&B256::from(*request.digest()))
            .expect("recover"),
        keystore.address
    );
}

#[tokio::test]
async fn repeat_signing_reads_the_unlock_file_for_each_call() {
    let keystore = test_keystore();
    let request = evm_request(keystore.address);
    let provider = provider(&keystore, keystore.entry_id);

    sign(&provider, &request).await.expect("initial sign");
    fs::write(&keystore.unlock_file, "wrong_password_123\n").expect("replace unlock file");
    assert!(matches!(
        sign(&provider, &request).await,
        Err(SigningError::Provider { .. })
    ));
    fs::write(&keystore.unlock_file, format!("{PASSWORD}\r\n")).expect("restore unlock file");
    sign(&provider, &request).await.expect("restored sign");
}

#[tokio::test]
async fn provider_accepts_caller_owned_domain_and_purpose() {
    let keystore = test_keystore();
    let request = request(
        keystore.address,
        algorithm(),
        profile(),
        "protocol.other",
        "operation.authorize.alternate",
    );

    let provider = provider(&keystore, keystore.entry_id);
    let result = sign(&provider, &request)
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

    let provider = provider(&keystore, keystore.other_entry_id);
    let error = sign(&provider, &request).await.expect_err("wrong key");

    assert!(matches!(
        error,
        SigningError::Provider {
            reason: SigningProviderError::BindingMismatch
        }
    ));
}

#[tokio::test]
async fn unsupported_binding_algorithm_and_profile_fail_before_file_access() {
    let dir = tempfile::tempdir().expect("tempdir");
    let expected = Address::from([0x11; 20]);
    let guard = TestGenerationGuard::new(GuardVerdict::Current);
    let provider = KeystoreSignerProvider::new(
        binding(expected, 0x20),
        Uuid::new_v4(),
        dir.path().join("missing.keystore"),
        dir.path().join("missing.password"),
        guard.clone(),
    )
    .expect("checked provider paths");

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
        request(
            Address::from([0x22; 20]),
            algorithm(),
            profile(),
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
            sign(&provider, &request).await,
            Err(SigningError::Provider {
                reason: SigningProviderError::BindingMismatch
            })
        );
    }
    assert_eq!(guard.checks(), 0);
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

    let guard = TestGenerationGuard::new(GuardVerdict::Current);
    let provider = provider_with_guard(&keystore, keystore.entry_id, guard.clone());
    assert_eq!(
        sign(&provider, &request).await,
        Err(SigningError::Provider {
            reason: SigningProviderError::BindingMismatch
        })
    );
    assert_eq!(guard.checks(), 0);
}

#[tokio::test]
async fn every_failure_and_debug_surface_redacts_runtime_secrets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let secret_path = dir.path().join("secret-wallet-file-name.keystore");
    let unlock_file = dir.path().join("secret-unlock-file-name.txt");
    fs::write(&unlock_file, "very_secret_password\n").expect("unlock file");
    let provider = KeystoreSignerProvider::new(
        binding(Address::from([0x11; 20]), 0x30),
        Uuid::new_v4(),
        &secret_path,
        &unlock_file,
        TestGenerationGuard::new(GuardVerdict::Current),
    )
    .expect("checked provider paths");

    let error = sign(&provider, &evm_request(Address::from([0x11; 20])))
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
async fn missing_read_keystore_does_not_create_a_parent_or_replacement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing_parent = dir.path().join("missing-parent");
    let missing_keystore = missing_parent.join("wallet.keystore");
    let unlock_file = dir.path().join("unlock.txt");
    fs::write(&unlock_file, PASSWORD).expect("unlock file");
    let address = Address::from([0x11; 20]);
    let provider = KeystoreSignerProvider::new(
        binding(address, 0x35),
        Uuid::new_v4(),
        &missing_keystore,
        &unlock_file,
        TestGenerationGuard::new(GuardVerdict::Current),
    )
    .expect("checked provider paths");

    let error = sign(&provider, &evm_request(address))
        .await
        .expect_err("missing keystore");
    assert!(matches!(error, SigningError::Provider { .. }));
    assert!(!missing_parent.exists());
    assert!(!missing_keystore.exists());
}

#[tokio::test]
async fn tampered_keystore_failure_stays_redacted() {
    let keystore = test_keystore();
    fs::write(&keystore.keystore_path, b"{\"tampered\":true}").expect("tamper");

    let provider = provider(&keystore, keystore.entry_id);
    let error = sign(&provider, &evm_request(keystore.address))
        .await
        .expect_err("tampered keystore");

    assert!(matches!(error, SigningError::Provider { .. }));
    assert!(!format!("{error:?} {error}").contains("tampered"));
}

#[test]
fn qualified_provider_and_bearer_values_have_no_direct_or_persistable_surface() {
    assert_not_impl_any!(KeystoreSignerProvider: SigningProvider, DeterministicSigningProvider, Clone);
    assert_not_impl_any!(QualifiedKeystoreSigner: Clone, serde::Serialize);
    assert_not_impl_any!(QualifiedReadSigningProvider: serde::Serialize);
    assert_not_impl_any!(SigningRequest: Clone, serde::Serialize);
    assert_not_impl_any!(SignatureBytes: Clone, serde::Serialize);
    assert_not_impl_any!(SigningResult: Clone, serde::Serialize);
}

#[test]
fn keystore_provider_rejects_wrong_implementation_algorithm_and_profile_bindings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let address = Address::from([0x11; 20]);
    let identity = |algorithm: SigningAlgorithmId| {
        PublicSigningIdentity::new(algorithm, None, Some(format!("{address:?}"))).expect("identity")
    };
    let cases = [
        qualified_binding_for_implementation(address, 0x49, "mfm.signing.keystore.rfc6979.v1"),
        VerifiedGenerationGuardedSignerBinding::verify(
            signer_ref(),
            "mfm.signing.other-provider.v1",
            algorithm(),
            profile(),
            identity(algorithm()),
            content_ref(0x50),
            content_ref(0x51),
            content_ref(0x52),
        )
        .expect("structurally valid provider binding"),
        VerifiedGenerationGuardedSignerBinding::verify(
            signer_ref(),
            KEYSTORE_SIGNING_IMPLEMENTATION_ID,
            SigningAlgorithmId::new("ed25519.sha512").expect("algorithm"),
            profile(),
            identity(SigningAlgorithmId::new("ed25519.sha512").expect("algorithm")),
            content_ref(0x53),
            content_ref(0x54),
            content_ref(0x55),
        )
        .expect("structurally valid algorithm binding"),
        VerifiedGenerationGuardedSignerBinding::verify(
            signer_ref(),
            KEYSTORE_SIGNING_IMPLEMENTATION_ID,
            algorithm(),
            SigningProfileId::new("secp256k1.other.deterministic.v1").expect("profile"),
            identity(algorithm()),
            content_ref(0x56),
            content_ref(0x57),
            content_ref(0x58),
        )
        .expect("structurally valid profile binding"),
    ];

    for binding in cases {
        let guard = TestGenerationGuard::new(GuardVerdict::Current);
        let error = KeystoreSignerProvider::new(
            binding,
            Uuid::new_v4(),
            dir.path().join("missing.keystore"),
            dir.path().join("missing.password"),
            guard.clone(),
        )
        .expect_err("unsupported provider binding");
        assert_eq!(
            error,
            SigningError::Provider {
                reason: SigningProviderError::BindingMismatch
            }
        );
        assert_eq!(guard.checks(), 0);
    }
}

#[test]
fn observational_v2_provider_has_a_distinct_public_descriptor() {
    let address = Address::from([0x11; 20]);
    let v1 = qualified_binding_for_implementation(address, 0x5a, "mfm.signing.keystore.rfc6979.v1");
    let v2 = qualified_binding(address, 0x5a);

    assert_eq!(
        v2.provider_implementation_id().as_str(),
        "mfm.signing.keystore.rfc6979.v2"
    );
    assert_ne!(
        v1.public_descriptor().expect("v1 descriptor").reference(),
        v2.public_descriptor().expect("v2 descriptor").reference()
    );
}

#[tokio::test]
async fn expected_sibling_generation_is_rejected_before_guard_or_file_access() {
    let dir = tempfile::tempdir().expect("tempdir");
    let address = Address::from([0x11; 20]);
    let guard = TestGenerationGuard::new(GuardVerdict::Current);
    let provider = KeystoreSignerProvider::new(
        binding(address, 0x60),
        Uuid::new_v4(),
        dir.path().join("missing.keystore"),
        dir.path().join("missing.password"),
        guard.clone(),
    )
    .expect("provider");
    let request = evm_request(address);

    let error = provider
        .sign_guarded(&content_ref(0x70), &request)
        .await
        .expect_err("sibling generation");
    assert_eq!(
        error,
        SigningError::Provider {
            reason: SigningProviderError::GenerationMismatch
        }
    );
    assert_eq!(guard.checks(), 0);
}

#[tokio::test]
async fn stale_unavailable_and_direct_sign_overlap_guards_fail_before_file_access() {
    let dir = tempfile::tempdir().expect("tempdir");
    let address = Address::from([0x11; 20]);
    let request = evm_request(address);
    let cases = [
        (GuardVerdict::Fenced, SigningProviderError::GenerationFenced),
        (
            GuardVerdict::Unavailable,
            SigningProviderError::GenerationGuardUnavailable,
        ),
        (
            GuardVerdict::DirectSigningOverlap,
            SigningProviderError::DirectSigningOverlap,
        ),
    ];

    for (verdict, expected_reason) in cases {
        let guard = TestGenerationGuard::new(verdict);
        let provider = KeystoreSignerProvider::new(
            binding(address, 0x61),
            Uuid::new_v4(),
            dir.path().join("missing.keystore"),
            dir.path().join("missing.password"),
            guard.clone(),
        )
        .expect("provider");
        let generation = provider.binding().durable_generation_ref().clone();
        let error = provider
            .sign_guarded(&generation, &request)
            .await
            .expect_err("guard rejection");
        assert_eq!(
            error,
            SigningError::Provider {
                reason: expected_reason
            }
        );
        assert_eq!(guard.checks(), 1);
    }
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
        let error = KeystoreSignerProvider::new(
            binding(Address::from([0x11; 20]), 0x40),
            Uuid::new_v4(),
            path,
            "/run/mfm/unlock",
            TestGenerationGuard::new(GuardVerdict::Current),
        )
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
