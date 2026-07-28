use super::*;

pub(in crate::tests::support) async fn append_manual_resolution<S>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    current: VerifiedCurrentRun,
    outcome: events::ManualResolutionOutcome,
) -> VerifiedCurrentRun
where
    S: store::RunJournalStore + ?Sized,
{
    let expected_next_sequence = current
        .lifecycle()
        .next_sequence()
        .expect("manual prefix next sequence");
    let mut prefix_record_count = 0;
    let _ = current.lifecycle().visit_records(|_| {
        prefix_record_count += 1;
        std::ops::ControlFlow::<()>::Continue(())
    });
    let request = manual_resolution_request_for_current(&current, outcome);
    let current = scheduler
        .record_manual_resolution(store, current, request)
        .await
        .expect("append manual resolution");
    let lifecycle = current.lifecycle();
    let mut manual_sequence = None;
    let mut manual_position = None;
    let _ = lifecycle.visit_records(|record| {
        if matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::ManualResolutionRecorded(_)
        ) {
            manual_sequence = Some(record.sequence());
            manual_position = Some(record.position());
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(
        manual_sequence,
        Some(expected_next_sequence),
        "manual resolution must remain adjacent to its authorized prefix"
    );
    assert_eq!(
        manual_position,
        Some(prefix_record_count),
        "manual resolution record must immediately follow its authorized prefix"
    );
    current
}

pub(in crate::tests::support) fn manual_resolution_request_for_current(
    current: &VerifiedCurrentRun,
    outcome: events::ManualResolutionOutcome,
) -> ManualResolutionRequest {
    let manual = match &current.runtime_spec().spec().saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual.as_ref().clone(),
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manual.as_ref().clone(),
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
    let prefix = current
        .lifecycle()
        .manual_resolution_prefix_authority()
        .expect("manual prefix authority");
    assert_eq!(
        prefix.expected_next_seq(),
        current
            .lifecycle()
            .next_sequence()
            .expect("manual prefix next sequence")
            .as_u64()
    );
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
    ManualResolutionRequest {
        outcome,
        evidence_artifact: ManualResolutionEvidenceArtifact {
            bytes: evidence_bytes,
            media_type: spec::MediaType::new("application/json").expect("media"),
        },
        proof_bytes,
        note: None,
    }
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
