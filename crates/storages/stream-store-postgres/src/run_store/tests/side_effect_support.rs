use super::*;

pub(super) fn side_effect_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key")
}

pub(super) fn side_effect_pair_id() -> SideEffectPairId {
    side_effect_pair_id_for_contract(&default_side_effect_contract())
}

pub(super) fn side_effect_pair_id_for_contract(
    contract: &spec::SideEffectContractSpec,
) -> SideEffectPairId {
    spec::side_effect_pair_id(&submit_node_id(), &side_effect_output_cell(), contract)
        .expect("certified side-effect pair id")
}

pub(super) fn submit_node_id() -> NodeId {
    node_id(70)
}

pub(super) fn submit_attempt_id() -> AttemptId {
    attempt_id(72)
}

pub(super) fn verify_node_id() -> NodeId {
    node_id(170)
}

pub(super) fn verify_attempt_id() -> AttemptId {
    attempt_id(172)
}

pub(super) fn side_effect_output_cell() -> CellId {
    cell_id(78)
}

pub(super) fn default_side_effect_contract() -> spec::SideEffectContractSpec {
    spec::SideEffectContractSpec {
        contract_digest: content_digest(77),
        resource_claim: spec::ResourceClaimSpec::ManualOnly,
        verification: spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    }
}

pub(super) fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

pub(super) fn side_effect_attempt_started() -> KernelEventPayload {
    side_effect_attempt_started_for(submit_node_id(), submit_attempt_id())
}

pub(super) fn side_effect_verify_attempt_started() -> KernelEventPayload {
    side_effect_attempt_started_for(verify_node_id(), verify_attempt_id())
}

pub(super) fn side_effect_attempt_started_for(
    node_id: NodeId,
    attempt_id: AttemptId,
) -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        attempt_no: 1,
        state_kind: state_kind(70),
        state_version: StateVersion::new("mfm.test.side_effect_state.v1").expect("state version"),
    })
}

pub(super) fn side_effect_intent(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        schema_id("mfm.test.side_effect_intent", 70),
        ArtifactRole::SideEffectIntent,
    );
    KernelEventPayload::SideEffectIntentPersisted(events::side_effect::IntentPersisted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        scope_id: scope_id(71),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.test.side_effect_intent", 70),
        intent_hash: digest,
        intent_artifact_id: artifact_id,
        intent_artifact_evidence_hash: evidence.evidence_hash().expect("intent evidence hash"),
        idempotency_input_schema_id: schema_id("mfm.test.idempotency_input", 73),
        idempotency_input_hash: content_digest(74),
        idempotency_key: events::IdempotencyKeyRef::new("idem-key-1").expect("idempotency key"),
        capability_kind: CapabilityKind::new(
            "mfm.test",
            "side_effect",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(75),
        )
        .expect("capability kind"),
        capability_version: CapabilityVersion::new("mfm.test.side_effect.v1")
            .expect("capability version"),
        adapter_kind: AdapterKind::new(
            "mfm.test",
            "adapter",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(76),
        )
        .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
    })
}

pub(super) fn side_effect_claim() -> KernelEventPayload {
    KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        invocation_epoch: 1,
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
    })
}

pub(super) fn side_effect_prepared() -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationPrepared(events::side_effect::InvocationPrepared {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
        resource_key: None,
        prepared_artifact_id: None,
        prepared_hash: None,
        prepared_artifact_evidence_hash: None,
    })
}

pub(super) fn resource_namespace() -> ResourceNamespace {
    ResourceNamespace::new("mfm.test.account_nonce").expect("resource namespace")
}

pub(super) fn resource_key(value: &str, schema_byte: u8) -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace: resource_namespace(),
        key_schema_id: schema_id("mfm.test.resource_key", schema_byte),
        key: events::ResourceKey::new(value).expect("resource key"),
    }
}

pub(super) fn resource_lane_key(value: &str) -> ResourceLaneKey {
    ResourceLaneKey::from_evidence(&resource_key(value, 201))
}

pub(super) fn resource_lane_claim_intent(
    resource_key: events::ResourceKeyEvidence,
) -> KernelEventPayload {
    KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        resource_key,
        requirement_digest: content_digest(210),
        resolved_by_capability_impl: events::RunnerFactoryId::new("mfm.test.postgres.runner")
            .expect("runner factory"),
    })
}

pub(super) fn side_effect_started() -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationStarted(events::side_effect::InvocationStarted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        claim_generation: 1,
        claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1").expect("token"),
    })
}

pub(super) fn unknown_schema() -> SchemaId {
    schema_id("mfm.test.submission_unknown", 83)
}

pub(super) fn submission_schema() -> SchemaId {
    schema_id("mfm.test.submission", 77)
}

pub(super) fn side_effect_submission_unknown(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    KernelEventPayload::SideEffectSubmissionUnknown(events::side_effect::SubmissionUnknown {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        evidence_schema_id: unknown_schema(),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
        evidence_artifact_evidence_hash: evidence.evidence_hash().expect("unknown evidence hash"),
    })
}

pub(super) fn side_effect_submission_observed(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    KernelEventPayload::SideEffectSubmissionObserved(events::side_effect::SubmissionObserved {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        submission_schema_id: submission_schema(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
        submission_artifact_evidence_hash: evidence
            .evidence_hash()
            .expect("submission evidence hash"),
    })
}

pub(super) fn side_effect_ambiguous(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    let evidence = side_effect_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        schema_id("mfm.test.ambiguity", 84),
        ArtifactRole::AmbiguityEvidence,
    );
    KernelEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
        spec_hash: spec_hash(1),
        node_id: verify_node_id(),
        attempt_id: verify_attempt_id(),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        ambiguity_code: events::AmbiguityCode::new("ambiguous").expect("ambiguity code"),
        evidence_schema_id: schema_id("mfm.test.ambiguity", 84),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
        evidence_artifact_evidence_hash: evidence.evidence_hash().expect("ambiguity evidence hash"),
    })
}

pub(super) fn side_effect_attempt_failed() -> KernelEventPayload {
    side_effect_attempt_failed_for(verify_node_id(), verify_attempt_id())
}

pub(super) fn side_effect_submit_boundary_output_skipped() -> KernelEventPayload {
    KernelEventPayload::CellSkipped(events::CellSkipped {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        cell_id: side_effect_output_cell(),
        scope_id: scope_id(71),
        attempt_id: submit_attempt_id(),
        semantic_type_id: semantic_id("side_effect_output", 98),
        schema_id: schema_id("mfm.test.side_effect_output", 97),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(99),
        },
        context: spec::CellContextSpec::no_context(),
        skip_reason: events::SkipReason {
            code: events::ErrorCode::new("side_effect_submission_boundary")
                .expect("skip reason code"),
            safe_message: "side-effect submit boundary recorded; verification is delegated to the paired verify node".to_owned(),
        },
    })
}

pub(super) fn side_effect_submit_attempt_completed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: submit_node_id(),
        attempt_id: submit_attempt_id(),
        output_cell_id: side_effect_output_cell(),
    })
}

pub(super) fn side_effect_attempt_failed_for(
    node_id: NodeId,
    attempt_id: AttemptId,
) -> KernelEventPayload {
    KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        retryable: false,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("side_effect_ambiguous").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable: false,
            safe_message: "side-effect outcome is ambiguous".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

pub(super) fn side_effect_failed() -> KernelEventPayload {
    side_effect_failed_for(verify_node_id(), verify_attempt_id())
}

pub(super) fn side_effect_failed_for(node_id: NodeId, attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::SideEffectFailed(events::side_effect::Failed {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Verify,
        invocation_epoch: 1,
        failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
        retryable: false,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable: false,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}
