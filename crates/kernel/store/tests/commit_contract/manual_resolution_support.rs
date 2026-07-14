use super::*;

pub(super) fn manual_resolution_recorded_for_run(run_id: RunId, byte: u8) -> KernelEventPayload {
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(byte + 1),
        digest: content_digest(byte + 1),
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.manual_evidence", byte + 1)),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::ManualResolutionEvidence,
    };
    let authorization = ArtifactEvidenceRef {
        artifact_id: artifact_id(byte + 2),
        digest: content_digest(byte + 2),
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.manual_authorization", byte + 2)),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::ManualResolutionAuthorization,
    };
    KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id,
        spec_hash: spec_hash(1),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence_schema_id: schema_id("mfm.test.manual_evidence", byte + 1),
        evidence_hash: content_digest(byte + 1),
        evidence_artifact_id: artifact_id(byte + 1),
        evidence_artifact_evidence_hash: evidence.evidence_hash().expect("manual evidence hash"),
        authorization_schema_id: schema_id("mfm.test.manual_authorization", byte + 2),
        authorization_hash: content_digest(byte + 2),
        authorization_artifact_id: artifact_id(byte + 2),
        authorization_artifact_evidence_hash: authorization
            .evidence_hash()
            .expect("manual authorization evidence hash"),
        note: Some(events::ManualResolutionNote::new("reviewed evidence").expect("note")),
    })
}

fn manual_resolution_artifacts(byte: u8) -> Vec<ArtifactEvidenceRef> {
    vec![
        ArtifactEvidenceRef {
            artifact_id: artifact_id(byte + 1),
            digest: content_digest(byte + 1),
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(schema_id("mfm.test.manual_evidence", byte + 1)),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionEvidence,
        },
        ArtifactEvidenceRef {
            artifact_id: artifact_id(byte + 2),
            digest: content_digest(byte + 2),
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(schema_id("mfm.test.manual_authorization", byte + 2)),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionAuthorization,
        },
    ]
}

pub(super) fn manual_saga_policy(byte: u8) -> SagaPolicySpec {
    SagaPolicySpec::ManualResolution {
        manual: ManualResolutionEvidenceSpec {
            evidence_schema: schema_id("mfm.test.manual_evidence", byte + 1),
            authorization: manual_authorization(byte),
        },
    }
}

pub(super) fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
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
                public_identity: spec::OperatorPublicIdentity::new(format!(
                    "operator-public-{byte}"
                ))
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

pub(super) fn compensate_saga_policy() -> SagaPolicySpec {
    SagaPolicySpec::CompensateCompleted {
        on_remediation_unresolved: RemediationUnresolvedSpec::FailWithoutAcdcClaim,
    }
}

pub(super) fn saga_preconditions(run_id: &RunId, policy: SagaPolicySpec) -> CommitPreconditions {
    let spec = saga_authority_spec(policy);
    CommitPreconditions {
        certified_run_authority: Some(
            CertifiedRunStoreAuthority::from_spec(run_id.clone(), &spec)
                .expect("certified run authority"),
        ),
        ..CommitPreconditions::default()
    }
}

pub(super) fn run_state_preconditions(required_run_state: RequiredRunState) -> CommitPreconditions {
    CommitPreconditions {
        required_run_state,
        ..CommitPreconditions::default()
    }
}

pub(super) fn append_run_state_commit(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: impl AsRef<str>,
    payloads: Vec<KernelEventPayload>,
    required_artifacts: Vec<ArtifactEvidenceRef>,
    required_run_state: RequiredRunState,
) -> std::result::Result<CommitOutcome, StoreError> {
    store.append_prepared_commit(typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(run_id),
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: payloads,
        required_artifacts: required_artifacts,
        preconditions: run_state_preconditions(required_run_state),
    })
}

pub(super) fn manual_resolution_request(
    run_id: &RunId,
    expected_next_seq: StreamSeq,
    commit_key: &str,
    byte: u8,
    policy: SagaPolicySpec,
) -> CommitRequest {
    let spec = saga_authority_spec(policy);
    let spec_hash = spec.spec_hash().expect("saga authority spec hash");
    let mut manual_resolution = manual_resolution_recorded_for_run(run_id.clone(), byte);
    let KernelEventPayload::ManualResolutionRecorded(payload) = &mut manual_resolution else {
        unreachable!("helper returns manual resolution payload");
    };
    payload.spec_hash = spec_hash;
    CommitRequest::from_payloads(
        run_id.clone(),
        expected_next_seq,
        CommitKey::new(commit_key).expect("commit key"),
        vec![manual_resolution],
        manual_resolution_artifacts(byte),
        CommitPreconditions {
            certified_run_authority: Some(
                CertifiedRunStoreAuthority::from_spec(run_id.clone(), &spec)
                    .expect("certified run authority"),
            ),
            ..CommitPreconditions::default()
        },
    )
    .expect("manual resolution request")
}

pub(super) fn proof_manual_saga_policy() -> SagaPolicySpec {
    SagaPolicySpec::ManualResolution {
        manual: proof_manual_evidence_spec(),
    }
}

fn proof_manual_evidence_spec() -> ManualResolutionEvidenceSpec {
    ManualResolutionEvidenceSpec {
        evidence_schema: schema_id("mfm.test.manual_evidence", 201),
        authorization: spec::ManualResolutionAuthorizationSpec {
            verifier_id: spec::ManualAuthorizationVerifierId::new("mfm.test.manual.verifier.proof")
                .expect("verifier id"),
            signing_scheme: spec::ManualSigningSchemeSpec::new(
                "mfm.manual_resolution.digest_signature.v1",
            )
            .expect("signing scheme"),
            authority: spec::OperatorAuthoritySnapshotSpec {
                authority_id: spec::OperatorAuthorityId::new("mfm.test.manual.authority.proof")
                    .expect("authority id"),
                operators: vec![spec::OperatorAuthorityMemberSpec {
                    operator_id: spec::OperatorId::new("operator.proof").expect("operator id"),
                    public_identity: spec::OperatorPublicIdentity::new(
                        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                    )
                    .expect("operator public identity"),
                }],
            },
            quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
        },
    }
}

fn proof_manual_evidence_ref() -> ManualResolutionEvidenceRef {
    ManualResolutionEvidenceRef {
        schema_id: schema_id("mfm.test.manual_evidence", 201),
        content_hash: content_digest(201),
        artifact_id: artifact_id(201),
    }
}

pub(super) fn verified_manual_resolution_for_seq(
    expected_next_seq: u64,
) -> VerifiedManualResolutionForPrefix {
    let run_id = run_id_with_saga_policy(220, &proof_manual_saga_policy());
    verified_manual_resolution_for_run_seq(&run_id, expected_next_seq)
}

pub(super) fn verified_manual_resolution_for_run_seq(
    run_id: &RunId,
    expected_next_seq: u64,
) -> VerifiedManualResolutionForPrefix {
    let spec_hash = saga_authority_spec(proof_manual_saga_policy())
        .spec_hash()
        .expect("manual saga authority spec hash");
    let prefix = ManualResolutionPrefixAuthority::new(
        run_id.clone(),
        spec_hash,
        expected_next_seq,
        content_digest(250),
        ManualResolutionBlockReason::PolicyManualResolution,
        content_digest(251),
        proof_manual_evidence_spec(),
    )
    .expect("manual prefix authority");
    let evidence = proof_manual_evidence_ref();
    let claim = prefix
        .authorization_claim(
            events::ManualResolutionOutcome::ConfirmRemediated,
            evidence.clone(),
        )
        .expect("manual authorization claim");
    for signature in manual_signature_candidates(expected_next_seq, &claim) {
        let authorization = manual_authorization_ref(signature.proof_bytes.as_bytes());
        if let Ok(verified) = ManualResolutionProofAuthority::new(
            prefix.clone(),
            claim.outcome,
            evidence.clone(),
            authorization,
            signature.proof_bytes.to_vec(),
        )
        .and_then(ManualResolutionProofAuthority::verify)
        {
            return verified;
        }
    }
    panic!("no manual signature fixture verified for sequence {expected_next_seq}");
}

struct ManualSignatureCandidate {
    proof_bytes: PlainCanonicalJsonBytes,
}

fn manual_signature_candidates(
    expected_next_seq: u64,
    claim: &ManualResolutionAuthorizationClaim,
) -> Vec<ManualSignatureCandidate> {
    let _ = expected_next_seq;
    let policy = proof_manual_evidence_spec().authorization;
    let operator = policy.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("manual claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: policy.verifier_id.clone(),
        signing_scheme: policy.signing_scheme.clone(),
        claim: claim.clone(),
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                claim_digest.digest().as_bytes(),
            ))
            .expect("manual signature bytes"),
        }],
    };
    vec![ManualSignatureCandidate {
        proof_bytes: proof.canonical_json().expect("manual proof canonical json"),
    }]
}

fn sign_manual_claim_digest(digest: &[u8; 32]) -> Vec<u8> {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    let signing_key = k256::ecdsa::SigningKey::from(&secret_key);
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

fn manual_authorization_ref(proof_bytes: &[u8]) -> ManualResolutionEvidenceRef {
    let content_hash = PlainCanonicalJsonBytes::from_canonical_json_slice(proof_bytes)
        .expect("canonical proof bytes")
        .content_digest();
    ManualResolutionEvidenceRef {
        schema_id: manual_authorization_proof_schema_id().expect("manual authorization schema"),
        artifact_id: ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest()),
        content_hash,
    }
}

pub(super) fn manual_resolution_payload_from_verified(
    verified: &VerifiedManualResolutionForPrefix,
) -> KernelEventPayload {
    let claim = verified.claim();
    let authorization = verified.authorization();
    let artifacts = manual_resolution_artifacts_from_verified(verified);
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
        note: None,
    })
}

pub(super) fn manual_resolution_artifacts_from_verified(
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
            producer_seed_id: None::<SeedId>,
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
            producer_seed_id: None::<SeedId>,
            artifact_role: ArtifactRole::ManualResolutionAuthorization,
        },
    ]
}

pub(super) fn manual_resolution_request_from_verified(
    verified: &VerifiedManualResolutionForPrefix,
    expected_next_seq: StreamSeq,
    commit_key: &str,
    policy: SagaPolicySpec,
) -> CommitRequest {
    CommitRequest::from_payloads(
        verified.claim().run_id.clone(),
        expected_next_seq,
        CommitKey::new(commit_key).expect("commit key"),
        vec![manual_resolution_payload_from_verified(verified)],
        manual_resolution_artifacts_from_verified(verified),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            ..saga_preconditions(&verified.claim().run_id, policy)
        },
    )
    .expect("manual resolution request")
}

pub(super) fn prepared_manual_resolution_commit(
    verified: &VerifiedManualResolutionForPrefix,
    expected_next_seq: StreamSeq,
    commit_key: &str,
    policy: SagaPolicySpec,
) -> PreparedCommit<ManualResolution> {
    let request =
        manual_resolution_request_from_verified(verified, expected_next_seq, commit_key, policy);
    let artifacts = manual_resolution_artifacts_from_verified(verified);
    PreparedCommit::<ManualResolution>::new(
        request,
        CommitArtifactEvidenceSet::new(artifacts.clone(), artifacts)
            .expect("manual artifact evidence set"),
        verified,
    )
    .expect("proof-backed manual resolution prepared commit")
}
