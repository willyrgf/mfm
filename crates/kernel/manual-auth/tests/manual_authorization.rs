use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, DigestBytes, RunId, SchemaId, SpecHash};
use mfm_manual_auth::*;
use mfm_spec::v1 as spec;

struct AcceptingVerifier;

impl ManualAuthorizationVerifier for AcceptingVerifier {
    fn verify(&self, _verification: ManualAuthorizationVerification<'_>) -> Result<()> {
        Ok(())
    }
}

#[test]
fn claim_digest_is_canonical_and_stable() {
    let claim = claim();

    assert_eq!(
        claim.digest().expect("digest").as_str(),
        "content:sha256-jcs-v1:3e2a6ff2fb3b393f9530d4239e223181b863a58839c962629f62027eefcff611"
    );
}

#[test]
fn proof_round_trips_only_canonical_json() {
    let proof = proof();
    let canonical = proof.canonical_json().expect("canonical");

    let parsed =
        ManualResolutionAuthorizationProof::from_json_slice(canonical.as_bytes()).expect("parse");
    assert_eq!(parsed, proof);

    let mut value: serde_json::Value =
        serde_json::from_slice(canonical.as_bytes()).expect("json value");
    value["extra"] = serde_json::json!("field");
    let raw = serde_json::to_vec(&value).expect("json");
    let error = ManualResolutionAuthorizationProof::from_json_slice(&raw)
        .expect_err("non-canonical/unknown field rejects");
    assert!(matches!(
        error,
        ManualAuthorizationError::Canonical(_) | ManualAuthorizationError::InvalidShape(_)
    ));
}

#[test]
fn registry_mints_verified_manual_resolution_after_verifier_accepts() {
    let policy = policy();
    let claim = claim();
    let proof = proof();
    let mut registry = ManualAuthorizationVerifierRegistry::new();
    registry
        .register(policy.verifier_id.clone(), AcceptingVerifier)
        .expect("register verifier");

    let verified = registry
        .verify(&policy, claim.clone(), proof)
        .expect("verified");

    assert_eq!(verified.claim(), &claim);
}

#[test]
fn registry_rejects_policy_and_quorum_mismatches_before_verifier() {
    let policy = policy();
    let mut missing_quorum_proof = proof();
    missing_quorum_proof.signatures.clear();
    let registry = ManualAuthorizationVerifierRegistry::new();

    let error = registry
        .verify(&policy, claim(), missing_quorum_proof)
        .expect_err("quorum rejects");

    assert!(matches!(
        error,
        ManualAuthorizationError::QuorumUnsatisfied { .. }
    ));

    let mut proof = proof();
    proof.verifier_id = spec::ManualAuthorizationVerifierId::new("mfm.manual_auth.test.other")
        .expect("verifier id");
    let error = registry
        .verify(&policy, claim(), proof)
        .expect_err("verifier mismatch rejects");
    assert_eq!(
        error,
        ManualAuthorizationError::PolicyMismatch("verifier_id")
    );
}

#[test]
fn proof_authority_verifies_canonical_proof_bytes_for_prefix() {
    let policy = policy();
    let claim = claim();
    let manual_policy = spec::ManualResolutionEvidenceSpec {
        evidence_schema: claim.evidence.schema_id.clone(),
        authorization: policy.clone(),
    };
    let prefix = ManualResolutionPrefixAuthority::new(
        claim.run_id.clone(),
        claim.spec_hash.clone(),
        claim.expected_next_seq,
        claim.stream_prefix_digest.clone(),
        claim.manual_block_reason,
        claim.unresolved_obligations_digest.clone(),
        manual_policy,
    )
    .expect("prefix authority");
    let proof = signed_proof(&policy, claim.clone());
    let proof_bytes = proof.canonical_json().expect("canonical proof").to_vec();
    let authorization = authorization_ref(&proof_bytes);

    let verified = ManualResolutionProofAuthority::new(
        prefix,
        claim.outcome,
        claim.evidence.clone(),
        authorization,
        proof_bytes,
    )
    .expect("proof authority")
    .verify()
    .expect("verified for prefix");

    assert_eq!(verified.claim(), &claim);
    assert_eq!(verified.proof(), &proof);
}

fn proof() -> ManualResolutionAuthorizationProof {
    let policy = policy();
    let operator = policy.authority.operators[0].clone();
    ManualResolutionAuthorizationProof {
        verifier_id: policy.verifier_id,
        signing_scheme: policy.signing_scheme,
        claim: claim(),
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(vec![0xab; 64]).expect("signature"),
        }],
    }
}

fn policy() -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new("mfm.manual_auth.test.verifier")
            .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new("mfm.manual_auth.test.authority")
                .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new("operator.manual-auth").expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(
                    "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                )
                .expect("public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

fn claim() -> ManualResolutionAuthorizationClaim {
    ManualResolutionAuthorizationClaim {
        run_id: run_id(0x11),
        spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x12)),
        expected_next_seq: 9,
        stream_prefix_digest: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0x13),
        ),
        manual_block_reason: ManualResolutionBlockReason::PolicyManualResolution,
        unresolved_obligations_digest: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0x14),
        ),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence: ManualResolutionEvidenceRef {
            schema_id: SchemaId::new(
                "mfm.manual_auth.test.evidence",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x15),
            )
            .expect("schema"),
            content_hash: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x16),
            ),
            artifact_id: ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x16)),
        },
    }
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(byte))
}

fn signed_proof(
    policy: &spec::ManualResolutionAuthorizationSpec,
    claim: ManualResolutionAuthorizationClaim,
) -> ManualResolutionAuthorizationProof {
    let operator = policy.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    ManualResolutionAuthorizationProof {
        verifier_id: policy.verifier_id.clone(),
        signing_scheme: policy.signing_scheme.clone(),
        claim,
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    }
}

fn authorization_ref(proof_bytes: &[u8]) -> ManualResolutionEvidenceRef {
    let content_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(proof_bytes),
    );
    ManualResolutionEvidenceRef {
        schema_id: manual_authorization_proof_schema_id().expect("authorization schema"),
        artifact_id: ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest()),
        content_hash,
    }
}

fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

fn digest_byte(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}
