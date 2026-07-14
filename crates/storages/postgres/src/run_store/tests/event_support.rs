use super::*;

pub(super) fn prepared_artifact_bytes_from_bytes(
    bytes: Vec<u8>,
    role: ArtifactRole,
) -> PreparedArtifactBytes {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let mut evidence = store_artifact_ref(artifact_id, digest, role);
    evidence.byte_len = bytes.len() as u64;
    PreparedArtifactBytes::new(bytes, evidence).expect("prepared artifact bytes")
}

pub(super) fn run_admitted_with_saga_policy(
    run_id: RunId,
    saga_policy: &SagaPolicySpec,
) -> KernelEventPayload {
    let authority_spec = saga_authority_spec(saga_policy.clone());
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("saga authority spec hash");
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    let identity_material = run_identity_material_for_run_id(&run_id, saga_policy);
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: entry_point_launch_evidence(),
        spec_hash: certified_spec_hash,
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        saga_policy_digest: saga_policy
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(9),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        seed_cells: Vec::new(),
    }))
}

pub(super) fn run_admitted_with_fact_descriptor(run_id: RunId) -> KernelEventPayload {
    let authority_spec = fact_authority_spec();
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("fact authority spec hash");
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    let fact_descriptor_artifact = fact_descriptor_artifact_ref();
    let identity_material = run_identity_material_for_fact_run_id(&run_id);
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        identity_material,
        entry_point: entry_point_launch_evidence(),
        spec_hash: certified_spec_hash,
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
        fact_descriptor_artifacts: vec![run_artifact_ref(&fact_descriptor_artifact)],
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        saga_policy_digest: SagaPolicySpec::NoSideEffects
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(9),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        seed_cells: Vec::new(),
    }))
}

pub(super) fn entry_point_launch_evidence() -> events::EntryPointLaunchEvidence {
    events::EntryPointLaunchEvidence::new("mfm.test/portfolio_snapshot@1", Vec::new())
        .expect("entry-point evidence")
}

pub(super) fn state_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        attempt_no: 1,
        state_kind: state_kind(12),
        state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
    })
}

pub(super) fn state_attempt_completed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        output_cell_id: cell_id(21),
    })
}

pub(super) fn state_attempt_interrupted() -> KernelEventPayload {
    KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
    })
}

pub(super) fn cell_produced(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    let evidence = store_artifact_ref(
        artifact_id.clone(),
        digest.clone(),
        ArtifactRole::StateOutput,
    );
    KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        cell_id: cell_id(21),
        scope_id: scope_id(22),
        attempt_id: attempt_id(23),
        semantic_type_id: semantic_id("position", 24),
        schema_id: schema_id("mfm.test.position", 25),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(26),
        },
        context: spec::CellContextSpec::no_context(),
        artifact_id,
        content_digest: digest,
        evidence_hash: evidence.evidence_hash().expect("cell evidence hash"),
        producer_state_kind: None,
        producer_state_version: None,
    })
}

pub(super) fn run_completed(
    run_id: RunId,
    outcome: events::RunCompletionOutcome,
) -> KernelEventPayload {
    KernelEventPayload::RunCompleted(events::RunCompleted {
        run_id,
        spec_hash: spec_hash(1),
        outcome,
    })
}

pub(super) fn store_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
) -> ArtifactEvidenceRef {
    let (schema_id, semantic_type_id, producer_node_id) = match role {
        ArtifactRole::StateOutput => (
            Some(schema_id("mfm.test.position", 25)),
            Some(semantic_id("position", 24)),
            Some(node_id(20)),
        ),
        ArtifactRole::FactResponse => (
            Some(schema_id("mfm.test.fact_response", 36)),
            None,
            Some(node_id(30)),
        ),
        _ => (None, None, None),
    };
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id,
        semantic_type_id,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    }
}

pub(super) fn retention_refs_appended(
    run_id: RunId,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
) -> KernelEventPayload {
    retention_refs_appended_with_reason(
        run_id,
        artifact_id,
        digest,
        role,
        events::RetentionReason::RuntimeEvidence,
    )
}

pub(super) fn retention_refs_appended_with_reason(
    run_id: RunId,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
    reason: events::RetentionReason,
) -> KernelEventPayload {
    let evidence_hash = store_artifact_ref(artifact_id.clone(), digest.clone(), role)
        .evidence_hash()
        .expect("retention evidence hash");
    KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
        run_id,
        spec_hash: spec_hash(1),
        refs: vec![events::RetentionRef {
            artifact_id,
            role,
            evidence_hash,
            content_digest: digest,
        }],
        reason,
    })
}

pub(super) fn retention_refs_appended_for_evidence(
    run_id: RunId,
    evidence: &ArtifactEvidenceRef,
    reason: events::RetentionReason,
) -> KernelEventPayload {
    KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
        run_id,
        spec_hash: spec_hash(1),
        refs: vec![events::RetentionRef {
            artifact_id: evidence.artifact_id.clone(),
            role: evidence.artifact_role,
            evidence_hash: evidence.evidence_hash().expect("retention evidence hash"),
            content_digest: evidence.digest.clone(),
        }],
        reason,
    })
}

pub(super) fn side_effect_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    role: ArtifactRole,
) -> ArtifactEvidenceRef {
    let producer_node_id = match role {
        ArtifactRole::Receipt | ArtifactRole::Confirmation | ArtifactRole::AmbiguityEvidence => {
            verify_node_id()
        }
        _ => submit_node_id(),
    };
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id),
        producer_seed_id: None,
        artifact_role: role,
    }
}

pub(super) fn spec_artifact_ref() -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(2),
        digest: content_digest(2),
        byte_len: 128,
        media_type: media_type("application/vnd.mfm.typed-execution-spec+json;version=1"),
        schema_id: Some(spec::typed_execution_spec_schema_id().expect("typed spec schema")),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedExecutionSpec,
    }
}

pub(super) fn certificate_artifact_ref() -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(4),
        digest: content_digest(4),
        byte_len: 128,
        media_type: media_type("application/vnd.mfm.typed-spec-certificate+json;version=1"),
        schema_id: Some(
            mfm_certify::typed_spec_certificate_schema_id().expect("typed certificate schema"),
        ),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedSpecCertificate,
    }
}

pub(super) fn request(
    run_id: RunId,
    seq: u64,
    key: &str,
    payloads: Vec<KernelEventPayload>,
) -> mfm_store::v1::CommitRequest {
    mfm_store::v1::CommitRequest::from_payloads(
        run_id,
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(key).expect("commit key"),
        payloads,
        Vec::new(),
        CommitPreconditions::default(),
    )
    .expect("typed commit request")
}

pub(super) fn certified_request(
    run_id: RunId,
    seq: u64,
    key: &str,
    payloads: Vec<KernelEventPayload>,
) -> mfm_store::v1::CommitRequest {
    certified_request_with_saga_policy(run_id, seq, key, payloads, SagaPolicySpec::NoSideEffects)
}

pub(super) fn certified_request_with_saga_policy(
    run_id: RunId,
    seq: u64,
    key: &str,
    mut payloads: Vec<KernelEventPayload>,
    saga_policy: SagaPolicySpec,
) -> mfm_store::v1::CommitRequest {
    let preconditions = certify_payloads_for_policy(&run_id, saga_policy, &mut payloads);
    mfm_store::v1::CommitRequest::from_payloads(
        run_id,
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(key).expect("commit key"),
        payloads,
        Vec::new(),
        preconditions,
    )
    .expect("typed certified commit request")
}

pub(super) fn certified_fact_request(
    run_id: RunId,
    seq: u64,
    key: &str,
    mut payloads: Vec<KernelEventPayload>,
) -> mfm_store::v1::CommitRequest {
    let preconditions = certify_payloads_for_fact_spec(&run_id, &mut payloads);
    mfm_store::v1::CommitRequest::from_payloads(
        run_id,
        StreamSeq::new(seq).expect("seq"),
        CommitKey::new(key).expect("commit key"),
        payloads,
        Vec::new(),
        preconditions,
    )
    .expect("typed certified fact commit request")
}

pub(super) fn certify_payloads_for_policy(
    run_id: &RunId,
    saga_policy: SagaPolicySpec,
    payloads: &mut [KernelEventPayload],
) -> CommitPreconditions {
    let authority_spec = saga_authority_spec(saga_policy);
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("saga authority spec hash");
    for payload in payloads {
        set_payload_spec_hash(payload, &certified_spec_hash);
    }
    CommitPreconditions {
        certified_run_authority: Some(
            CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                .expect("certified run authority"),
        ),
        ..CommitPreconditions::default()
    }
}

pub(super) fn certify_payloads_for_fact_spec(
    run_id: &RunId,
    payloads: &mut [KernelEventPayload],
) -> CommitPreconditions {
    let authority_spec = fact_authority_spec();
    let certified_spec_hash = authority_spec
        .spec_hash()
        .expect("fact authority spec hash");
    for payload in payloads {
        set_payload_spec_hash(payload, &certified_spec_hash);
    }
    CommitPreconditions {
        certified_run_authority: Some(
            CertifiedRunStoreAuthority::from_spec(run_id.clone(), &authority_spec)
                .expect("certified run authority"),
        ),
        ..CommitPreconditions::default()
    }
}

pub(super) fn set_payload_spec_hash(payload: &mut KernelEventPayload, spec_hash: &SpecHash) {
    match payload {
        KernelEventPayload::RunAdmitted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::StateAttemptStarted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::FactRecorded(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ArtifactReferenced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::CellProduced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::CellSkipped(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectClaimed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::ResourceLaneClaimed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneClaimIntent(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectInvocationStarted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectReceiptObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::SideEffectAmbiguous(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::SideEffectFailed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneReleased(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ResourceLaneReleaseIntent(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::PublicOutputProduced(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::StateAttemptCompleted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::StateAttemptFailed(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
        KernelEventPayload::RunCompleted(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::RetentionRefsAppended(payload) => payload.spec_hash = spec_hash.clone(),
        KernelEventPayload::RetentionManifestProjected(payload) => {
            payload.spec_hash = spec_hash.clone();
        }
    }
}
