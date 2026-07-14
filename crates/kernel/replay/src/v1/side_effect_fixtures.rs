use super::*;

pub(super) fn intent_persisted(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
) -> side_effect::IntentPersisted {
    side_effect::IntentPersisted {
        spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x01)),
        node_id: node_id(0x02),
        scope_id: mfm_ids::ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x03)),
        attempt_id: attempt_id(0x04),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.replay.test.intent", 0x05),
        intent_hash: content_digest(0x06),
        intent_artifact_id: artifact_id(0x07),
        intent_artifact_evidence_hash: content_digest(0x07),
        idempotency_input_schema_id: schema_id("mfm.replay.test.idempotency", 0x08),
        idempotency_input_hash: content_digest(0x09),
        idempotency_key: events::IdempotencyKeyRef::new("idem-key").expect("idempotency key"),
        capability_kind: CapabilityKind::new(
            "mfm.replay.test",
            "mutation",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0x0a),
        )
        .expect("capability kind"),
        capability_version: CapabilityVersion::new("mfm.replay.test.capability.v1")
            .expect("capability version"),
        adapter_kind: AdapterKind::new(
            "mfm.replay.test",
            "adapter",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0x0b),
        )
        .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.replay.test.adapter.v1")
            .expect("adapter version"),
    }
}

pub(super) fn resource_lane_released(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
    release_authority: events::ResourceLaneReleaseAuthority,
) -> KernelEventPayload {
    resource_lane_released_with_role(
        pair_id,
        ledger_key,
        events::SideEffectPairRole::Verify,
        release_authority,
    )
}

pub(super) fn resource_lane_released_with_role(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
    pair_role: events::SideEffectPairRole,
    release_authority: events::ResourceLaneReleaseAuthority,
) -> KernelEventPayload {
    KernelEventPayload::ResourceLaneReleased(events::ResourceLaneReleased {
        spec_hash: spec_hash(0x80),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role,
        invocation_epoch: 1,
        claim_id: events::ResourceLaneClaimId::new("mfm.replay.test.claim").expect("claim id"),
        release_id: events::ResourceLaneReleaseId::new("mfm.replay.test.release")
            .expect("release id"),
        claim_fencing_token: 1,
        release_authority,
        release_reason: events::ResourceLaneReleaseReason::new("side_effect.terminal")
            .expect("release reason"),
        lane_transition_seq: 2,
    })
}

pub(super) fn side_effect_confirmation(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectConfirmationObserved(side_effect::ConfirmationObserved {
        spec_hash: spec_hash(0x80),
        node_id: node_id(0x82),
        attempt_id: attempt_id(0x83),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        confirmation_schema_id: schema_id("mfm.replay.test.confirmation", 0x84),
        confirmation_hash: content_digest(0x85),
        confirmation_artifact_id: artifact_id(0x86),
        confirmation_artifact_evidence_hash: content_digest(0x87),
        replay_verifier_id: events::ReplayVerifierId::new("mfm.replay.test.confirmation.verifier")
            .expect("replay verifier id"),
        resource_touched_set: None,
    })
}

pub(super) fn side_effect_failed(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
    pair_role: events::SideEffectPairRole,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectFailed(side_effect::Failed {
        spec_hash: spec_hash(0x80),
        node_id: node_id(0x8e),
        attempt_id: attempt_id(0x8f),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role,
        invocation_epoch: 1,
        failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
        retryable: false,
        error: events::MfmErrorInfo::new(
            events::ErrorCode::new("mfm.replay.test.side_effect_failed").expect("error code"),
            events::ErrorCategory::SideEffect,
            false,
            "side-effect failed",
        )
        .expect("error info"),
    })
}

pub(super) fn side_effect_receipt(
    pair_id: SideEffectPairId,
    ledger_key: events::SideEffectLedgerKey,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectReceiptObserved(side_effect::ReceiptObserved {
        spec_hash: spec_hash(0x80),
        node_id: node_id(0x88),
        attempt_id: attempt_id(0x89),
        ledger_key,
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id,
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        receipt_schema_id: schema_id("mfm.replay.test.receipt", 0x8a),
        receipt_hash: content_digest(0x8b),
        receipt_artifact_id: artifact_id(0x8c),
        receipt_artifact_evidence_hash: content_digest(0x8d),
        replay_verifier_id: events::ReplayVerifierId::new("mfm.replay.test.receipt.verifier")
            .expect("replay verifier id"),
        resource_touched_set: None,
    })
}

pub(super) fn manual_resolution_recorded() -> KernelEventPayload {
    KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id: run_id(0x90),
        spec_hash: spec_hash(0x80),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence_schema_id: schema_id("mfm.replay.test.manual_evidence", 0x91),
        evidence_hash: content_digest(0x92),
        evidence_artifact_id: artifact_id(0x93),
        evidence_artifact_evidence_hash: content_digest(0x93),
        authorization_schema_id: schema_id("mfm.replay.test.manual_authorization", 0x94),
        authorization_hash: content_digest(0x95),
        authorization_artifact_id: artifact_id(0x96),
        authorization_artifact_evidence_hash: content_digest(0x96),
        note: None,
    })
}
