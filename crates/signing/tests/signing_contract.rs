use mfm_canonical::sha256_digest_bytes;
use mfm_ids::DigestBytes;
use mfm_signing::*;

fn signer_ref() -> SignerRef {
    SignerRef::new("deployer").expect("signer ref")
}

fn algorithm() -> SigningAlgorithmId {
    SigningAlgorithmId::new("mfm.signing.test").expect("algorithm")
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

fn request() -> SigningRequest {
    SigningRequest::from_digest(signer_ref(), algorithm(), domain(), purpose(), digest())
}

#[test]
fn manual_resolution_signing_request_is_digest_only_and_domain_separated() {
    let request = manual_resolution_signing_request(signer_ref(), algorithm(), digest())
        .expect("manual request");

    assert_eq!(
        request.domain().as_str(),
        MANUAL_RESOLUTION_SIGNING_DOMAIN_ID
    );
    assert_eq!(
        request.purpose().as_str(),
        MANUAL_RESOLUTION_SIGNING_PURPOSE_ID
    );
    assert_eq!(request.digest(), &digest());
}

#[test]
fn signer_ref_validation_accepts_public_refs() {
    assert_eq!(
        SignerRef::new("local.deployer-1")
            .expect("valid signer ref")
            .as_str(),
        "local.deployer-1"
    );
}

#[test]
fn signer_ref_validation_rejects_invalid_refs_without_echoing_values() {
    for value in ["", "Deployer", "bad/path", "-bad", "bad-"] {
        let err = SignerRef::new(value).expect_err("invalid signer ref");
        if !value.is_empty() {
            assert!(!err.to_string().contains(value));
        }
    }
}

#[test]
fn signing_request_debug_does_not_render_digest_bytes() {
    let request = request();
    let rendered = format!("{request:?}");

    assert!(rendered.contains("digest"));
    assert!(!rendered.contains(&digest().to_string()));
}

#[test]
fn signing_result_verifies_expected_public_identity() {
    let public_key = PublicKeyBytes::new(vec![1, 2, 3]).expect("public key");
    let request =
        request().require_public_identity(ExpectedSignerIdentity::public_key(public_key.clone()));
    let identity =
        PublicSigningIdentity::new(algorithm(), Some(public_key), None).expect("identity");
    let signature = SignatureBytes::new(vec![4, 5, 6]).expect("signature");

    let result = SigningResult::for_request(&request, identity, signature).expect("signing result");

    assert_eq!(result.signer_ref(), request.signer_ref());
}

#[test]
fn signing_result_rejects_public_identity_mismatch() {
    let expected = PublicKeyBytes::new(vec![1, 2, 3]).expect("expected public key");
    let actual = PublicKeyBytes::new(vec![9, 9, 9]).expect("actual public key");
    let request = request().require_public_identity(ExpectedSignerIdentity::public_key(expected));
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
