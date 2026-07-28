use super::*;

pub(in crate::tests::support) async fn append_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    outcome: events::ManualResolutionOutcome,
) {
    let manual = match &fixture.runtime_spec.spec().saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual,
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manual,
        _ => panic!("fixture does not carry manual resolution schemas"),
    };
    let evidence_bytes = br#"{"operator_note":"reviewed"}"#.to_vec();
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&evidence_bytes),
    );
    let evidence_artifact_id =
        ArtifactId::from_digest(evidence_hash.algorithm(), *evidence_hash.digest());
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        content_hash: evidence_hash,
        artifact_id: evidence_artifact_id,
    };
    let prefix = build_manual_resolution_prefix_authority_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        store,
        manual.as_ref().clone(),
    )
    .expect("manual prefix authority");
    let claim = prefix
        .authorization_claim(outcome, evidence)
        .expect("manual claim");
    let operator = manual.authorization.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: manual.authorization.verifier_id.clone(),
        signing_scheme: manual.authorization.signing_scheme.clone(),
        claim: claim.clone(),
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    };
    let proof_bytes = proof
        .canonical_json()
        .expect("canonical manual proof")
        .to_vec();
    record_manual_resolution(
        scheduler,
        store,
        &fixture.runtime_spec,
        &fixture.run_id,
        ManualResolutionRequest {
            outcome,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes,
            note: None,
        },
    )
    .await
    .expect("append manual resolution");
}

pub(in crate::tests::support) fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

pub(in crate::tests::support) fn sign_manual_claim_digest(
    signing_key: &k256::ecdsa::SigningKey,
    digest: &[u8; 32],
) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}
