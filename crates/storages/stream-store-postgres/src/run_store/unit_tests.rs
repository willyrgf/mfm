use super::*;
use mfm_ids::{DigestAlgorithm, DigestBytes};
use mfm_spec::v1::ResourceNamespace;

#[test]
fn artifact_role_contract_postgres_tag_roundtrip_uses_events_contract() {
    for role in events::ArtifactRole::ALL {
        assert_eq!(
            decode_artifact_role_tag(role.as_str()).expect("role tag parses"),
            *role
        );
    }

    assert!(matches!(
        decode_artifact_role_tag("unknown_artifact_role"),
        Err(PostgresStoreError::Store(StoreError::Identity(message)))
            if message.contains("unknown artifact role unknown_artifact_role")
    ));
}

#[test]
fn resource_wait_fifo_admission_token_is_stable_for_identical_claim_retries() {
    let run_id = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([1; 32]),
    );
    let evidence = events::ResourceKeyEvidence {
        namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
        key_schema_id: SchemaId::new(
            "mfm.test.resource_key",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([2; 32]),
        )
        .expect("schema id"),
        key: events::ResourceKey::new("wallet-1").expect("resource key"),
    };
    let lane = mfm_store::v1::ResourceAdmissionLane::from_resource_key_evidence(&evidence)
        .expect("lane id");
    let mut intent = events::ResourceLaneClaimIntent {
        spec_hash: mfm_ids::SpecHash::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([3; 32]),
        ),
        node_id: NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([4; 32]),
        ),
        attempt_id: mfm_ids::AttemptId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([5; 32]),
        ),
        ledger_key: events::SideEffectLedgerKey::new("ledger-1").expect("ledger key"),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        invocation_epoch: 1,
        resource_key: evidence,
        requirement_digest: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([6; 32]),
        ),
        resolved_by_capability_impl: events::RunnerFactoryId::new("mfm.test.runner")
            .expect("runner id"),
    };

    let first =
        mfm_store::v1::resource_wait_fifo_admission_token(&run_id, &lane, &intent).expect("first");
    let retry =
        mfm_store::v1::resource_wait_fifo_admission_token(&run_id, &lane, &intent).expect("retry");
    intent.invocation_epoch = 2;
    let different_epoch =
        mfm_store::v1::resource_wait_fifo_admission_token(&run_id, &lane, &intent)
            .expect("different");

    assert_eq!(first, retry);
    assert_ne!(first, different_epoch);
    assert_eq!(
        mfm_store::v1::admission_waiter_id(&first).expect("first waiter id"),
        mfm_store::v1::admission_waiter_id(&retry).expect("retry waiter id")
    );
}
