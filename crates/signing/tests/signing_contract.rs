use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_signing::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

fn signer_ref() -> SignerRef {
    SignerRef::new("deployer").expect("signer ref")
}

fn algorithm() -> SigningAlgorithmId {
    SigningAlgorithmId::new("mfm.signing.test").expect("algorithm")
}

fn profile() -> SigningProfileId {
    SigningProfileId::new("mfm.signing.test.deterministic.v1").expect("profile")
}

fn domain() -> SigningDomainId {
    SigningDomainId::new("mfm.test").expect("domain")
}

fn purpose() -> SigningPurposeId {
    SigningPurposeId::new("test.sign").expect("purpose")
}

fn digest() -> DigestBytes {
    sha256_digest_bytes(b"message")
}

fn content_ref(seed: u8) -> ContentRef {
    let schema_id = SchemaId::new(
        "mfm.test.signer-binding",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([seed; 32]),
    )
    .expect("schema id");
    ContentRef::new(
        schema_id,
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            DigestBytes::from_array([seed; 32]),
        ),
    )
    .expect("content ref")
}

fn guarded_binding(
    implementation_id: &str,
    generation_seed: u8,
) -> VerifiedGenerationGuardedSignerBinding {
    guarded_binding_fields(
        "deployer",
        implementation_id,
        "mfm.signing.test",
        "mfm.signing.test.deterministic.v1",
        "0x1111",
        None,
        generation_seed,
        generation_seed.wrapping_add(1),
        generation_seed.wrapping_add(2),
    )
}

#[allow(clippy::too_many_arguments)]
fn guarded_binding_fields(
    signer: &str,
    implementation_id: &str,
    algorithm_id: &str,
    profile_id: &str,
    account_id: &str,
    public_key_seed: Option<u8>,
    generation_seed: u8,
    fence_seed: u8,
    exclusion_seed: u8,
) -> VerifiedGenerationGuardedSignerBinding {
    let algorithm = SigningAlgorithmId::new(algorithm_id).expect("algorithm");
    VerifiedGenerationGuardedSignerBinding::verify(
        SignerRef::new(signer).expect("signer"),
        implementation_id,
        algorithm.clone(),
        SigningProfileId::new(profile_id).expect("profile"),
        PublicSigningIdentity::new(
            algorithm,
            public_key_seed.map(|seed| PublicKeyBytes::new(vec![seed; 33]).expect("public key")),
            Some(account_id.to_owned()),
        )
        .expect("identity"),
        content_ref(generation_seed),
        content_ref(fence_seed),
        content_ref(exclusion_seed),
    )
    .expect("binding")
}

fn request() -> SigningRequest {
    SigningRequest::from_digest(
        signer_ref(),
        algorithm(),
        profile(),
        domain(),
        purpose(),
        digest(),
    )
}

#[test]
fn signing_request_debug_does_not_render_digest_bytes() {
    let request = request();
    let rendered = format!("{request:?}");

    assert!(rendered.contains("digest"));
    assert!(!rendered.contains(&digest().to_string()));
}

#[test]
fn signing_result_enforces_expected_public_identity() {
    let public_key = PublicKeyBytes::new(vec![1, 2, 3]).expect("public key");
    let request =
        request().require_public_identity(ExpectedSignerIdentity::public_key(public_key.clone()));
    let identity =
        PublicSigningIdentity::new(algorithm(), Some(public_key), None).expect("identity");
    let signature = SignatureBytes::new(vec![4, 5, 6]).expect("signature");

    let result = SigningResult::for_request(&request, identity, signature).expect("signing result");

    assert_eq!(result.signer_ref(), request.signer_ref());
    assert_eq!(result.profile(), request.profile());
    assert!(!format!("{result:?}").contains("[4, 5, 6]"));
    let actual = PublicKeyBytes::new(vec![9, 9, 9]).expect("actual public key");
    let identity =
        PublicSigningIdentity::new(algorithm(), Some(actual), None).expect("actual identity");
    let signature = SignatureBytes::new(vec![4, 5, 6]).expect("signature");

    let err =
        SigningResult::for_request(&request, identity, signature).expect_err("identity mismatch");

    assert!(matches!(
        err,
        SigningError::PublicIdentityMismatch {
            field: "public_key",
            ..
        }
    ));
}

#[test]
fn provider_errors_are_redaction_safe() {
    let source = "/tmp/secret/mfm-key.json";
    let err = SigningError::redacted_provider_failure(source);
    let rendered = err.to_string();

    assert!(matches!(
        err,
        SigningError::Provider {
            reason: SigningProviderError::Failed
        }
    ));
    assert!(!rendered.contains(source));
    assert!(!rendered.contains("/tmp/secret"));
}

#[test]
fn guarded_binding_fixes_every_wallet_identity_and_exclusion_field() {
    let binding = guarded_binding("mfm.test.guarded-signer", 0x10);
    let exact_request = request().require_public_identity(
        ExpectedSignerIdentity::account_id("0x1111").expect("expected account"),
    );

    assert_eq!(binding.signer_ref(), exact_request.signer_ref());
    assert_eq!(
        binding.provider_implementation_id().as_str(),
        "mfm.test.guarded-signer"
    );
    assert_eq!(binding.algorithm(), exact_request.algorithm());
    assert_eq!(binding.profile(), exact_request.profile());
    assert_eq!(
        binding.expected_public_identity().account_id(),
        Some("0x1111")
    );
    assert_eq!(binding.durable_generation_ref(), &content_ref(0x10));
    assert_eq!(binding.fence_attestation_ref(), &content_ref(0x11));
    assert_eq!(binding.direct_sign_exclusion_ref(), &content_ref(0x12));
    binding
        .verify_request(&exact_request)
        .expect("exact request");

    let wrong_account = request().require_public_identity(
        ExpectedSignerIdentity::account_id("0x2222").expect("expected account"),
    );
    assert_eq!(
        binding
            .verify_request(&wrong_account)
            .expect_err("account mismatch"),
        SigningError::Provider {
            reason: SigningProviderError::BindingMismatch
        }
    );
}

#[test]
fn guarded_signer_descriptor_content_identity_fixes_every_public_field() {
    let base = guarded_binding("mfm.test.guarded-signer", 0x40)
        .public_descriptor()
        .expect("base descriptor");
    let variants = [
        guarded_binding_fields(
            "alternate",
            "mfm.test.guarded-signer",
            "mfm.signing.test",
            "mfm.signing.test.deterministic.v1",
            "0x1111",
            None,
            0x40,
            0x41,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.alternate-signer",
            "mfm.signing.test",
            "mfm.signing.test.deterministic.v1",
            "0x1111",
            None,
            0x40,
            0x41,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.guarded-signer",
            "mfm.signing.alternate",
            "mfm.signing.test.deterministic.v1",
            "0x1111",
            None,
            0x40,
            0x41,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.guarded-signer",
            "mfm.signing.test",
            "mfm.signing.alternate.deterministic.v1",
            "0x1111",
            None,
            0x40,
            0x41,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.guarded-signer",
            "mfm.signing.test",
            "mfm.signing.test.deterministic.v1",
            "0x2222",
            None,
            0x40,
            0x41,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.guarded-signer",
            "mfm.signing.test",
            "mfm.signing.test.deterministic.v1",
            "0x1111",
            Some(0x05),
            0x40,
            0x41,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.guarded-signer",
            "mfm.signing.test",
            "mfm.signing.test.deterministic.v1",
            "0x1111",
            None,
            0x43,
            0x41,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.guarded-signer",
            "mfm.signing.test",
            "mfm.signing.test.deterministic.v1",
            "0x1111",
            None,
            0x40,
            0x44,
            0x42,
        ),
        guarded_binding_fields(
            "deployer",
            "mfm.test.guarded-signer",
            "mfm.signing.test",
            "mfm.signing.test.deterministic.v1",
            "0x1111",
            None,
            0x40,
            0x41,
            0x45,
        ),
    ];

    for variant in variants {
        let variant = variant.public_descriptor().expect("variant descriptor");
        assert_ne!(variant.reference(), base.reference());
        assert_ne!(variant.canonical(), base.canonical());
    }
}

#[test]
fn guarded_binding_requires_an_account_and_matching_identity_algorithm() {
    let missing_account = VerifiedGenerationGuardedSignerBinding::verify(
        signer_ref(),
        "mfm.test.guarded-signer",
        algorithm(),
        profile(),
        PublicSigningIdentity::new(
            algorithm(),
            Some(PublicKeyBytes::new(vec![1, 2, 3]).expect("public key")),
            None,
        )
        .expect("public identity"),
        content_ref(0x20),
        content_ref(0x21),
        content_ref(0x22),
    )
    .expect_err("account is mandatory");
    assert_eq!(
        missing_account,
        SigningError::InvalidRequest {
            reason: SigningRequestError::MissingAccountIdentity
        }
    );

    let mismatch = VerifiedGenerationGuardedSignerBinding::verify(
        signer_ref(),
        "mfm.test.guarded-signer",
        algorithm(),
        profile(),
        PublicSigningIdentity::new(
            SigningAlgorithmId::new("mfm.signing.other").expect("algorithm"),
            None,
            Some("0x1111".to_owned()),
        )
        .expect("public identity"),
        content_ref(0x23),
        content_ref(0x24),
        content_ref(0x25),
    )
    .expect_err("algorithm mismatch");
    assert!(matches!(
        mismatch,
        SigningError::PublicIdentityMismatch {
            field: "algorithm",
            ..
        }
    ));
}

struct FixedGuardedProvider {
    binding: VerifiedGenerationGuardedSignerBinding,
    substituted_binding: VerifiedGenerationGuardedSignerBinding,
    eligible: AtomicBool,
    substitute_during_qualification: bool,
    revoke_during_qualification: bool,
    substituted: AtomicBool,
    qualifications: AtomicUsize,
    signs: AtomicUsize,
}

impl FixedGuardedProvider {
    fn new(binding: VerifiedGenerationGuardedSignerBinding, eligible: bool) -> Self {
        Self {
            substituted_binding: binding.clone(),
            binding,
            eligible: AtomicBool::new(eligible),
            substitute_during_qualification: false,
            revoke_during_qualification: false,
            substituted: AtomicBool::new(false),
            qualifications: AtomicUsize::new(0),
            signs: AtomicUsize::new(0),
        }
    }
}

impl GenerationGuardedDeterministicSigningProvider for FixedGuardedProvider {
    fn is_read_attestation_eligible(&self) -> bool {
        self.eligible.load(Ordering::SeqCst)
    }

    fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        if self.substituted.load(Ordering::SeqCst) {
            &self.substituted_binding
        } else {
            &self.binding
        }
    }

    fn verify_read_attestation_qualification<'a>(
        &'a self,
        expected_binding: &'a VerifiedGenerationGuardedSignerBinding,
    ) -> ReadAttestationQualificationFuture<'a> {
        self.qualifications.fetch_add(1, Ordering::SeqCst);
        if self.substitute_during_qualification {
            self.substituted.store(true, Ordering::SeqCst);
        }
        if self.revoke_during_qualification {
            self.eligible.store(false, Ordering::SeqCst);
        }
        let result = if expected_binding == &self.binding {
            Ok(())
        } else {
            Err(SigningError::Provider {
                reason: SigningProviderError::BindingMismatch,
            })
        };
        Box::pin(async move { result })
    }

    fn sign_guarded<'a>(
        &'a self,
        _expected_generation_ref: &'a ContentRef,
        _request: &'a SigningRequest,
    ) -> SigningFuture<'a> {
        self.signs.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(SigningError::Provider {
                reason: SigningProviderError::Failed,
            })
        })
    }
}

#[tokio::test]
async fn read_qualification_rejects_every_semantic_mutation_before_provider_callbacks() {
    for mutation in [
        "quota",
        "approval",
        "anti-replay",
        "billing",
        "rate-limit",
        "other-semantic-state",
    ] {
        let provider = Arc::new(FixedGuardedProvider::new(
            guarded_binding("mfm.test.guarded-signer", 0x30),
            false,
        ));
        let raw: Arc<dyn GenerationGuardedDeterministicSigningProvider> = provider.clone();
        assert!(
            matches!(
                QualifiedReadSigningProvider::try_qualify(raw).await,
                Err(SigningError::Provider {
                    reason: SigningProviderError::ReadAttestationIneligible
                })
            ),
            "accepted {mutation} mutation"
        );
        assert_eq!(provider.qualifications.load(Ordering::SeqCst), 0);
        assert_eq!(provider.signs.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn read_qualification_rejects_changed_eligibility_or_binding_and_rechecks_each_sign() {
    let binding = guarded_binding("mfm.test.guarded-signer", 0x31);
    let mut revoked = FixedGuardedProvider::new(binding.clone(), true);
    revoked.revoke_during_qualification = true;
    let revoked = Arc::new(revoked);
    let raw: Arc<dyn GenerationGuardedDeterministicSigningProvider> = revoked.clone();
    assert!(matches!(
        QualifiedReadSigningProvider::try_qualify(raw).await,
        Err(SigningError::Provider {
            reason: SigningProviderError::ReadAttestationIneligible
        })
    ));
    assert_eq!(revoked.qualifications.load(Ordering::SeqCst), 1);

    let mut substituted = FixedGuardedProvider::new(binding.clone(), true);
    substituted.substituted_binding = guarded_binding("mfm.test.substituted-signer", 0x32);
    substituted.substitute_during_qualification = true;
    let substituted = Arc::new(substituted);
    let raw: Arc<dyn GenerationGuardedDeterministicSigningProvider> = substituted.clone();
    assert!(matches!(
        QualifiedReadSigningProvider::try_qualify(raw).await,
        Err(SigningError::Provider {
            reason: SigningProviderError::BindingMismatch
        })
    ));

    let exact = Arc::new(FixedGuardedProvider::new(binding, true));
    let raw: Arc<dyn GenerationGuardedDeterministicSigningProvider> = exact.clone();
    let qualified = QualifiedReadSigningProvider::try_qualify(raw)
        .await
        .expect("eligible exact provider");
    assert_eq!(exact.qualifications.load(Ordering::SeqCst), 1);
    exact.eligible.store(false, Ordering::SeqCst);
    assert!(matches!(
        qualified
            .sign_guarded(qualified.binding().durable_generation_ref(), &request())
            .await,
        Err(SigningError::Provider {
            reason: SigningProviderError::ReadAttestationIneligible
        })
    ));
    assert_eq!(exact.signs.load(Ordering::SeqCst), 0);
}
