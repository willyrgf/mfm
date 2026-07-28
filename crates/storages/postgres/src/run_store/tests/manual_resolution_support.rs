use super::*;

pub(super) fn manual_resolution_recorded(
    verified: &VerifiedManualResolutionForPrefix,
) -> KernelEventPayload {
    let claim = verified.claim();
    let authorization = verified.authorization();
    let artifacts = manual_resolution_artifacts(verified);
    KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id: claim.run_id.clone(),
        spec_hash: claim.spec_hash.clone(),
        outcome: claim.outcome,
        evidence_schema_id: claim.evidence.schema_id.clone(),
        evidence_hash: claim.evidence.content_hash.clone(),
        evidence_artifact_id: claim.evidence.artifact_id.clone(),
        evidence_artifact_evidence_hash: artifacts[0]
            .evidence_hash()
            .expect("manual evidence hash"),
        authorization_schema_id: authorization.schema_id.clone(),
        authorization_hash: authorization.content_hash.clone(),
        authorization_artifact_id: authorization.artifact_id.clone(),
        authorization_artifact_evidence_hash: artifacts[1]
            .evidence_hash()
            .expect("manual authorization evidence hash"),
        note: Some(events::ManualResolutionNote::new("reviewed evidence").expect("note")),
    })
}

pub(super) fn manual_resolution_artifacts(
    verified: &VerifiedManualResolutionForPrefix,
) -> Vec<ArtifactEvidenceRef> {
    let claim = verified.claim();
    let authorization = verified.authorization();
    vec![
        ArtifactEvidenceRef {
            artifact_id: claim.evidence.artifact_id.clone(),
            digest: claim.evidence.content_hash.clone(),
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(claim.evidence.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionEvidence,
        },
        ArtifactEvidenceRef {
            artifact_id: authorization.artifact_id.clone(),
            digest: authorization.content_hash.clone(),
            byte_len: verified.proof_bytes().len() as u64,
            media_type: media_type("application/json"),
            schema_id: Some(authorization.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionAuthorization,
        },
    ]
}

pub(super) fn manual_resolution_prepared_artifact_bytes(
    verified: &VerifiedManualResolutionForPrefix,
) -> mfm_store::v1::Result<Vec<PreparedArtifactBytes>> {
    let artifacts = manual_resolution_artifacts(verified);
    Ok(vec![
        test_prepared_artifact_bytes(&artifacts[0])?,
        PreparedArtifactBytes::new(verified.proof_bytes().to_vec(), artifacts[1].clone())?,
    ])
}

pub(super) fn manual_saga_policy(byte: u8) -> SagaPolicySpec {
    SagaPolicySpec::ManualResolution {
        manual: Box::new(ManualResolutionEvidenceSpec {
            evidence_schema: schema_id("mfm.test.manual_evidence", byte + 1),
            authorization: manual_authorization(byte),
        }),
    }
}

fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
            "mfm.test.manual.verifier.{byte}"
        ))
        .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new(format!(
                "mfm.test.manual.authority.{byte}"
            ))
            .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                    .expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(
                    "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                )
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

pub(super) fn verified_manual_resolution_for_seq(
    run_id: &RunId,
    expected_next_seq: u64,
    byte: u8,
) -> VerifiedManualResolutionForPrefix {
    let policy = manual_saga_policy(byte);
    let spec_hash = saga_authority_spec(policy.clone())
        .spec_hash()
        .expect("manual saga authority spec hash");
    let SagaPolicySpec::ManualResolution { manual } = policy else {
        unreachable!("manual_saga_policy builds manual policy")
    };
    let prefix = ManualResolutionPrefixAuthority::new(
        run_id.clone(),
        spec_hash,
        expected_next_seq,
        content_digest(byte + 3),
        ManualResolutionBlockReason::PolicyManualResolution,
        content_digest(byte + 4),
        manual.as_ref().clone(),
    )
    .expect("manual prefix authority");
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        content_hash: content_digest(byte + 1),
        artifact_id: artifact_id(byte + 1),
    };
    let claim = prefix
        .authorization_claim(
            events::ManualResolutionOutcome::ConfirmRemediated,
            evidence.clone(),
        )
        .expect("manual authorization claim");
    let proof = signed_manual_resolution_proof(&manual.authorization, claim.clone());
    let proof_bytes = proof
        .canonical_json()
        .expect("manual proof canonical json")
        .to_vec();
    let authorization = manual_authorization_ref(&proof_bytes);
    ManualResolutionProofAuthority::new(prefix, claim.outcome, evidence, authorization, proof_bytes)
        .and_then(ManualResolutionProofAuthority::verify)
        .expect("verified manual resolution")
}

fn signed_manual_resolution_proof(
    policy: &spec::ManualResolutionAuthorizationSpec,
    claim: mfm_manual_auth::ManualResolutionAuthorizationClaim,
) -> ManualResolutionAuthorizationProof {
    let operator = policy.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("manual claim digest");
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
            .expect("manual signature bytes"),
        }],
    }
}

fn manual_authorization_ref(proof_bytes: &[u8]) -> ManualResolutionEvidenceRef {
    let proof = ManualResolutionAuthorizationProof::from_json_slice(proof_bytes)
        .expect("manual authorization proof");
    let content_hash = proof.content_digest().expect("manual proof content digest");
    ManualResolutionEvidenceRef {
        schema_id: manual_authorization_proof_schema_id().expect("manual authorization schema"),
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

pub(super) fn saga_preconditions(run_id: &RunId, policy: SagaPolicySpec) -> CommitPreconditions {
    let spec = saga_authority_spec(policy);
    CommitPreconditions {
        certified_run_authority: Some(
            mfm_store::v1::CertifiedRunStoreAuthority::from_spec(run_id.clone(), &spec)
                .expect("certified run authority"),
        ),
        ..CommitPreconditions::default()
    }
}
