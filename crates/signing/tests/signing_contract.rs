use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_signing::*;
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
    VerifiedGenerationGuardedSignerBinding::verify(
        signer_ref(),
        implementation_id,
        algorithm(),
        profile(),
        PublicSigningIdentity::new(algorithm(), None, Some("0x1111".to_owned())).expect("identity"),
        content_ref(generation_seed),
        content_ref(generation_seed.wrapping_add(1)),
        content_ref(generation_seed.wrapping_add(2)),
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
}

impl GenerationGuardedDeterministicSigningProvider for FixedGuardedProvider {
    fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        &self.binding
    }

    fn sign_guarded<'a>(
        &'a self,
        _expected_generation_ref: &'a ContentRef,
        _request: &'a SigningRequest,
    ) -> SigningFuture<'a> {
        Box::pin(async {
            Err(SigningError::Provider {
                reason: SigningProviderError::Failed,
            })
        })
    }
}

#[tokio::test]
async fn guarded_binder_owns_one_binding_and_rejects_provider_substitution() {
    let expected = guarded_binding("mfm.test.guarded-signer", 0x30);
    let returned = expected.clone();
    let binder =
        GenerationGuardedDeterministicSigningProviderBinder::new(expected.clone(), move || {
            let provider: Arc<dyn GenerationGuardedDeterministicSigningProvider> =
                Arc::new(FixedGuardedProvider {
                    binding: returned.clone(),
                });
            Box::pin(async move { Ok(provider) })
        });
    assert_eq!(binder.binding(), &expected);
    let provider = binder.bind().await.expect("exact provider");
    assert_eq!(provider.binding(), &expected);

    let substituted = guarded_binding("mfm.test.substituted-signer", 0x31);
    let binder = GenerationGuardedDeterministicSigningProviderBinder::new(expected, move || {
        let provider: Arc<dyn GenerationGuardedDeterministicSigningProvider> =
            Arc::new(FixedGuardedProvider {
                binding: substituted.clone(),
            });
        Box::pin(async move { Ok(provider) })
    });
    assert!(matches!(
        binder.bind().await,
        Err(SigningError::Provider {
            reason: SigningProviderError::BindingMismatch
        })
    ));
}
