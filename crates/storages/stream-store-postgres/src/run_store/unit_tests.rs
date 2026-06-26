use super::*;
use mfm_ids::{AttemptId, DigestAlgorithm, DigestBytes, EventId, SideEffectPairId};
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
        pair_id: mfm_ids::SideEffectPairId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([7; 32]),
        ),
        pair_role: events::SideEffectPairRole::Submit,
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

#[test]
fn resource_lane_release_resolution_is_pair_bound_not_attempt_bound() {
    let run_id = RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([11; 32]),
    );
    let pair_id = SideEffectPairId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([12; 32]),
    );
    let resource_key = events::ResourceKeyEvidence {
        namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
        key_schema_id: SchemaId::new(
            "mfm.test.resource_key",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([13; 32]),
        )
        .expect("schema id"),
        key: events::ResourceKey::new("wallet-1").expect("resource key"),
    };
    let lane_key = ResourceLaneKey::from_evidence(&resource_key);
    let claim_id = events::ResourceLaneClaimId::new("mfm.test.claim.1").expect("claim id");
    let mut resource_lanes = BTreeMap::new();
    resource_lanes.insert(
        lane_key.clone(),
        ResourceLaneProjection {
            event_id: EventId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([14; 32]),
            ),
            holder: mfm_store::v1::SideEffectPairLedgerRef::new(run_id.clone(), pair_id.clone()),
            ledger_key: events::SideEffectLedgerKey::new("ledger-1").expect("ledger key"),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: pair_id.clone(),
            node_id: NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([15; 32]),
            ),
            attempt_id: AttemptId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([16; 32]),
            ),
            invocation_epoch: 1,
            claim_id: claim_id.clone(),
            claim_fencing_token: 1,
            lane_transition_seq: 1,
        },
    );
    let projections = ProjectionSnapshot::from_parts(ProjectionSnapshotParts {
        resource_lanes,
        ..ProjectionSnapshotParts::default()
    })
    .expect("projection snapshot");
    let intent = events::ResourceLaneReleaseIntent {
        spec_hash: mfm_ids::SpecHash::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([17; 32]),
        ),
        node_id: NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([18; 32]),
        ),
        attempt_id: AttemptId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([19; 32]),
        ),
        ledger_key: events::SideEffectLedgerKey::new("ledger-1").expect("ledger key"),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        claim_id,
        release_reason: events::ResourceLaneReleaseReason::new("mfm.test.release")
            .expect("release reason"),
    };

    let lane = resource_lane_for_release(&run_id, &projections, &intent)
        .expect("verify attempt releases pair-held lane");
    let expected_lane = mfm_store::v1::ResourceAdmissionLane::from_resource_lane_key(&lane_key)
        .expect("expected lane");
    assert_eq!(lane.erased_key(), expected_lane.erased_key());
}
